---
slug: mcp
type: module
last_updated_commit: 19da4d6
confidence: 0.95
sources:
- crates/coral-mcp/src/lib.rs
- crates/coral-mcp/src/server.rs
- crates/coral-mcp/src/tools.rs
- crates/coral-mcp/src/sampling.rs
backlinks:
- cli
- runner
- core
status: draft
---

# mcp

The Model Context Protocol server. `coral-mcp` exposes the wiki, manifest, and tool surface to any MCP-speaking agent (Claude Code, Cursor, Continue, Cline, Goose, Codex) over stdio or Streamable HTTP/SSE transports.

## Responsibilities

- Implement JSON-RPC 2.0 server for MCP protocol (hand-rolled, no external MCP SDK)
- Expose wiki pages as MCP resources with cursor-based pagination
- Expose tools (query, search, find_backlinks, affected_repos, verify, contract_status, etc.)
- Expose prompt templates for onboarding and context building
- Bridge MCP sampling (`sampling/createMessage`) for host-delegated LLM calls
- Support both stdio and HTTP/SSE transports (v0.21.1+)
- Enforce read-only defaults with opt-in write tools (`--allow-write-tools`)

## Server Configuration

```rust
// from crates/coral-mcp/src/lib.rs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub transport: Transport,
    pub read_only: bool,
    pub allow_write_tools: bool,
    pub port: Option<u16>,
    pub bind_addr: Option<std::net::IpAddr>,
    pub allow_experimental_tasks: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Transport {
    Stdio,
    HttpSse,
}
```

## McpHandler

```rust
// from crates/coral-mcp/src/server.rs
pub const PROTOCOL_VERSION: &str = "2025-11-25";
pub const PAGINATION_PAGE_SIZE: usize = 100;

pub struct McpHandler {
    pub config: ServerConfig,
    pub resources: Arc<dyn ResourceProvider>,
    pub tools: Arc<dyn ToolDispatcher>,
    subscriptions: Mutex<HashSet<String>>,
    notification_tx: Mutex<Option<std::sync::mpsc::Sender<serde_json::Value>>>,
    tasks: Mutex<Vec<McpTask>>,
    task_id_counter: AtomicU64,
}
```

The handler implements methods: `initialize`, `resources/list`, `resources/read`, `tools/list`, `tools/call`, `prompts/list`, `prompts/get`, and experimental `tasks/create`, `tasks/list`.

## Tool Catalog

```rust
// from crates/coral-mcp/src/tools.rs
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub kind: ToolKind,
    pub input_schema_json: String,
    pub read_only: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    Query,
    Search,
    FindBacklinks,
    AffectedRepos,
    Verify,
    ListInterfaces,
    ContractStatus,
    RunTest,
    Up,
    Down,
}
```

Read-only tools (always available):
- `query` - Streamed LLM answer using wiki as context
- `search` - TF-IDF full-text search
- `find_backlinks` - All slugs linking to a given slug
- `affected_repos` - Repos changed since a SHA
- `verify` - Liveness healthchecks
- `list_interfaces` - Interface/contract pages
- `contract_status` - Latest drift check results

Write tools (require `--allow-write-tools`):
- `run_test` - Run a specific test case
- `up` - Bring up dev environment
- `down` - Tear down dev environment

## Tool Dispatcher Trait

```rust
// from crates/coral-mcp/src/server.rs
pub trait ToolDispatcher: Send + Sync {
    fn call(&self, name: &str, args: &serde_json::Value) -> ToolCallResult;
}

pub enum ToolCallResult {
    Ok(serde_json::Value),
    Skip { reason: String },
    Error { message: String },
}
```

## Sampling Bridge (v0.41)

```rust
// from crates/coral-mcp/src/sampling.rs
pub struct SamplingBridge {
    client_supports_sampling: AtomicBool,
    request_tx: Mutex<Option<std::sync::mpsc::Sender<SamplingExchange>>>,
    next_id: AtomicU64, // starts at 900_000 to avoid ID collision
}

pub struct SamplingExchange {
    pub request: serde_json::Value,
    pub response_tx: std::sync::mpsc::Sender<serde_json::Value>,
}

impl SamplingBridge {
    pub fn new() -> Arc<Self>;
    pub fn set_client_supports_sampling(&self, supports: bool);
    pub fn client_supports_sampling(&self) -> bool;
    pub fn send_sampling_request(&self, params: serde_json::Value) -> Result<serde_json::Value, String>;
}
```

The sampling bridge enables the MCP server to send `sampling/createMessage` requests back to the host client (Claude Code), allowing the [[runner]] `HostSamplingRunner` to leverage the active session for LLM inference without a separate API key.

## Error Handling

```rust
// from crates/coral-mcp/src/server.rs
enum HandlerError {
    InvalidParams(String),   // -32602
    NotFound(String),        // -32002
    Gated(String),           // -32001
    MethodNotFound(String),  // -32601
    Internal(String),        // -32603
}
```

## Module Exports

```rust
// from crates/coral-mcp/src/lib.rs
pub use card::server_card;
pub use prompts::{PromptCatalog, PromptDescriptor};
pub use resources::{Resource, ResourceProvider, WikiResourceProvider};
pub use sampling::{BridgeTransport, SamplingBridge, SamplingExchange};
pub use server::{ContractToolDispatcher, McpHandler, McpTask, NoOpDispatcher, PROTOCOL_VERSION, ToolCallResult, ToolDispatcher};
pub use state::WikiState;
pub use tools::{Tool, ToolCatalog, ToolKind};
```

## Security Posture

- Default-deny: `read_only: true`, `allow_write_tools: false`
- HTTP transport: loopback-only by default, `Origin` validation (DNS-rebinding mitigation)
- 4 MiB body cap on HTTP transport
- Unreviewed-distilled page filter (pages with `reviewed: false` are excluded)
- `--bind 0.0.0.0` is opt-in with a stderr warning banner

## Cross-Crate Relationships

- [[core]] provides page model, walk, search used by `WikiResourceProvider`
- [[runner]] provides `HostSamplingRunner` that connects via `SamplingBridge`
- [[cli]] instantiates the MCP server via `commands::mcp`
