//! v0.41 P3: `HostSamplingRunner` — uses MCP `sampling/createMessage` to
//! leverage the active Claude Code session as the LLM inference engine.
//!
//! # Architecture
//!
//! When Coral runs inside Claude Code (detected via `CLAUDECODE=1`), and
//! the MCP host advertises `sampling` capability during the initialize
//! handshake, this runner sends `sampling/createMessage` requests back to
//! the host instead of spawning the `claude` CLI binary.
//!
//! This eliminates the macOS Endpoint Security host-managed-mode wall
//! (BACKLOG #12 L3) by never spawning the `claude` binary at all — the
//! host session handles inference directly.
//!
//! # Status
//!
//! **Feature-gated** (`host-sampling`). As of 2026-05-16, Claude Code does
//! NOT support `sampling/createMessage` (see PRD Anexo C, GH issue
//! anthropics/claude-code#1785). This module is ready to activate once
//! Anthropic ships the capability.
//!
//! # Wire protocol (MCP 2025-11-25)
//!
//! Request (server → client):
//! ```json
//! {
//!   "jsonrpc": "2.0",
//!   "id": 1,
//!   "method": "sampling/createMessage",
//!   "params": {
//!     "messages": [{ "role": "user", "content": { "type": "text", "text": "..." } }],
//!     "systemPrompt": "...",
//!     "maxTokens": 4096,
//!     "modelPreferences": {
//!       "hints": [{ "name": "claude-sonnet-4-5" }],
//!       "intelligencePriority": 0.8,
//!       "speedPriority": 0.5,
//!       "costPriority": 0.3
//!     }
//!   }
//! }
//! ```
//!
//! Response (client → server):
//! ```json
//! {
//!   "jsonrpc": "2.0",
//!   "id": 1,
//!   "result": {
//!     "role": "assistant",
//!     "content": { "type": "text", "text": "..." },
//!     "model": "claude-sonnet-4-5-20260514",
//!     "stopReason": "endTurn"
//!   }
//! }
//! ```

use std::io::{BufRead, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::runner::{Prompt, RunOutput, Runner, RunnerError, RunnerResult};

/// JSON-RPC request envelope for outgoing sampling requests.
#[derive(Debug, Serialize)]
struct SamplingRequest {
    jsonrpc: &'static str,
    id: u64,
    method: &'static str,
    params: SamplingParams,
}

/// Parameters for `sampling/createMessage`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SamplingParams {
    messages: Vec<SamplingMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system_prompt: Option<String>,
    max_tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    model_preferences: Option<ModelPreferences>,
}

/// A single message in the sampling request.
#[derive(Debug, Serialize)]
struct SamplingMessage {
    role: &'static str,
    content: MessageContent,
}

/// Content block — text only for now (images are future work).
#[derive(Debug, Serialize)]
struct MessageContent {
    #[serde(rename = "type")]
    content_type: &'static str,
    text: String,
}

/// Model preferences hint for the host to select a model.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelPreferences {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hints: Option<Vec<ModelHint>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intelligence_priority: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speed_priority: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_priority: Option<f64>,
}

/// A model hint — the `name` field is a model identifier like "claude-sonnet-4-5".
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelHint {
    pub name: String,
}

/// JSON-RPC response envelope for incoming sampling responses.
#[derive(Debug, Deserialize)]
struct JsonRpcResponse {
    #[allow(dead_code)]
    jsonrpc: String,
    #[allow(dead_code)]
    id: Option<serde_json::Value>,
    result: Option<SamplingResult>,
    error: Option<JsonRpcError>,
}

/// Result of a successful `sampling/createMessage`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SamplingResult {
    #[allow(dead_code)]
    role: String,
    content: SamplingContent,
    #[allow(dead_code)]
    model: Option<String>,
    #[allow(dead_code)]
    stop_reason: Option<String>,
}

/// Content from the sampling response.
#[derive(Debug, Deserialize)]
struct SamplingContent {
    #[allow(dead_code)]
    #[serde(rename = "type")]
    content_type: Option<String>,
    text: Option<String>,
}

/// JSON-RPC error from a rejected sampling request.
#[derive(Debug, Deserialize)]
struct JsonRpcError {
    code: i64,
    message: String,
}

/// Handle to the MCP transport (stdin/stdout of the MCP connection).
///
/// The `HostSamplingRunner` sends JSON-RPC requests via this handle and
/// reads responses back. In production, this is wired to the coral-mcp
/// server's stdio transport (which relays to the host client). In tests,
/// it's a mock.
pub trait McpTransport: Send + Sync {
    /// Send a JSON-RPC request line and receive the response.
    /// The implementation handles newline framing and blocking I/O.
    fn request(&self, body: &serde_json::Value) -> Result<serde_json::Value, String>;
}

