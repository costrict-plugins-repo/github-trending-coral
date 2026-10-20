//! v0.41 P7: End-to-end test for the sampling bridge + HostSamplingRunner.
//!
//! Validates the full pipeline:
//! HostSamplingRunner → BridgeTransport → SamplingBridge → mock host → response
//!
//! This test does NOT require a real Claude Code instance; it uses the
//! channel-based bridge to simulate the host side.

use coral_mcp::{BridgeTransport, SamplingBridge, SamplingExchange};
use coral_runner::{HostSamplingRunner, McpTransport, Prompt, Runner};
use std::sync::Arc;

/// Suppress progress output for clean test output.
fn quiet_env() {
    unsafe { std::env::set_var("CORAL_QUIET", "1") };
}

/// Full pipeline: runner sends a prompt, bridge relays to mock host,
/// mock host responds, runner gets the answer.
#[test]
fn e2e_host_sampling_via_bridge() {
    quiet_env();

    // Set up the bridge
    let bridge = SamplingBridge::new();
    bridge.set_client_supports_sampling(true);

    let (tx, rx) = std::sync::mpsc::channel::<SamplingExchange>();
    bridge.set_request_sender(tx);

    // Create the transport adapter and runner
    let transport = Arc::new(BridgeTransport::new(bridge));
    let runner = HostSamplingRunner::new(transport as Arc<dyn McpTransport>);

    // Mock host thread — simulates Claude Code responding to sampling requests
    let host_handle = std::thread::spawn(move || {
        let exchange = rx.recv().expect("should receive sampling exchange");

        // Verify the request shape
        let params = &exchange.request["params"];
        assert_eq!(params["messages"][0]["role"], "user");
        assert_eq!(
            params["messages"][0]["content"]["text"],
            "Explain the authentication module."
        );
        assert_eq!(params["systemPrompt"], "You are a Coral wiki writer.");
        assert_eq!(params["maxTokens"], 4096);

        // Send back a response
        exchange
            .response_tx
            .send(serde_json::json!({
                "jsonrpc": "2.0",
                "id": exchange.request["id"],
                "result": {
                    "role": "assistant",
                    "content": {
                        "type": "text",
                        "text": "# Authentication\n\nThe auth module uses JWT tokens..."
                    },
                    "model": "claude-sonnet-4-5-20260514",
                    "stopReason": "endTurn"
                }
            }))
            .expect("should send response");
    });

    // Execute the runner
    let prompt = Prompt {
        system: Some("You are a Coral wiki writer.".to_string()),
        user: "Explain the authentication module.".to_string(),
        model: Some("claude-sonnet-4-5".to_string()),
        cwd: None,
        timeout: None,
    };

    let output = runner.run(&prompt).expect("run should succeed");
    assert_eq!(
        output.stdout,
        "# Authentication\n\nThe auth module uses JWT tokens..."
    );
    assert!(output.stderr.is_empty());
    assert!(output.usage.is_none()); // Sampling doesn't report usage

    host_handle.join().expect("host thread should complete");
}

/// Streaming path falls back to non-streaming and emits a single chunk.
#[test]
fn e2e_host_sampling_streaming_fallback() {
    quiet_env();

    let bridge = SamplingBridge::new();
    bridge.set_client_supports_sampling(true);

    let (tx, rx) = std::sync::mpsc::channel::<SamplingExchange>();
    bridge.set_request_sender(tx);

    let transport = Arc::new(BridgeTransport::new(bridge));
    let runner = HostSamplingRunner::new(transport as Arc<dyn McpTransport>);

    let host_handle = std::thread::spawn(move || {
        let exchange = rx.recv().expect("should receive");
        exchange
            .response_tx
            .send(serde_json::json!({
                "jsonrpc": "2.0",
                "id": exchange.request["id"],
                "result": {
                    "role": "assistant",
                    "content": { "type": "text", "text": "streamed content" },
                    "model": "test",
                    "stopReason": "endTurn"
                }
            }))
            .expect("should send");
    });

    let prompt = Prompt {
        system: None,
        user: "test".to_string(),
        model: None,
        cwd: None,
        timeout: None,
    };

    let mut chunks = Vec::new();
    let output = runner
        .run_streaming(&prompt, &mut |chunk| chunks.push(chunk.to_string()))
        .expect("streaming should succeed");

    // Single chunk since streaming is a fallback
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0], "streamed content");
    assert_eq!(output.stdout, "streamed content");

    host_handle.join().expect("host complete");
}

