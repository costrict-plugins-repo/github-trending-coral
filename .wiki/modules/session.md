---
slug: session
type: module
last_updated_commit: 19da4d6
confidence: 0.95
sources:
- crates/coral-session/src/lib.rs
backlinks:
- cli
- runner
- core
status: draft
---

# session

Agent transcript capture, distillation, and housekeeping. `coral-session` implements the `coral session` subcommand family for importing agent conversations into the wiki via LLM-powered summarization with trust-by-curation guardrails.

## Responsibilities

- **Capture**: Copy raw transcripts from agent clients into `.coral/sessions/`
- **Distill**: Single-pass LLM summarization into wiki synthesis pages (`reviewed: false`)
- **Distill-as-patch**: Alternative distillation mode producing structured patch sidecars
- **List / Forget / Show**: Local housekeeping (no LLM)
- **Privacy scrubbing**: Redact sensitive data from transcripts by default

## High-Level Flow

1. `coral session capture` copies a raw transcript from the agent's local store into `.coral/sessions/<date>_<source>_<sha8>.jsonl`
2. `coral session distill` runs a single-pass `Runner::run` against the transcript, emitting a YAML synthesis page with `reviewed: false` frontmatter
3. The [[lint]] trust-by-curation gate blocks any commit containing `reviewed: false` pages
4. A human reviews the distilled page and flips `reviewed` to `true` before commit

## Module Exports

```rust
// from crates/coral-session/src/lib.rs
pub use capture::{CaptureOptions, CaptureOutcome, CaptureSource, capture_from_path};
pub use claude_code::{ClaudeCodeMessage, ClaudeCodeRecord, find_latest_for_cwd};
pub use distill::{DistillOptions, DistillOutcome, distill_session};
pub use distill_patch::{
    DistillPatchOptions, DistillPatchOutcome, Patch, PatchSidecar, distill_patch_session,
};
pub use error::{SessionError, SessionResult};
pub use forget::{ForgetOptions, forget_session};
pub use list::{ListFormat, SessionEntry, list_sessions};
pub use scrub::{Redaction, RedactionKind, scrub};
```

## Submodules

| Module | Purpose |
|--------|---------|
| `capture` | Copy transcripts into `.coral/sessions/`, apply privacy scrubber |
| `claude_code` | Parse Claude Code's local JSONL format, find latest session for cwd |
| `distill` | LLM summarization into synthesis wiki pages |
| `distill_patch` | Alternative patch-based distillation (v0.21.3 option b) |
| `error` | `SessionError` / `SessionResult` types |
| `forget` | Delete captured sessions from disk |
| `list` | Enumerate sessions with metadata |
| `scrub` | Privacy redaction engine |

## Capture Sources

```rust
// from crates/coral-session/src/lib.rs
pub use capture::CaptureSource;
```

Currently supports Claude Code (`ClaudeCodeRecord` / `ClaudeCodeMessage`); Cursor and ChatGPT are tracked in issue #16.

## Privacy Posture

- Sessions live under `.coral/sessions/` (added to `.gitignore` by `coral init`)
- Distilled output under `.coral/sessions/distilled/` is also gitignored
- Privacy scrubber runs by default; opt-out requires `--no-scrub --yes-i-really-mean-it` (two-flag safety combo)
- The `scrub` module produces `Redaction` records with `RedactionKind` categorization

## Distill-as-Patch (v0.21.3)

A separate module (`distill_patch`) kept disjoint from the original `distill` to maintain byte-identical backward compatibility:

```rust
// from crates/coral-session/src/lib.rs
pub use distill_patch::{
    DistillPatchOptions, DistillPatchOutcome, Patch, PatchSidecar, distill_patch_session,
};
```

## Cross-Crate Relationships

- [[runner]] provides the `Runner` trait used for LLM distillation
- [[core]] provides `Page` and `Frontmatter` for output pages
- [[cli]] wraps this crate in `commands::session`
- [[lint]] enforces `check_unreviewed_distilled` to gate distilled output