/// Stdio-based transport that writes to a process's stdin and reads from stdout.
/// Used when coral-mcp acts as the relay between the runner and the MCP host.
pub struct StdioTransport {
    writer: Mutex<Box<dyn Write + Send>>,
    reader: Mutex<Box<dyn BufRead + Send>>,
}

impl StdioTransport {
    pub fn new(writer: Box<dyn Write + Send>, reader: Box<dyn BufRead + Send>) -> Self {
        Self {
            writer: Mutex::new(writer),
            reader: Mutex::new(reader),
        }
    }
}

impl McpTransport for StdioTransport {
    fn request(&self, body: &serde_json::Value) -> Result<serde_json::Value, String> {
        let line = serde_json::to_string(body).map_err(|e| format!("serialize: {e}"))?;

        // Write request
        {
            let mut w = self.writer.lock().unwrap_or_else(|p| p.into_inner());
            writeln!(w, "{line}").map_err(|e| format!("write: {e}"))?;
            w.flush().map_err(|e| format!("flush: {e}"))?;
        }

        // Read response
        let mut response_line = String::new();
        {
            let mut r = self.reader.lock().unwrap_or_else(|p| p.into_inner());
            r.read_line(&mut response_line)
                .map_err(|e| format!("read: {e}"))?;
        }

        serde_json::from_str(response_line.trim()).map_err(|e| format!("parse response: {e}"))
    }
}

/// In-process channel transport for testing — pairs with a mock host.
pub struct ChannelTransport {
    tx: std::sync::mpsc::Sender<serde_json::Value>,
    rx: Mutex<std::sync::mpsc::Receiver<serde_json::Value>>,
}

impl ChannelTransport {
    /// Create a paired (transport, mock_endpoint) for tests.
    /// The mock_endpoint receives requests and must send responses.
    pub fn pair() -> (Self, ChannelMockEndpoint) {
        let (req_tx, req_rx) = std::sync::mpsc::channel();
        let (resp_tx, resp_rx) = std::sync::mpsc::channel();
        let transport = Self {
            tx: req_tx,
            rx: Mutex::new(resp_rx),
        };
        let endpoint = ChannelMockEndpoint {
            rx: Mutex::new(req_rx),
            tx: resp_tx,
        };
        (transport, endpoint)
    }
}

impl McpTransport for ChannelTransport {
    fn request(&self, body: &serde_json::Value) -> Result<serde_json::Value, String> {
        self.tx
            .send(body.clone())
            .map_err(|e| format!("channel send: {e}"))?;
        let rx = self.rx.lock().unwrap_or_else(|p| p.into_inner());
        rx.recv().map_err(|e| format!("channel recv: {e}"))
    }
}

/// The mock-host side of a [`ChannelTransport`].
/// Receives sampling requests and sends back responses.
pub struct ChannelMockEndpoint {
    rx: Mutex<std::sync::mpsc::Receiver<serde_json::Value>>,
    tx: std::sync::mpsc::Sender<serde_json::Value>,
}

impl ChannelMockEndpoint {
    /// Receive the next request from the runner.
    pub fn recv_request(&self) -> Option<serde_json::Value> {
        let rx = self.rx.lock().unwrap_or_else(|p| p.into_inner());
        rx.recv().ok()
    }

    /// Send a response back to the runner.
    pub fn send_response(&self, resp: serde_json::Value) -> Result<(), String> {
        self.tx.send(resp).map_err(|e| format!("mock send: {e}"))
    }
}

/// Runner that uses MCP `sampling/createMessage` to leverage the host
/// Claude Code session for LLM inference.
///
/// # Detection
///
/// In `make_runner` dispatch:
/// 1. If `CORAL_RUNNER=host-sampling` is set → use `HostSamplingRunner`.
/// 2. If `CLAUDECODE=1` AND the MCP transport is available with sampling
///    capability → default to `HostSamplingRunner`.
/// 3. Otherwise → fallback to provider-configured runner.
pub struct HostSamplingRunner {
    /// MCP transport for sending sampling requests to the host.
    transport: Arc<dyn McpTransport>,
    /// Model preferences forwarded in every sampling request.
    preferences: Option<ModelPreferences>,
    /// Max tokens per request (default: 4096).
    max_tokens: u64,
    /// Monotonic request ID counter.
    next_id: AtomicU64,
}

