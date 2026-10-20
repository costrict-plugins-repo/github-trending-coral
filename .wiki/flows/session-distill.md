---
slug: session-distill
type: flow
last_updated_commit: 19da4d6
confidence: 0.93
sources:
  - crates/coral-session/src/distill.rs
  - crates/coral-session/src/capture.rs
  - crates/coral-session/src/scrub.rs
backlinks:
  - runner-trait
  - adr-multi-runner
  - mcp-transport
status: draft
---

# Session Distill

Session distillation captures raw agent transcripts and distills them into wiki-ready synthesis pages. The flow extracts key decisions, code changes, and learnings from conversations with AI agents while scrubbing PII.

## Constants

```rust
// crates/coral-session/src/distill.rs

/// Versioned to bump when the prompt template changes meaningfully.
pub const DISTILL_PROMPT_VERSION: u32 = 1;

/// Hard cap on findings per session. The prompt asks for 1-3; we
/// clamp at 3 in case the model exceeds.
const MAX_FINDINGS_PER_SESSION: usize = 3;
```

## Core Types

```rust
// crates/coral-session/src/distill.rs

#[derive(Debug, Clone)]
pub struct DistillOptions {
    pub project_root: PathBuf,
    pub session_id: String,
    pub apply: bool,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub slug: String,
    pub title: String,
    pub body: String,
    #[serde(default)]
    pub sources: Vec<String>,
}
```

## End-to-End Flow (distill_session)

```rust
// crates/coral-session/src/distill.rs

pub fn distill_session(
    opts: &DistillOptions,
    runner: &dyn Runner,
    runner_name: &str,
) -> SessionResult<DistillOutcome> {
    let sessions_dir = opts.project_root.join(".coral").join("sessions");
    let index_path = sessions_dir.join("index.json");
    let index = read_index(&index_path)?;
    let entry = resolve_entry(&index, &opts.session_id)?;
    let parsed = parse_transcript(&entry.captured_path)?;

    let prompt = Prompt {
        system: None,
        user: build_prompt(&parsed.messages),
        model: opts.model.clone(),
        cwd: None,
        timeout: None,
    };
    let out = runner.run(&prompt)?;
    let findings = parse_findings(&out.stdout)?;
    // ...
}
```

## Prompt Construction (build_prompt)

Per-message text is truncated to 2000 chars; total across all messages is capped at ~80k chars:

```rust
// crates/coral-session/src/distill.rs

pub fn build_prompt(messages: &[ClaudeCodeMessage]) -> String {
    const PER_MSG_CHARS: usize = 2_000;
    const TOTAL_CHARS: usize = 80_000;

    let mut convo = String::new();
    let mut spent = 0usize;
    for m in messages {
        let snippet: String = m.text.chars().take(PER_MSG_CHARS).collect();
        let chunk = format!("[{}] {}\n", m.role, snippet);
        if spent + chunk.len() > TOTAL_CHARS {
            convo.push_str("[...truncated for prompt budget...]\n");
            break;
        }
        spent += chunk.len();
        convo.push_str(&chunk);
    }
    // ...wraps with distillation instructions...
}
```

## Findings Parser

Strict slug validation rejects entries with unsafe slugs early so we never write outside the synthesis directory:

```rust
// crates/coral-session/src/distill.rs

pub fn parse_findings(stdout: &str) -> SessionResult<Vec<Finding>> {
    let body = strip_fences(stdout);
    let parsed: DistillerYaml = serde_yaml_ng::from_str(body).map_err(|e| {
        SessionError::DistillMalformed(format!("YAML parse failed: {e}; output was: {stdout}"))
    })?;
    let mut out: Vec<Finding> = Vec::new();
    for (idx, f) in parsed.findings.into_iter().enumerate() {
        if !coral_core::slug::is_safe_filename_slug(&f.slug) {
            return Err(SessionError::DistillMalformed(format!(
                "finding[{idx}].slug '{}' is not a safe filename slug", f.slug
            )));
        }
        // ...validate title/body non-empty...
        out.push(f);
    }
    out.truncate(MAX_FINDINGS_PER_SESSION);
    Ok(out)
}
```

## Trust-by-Curation Gate (render_page)

```rust
// crates/coral-session/src/distill.rs

pub fn render_page(finding: &Finding, runner_name: &str, session_id: &str, captured_at: &str) -> String {
    let mut out = String::new();
    out.push_str("---\n");
    out.push_str(&format!("slug: {}\n", finding.slug));
    out.push_str("type: synthesis\n");
    // ...
    // Trust-by-curation gate: distilled output ALWAYS lands as
    // `reviewed: false` and the lint check rejects committing a
    // page without flipping it to `true` after human review.
    out.push_str("reviewed: false\n");
    out.push_str("---\n\n");
    out.push_str(&format!("# {}\n\n", finding.title));
    out.push_str(&finding.body);
    out
}
```

## Output Locations

- Always: `.coral/sessions/distilled/<slug>.md` (local, gitignored)
- With `--apply`: additionally `.wiki/pages/synthesis/<slug>.md` (enters the wiki)

Both copies carry `reviewed: false`. The user must flip to `true` before commit; otherwise `coral lint` raises Critical and the pre-commit hook blocks it.

## Privacy (Scrub Module)

The scrubber covers Anthropic keys, OpenAI keys, GitHub tokens, AWS credentials, Slack tokens, GitLab PATs, JWTs, and Bearer headers. Opt-out requires `--no-scrub --yes-i-really-mean-it`. False positives are acceptable; false negatives are not.
