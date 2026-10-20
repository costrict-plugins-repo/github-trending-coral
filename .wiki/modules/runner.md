---
slug: runner
type: module
last_updated_commit: 19da4d6
confidence: 0.95
sources:
- crates/coral-runner/src/lib.rs
- crates/coral-runner/src/runner.rs
backlinks:
- cli
- mcp
- session
- core
status: draft
---

# runner

The LLM invocation layer. `coral-runner` wraps multiple provider CLIs and APIs behind a unified `Runner` trait, enabling test-substitutable LLM calls across the workspace.

## Responsibilities

- Define the `Runner` trait (synchronous and streaming variants)
- Implement `ClaudeRunner` (subprocess `claude --print --output-format json`)
- Implement `GeminiRunner`, `LocalRunner` (llama.cpp), `HttpRunner` (OpenAI-compat)
- Implement `AnthropicApiRunner` (direct Messages API, bypasses CLI)
- Implement `HostSamplingRunner` (MCP `sampling/createMessage`)
- Provide `MultiStepRunner` for tiered (budget-gated) multi-call workflows
- Provide `MockRunner` for deterministic testing
- Parse token usage from provider JSON responses for cost tracking
- Scrub secrets from error output (bearer tokens, API keys)

## Runner Trait

```rust
// from crates/coral-runner/src/runner.rs
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

## Core Types

```rust
// from crates/coral-runner/src/runner.rs
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
}
```

## ClaudeRunner

```rust
// from crates/coral-runner/src/runner.rs
#[derive(Debug, Clone)]
pub struct ClaudeRunner {
    binary: PathBuf,
    env_overrides: Vec<(String, String)>,
}

impl ClaudeRunner {
    pub fn new() -> Self;
    pub fn with_binary(binary: impl Into<PathBuf>) -> Self;
    pub fn with_env_var(mut self, key: impl Into<String>, value: impl Into<String>) -> Self;
}
```

The `ClaudeRunner::run` implementation:
1. Performs a host-managed-mode pre-flight check (refuses when `CLAUDECODE=1`)
2. Emits progress via `coral_core::progress!` macro
3. Spawns `claude --print --output-format json -- <user_prompt>`
4. Parses the JSON envelope for `result` (text) and `usage` (token counts)
5. Handles timeout via poll-based `try_wait` loop
6. Scrubs secrets from error output before surfacing

## Error Types

```rust
// from crates/coral-runner/src/runner.rs
#[derive(Debug, Error)]
pub enum RunnerError {
    #[error("runner binary not found in PATH...")]
    NotFound,
    #[error("runner not authenticated...")]
    AuthFailed(String),
    #[error("runner exited with code {code:?}: {stderr}")]
    NonZeroExit { code: Option<i32>, stderr: String },
    #[error("runner invocation timed out after {0:?}")]
    Timeout(Duration),
    #[error("io error invoking runner: {0}")]
    Io(#[from] std::io::Error),
    #[error("tiered run aborted: cumulative token estimate {actual} would exceed budget {budget}...")]
    BudgetExceeded { actual: u64, budget: u64 },
}
```

## Module Exports

```rust
// from crates/coral-runner/src/lib.rs
pub use anthropic_api::AnthropicApiRunner;
pub use gemini::GeminiRunner;
pub use host_sampling::{HostSamplingRunner, host_sampling_available, host_sampling_explicit};
pub use http::HttpRunner;
pub use local::LocalRunner;
pub use mock::MockRunner;
pub use multi_step::{
    BudgetConfig, DEFAULT_MAX_TOKENS_PER_RUN, MultiStepRunner,
    TierSpec, TieredConfig, TieredOutput, TieredRunner, approx_tokens,
};
pub use prompt::PromptBuilder;
pub use runner::{ClaudeRunner, Prompt, RunOutput, Runner, RunnerError, RunnerResult, TokenUsage};
```

## Security Features

- **Secret scrubbing** (`scrub_secrets`): redacts `Authorization: Bearer`, `x-api-key:`, and bare `Bearer` tokens from error messages
- **Double-dash injection prevention**: inserts `--` before user prompt to prevent flag injection (CVE-2017-1000117 pattern)
- **Host-managed mode gate**: refuses to run real `claude` binary inside Claude Code shells where it cannot authenticate

## Streaming Architecture

```rust
// from crates/coral-runner/src/runner.rs
pub(crate) fn run_streaming_command(
    mut cmd: Command,
    timeout: Option<Duration>,
    on_chunk: &mut dyn FnMut(&str),
) -> RunnerResult<RunOutput> {
    // Spawns child, pipes stdout to a BufReader in a worker thread,
    // delivers lines via mpsc channel with recv_timeout for deadline.
    // Auth/non-zero-exit handling matches the non-streaming path.
}
```

## Token Usage Parsing

```rust
// from crates/coral-runner/src/runner.rs
pub(crate) fn parse_usage_from_stdout(stdout: &str) -> (Option<TokenUsage>, Option<String>) {
    // Tries two shapes:
    // 1. Anthropic JSON envelope: {"result":"<text>", "usage":{...}}
    // 2. OpenAI-compat: {"choices":[...], "usage":{...}}
    // Accepts field aliases: input_tokens/prompt_tokens/inputTokens
}
```

## Cross-Crate Relationships

- [[core]] provides `observability::progress!` macros used for user-facing feedback
- [[cli]] creates runners via `runner_helper::make_runner()` based on `--provider` flag
- [[mcp]] uses `HostSamplingRunner` for MCP sampling bridge
- [[session]] uses `Runner` for the `distill` step (LLM summarization)
