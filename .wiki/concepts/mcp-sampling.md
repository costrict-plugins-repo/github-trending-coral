---
slug: mcp-sampling
type: concept
last_updated_commit: 19da4d6
confidence: 0.95
sources:
- crates/coral-mcp/src/sampling.rs
- crates/coral-runner/src/host_sampling.rs
backlinks:
- runner-trait
status: draft
---

# MCP Sampling

The MCP sampling bridge enables `coral-mcp` (the MCP server) to request LLM inference from the host client (Claude Code) via the `sampling/createMessage` JSON-RPC method. This is a **server-to-client** request pattern -- the reverse of normal tool calls.

## Architecture

```text
+------------------+         stdio stdout         +------------------+
|  coral-mcp       |  ---- request ----------->  |  Claude Code     |
|  (MCP server)    |  <--- response ----------   |  (MCP client)    |
|                  |         stdio stdin          |                  |
+------------------+                              +------------------+
```

During `initialize`, the client MAY advertise `"sampling": {}` in its capabilities. If it does, the server can send `sampling/createMessage` JSON-RPC requests on the transport's output stream (stdout for stdio transport).

## SamplingBridge

```rust
// crates/coral-mcp/src/sampling.rs

#[derive(Debug)]
pub struct SamplingBridge {
    /// Whether the client advertised sampling capability.
    client_supports_sampling: AtomicBool,
    /// Channel for sending sampling requests to the transport layer.
    request_tx: Mutex<Option<std::sync::mpsc::Sender<SamplingExchange>>>,
    /// Next request ID (starts at 900,000 to avoid ID collision).
    next_id: AtomicU64,
}

/// A pending sampling exchange.
#[derive(Debug)]
pub struct SamplingExchange {
    pub request: serde_json::Value,
    pub response_tx: std::sync::mpsc::Sender<serde_json::Value>,
}
```

The bridge is wrapped in `Arc` and shared between the MCP handler (which parses `initialize` params) and the `HostSamplingRunner` (which sends sampling requests).

## Request Flow

The `send_sampling_request` method implements the full round-trip:

```rust
// crates/coral-mcp/src/sampling.rs

pub fn send_sampling_request(
    &self,
    params: serde_json::Value,
) -> Result<serde_json::Value, String> {
    if !self.client_supports_sampling() {
        return Err("MCP client does not support sampling/createMessage...".to_string());
    }
    let id = self.next_id.fetch_add(1, Ordering::Relaxed);
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "sampling/createMessage",
        "params": params,
    });
    let (response_tx, response_rx) = std::sync::mpsc::channel();
    let exchange = SamplingExchange { request, response_tx };

    // Send to transport layer
    let tx_guard = self.request_tx.lock().unwrap_or_else(|p| p.into_inner());
    let tx = tx_guard.as_ref().ok_or_else(|| "not connected to transport")?;
    tx.send(exchange).map_err(|_| "Transport channel closed")?;
    drop(tx_guard);

    // Wait with 5-minute timeout
    let timeout = std::time::Duration::from_secs(300);
    response_rx.recv_timeout(timeout).map_err(|e| match e {
        RecvTimeoutError::Timeout => "timed out after 5 minutes".to_string(),
        RecvTimeoutError::Disconnected => "response channel closed".to_string(),
    })
}
```

## BridgeTransport Adapter

`BridgeTransport` adapts `SamplingBridge` to the `McpTransport` trait defined in `coral-runner`:

```rust
// crates/coral-mcp/src/sampling.rs

pub struct BridgeTransport {
    bridge: Arc<SamplingBridge>,
}

impl coral_runner::McpTransport for BridgeTransport {
    fn request(&self, body: &serde_json::Value) -> Result<serde_json::Value, String> {
        let params = body.get("params").cloned()
            .unwrap_or(serde_json::Value::Null);
        self.bridge.send_sampling_request(params)
    }
}
```

This is the glue between the MCP server's transport and the `HostSamplingRunner` in `coral-runner`.

## McpTransport Trait

```rust
// crates/coral-runner/src/host_sampling.rs

pub trait McpTransport: Send + Sync {
    fn request(&self, body: &serde_json::Value) -> Result<serde_json::Value, String>;
}
```

Implementations:
- `StdioTransport`: writes to a process's stdin, reads from stdout (production path)
- `ChannelTransport`: in-process mpsc channel pair for tests; comes with `ChannelMockEndpoint` for the mock host side

## HostSamplingRunner

The `HostSamplingRunner` in `coral-runner` implements the [[runner-trait]] by delegating to the MCP sampling bridge:

```rust
// crates/coral-runner/src/host_sampling.rs

pub struct HostSamplingRunner {
    transport: Arc<dyn McpTransport>,
    // ... model preferences, max_tokens, next_id
}
```

Its `run()` builds a `sampling/createMessage` request with messages, system prompt, max tokens, and optional model preferences. The response's `result.content.text` becomes `RunOutput.stdout`. Token usage is always `None` since the MCP sampling spec does not expose it.

## Lifecycle Management

```rust
// crates/coral-mcp/src/sampling.rs

impl SamplingBridge {
    pub fn new() -> Arc<Self> { ... }

    /// Called by the transport layer to register the request channel.
    pub fn set_request_sender(&self, tx: Sender<SamplingExchange>) { ... }

    /// Called during `initialize` to record client capabilities.
    pub fn set_client_supports_sampling(&self, supports: bool) { ... }

    /// Returns `true` if the client advertised sampling capability.
    pub fn client_supports_sampling(&self) -> bool { ... }
}
```

The bridge starts disconnected. The transport layer registers the request sender during server startup. During `initialize` handshake, the handler checks the client's capabilities and calls `set_client_supports_sampling`.

## Error Conditions

1. **Client does not support sampling**: returns error referencing `anthropics/claude-code#1785`
2. **Transport not connected**: internal error (server should register channel at startup)
3. **Transport channel closed**: MCP server may have shut down
4. **5-minute timeout**: host did not respond (may be stuck or disconnected)

## See Also

- [[runner-trait]] -- `HostSamplingRunner` implements the `Runner` trait via this bridge