impl HostSamplingRunner {
    /// Create a new runner with the given MCP transport handle.
    pub fn new(transport: Arc<dyn McpTransport>) -> Self {
        Self {
            transport,
            preferences: None,
            max_tokens: 4096,
            next_id: AtomicU64::new(1),
        }
    }

    /// Set model preferences for all requests.
    pub fn with_preferences(mut self, prefs: ModelPreferences) -> Self {
        self.preferences = Some(prefs);
        self
    }

    /// Override the default max tokens (4096).
    pub fn with_max_tokens(mut self, max_tokens: u64) -> Self {
        self.max_tokens = max_tokens;
        self
    }

    /// Build the JSON-RPC request body for a `sampling/createMessage` call.
    fn build_request(&self, prompt: &Prompt) -> serde_json::Value {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);

        let mut messages = Vec::new();
        messages.push(SamplingMessage {
            role: "user",
            content: MessageContent {
                content_type: "text",
                text: prompt.user.clone(),
            },
        });

        let model_preferences = self.preferences.clone().or_else(|| {
            prompt.model.as_ref().map(|m| ModelPreferences {
                hints: Some(vec![ModelHint { name: m.clone() }]),
                intelligence_priority: None,
                speed_priority: None,
                cost_priority: None,
            })
        });

        let params = SamplingParams {
            messages,
            system_prompt: prompt.system.clone(),
            max_tokens: self.max_tokens,
            model_preferences,
        };

        let request = SamplingRequest {
            jsonrpc: "2.0",
            id,
            method: "sampling/createMessage",
            params,
        };

        // Infallible: SamplingRequest contains only primitive Serialize types.
        serde_json::to_value(request).expect(
            "SamplingRequest serialization is infallible — all fields are String/u64/Option",
        )
    }

    /// Parse the JSON-RPC response into a RunOutput.
    fn parse_response(
        &self,
        response: serde_json::Value,
        duration: Duration,
    ) -> RunnerResult<RunOutput> {
        let parsed: JsonRpcResponse =
            serde_json::from_value(response).map_err(|e| RunnerError::NonZeroExit {
                code: None,
                stderr: format!("failed to parse sampling response: {e}"),
            })?;

        // Check for JSON-RPC error
        if let Some(err) = parsed.error {
            // Code -1 means user rejected the sampling request
            if err.code == -1 {
                return Err(RunnerError::AuthFailed(format!(
                    "Host rejected sampling request (user declined): {}",
                    err.message
                )));
            }
            return Err(RunnerError::NonZeroExit {
                code: Some(err.code as i32),
                stderr: format!("sampling/createMessage error: {}", err.message),
            });
        }

        // Extract the text from the result
        let result = parsed.result.ok_or_else(|| RunnerError::NonZeroExit {
            code: None,
            stderr: "sampling response has neither result nor error".to_string(),
        })?;

        let text = result.content.text.unwrap_or_default();

        // Token usage from sampling responses is not reliably available
        // in the MCP spec (optional). We return None and let callers
        // fall back to heuristic cost estimation.
        Ok(RunOutput {
            stdout: text,
            stderr: String::new(),
            duration,
            usage: None,
        })
    }
}

impl std::fmt::Debug for HostSamplingRunner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostSamplingRunner")
            .field("max_tokens", &self.max_tokens)
            .field("preferences", &self.preferences)
            .finish_non_exhaustive()
    }
}

impl Runner for HostSamplingRunner {
    fn run(&self, prompt: &Prompt) -> RunnerResult<RunOutput> {
        // v0.41 P1: progress before sending.
        coral_core::progress!(step, "Sending sampling/createMessage to host..."; max_tokens = self.max_tokens);

        let start = Instant::now();
        let request = self.build_request(prompt);

        // v0.41 P2: verbose dump.
        if std::env::var("CORAL_VERBOSE").as_deref() == Ok("1")
            && coral_core::observability::progress_enabled()
        {
            let truncated = coral_core::observability::truncate_for_display(&prompt.user, 2000);
            coral_core::progress!(step, "Prompt (user)"; chars = prompt.user.len());
            eprintln!("  {truncated}");
        }

        // Send via MCP transport
        let response = self
            .transport
            .request(&request)
            .map_err(|e| RunnerError::NonZeroExit {
                code: None,
                stderr: format!("MCP transport error: {e}"),
            })?;

        let duration = start.elapsed();

        // v0.41 P2: verbose response dump.
        let result = self.parse_response(response, duration)?;
        if std::env::var("CORAL_VERBOSE").as_deref() == Ok("1")
            && coral_core::observability::progress_enabled()
        {
            let truncated = coral_core::observability::truncate_for_display(&result.stdout, 2000);
            coral_core::progress!(step, "Response"; chars = result.stdout.len());
            eprintln!("  {truncated}");
        }

        coral_core::progress!(done, "host_sampling", "Got response from host"; elapsed_ms = duration.as_millis());

        Ok(result)
    }

