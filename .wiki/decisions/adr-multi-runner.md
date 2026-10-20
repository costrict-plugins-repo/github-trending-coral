---
slug: adr-multi-runner
type: decision
last_updated_commit: 19da4d6
confidence: 0.94
sources:
  - crates/coral-runner/src/runner.rs
  - crates/coral-runner/src/lib.rs
  - crates/coral-runner/Cargo.toml
backlinks:
  - bootstrap-flow
  - ingest-cycle
  - session-distill
  - adr-rust
status: draft
---

# ADR: Multiple LLM Runners

## Context

Coral generates wiki pages by calling LLMs. Different users have different constraints: some want quality (Claude), some want free tier (Gemini), some need privacy (local models), and some operate behind corporate proxies that only expose OpenAI-compatible endpoints.

## Decision

Coral defines a `Runner` trait and ships multiple interchangeable implementations rather than hardcoding a single provider.

## The Runner Trait

```rust
// crates/coral-runner/src/runner.rs

pub trait Runner: Send + Sync {
    /// Execute a prompt and return the captured output.
    /// Implementations must NOT panic on bad input — return RunnerError instead.
    fn run(&self, prompt: &Prompt) -> RunnerResult<RunOutput>;

    /// Streaming variant: invokes `on_chunk` with each newly-emitted chunk.
    /// Default falls back to `run()` and emits a single chunk.
    fn run_streaming(
        &self,
        prompt: &Prompt,
        on_chunk: &mut dyn FnMut(&str),
    ) -> RunnerResult<RunOutput>;
}
```

## Shared Types

```rust
// crates/coral-runner/src/runner.rs

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Prompt {
    pub system: Option<String>,
    pub user: String,
    pub model: Option<String>,
    pub cwd: Option<PathBuf>,
    pub timeout: Option<Duration>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RunOutput {
    pub stdout: String,
    pub stderr: String,
    pub duration: Duration,
    pub usage: Option<TokenUsage>,
}
```

## Implementations

All exported from the runner crate:

```rust
// crates/coral-runner/src/lib.rs

pub use anthropic_api::AnthropicApiRunner;
pub use gemini::GeminiRunner;
pub use host_sampling::HostSamplingRunner;
pub use http::HttpRunner;
pub use local::LocalRunner;
pub use mock::MockRunner;
pub use runner::{ClaudeRunner, Prompt, RunOutput, Runner, RunnerError, RunnerResult, TokenUsage};
```

| Runner | Provider | Selection | Notes |
|--------|----------|-----------|-------|
| `ClaudeRunner` | Anthropic (via `claude` CLI) | `--provider claude` (default) | Spawns `claude --print --output-format json` |
| `GeminiRunner` | Google (via `gemini` CLI) | `--provider gemini` | Free tier available |
| `LocalRunner` | Local models (via `llama-cli`) | `--provider local` | Full privacy; no usage tracking |
| `HttpRunner` | Any OpenAI-compatible endpoint | `--provider http` | Reads `CORAL_HTTP_ENDPOINT` |
| `HostSamplingRunner` | Claude Code session (MCP sampling) | `--provider host-sampling` | Bypasses macOS ES gate |
| `AnthropicApiRunner` | Anthropic Messages API (direct) | `--provider anthropic-api` | Uses curl; works inside Claude Code shells |

## ClaudeRunner: Host-Managed Mode Check

```rust
// crates/coral-runner/src/runner.rs

fn host_managed_mode_check(&self) -> RunnerResult<()> {
    if self.is_real_claude_binary() && std::env::var("CLAUDECODE").as_deref() == Ok("1") {
        return Err(RunnerError::AuthFailed(
            "claude CLI cannot authenticate from inside a Claude Code shell — \
             it detects the parent process via macOS Endpoint Security and switches to \
             host-managed mode, ignoring its keychain credential. \
             Fix: run `coral` from a plain Terminal."
                .to_string(),
        ));
    }
    Ok(())
}
```

## Argument Injection Prevention

```rust
// crates/coral-runner/src/runner.rs (ClaudeRunner::run)

// v0.30.0 audit: `prompt.user` is user-controlled. Inserting `--`
// forces the rest of the args to be treated as positionals.
cmd.arg("--");
cmd.arg(&prompt.user);
```

## Usage Parsing

```rust
// crates/coral-runner/src/runner.rs

pub(crate) fn parse_usage_from_stdout(stdout: &str) -> (Option<TokenUsage>, Option<String>) {
    // Tries Anthropic shape: {"result":"<text>", "usage":{...}}
    // Then OpenAI shape: {"choices":[...], "usage":{...}}
    let pick = |keys: &[&str]| -> u64 {
        for k in keys {
            if let Some(n) = u.get(*k).and_then(|v| v.as_u64()) { return n; }
        }
        0
    };
    let input = pick(&["input_tokens", "prompt_tokens", "inputTokens"]);
    let output = pick(&["output_tokens", "completion_tokens", "outputTokens"]);
    // ...
}
```

## Secret Scrubbing

```rust
// crates/coral-runner/src/runner.rs

pub(crate) fn scrub_secrets(text: &str) -> String {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(
            r"(?i)(?:authorization|x-api-key)\s*:\s*\S+(?:\s+\S+)?|bearer\s+\S+"
        ).expect("valid regex")
    });
    re.replace_all(text, "<redacted>").to_string()
}
```

## Consequences

- All runners return the same `RunOutput` struct -- cost estimation works uniformly
- `MockRunner` enables deterministic testing without real LLM calls
- The `test_script_lock()` helper serializes tests that spawn shell scripts to avoid ETXTBSY races
- The `--` separator security fix only applies to `ClaudeRunner` (others use `-p` which is safe by construction)
