---
slug: prompt
type: entity
last_updated_commit: 19da4d6
confidence: 0.95
sources:
- crates/coral-runner/src/prompt.rs
- crates/coral-runner/src/runner.rs
backlinks:
- runner-trait
- runner
- bootstrap-flow
- session-distill
- cost-estimation
status: draft
---

# PromptBuilder

The `PromptBuilder` struct is responsible for constructing final prompt strings from templates with variable substitution. It uses Handlebars-style `{{name}}` placeholders, providing a simple but effective templating system for the LLM runner pipeline.

## Struct definition

```rust
// crates/coral-runner/src/prompt.rs

/// Builds a final prompt string from a template + variable bindings.
/// Variables in the template have the form `{{name}}` (Handlebars-like).
/// Unknown variables are kept as-is in the output (useful for partial fills).
pub struct PromptBuilder {
    template: String,
    vars: BTreeMap<String, String>,
}
```

The struct uses a `BTreeMap` for deterministic variable ordering, ensuring reproducible prompt output regardless of insertion order.

## Template syntax

Variables follow the pattern `{{name}}` where `name` matches `[a-zA-Z_][a-zA-Z0-9_-]*`. Whitespace inside braces is tolerated: `{{ name }}` is equivalent to `{{name}}`.

The regex used for variable detection:

```rust
// crates/coral-runner/src/prompt.rs

static VAR_RE: OnceLock<Regex> = OnceLock::new();
let re = VAR_RE.get_or_init(||
    Regex::new(r"\{\{\s*([a-zA-Z_][a-zA-Z0-9_-]*)\s*\}\}").unwrap()
);
```

The regex is compiled once (via `OnceLock`) and reused across all renders for performance.

## Key methods

### Construction

```rust
// crates/coral-runner/src/prompt.rs

impl PromptBuilder {
    pub fn new(template: impl Into<String>) -> Self {
        Self {
            template: template.into(),
            vars: BTreeMap::new(),
        }
    }
}
```

### Variable binding (builder pattern)

```rust
// crates/coral-runner/src/prompt.rs

impl PromptBuilder {
    /// Sets `{{name}}` → `value`. Returns self for chaining.
    pub fn var(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.vars.insert(name.into(), value.into());
        self
    }
}
```

Fluent chaining example:

```rust
let prompt = PromptBuilder::new("Describe {{entity}} in {{repo}}")
    .var("entity", "Page")
    .var("repo", "coral")
    .render();
// => "Describe Page in coral"
```

### Rendering

```rust
// crates/coral-runner/src/prompt.rs

impl PromptBuilder {
    /// Renders the template, substituting all known variables.
    /// Unknown variables remain literal in the output (e.g., `{{unknown}}`).
    pub fn render(&self) -> String {
        static VAR_RE: OnceLock<Regex> = OnceLock::new();
        let re = VAR_RE.get_or_init(||
            Regex::new(r"\{\{\s*([a-zA-Z_][a-zA-Z0-9_-]*)\s*\}\}").unwrap()
        );
        re.replace_all(&self.template, |caps: &regex::Captures| -> String {
            let name = &caps[1];
            self.vars
                .get(name)
                .cloned()
                .unwrap_or_else(|| caps[0].to_string())
        })
        .into_owned()
    }
}
```

## Design decisions

### Partial fill semantics

Unknown variables are preserved as literal `{{name}}` in the output rather than being replaced with empty strings or causing errors. This enables **partial fills** — a template can be rendered in multiple passes, with different variable sets bound at each stage of the pipeline.

### Variable name rules

- Must start with a letter or underscore: `[a-zA-Z_]`
- May contain letters, digits, underscores, and hyphens: `[a-zA-Z0-9_-]*`
- Examples: `{{name}}`, `{{page_type}}`, `{{repo-name}}`, `{{_internal}}`

### No escaping mechanism

The implementation does not provide a way to emit a literal `{{name}}` in output if `name` is a bound variable. This is intentional — templates are authored by the tool's internal prompts and do not need user-facing escape hatches.

### Static regex compilation

The `OnceLock<Regex>` pattern ensures the regex is compiled exactly once across all `render()` invocations, even across threads. The `#[allow(clippy::unwrap_used)]` annotation is guarded by unit tests that validate the regex literal.

## Usage in the runner pipeline

The `PromptBuilder` is used by the runner (see [[runner]]) to assemble prompts sent to the LLM. It is the template layer that sits between raw prompt text (loaded from configuration or hardcoded) and the final string passed to the [[runner-trait]] implementations.

Typical variables include:

| Variable | Purpose |
|----------|---------|
| `source_code` | Raw source file content to analyze |
| `existing_page` | Current wiki page content (for updates) |
| `slug` | Target page slug |
| `page_type` | Target page type (entity, module, etc.) |
| `repo_context` | Repository-level context from the wiki |

## Relationships

- Used by the [[runner]] to generate [[page]] content
- Templates are loaded by the runner configuration and fed into the builder
- Output is consumed by `Page::from_content` for parsing and disk persistence
- Part of the [[bootstrap-flow]] and [[session-distill]] pipelines
- Prompt size contributes to [[cost-estimation]] calculations
