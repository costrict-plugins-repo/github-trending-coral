---
slug: mcp-transport
type: flow
last_updated_commit: 19da4d6
confidence: 0.92
sources:
  - crates/coral-mcp/src/transport/mod.rs
  - crates/coral-mcp/src/transport/stdio.rs
  - crates/coral-mcp/src/server.rs
backlinks:
  - adr-multi-runner
  - session-distill
status: draft
---

# MCP Transport

The MCP transport layer provides two wire-format implementations for the Model Context Protocol server. Both share the same `McpHandler::handle_line` dispatcher -- only the framing differs.

## Protocol Constants

```rust
// crates/coral-mcp/src/server.rs

/// MCP protocol version. Coral pins to the 2025-11-25 spec freeze.
pub const PROTOCOL_VERSION: &str = "2025-11-25";

/// Page size for `resources/list` + `tools/list` cursor pagination.
pub const PAGINATION_PAGE_SIZE: usize = 100;
```

## McpHandler Core

```rust
// crates/coral-mcp/src/server.rs

pub struct McpHandler {
    pub config: ServerConfig,
    pub resources: Arc<dyn ResourceProvider>,
    pub tools: Arc<dyn ToolDispatcher>,
    subscriptions: Mutex<HashSet<String>>,
    notification_tx: Mutex<Option<std::sync::mpsc::Sender<serde_json::Value>>>,
    tasks: Mutex<Vec<McpTask>>,
    task_id_counter: AtomicU64,
}

pub trait ToolDispatcher: Send + Sync {
    fn call(&self, name: &str, args: &serde_json::Value) -> ToolCallResult;
}
```

## Typed Error Codes

```rust
// crates/coral-mcp/src/server.rs

enum HandlerError {
    InvalidParams(String),   // -32602
    NotFound(String),        // -32002 (resource/prompt URI does not exist)
    Gated(String),           // -32001 (feature behind --allow-write-tools)
    MethodNotFound(String),  // -32601
    Internal(String),        // -32603
}

impl HandlerError {
    fn code(&self) -> i64 {
        match self {
            HandlerError::InvalidParams(_) => -32602,
            HandlerError::NotFound(_) => -32002,
            HandlerError::Gated(_) => -32001,
            HandlerError::MethodNotFound(_) => -32601,
            HandlerError::Internal(_) => -32603,
        }
    }
}
```

## Stdio Transport

The canonical transport for every shipped MCP client (Claude Code, VS Code extensions):

```rust
// crates/coral-mcp/src/transport/stdio.rs

pub fn serve_stdio(handler: &McpHandler) -> std::io::Result<()> {
    let (tx, rx) = mpsc::channel();
    handler.set_notification_sender(tx);

    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if line.trim().is_empty() {
            continue;
        }
        if let Some(response) = handler.handle_line(&line) {
            let serialized = serde_json::to_string(&response).unwrap_or_else(|_| "{}".into());
            writeln!(handle, "{serialized}")?;
            handle.flush()?;
        }
        // Drain pending push notifications (resource subscription updates)
        while let Ok(notification) = rx.try_recv() {
            let serialized = serde_json::to_string(&notification).unwrap_or_else(|_| "{}".into());
            writeln!(handle, "{serialized}")?;
            handle.flush()?;
        }
    }
    Ok(())
}
```

**Wire format:** One JSON-RPC 2.0 envelope per line on stdin; one response line on stdout. Stderr is reserved for server-side logs. Notifications (no `id` field) are dispatched for side effects but no response is emitted, per JSON-RPC 2.0 section 4.1.

## HTTP/SSE Transport (Streamable HTTP)

The alternative transport per MCP 2025-11-25 spec, behind `coral mcp serve --transport http`.

**Endpoints:**

| Method | Path | Response |
|--------|------|----------|
| `POST /mcp` | JSON-RPC envelope | 200 application/json (request) or 204 (notification) |
| `GET /mcp` | Accept: text/event-stream | 200 SSE stream with notifications + keep-alive |
| `DELETE /mcp` | Mcp-Session-Id header | 204 (success) or 404 (not found) |

**Security:**
- Default bind `127.0.0.1` (load-bearing defense)
- `--bind 0.0.0.0` requires `--token <hex>` (v0.35 SEC-01)
- Bearer auth checked before body read, before session table touch (O(1) rejection)
- 4 MiB body cap (413 on exceed), 32 concurrent handler threads max (503 on exceed)
- Session IDs use 122 bits of OS CSPRNG entropy (v0.35 SEC-07)
- Origin validation for DNS-rebinding mitigation

## Discovery

`GET /.well-known/mcp/server-card.json` returns a static JSON server card (exempt from auth/origin checks by design).