/// When client doesn't support sampling, runner errors clearly.
#[test]
fn e2e_host_sampling_not_supported() {
    quiet_env();

    let bridge = SamplingBridge::new();
    // Don't set sampling support — bridge.set_client_supports_sampling(false) is default

    let (tx, _rx) = std::sync::mpsc::channel::<SamplingExchange>();
    bridge.set_request_sender(tx);

    let transport = Arc::new(BridgeTransport::new(bridge));
    let runner = HostSamplingRunner::new(transport as Arc<dyn McpTransport>);

    let prompt = Prompt {
        system: None,
        user: "test".to_string(),
        model: None,
        cwd: None,
        timeout: None,
    };

    let err = runner.run(&prompt).unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("does not support sampling") || msg.contains("MCP transport error"),
        "unexpected error: {msg}"
    );
}

/// User rejection (code -1) maps to AuthFailed.
#[test]
fn e2e_host_sampling_user_rejected() {
    quiet_env();

    let bridge = SamplingBridge::new();
    bridge.set_client_supports_sampling(true);

    let (tx, rx) = std::sync::mpsc::channel::<SamplingExchange>();
    bridge.set_request_sender(tx);

    let transport = Arc::new(BridgeTransport::new(bridge));
    let runner = HostSamplingRunner::new(transport as Arc<dyn McpTransport>);

    let host_handle = std::thread::spawn(move || {
        let exchange = rx.recv().expect("should receive");
        exchange
            .response_tx
            .send(serde_json::json!({
                "jsonrpc": "2.0",
                "id": exchange.request["id"],
                "error": { "code": -1, "message": "User declined the sampling request" }
            }))
            .expect("should send");
    });

    let prompt = Prompt {
        system: None,
        user: "test".to_string(),
        model: None,
        cwd: None,
        timeout: None,
    };

    let err = runner.run(&prompt).unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("not authenticated") || msg.contains("user declined"),
        "unexpected error: {msg}"
    );

    host_handle.join().expect("host complete");
}

/// Model preferences are forwarded correctly.
#[test]
fn e2e_host_sampling_with_preferences() {
    quiet_env();

    let bridge = SamplingBridge::new();
    bridge.set_client_supports_sampling(true);

    let (tx, rx) = std::sync::mpsc::channel::<SamplingExchange>();
    bridge.set_request_sender(tx);

    let transport = Arc::new(BridgeTransport::new(bridge));
    let runner = HostSamplingRunner::new(transport as Arc<dyn McpTransport>)
        .with_preferences(coral_runner::ModelPreferences {
            hints: Some(vec![coral_runner::ModelHint {
                name: "claude-opus-4-6".to_string(),
            }]),
            intelligence_priority: Some(1.0),
            speed_priority: Some(0.2),
            cost_priority: Some(0.1),
        })
        .with_max_tokens(8192);

    let host_handle = std::thread::spawn(move || {
        let exchange = rx.recv().expect("should receive");
        let params = &exchange.request["params"];

        // Verify preferences were forwarded
        assert_eq!(
            params["modelPreferences"]["hints"][0]["name"],
            "claude-opus-4-6"
        );
        assert_eq!(params["modelPreferences"]["intelligencePriority"], 1.0);
        assert_eq!(params["maxTokens"], 8192);

        exchange
            .response_tx
            .send(serde_json::json!({
                "jsonrpc": "2.0",
                "id": exchange.request["id"],
                "result": {
                    "role": "assistant",
                    "content": { "type": "text", "text": "opus response" },
                    "model": "claude-opus-4-6-20260601",
                    "stopReason": "endTurn"
                }
            }))
            .expect("should send");
    });

    let prompt = Prompt {
        system: None,
        user: "complex task".to_string(),
        model: Some("ignored".to_string()), // Explicit prefs override prompt.model
        cwd: None,
        timeout: None,
    };

    let output = runner.run(&prompt).expect("should succeed");
    assert_eq!(output.stdout, "opus response");

    host_handle.join().expect("host complete");
}
