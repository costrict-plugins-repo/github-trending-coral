//! v0.41 — Direct Anthropic Messages API runner.
//!
//! Calls `https://api.anthropic.com/v1/messages` via curl, bypassing the
//! `claude` CLI binary entirely. This is the automatic default when Coral
//! runs inside Claude Code (`CLAUDECODE=1`) because the CLI binary is blocked
//! by macOS Endpoint Security host-managed-mode (BACKLOG #12 L3).
//!
//! # Why not `ClaudeRunner`?
//!
//! `ClaudeRunner` spawns the `claude` binary, which macOS Endpoint Security
//! blocks when the parent process is itself an ES-managed app (like Claude
//! Code). This runner sidesteps the issue by calling the HTTP API directly.
//!
//! # Authentication
//!
//! Requires `ANTHROPIC_API_KEY` (either from env or from
//! `[provider.anthropic].api_key` in `.coral/config.toml`).
//!
//! # Wire format (Anthropic Messages API 2023-06-01)
//!
//! ```json
//! POST https://api.anthropic.com/v1/messages
//! {
//!   "model": "claude-sonnet-4-5-20250514",
//!   "max_tokens": 4096,
//!   "system": "...",
//!   "messages": [{ "role": "user", "content": "..." }]
//! }
//! ```

use crate::body_tempfile::{TempFileGuard, body_tempfile_path, write_body_tempfile_secure};
use crate::runner::{
    Prompt, RunOutput, Runner, RunnerError, RunnerResult, TokenUsage, combine_outputs,
    is_auth_failure, scrub_secrets,
};
use serde::{Deserialize, Serialize};
use std::process::Command;
use std::time::Instant;

/// Default Anthropic API endpoint.
const ANTHROPIC_API_URL: &str = "https://api.anthropic.com/v1/messages";

/// Default model when `prompt.model` is None.
const DEFAULT_MODEL: &str = "claude-sonnet-4-5-20250514";

/// Default max_tokens for the Messages API.
const DEFAULT_MAX_TOKENS: u64 = 4096;

/// Anthropic API version header.
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Direct Anthropic Messages API runner.
///
/// Uses curl to POST to the Anthropic Messages API. Designed as the
/// automatic default inside Claude Code where the `claude` CLI binary
/// cannot be spawned.
#[derive(Debug, Clone)]
pub struct AnthropicApiRunner {
    api_key: String,
    endpoint: String,
    max_tokens: u64,
}

impl AnthropicApiRunner {
    /// Create a new runner with the given API key.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            endpoint: ANTHROPIC_API_URL.to_string(),
            max_tokens: DEFAULT_MAX_TOKENS,
        }
    }

    /// Override the endpoint (useful for testing or proxies).
    #[allow(dead_code)]
    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = endpoint.into();
        self
    }

    /// Override max_tokens.
    #[allow(dead_code)]
    pub fn with_max_tokens(mut self, max_tokens: u64) -> Self {
        self.max_tokens = max_tokens;
        self
    }
}

// --- Wire types -----------------------------------------------------------------

/// Outgoing request body (Anthropic Messages format).
#[derive(Debug, Serialize)]
struct MessagesRequest<'a> {
    model: &'a str,
    max_tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<&'a str>,
    messages: Vec<ApiMessage<'a>>,
}

#[derive(Debug, Serialize)]
struct ApiMessage<'a> {
    role: &'a str,
    content: &'a str,
}

/// Incoming response body (Anthropic Messages format).
#[derive(Debug, Deserialize)]
struct MessagesResponse {
    content: Vec<ContentBlock>,
    #[serde(default)]
    usage: Option<ApiUsage>,
    #[allow(dead_code)]
    model: Option<String>,
    #[allow(dead_code)]
    stop_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ContentBlock {
    #[allow(dead_code)]
    #[serde(rename = "type")]
    content_type: Option<String>,
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ApiUsage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
    #[serde(default)]
    cache_read_input_tokens: u64,
    #[serde(default)]
    cache_creation_input_tokens: u64,
}

/// Build the request payload for the Anthropic Messages API.
pub(crate) fn build_anthropic_payload(
    prompt: &Prompt,
    max_tokens: u64,
) -> Result<String, serde_json::Error> {
    let model = prompt.model.as_deref().unwrap_or(DEFAULT_MODEL);
    let req = MessagesRequest {
        model,
        max_tokens,
        system: prompt.system.as_deref().filter(|s| !s.is_empty()),
        messages: vec![ApiMessage {
            role: "user",
            content: &prompt.user,
        }],
    };
    serde_json::to_string(&req)
}

impl Runner for AnthropicApiRunner {
    fn run(&self, prompt: &Prompt) -> RunnerResult<RunOutput> {
        let start = Instant::now();
        let body = build_anthropic_payload(prompt, self.max_tokens).map_err(|e| {
            RunnerError::Io(std::io::Error::other(format!("serializing request: {e}")))
        })?;

        tracing::debug!(
            endpoint = %self.endpoint,
            model = ?prompt.model,
            "POST anthropic messages"
        );

        // Write body to tempfile (API key goes via stdin for H6 hardening).
        let body_path = body_tempfile_path("coral-anthropic-body");
        write_body_tempfile_secure(&body_path, body.as_bytes()).map_err(|e| {
            RunnerError::Io(std::io::Error::other(format!(
                "writing request body to {}: {e}",
                body_path.display()
            )))
        })?;
        let _body_guard = TempFileGuard::new(Some(body_path.clone()));

        let mut cmd = Command::new("curl");
        cmd.args([
            "-s",
            "--fail-with-body",
            "-X",
            "POST",
            self.endpoint.as_str(),
            "-H",
            "Content-Type: application/json",
            "-H",
            &format!("anthropic-version: {ANTHROPIC_VERSION}"),
            // Auth header via stdin (H6 hardening: key never in argv)
            "-H",
            "@-",
            "--data-binary",
            &format!("@{}", body_path.display()),
        ]);

        if let Some(t) = prompt.timeout {
            let secs = (t.as_secs_f64()).max(1.0);
            cmd.args(["--max-time", &format!("{secs:.0}")]);
        }

        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(RunnerError::NotFound);
            }
            Err(e) => {
                return Err(RunnerError::Io(e));
            }
        };

