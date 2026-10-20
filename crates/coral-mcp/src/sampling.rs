//! v0.41 P4: MCP sampling bridge.
//!
//! Enables `coral-mcp` (the MCP server) to send `sampling/createMessage`
//! requests back to the host client (Claude Code) and receive responses.
//!
//! # How it works
//!
//! In MCP, sampling is a **server-to-client request** — the server asks
//! the client to perform LLM inference. The flow is:
//!
//! 1. During `initialize`, the client MAY advertise `"sampling": {}` in
//!    its capabilities.
//! 2. If it does, the server can send `sampling/createMessage` JSON-RPC
//!    requests to the client via the transport's output stream (stdout
//!    for stdio transport).
//! 3. The client processes the request and sends back a JSON-RPC response
//!    on the same stream the server reads from (stdin for stdio transport).
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────┐         stdio stdout         ┌──────────────────┐
//! │  coral-mcp      │  ──── request ──────────►   │  Claude Code     │
//! │  (MCP server)   │  ◄─── response ─────────    │  (MCP client)    │
//! │                 │         stdio stdin          │                  │
//! └─────────────────┘                              └──────────────────┘
//! ```
//!
//! The `SamplingBridge` holds:
//! - A flag indicating whether the client supports sampling
//! - A channel to send requests to the transport layer
//! - A channel to receive responses from the transport layer
//!
//! The stdio loop in `transport/stdio.rs` is modified to recognize
//! incoming lines that are responses (have `id` matching a pending
//! sampling request) and route them to the bridge instead of the
//! request dispatcher.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Tracks whether the connected MCP client supports sampling.
///
/// Shared between the `McpHandler` (which parses `initialize` params)
/// and the `HostSamplingRunner` (which needs to know if it can send
/// sampling requests).
#[derive(Debug)]
pub struct SamplingBridge {
    /// Whether the client advertised `"sampling": {...}` in its capabilities.
    client_supports_sampling: AtomicBool,
    /// Channel for sending sampling requests to the transport layer.
    /// The transport writes these to stdout and routes the response back.
    request_tx: Mutex<Option<std::sync::mpsc::Sender<SamplingExchange>>>,
    /// Next request ID for sampling requests (avoids collision with
    /// server-originated responses by using a high starting value).
    next_id: AtomicU64,
}

/// A pending sampling exchange: the request to send and a channel for the response.
#[derive(Debug)]
pub struct SamplingExchange {
    /// The JSON-RPC request body to send to the client.
    pub request: serde_json::Value,
    /// Channel to deliver the client's response back to the caller.
    pub response_tx: std::sync::mpsc::Sender<serde_json::Value>,
}

impl SamplingBridge {
    /// Create a new bridge. Initially, sampling is not supported until
    /// the client's `initialize` params are parsed.
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            client_supports_sampling: AtomicBool::new(false),
            request_tx: Mutex::new(None),
            next_id: AtomicU64::new(900_000), // High range to avoid ID collision
        })
    }

    /// Called by the transport layer to register the request channel.
    pub fn set_request_sender(&self, tx: std::sync::mpsc::Sender<SamplingExchange>) {
        *self.request_tx.lock().unwrap_or_else(|p| p.into_inner()) = Some(tx);
    }

    /// Called during `initialize` to record client capabilities.
    pub fn set_client_supports_sampling(&self, supports: bool) {
        self.client_supports_sampling
            .store(supports, Ordering::Release);
    }

    /// Returns `true` if the client advertised sampling capability.
    pub fn client_supports_sampling(&self) -> bool {
        self.client_supports_sampling.load(Ordering::Acquire)
    }

    /// Send a `sampling/createMessage` request to the client and wait for
    /// the response. Returns an error if:
    /// - The client doesn't support sampling
    /// - The transport channel is not connected
    /// - The response channel was dropped (transport died)
    pub fn send_sampling_request(
        &self,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        if !self.client_supports_sampling() {
            return Err("MCP client does not support sampling/createMessage. \
                 The client did not advertise 'sampling' capability during initialize. \
                 This likely means Claude Code has not yet shipped this feature \
                 (see anthropics/claude-code#1785)."
                .to_string());
        }

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "sampling/createMessage",
            "params": params,
        });

        let (response_tx, response_rx) = std::sync::mpsc::channel();
        let exchange = SamplingExchange {
            request,
            response_tx,
        };

        // Send to transport layer
        let tx_guard = self.request_tx.lock().unwrap_or_else(|p| p.into_inner());
        let tx = tx_guard.as_ref().ok_or_else(|| {
            "Sampling bridge not connected to transport. \
             This is an internal error — the MCP server should register \
             the sampling channel during startup."
                .to_string()
        })?;
        tx.send(exchange)
            .map_err(|_| "Transport channel closed — MCP server may have shut down.".to_string())?;
        drop(tx_guard);

        // Wait for response with a 5-minute ceiling to prevent indefinite hangs
        // (e.g., if the host process is SIGKILL'd without closing the channel).
        let timeout = std::time::Duration::from_secs(300);
        response_rx.recv_timeout(timeout).map_err(|e| match e {
            std::sync::mpsc::RecvTimeoutError::Timeout => {
                "Sampling request timed out after 5 minutes — the MCP host did not respond. \
                 The client may be stuck or disconnected."
                    .to_string()
            }
            std::sync::mpsc::RecvTimeoutError::Disconnected => {
                "Response channel closed — the MCP transport did not deliver a response. \
                 The client may have disconnected or timed out."
                    .to_string()
            }
        })
    }
}

