---
slug: observability
type: concept
last_updated_commit: 19da4d6
confidence: 0.95
sources:
- crates/coral-core/src/observability.rs
- crates/coral-cli/src/main.rs
- crates/coral-runner/src/runner.rs
backlinks:
- runner-trait
status: draft
---

# Observability

Coral's observability layer combines two mechanisms: the `progress!` macro (v0.41 P1) for lightweight human-readable feedback on stderr, and `tracing-subscriber` for structured debug logging. Together they give users and developers visibility into LLM calls, multi-step operations, and internal state.

## The progress! Macro

Defined in `crates/coral-core/src/observability.rs`, the macro provides three forms:

```rust
// crates/coral-core/src/observability.rs

#[macro_export]
macro_rules! progress {
    // Step in progress — no annotations
    (step, $msg:expr) => {
        $crate::observability::emit_progress("▶", $msg, &[]);
    };
    // Step in progress — with key=value pairs
    (step, $msg:expr; $($key:ident = $val:expr),+ $(,)?) => {
        $crate::observability::emit_progress(
            "▶", $msg,
            &[$( (stringify!($key), &$val as &dyn ::std::fmt::Display) ),+],
        );
    };
    // Done (checkmark)
    (done, $step:expr, $msg:expr; $($key:ident = $val:expr),+ $(,)?) => {
        $crate::observability::emit_progress(
            "✓", $msg,
            &[$( (stringify!($key), &$val as &dyn ::std::fmt::Display) ),+],
        );
    };
    // Fail (cross)
    (fail, $step:expr, $msg:expr; $($key:ident = $val:expr),+ $(,)?) => {
        $crate::observability::emit_progress(
            "✗", $msg,
            &[$( (stringify!($key), &$val as &dyn ::std::fmt::Display) ),+],
        );
    };
}
```

Output example: `▶ Calling claude CLI...  (model: sonnet)` or `✓ Got response  (tokens: 1234, elapsed_ms: 3200)`.

## Gate Logic

```rust
// crates/coral-core/src/observability.rs

pub fn progress_enabled() -> bool {
    // Explicit quiet -> always suppress.
    if std::env::var("CORAL_QUIET").as_deref() == Ok("1") {
        return false;
    }
    // Explicit verbose -> always emit.
    if std::env::var("CORAL_VERBOSE").as_deref() == Ok("1") {
        return true;
    }
    // Otherwise: only emit when stderr is a real terminal.
    std::io::stderr().is_terminal()
}
```

Override table:

| CORAL_QUIET | CORAL_VERBOSE | stderr TTY | Output? |
|-------------|---------------|------------|---------|
| 1           | any           | any        | No      |
| unset       | 1             | any        | Yes     |
| unset       | unset         | yes        | Yes     |
| unset       | unset         | no         | No      |

`CORAL_QUIET=1` always wins. `CORAL_VERBOSE=1` forces output regardless of TTY state (useful in CI). Without either override, progress is shown only on real terminals to avoid noise in piped contexts.

The per-call check (not cached at startup) ensures env vars set after `main()` -- such as test helpers or the `--quiet` flag propagation -- are picked up correctly.

## Verbose Dump

When `CORAL_VERBOSE=1`, the [[runner-trait]] implementations emit full prompt and response bodies to stderr. The `truncate_for_display` helper caps output:

```rust
// crates/coral-core/src/observability.rs

pub fn truncate_for_display(s: &str, max_chars: usize) -> String {
    if max_chars == 0 || s.chars().count() <= max_chars {
        return s.to_owned();
    }
    let truncated: String = s.chars().take(max_chars).collect();
    format!("{truncated}…")
}
```

`max_chars == 0` means no limit (maps to `--verbose=full` semantics). Default truncation is 2000 characters per prompt/response dump.

## Tracing Setup

The CLI's `main.rs` configures `tracing-subscriber` based on CLI flags:

```rust
// crates/coral-cli/src/main.rs

fn setup_tracing(quiet: bool, verbose: bool) {
    use tracing_subscriber::EnvFilter;
    let default = if quiet {
        "warn"
    } else if verbose {
        "coral=debug,info"
    } else {
        "info"
    };
    let filter = EnvFilter::try_from_env("RUST_LOG")
        .unwrap_or_else(|_| EnvFilter::new(default));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_writer(std::io::stderr)
        .try_init();
}
```

- `--quiet`: only warnings and errors
- `--verbose`: `coral=debug` + `info` for other crates
- Default: `info` level
- `RUST_LOG` env var overrides all defaults

## CLI Flag Propagation

The `main()` function propagates `--quiet` and `--verbose` into environment variables before tracing init:

```rust
// crates/coral-cli/src/main.rs

if cli.quiet {
    unsafe { std::env::set_var("CORAL_QUIET", "1") };
}
if cli.verbose {
    unsafe { std::env::set_var("CORAL_VERBOSE", "1") };
}
setup_tracing(cli.quiet, cli.verbose);
```

This ensures the `progress!` macro gate reads the right values from the first call onward, without needing CLI args threaded through every call-site.

## How Runners Use It

`ClaudeRunner::run()` emits (from `crates/coral-runner/src/runner.rs`):

1. `progress!(step, "Prompt (user)"; chars = ...)` + truncated body (verbose only)
2. `progress!(step, "Calling claude CLI..."; model = ...)` before spawn
3. `progress!(step, "Response"; chars = ...)` + truncated body (verbose only)
4. `progress!(done, "run", "Got response"; tokens = ..., elapsed_ms = ...)` after response

The streaming path emits `"Calling claude CLI (streaming)..."` before spawn and `"Got streaming response"` on completion.

## Design Rationale

- Zero external dependencies for the progress path (just `eprintln!`)
- Per-call gate check (two `env::var` lookups) ensures env changes mid-process work correctly
- `pub mod observability` in `coral-core` so `coral-runner`, `coral-cli`, and `coral-mcp` can all import the macro
- KVs use `&[(&str, &dyn Display)]` to avoid boxing allocation

## See Also

- [[runner-trait]] -- primary consumer of the progress macro
