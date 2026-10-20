---
slug: ui
type: module
last_updated_commit: 19da4d6
confidence: 0.95
sources:
- crates/coral-ui/src/lib.rs
backlinks:
- cli
- core
- stats
status: draft
---

# ui

REST API and embedded SPA for the Coral WebUI. `coral-ui` provides `coral ui serve` -- a synchronous HTTP server with conservative security defaults, exposing wiki data and search over a small REST surface.

## Responsibilities

- Serve the embedded SPA (single-page application) from `assets/dist/`
- Expose REST API endpoints for wiki pages, search, graph, manifest, stats, and query
- Enforce security defaults: loopback-only, read-only, Host/Origin validation
- Gate LLM-touching endpoints behind bearer-token authentication
- Run without tokio (synchronous `tiny_http` server)

## Public API

```rust
// from crates/coral-ui/src/lib.rs
pub use error::ApiError;
pub use server::{ServeConfig, serve};
```

## Module Structure

```rust
// from crates/coral-ui/src/lib.rs
mod auth;
mod error;
mod routes;
mod server;
mod state;
mod static_assets;
```

| Module | Purpose |
|--------|---------|
| `auth` | Bearer token validation, Host/Origin checks |
| `error` | `ApiError` canonical error envelope |
| `routes` | REST endpoint handlers |
| `server` | `ServeConfig` + blocking `serve()` entry point |
| `state` | Shared server state (wiki data, config) |
| `static_assets` | Embedded SPA bundle serving |

## REST Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/api/v1/pages` | GET | List/filter wiki pages |
| `/api/v1/search` | GET | TF-IDF search over wiki |
| `/api/v1/graph` | GET | Wiki link graph (nodes + edges) |
| `/api/v1/manifest` | GET | Project manifest data |
| `/api/v1/lock` | GET | Lock/dependency data |
| `/api/v1/stats` | GET | Wiki health metrics |
| `/api/v1/query` | POST | LLM-augmented query (token-gated) |

## Security Defaults

- **Loopback-only**: binds `127.0.0.1` by default
- **Read-only**: no mutation endpoints
- **Host/Origin validation**: prevents DNS-rebinding attacks
- **Bearer-token gating**: required for off-loopback or LLM endpoints
- **No tokio**: synchronous runtime, mirrors the MCP server pattern

## Configuration

```rust
// from crates/coral-ui/src/lib.rs (via server module)
pub struct ServeConfig {
    // port, bind address, token, wiki root, etc.
}
```

The server defaults to port 3838 (chosen to avoid common dev-server port ranges). The `--token` / `CORAL_UI_TOKEN` environment variable gates access when binding off-loopback or calling `/api/v1/query`.

## History

- v0.32.0: Initial WebUI implementation
- v0.38.0: Legacy `coral wiki serve` removed after 3-version deprecation; `coral ui serve` is its full replacement

## Cross-Crate Relationships

- [[core]] provides page model, walk, search for data access
- [[stats]] provides `StatsReport` served at `/api/v1/stats`
- [[cli]] starts the server via `commands::ui` (feature-gated behind `ui` feature)