    fn run_streaming(
        &self,
        prompt: &Prompt,
        on_chunk: &mut dyn FnMut(&str),
    ) -> RunnerResult<RunOutput> {
        // MCP sampling doesn't natively support streaming via the
        // standard createMessage request. We fall back to the non-streaming
        // path and emit the full response as a single chunk.
        // When/if MCP adds streaming sampling (via notifications/progress),
        // this will be upgraded in P5.
        coral_core::progress!(
            step,
            "Sending sampling/createMessage (non-streaming fallback)..."
        );
        let result = self.run(prompt)?;
        on_chunk(&result.stdout);
        Ok(result)
    }
}

/// Check if the MCP host-sampling environment is available.
///
/// Returns `true` when:
/// 1. `CORAL_RUNNER=host-sampling` is explicitly set, OR
/// 2. `CLAUDECODE=1` is set (we're inside Claude Code)
///
/// Note: this does NOT verify that the host actually supports sampling
/// (that requires the initialize handshake). It only detects the
/// environment where host-sampling SHOULD be attempted. The actual
/// capability check happens at transport setup time.
pub fn host_sampling_available() -> bool {
    if std::env::var("CORAL_RUNNER").as_deref() == Ok("host-sampling") {
        return true;
    }
    std::env::var("CLAUDECODE").as_deref() == Ok("1")
}