        // Write API key via stdin so it never appears in argv.
        if let Some(mut stdin) = child.stdin.take() {
            use std::io::Write;
            let header_line = format!("x-api-key: {}\n", self.api_key);
            stdin.write_all(header_line.as_bytes()).map_err(|e| {
                RunnerError::Io(std::io::Error::other(format!(
                    "writing auth header to curl stdin: {e}"
                )))
            })?;
            drop(stdin);
        }

        let output = child.wait_with_output().map_err(RunnerError::Io)?;
        let duration = start.elapsed();

        if !output.status.success() {
            let stdout_str = String::from_utf8_lossy(&output.stdout);
            let stderr_str = String::from_utf8_lossy(&output.stderr);
            let combined = combine_outputs(&stdout_str, &stderr_str);
            let scrubbed = scrub_secrets(&combined);
            if is_auth_failure(&scrubbed) {
                return Err(RunnerError::AuthFailed(scrubbed));
            }
            return Err(RunnerError::NonZeroExit {
                code: output.status.code(),
                stderr: scrubbed,
            });
        }

        let parsed: MessagesResponse = serde_json::from_slice(&output.stdout).map_err(|e| {
            RunnerError::Io(std::io::Error::other(format!(
                "parsing Anthropic Messages response: {e}; body={}",
                String::from_utf8_lossy(&output.stdout)
            )))
        })?;

        let usage = parsed.usage.as_ref().map(|u| TokenUsage {
            input_tokens: u.input_tokens,
            output_tokens: u.output_tokens,
            cache_read_tokens: u.cache_read_input_tokens,
            cache_write_tokens: u.cache_creation_input_tokens,
        });

        // Concatenate all text content blocks.
        let content: String = parsed
            .content
            .into_iter()
            .filter_map(|block| block.text)
            .collect::<Vec<_>>()
            .join("");

        if content.is_empty() {
            return Err(RunnerError::Io(std::io::Error::other(
                "Anthropic Messages response had no text content blocks",
            )));
        }

        Ok(RunOutput {
            stdout: content,
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            duration,
            usage,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_payload_with_system_and_model() {
        let prompt = Prompt {
            system: Some("You are a wiki writer.".into()),
            user: "Explain auth.".into(),
            model: Some("claude-opus-4-6-20260601".into()),
            ..Default::default()
        };
        let body = build_anthropic_payload(&prompt, 4096).expect("serialize");
        let parsed: serde_json::Value = serde_json::from_str(&body).expect("parse");
        assert_eq!(parsed["model"], "claude-opus-4-6-20260601");
        assert_eq!(parsed["max_tokens"], 4096);
        assert_eq!(parsed["system"], "You are a wiki writer.");
        assert_eq!(parsed["messages"][0]["role"], "user");
        assert_eq!(parsed["messages"][0]["content"], "Explain auth.");
    }

    #[test]
    fn build_payload_omits_system_when_none() {
        let prompt = Prompt {
            system: None,
            user: "ping".into(),
            model: None,
            ..Default::default()
        };
        let body = build_anthropic_payload(&prompt, 8192).expect("serialize");
        let parsed: serde_json::Value = serde_json::from_str(&body).expect("parse");
        assert_eq!(parsed["model"], DEFAULT_MODEL);
        assert_eq!(parsed["max_tokens"], 8192);
        assert!(parsed.get("system").is_none() || parsed["system"].is_null());
    }

    #[test]
    fn build_payload_uses_default_model_when_none() {
        let prompt = Prompt {
            user: "test".into(),
            ..Default::default()
        };
        let body = build_anthropic_payload(&prompt, 4096).expect("serialize");
        assert!(body.contains(DEFAULT_MODEL));
    }

    #[test]
    fn runner_creation() {
        let runner = AnthropicApiRunner::new("sk-test-key");
        assert_eq!(runner.endpoint, ANTHROPIC_API_URL);
        assert_eq!(runner.max_tokens, DEFAULT_MAX_TOKENS);
    }

    #[test]
    fn runner_with_custom_endpoint() {
        let runner = AnthropicApiRunner::new("sk-test")
            .with_endpoint("http://localhost:8080/v1/messages")
            .with_max_tokens(8192);
        assert_eq!(runner.endpoint, "http://localhost:8080/v1/messages");
        assert_eq!(runner.max_tokens, 8192);
    }
}