impl Default for SamplingBridge {
    fn default() -> Self {
        Self {
            client_supports_sampling: AtomicBool::new(false),
            request_tx: Mutex::new(None),
            next_id: AtomicU64::new(900_000),
        }
    }
}

/// Adapter that implements `coral_runner::McpTransport` by delegating
/// to a `SamplingBridge`. This is how the `HostSamplingRunner` connects
/// to the MCP server's transport.
pub struct BridgeTransport {
    bridge: Arc<SamplingBridge>,
}

impl BridgeTransport {
    pub fn new(bridge: Arc<SamplingBridge>) -> Self {
        Self { bridge }
    }
}

impl coral_runner::McpTransport for BridgeTransport {
    fn request(&self, body: &serde_json::Value) -> Result<serde_json::Value, String> {
        // Extract params from the body — the bridge adds its own envelope
        let params = body
            .get("params")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        self.bridge.send_sampling_request(params)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bridge_initially_does_not_support_sampling() {
        let bridge = SamplingBridge::new();
        assert!(!bridge.client_supports_sampling());
    }

    #[test]
    fn bridge_supports_sampling_after_set() {
        let bridge = SamplingBridge::new();
        bridge.set_client_supports_sampling(true);
        assert!(bridge.client_supports_sampling());
    }

    #[test]
    fn bridge_rejects_request_when_not_supported() {
        let bridge = SamplingBridge::new();
        let result = bridge.send_sampling_request(serde_json::json!({}));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("does not support sampling"));
    }

    #[test]
    fn bridge_rejects_request_when_no_transport() {
        let bridge = SamplingBridge::new();
        bridge.set_client_supports_sampling(true);
        let result = bridge.send_sampling_request(serde_json::json!({}));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not connected to transport"));
    }

    #[test]
    fn bridge_round_trip_via_channel() {
        let bridge = SamplingBridge::new();
        bridge.set_client_supports_sampling(true);

        let (tx, rx) = std::sync::mpsc::channel::<SamplingExchange>();
        bridge.set_request_sender(tx);

        // Spawn a mock host that receives requests and sends responses
        let handle = std::thread::spawn(move || {
            let exchange = rx.recv().expect("should receive exchange");
            assert_eq!(exchange.request["method"], "sampling/createMessage");
            exchange
                .response_tx
                .send(serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": exchange.request["id"],
                    "result": {
                        "role": "assistant",
                        "content": { "type": "text", "text": "Hello from mock host!" },
                        "model": "test-model",
                        "stopReason": "endTurn"
                    }
                }))
                .expect("should send response");
        });

        let response = bridge
            .send_sampling_request(serde_json::json!({
                "messages": [{"role": "user", "content": {"type": "text", "text": "hi"}}],
                "maxTokens": 100
            }))
            .expect("should get response");

        assert_eq!(
            response["result"]["content"]["text"],
            "Hello from mock host!"
        );
        handle.join().expect("mock thread should complete");
    }

    #[test]
    fn bridge_transport_adapter_works() {
        use coral_runner::McpTransport;

        let bridge = SamplingBridge::new();
        bridge.set_client_supports_sampling(true);

        let (tx, rx) = std::sync::mpsc::channel::<SamplingExchange>();
        bridge.set_request_sender(tx);

        let adapter = BridgeTransport::new(bridge);

        let handle = std::thread::spawn(move || {
            let exchange = rx.recv().expect("should receive");
            exchange
                .response_tx
                .send(serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": 900000,
                    "result": {
                        "role": "assistant",
                        "content": { "type": "text", "text": "adapter works" },
                        "model": "test",
                        "stopReason": "endTurn"
                    }
                }))
                .expect("should send");
        });

        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "sampling/createMessage",
            "params": {
                "messages": [{"role": "user", "content": {"type": "text", "text": "test"}}],
                "maxTokens": 50
            }
        });

        let response = adapter.request(&body).expect("should succeed");
        assert_eq!(response["result"]["content"]["text"], "adapter works");
        handle.join().expect("mock complete");
    }
}