/// Check if host-sampling is explicitly requested via env var.
pub fn host_sampling_explicit() -> bool {
    std::env::var("CORAL_RUNNER").as_deref() == Ok("host-sampling")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_request_has_correct_structure() {
        let (transport, _mock) = ChannelTransport::pair();
        let runner = HostSamplingRunner::new(Arc::new(transport));

        let prompt = Prompt {
            system: Some("You are a wiki writer.".to_string()),
            user: "Generate a page about auth.".to_string(),
            model: Some("claude-sonnet-4-5".to_string()),
            cwd: None,
            timeout: None,
        };

        let req = runner.build_request(&prompt);
        assert_eq!(req["method"], "sampling/createMessage");
        assert_eq!(req["jsonrpc"], "2.0");
        assert_eq!(req["params"]["messages"][0]["role"], "user");
        assert_eq!(
            req["params"]["messages"][0]["content"]["text"],
            "Generate a page about auth."
        );
        assert_eq!(req["params"]["systemPrompt"], "You are a wiki writer.");
        assert_eq!(req["params"]["maxTokens"], 4096);
        assert_eq!(
            req["params"]["modelPreferences"]["hints"][0]["name"],
            "claude-sonnet-4-5"
        );
    }

    #[test]
    fn build_request_no_system_no_model() {
        let (transport, _mock) = ChannelTransport::pair();
        let runner = HostSamplingRunner::new(Arc::new(transport));

        let prompt = Prompt {
            system: None,
            user: "hello".to_string(),
            model: None,
            cwd: None,
            timeout: None,
        };

        let req = runner.build_request(&prompt);
        assert!(req["params"]["systemPrompt"].is_null());
        assert!(req["params"]["modelPreferences"].is_null());
    }

    #[test]
    fn build_request_with_explicit_preferences() {
        let (transport, _mock) = ChannelTransport::pair();
        let runner =
            HostSamplingRunner::new(Arc::new(transport)).with_preferences(ModelPreferences {
                hints: Some(vec![ModelHint {
                    name: "claude-opus-4-6".to_string(),
                }]),
                intelligence_priority: Some(1.0),
                speed_priority: Some(0.2),
                cost_priority: Some(0.1),
            });

        let prompt = Prompt {
            system: None,
            user: "test".to_string(),
            model: Some("ignored-when-prefs-set".to_string()),
            cwd: None,
            timeout: None,
        };

        let req = runner.build_request(&prompt);
        // Explicit preferences win over prompt.model
        assert_eq!(
            req["params"]["modelPreferences"]["hints"][0]["name"],
            "claude-opus-4-6"
        );
        assert_eq!(
            req["params"]["modelPreferences"]["intelligencePriority"],
            1.0
        );
    }

    #[test]
    fn parse_response_success() {
        let (transport, _mock) = ChannelTransport::pair();
        let runner = HostSamplingRunner::new(Arc::new(transport));

        let response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "role": "assistant",
                "content": { "type": "text", "text": "The capital of France is Paris." },
                "model": "claude-sonnet-4-5-20260514",
                "stopReason": "endTurn"
            }
        });

        let output = runner
            .parse_response(response, Duration::from_millis(500))
            .expect("should parse successfully");
        assert_eq!(output.stdout, "The capital of France is Paris.");
        assert_eq!(output.duration, Duration::from_millis(500));
        assert!(output.usage.is_none());
    }

    #[test]
    fn parse_response_user_rejected() {
        let (transport, _mock) = ChannelTransport::pair();
        let runner = HostSamplingRunner::new(Arc::new(transport));

        let response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "error": { "code": -1, "message": "User declined sampling" }
        });

        let err = runner
            .parse_response(response, Duration::from_millis(100))
            .unwrap_err();
        match err {
            RunnerError::AuthFailed(msg) => {
                assert!(msg.contains("user declined"), "msg = {msg}");
            }
            other => panic!("expected AuthFailed, got: {other:?}"),
        }
    }

    #[test]
    fn parse_response_generic_error() {
        let (transport, _mock) = ChannelTransport::pair();
        let runner = HostSamplingRunner::new(Arc::new(transport));

        let response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "error": { "code": -32600, "message": "Invalid request" }
        });

        let err = runner
            .parse_response(response, Duration::from_millis(100))
            .unwrap_err();
        match err {
            RunnerError::NonZeroExit { stderr, .. } => {
                assert!(stderr.contains("Invalid request"), "stderr = {stderr}");
            }
            other => panic!("expected NonZeroExit, got: {other:?}"),
        }
    }

    #[test]
    fn channel_transport_round_trip() {
        let (transport, mock) = ChannelTransport::pair();

        // Spawn a thread to handle the mock endpoint
        let handle = std::thread::spawn(move || {
            let req = mock.recv_request().expect("should receive request");
            assert_eq!(req["method"], "sampling/createMessage");
            mock.send_response(serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "result": {
                    "role": "assistant",
                    "content": { "type": "text", "text": "mock response" },
                    "model": "test-model",
                    "stopReason": "endTurn"
                }
            }))
            .expect("should send response");
        });

        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "sampling/createMessage",
            "params": {}
        });
        let response = transport.request(&request).expect("should get response");
        assert_eq!(response["result"]["content"]["text"], "mock response");

        handle.join().expect("mock thread should complete");
    }

    #[test]
    fn run_via_channel_transport() {
        // Suppress progress output in tests
        unsafe { std::env::set_var("CORAL_QUIET", "1") };

        let (transport, mock) = ChannelTransport::pair();
        let runner = HostSamplingRunner::new(Arc::new(transport));

        let handle = std::thread::spawn(move || {
            let _req = mock.recv_request().expect("should receive request");
            mock.send_response(serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "result": {
                    "role": "assistant",
                    "content": { "type": "text", "text": "# Auth\n\nAuthentication uses JWT..." },
                    "model": "claude-sonnet-4-5-20260514",
                    "stopReason": "endTurn"
                }
            }))
            .expect("should send response");
        });

        let prompt = Prompt {
            system: Some("Generate wiki page.".to_string()),
            user: "Write about auth module.".to_string(),
            model: Some("claude-sonnet-4-5".to_string()),
            cwd: None,
            timeout: None,
        };

        let output = runner.run(&prompt).expect("should succeed");
        assert_eq!(output.stdout, "# Auth\n\nAuthentication uses JWT...");
        assert!(output.duration.as_millis() < 5000);

        handle.join().expect("mock thread should complete");

        // Restore
        unsafe { std::env::remove_var("CORAL_QUIET") };
    }

    // Serialize env-touching tests to prevent parallel interference.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn host_sampling_available_detects_claudecode() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        unsafe { std::env::set_var("CLAUDECODE", "1") };
        unsafe { std::env::remove_var("CORAL_RUNNER") };
        assert!(host_sampling_available());
        unsafe { std::env::remove_var("CLAUDECODE") };
    }

    #[test]
    fn host_sampling_available_detects_explicit_env() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        unsafe { std::env::remove_var("CLAUDECODE") };
        unsafe { std::env::set_var("CORAL_RUNNER", "host-sampling") };
        assert!(host_sampling_available());
        assert!(host_sampling_explicit());
        unsafe { std::env::remove_var("CORAL_RUNNER") };
    }

    #[test]
    fn host_sampling_not_available_by_default() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        unsafe { std::env::remove_var("CLAUDECODE") };
        unsafe { std::env::remove_var("CORAL_RUNNER") };
        assert!(!host_sampling_available());
    }
}
