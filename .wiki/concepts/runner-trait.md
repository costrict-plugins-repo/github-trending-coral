---
slug: runner-trait
type: concept
last_updated_commit: 19da4d6
confidence: 0.95
sources:
- crates/coral-runner/src/runner.rs
- crates/coral-runner/Cargo.toml
backlinks:
- mcp-sampling
- cost-estimation
- observability
status: draft
---

# Runner Trait

The `Runner` trait is the central abstraction for LLM invocation in Coral. Every provider (Claude CLI, Gemini, local llama.cpp, HTTP-compatible endpoints, host-sampling) implements this trait, giving the rest of the codebase a uniform interface to spawn prompts and collect results.

## Trait Definition

```rust
// crates/coral-runner/src/runner.rs

pub trait Runner: Send + Sync {
    /// Execute a prompt and return the captured output.
    /// Implementations must NOT panic on bad input — return RunnerError instead.
    fn run(&self, prompt: &Prompt) -> RunnerResult<RunOutput>;

    /// Streaming variant: invokes `on_chunk` with each newly-emitted chunk
    /// from the underlying provider, then returns the full accumulated output.
    ///
    /// The default implementation falls back to `run()` and emits a single
    /// chunk with the full stdout — fine for mocks and tests.
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

The trait is `Send + Sync` so runners can be shared across threads (e.g. the MCP server holding a runner while serving concurrent tool calls).

## Prompt Structure

```rust
// crates/coral-runner/src/runner.rs

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Prompt {
    /// Optional system prompt passed via `--append-system-prompt`.
    pub system: Option<String>,
    /// The user prompt — what the LLM sees as the input.
    pub user: String,
    /// Optional model alias (e.g., "sonnet", "haiku", or a full model id).
    pub model: Option<String>,
    /// Optional working directory for the runner process.
    pub cwd: Option<PathBuf>,
    /// Optional max wall-clock seconds before the process is killed.
    pub timeout: Option<Duration>,
}
```

## RunOutput and Token Usage

```rust
// crates/coral-runner/src/runner.rs

#[derive(Debug, Clone, PartialEq)]
pub struct RunOutput {
    pub stdout: String,
    pub stderr: String,
    pub duration: Duration,
    /// Real token usage if the underlying provider reported it.
    /// `None` when the runner cannot extract it (e.g. LocalRunner streaming)
    /// or when the caller didn't opt into structured-output mode.
    pub usage: Option<TokenUsage>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Cache-read tokens (Anthropic: charged at 0.1x base rate).
    pub cache_read_tokens: u64,
    /// Cache-write tokens (Anthropic: charged at 1.25x base rate).
    pub cache_write_tokens: u64,
}
```

`TokenUsage` (v0.34.0) enables real mid-flight cost tracking during `coral bootstrap --max-cost`. The `add` method sums fields with saturating arithmetic, used by `MultiStepRunner` to roll up per-tier usage.

## Error Taxonomy

```rust
// crates/coral-runner/src/runner.rs

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

Each variant carries actionable diagnostics. `NotFound` links to install docs for each provider. `AuthFailed` includes the provider response for debugging. `BudgetExceeded` (v0.21.4) is raised by `MultiStepRunner` when a tiered run would exceed `max_tokens_per_run`.

## ClaudeRunner Implementation

The primary implementation spawns `claude --print --output-format json` as a subprocess:

```rust
// crates/coral-runner/src/runner.rs (simplified)

impl Runner for ClaudeRunner {
    fn run(&self, prompt: &Prompt) -> RunnerResult<RunOutput> {
        self.host_managed_mode_check()?;
        let mut cmd = Command::new(&self.binary);
        self.apply_env_overrides(&mut cmd);
        cmd.arg("--print").arg("--output-format").arg("json");
        if let Some(system) = &prompt.system {
            cmd.arg("--append-system-prompt").arg(system);
        }
        if let Some(model) = &prompt.model {
            cmd.arg("--model").arg(model);
        }
        // v0.30.0 audit B9: prevent user input from being parsed as flags
        cmd.arg("--").arg(&prompt.user);
        // ... spawn, wait, parse usage from JSON envelope ...
    }
}
```

Key features:
- **Argument injection prevention** (v0.30.0): inserts `--` before user prompt so `prompt.user` values starting with `--` cannot be parsed as CLI flags.
- **Secret scrubbing** (v0.19.5): strips bearer tokens and API keys from error output using regex before surfacing to the user.
- **Host-managed mode check** (v0.40.1): detects `CLAUDECODE=1` env and refuses to run, since the `claude` CLI cannot authenticate from inside a Claude Code shell due to macOS Endpoint Security.
- **Env overrides**: `with_env_var` injects per-spawn environment variables (e.g. `ANTHROPIC_API_KEY`) scoped to the child process only.

## Usage Parsing

`parse_usage_from_stdout` handles two JSON shapes:

```rust
// crates/coral-runner/src/runner.rs

pub(crate) fn parse_usage_from_stdout(stdout: &str) -> (Option<TokenUsage>, Option<String>) {
    // Anthropic: {"result":"<text>", "usage":{input_tokens, output_tokens, cache_*}}
    // OpenAI-compat: {"usage":{prompt_tokens, completion_tokens}}
    // Field aliases accepted: input_tokens / prompt_tokens / inputTokens, etc.
}
```

Returns `(usage, inner_text)` where `inner_text` is the unwrapped `result` field from the Claude JSON envelope so downstream callers still see plain prose.

## Streaming Architecture

The shared `run_streaming_command` helper spawns the process, reads stdout line-by-line in a worker thread via `mpsc::channel`, invokes `on_chunk` for each line, and enforces timeouts via `recv_timeout`. Auth detection and secret scrubbing are applied identically to the non-streaming path.

## Integration with Observability

All runner implementations emit [[observability]] `progress!` macros: `step` before spawn, `done` after response, with annotations like `model`, `tokens`, `elapsed_ms`. When `CORAL_VERBOSE=1`, prompt and response bodies are truncated-dumped to stderr.

## See Also

- [[cost-estimation]] -- consumes `TokenUsage` for USD projections
- [[mcp-sampling]] -- alternative runner path using the MCP sampling bridge
- [[observability]] -- `progress!` macro emitted before/after runner calls
