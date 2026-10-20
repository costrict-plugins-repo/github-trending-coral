---
slug: adr-rust
type: decision
last_updated_commit: 19da4d6
confidence: 0.95
sources:
  - Cargo.toml
  - crates/coral-runner/src/runner.rs
  - crates/coral-cli/src/commands/bootstrap/mod.rs
backlinks:
  - bootstrap-flow
  - adr-multi-runner
  - mcp-transport
status: draft
---

# ADR: Why Rust

## Context

Coral is a CLI tool that runs alongside developer workflows -- invoked by pre-commit hooks, MCP clients, and CI pipelines. It needs to start fast, distribute easily, and handle parallel workloads safely.

## Decision

Coral is implemented in Rust (edition 2024, MSRV 1.89).

```toml
# Cargo.toml (workspace root)

[workspace.package]
version = "0.40.2"
edition = "2024"
rust-version = "1.89"
```

## Rationale

**Single static binary (no runtime dependency):** Coral ships as one binary with zero runtime requirements. Users install by copying or running a shell script. The release profile is optimized for size:

```toml
# Cargo.toml

[profile.release]
lto = "thin"
codegen-units = 1
strip = true
opt-level = 3
panic = "abort"
```

**Fearless concurrency for parallel page generation:** The [[bootstrap-flow]] renders up to 4 wiki pages simultaneously via rayon thread pools. The [[adr-multi-runner]] `Runner` trait requires `Send + Sync`, and the three-phase design is enforced by the type system:

```rust
// crates/coral-runner/src/runner.rs

pub trait Runner: Send + Sync {
    fn run(&self, prompt: &Prompt) -> RunnerResult<RunOutput>;
    fn run_streaming(
        &self,
        prompt: &Prompt,
        on_chunk: &mut dyn FnMut(&str),
    ) -> RunnerResult<RunOutput> {
        let out = self.run(prompt)?;
        on_chunk(&out.stdout);
        Ok(out)
    }
}
```

**Compile-time safety for wiki schema:** The frontmatter module uses typed enums for `PageType`, `Status`, and a validated `Confidence` newtype. A malformed wiki page triggers a compile-time serde error, not a runtime KeyError.

**Performance-sensitive operations:** The workspace uses `mimalloc` as the global allocator for allocation-heavy workloads (TF-IDF scoring, wiki parsing):

```toml
# Cargo.toml

mimalloc = { version = "0.1", default-features = false }
```

## Tradeoffs Accepted

- Longer compile times than Go/TypeScript (mitigated by workspace incremental builds and `debug = "line-tables-only"`)
- Steeper contributor onboarding (mitigated by 1900+ tests and `MockRunner` pattern)
- No REPL (mitigated by `cargo test --watch` loop)

## Consequences

- All crates share a single workspace `Cargo.toml` dependency resolution (10 crates in `crates/*`)
- `tiny_http` was chosen over `hyper`/`axum` for the HTTP/SSE [[mcp-transport]] specifically to avoid pulling tokio into the tree
- Workspace-wide clippy lints surface panic-risk sites as warnings:

```toml
# Cargo.toml

[workspace.lints.clippy]
unwrap_used = "warn"
expect_used = "warn"
panic = "warn"
```

- Cross-compilation for Linux/macOS/Windows is handled by standard Rust toolchain targets
