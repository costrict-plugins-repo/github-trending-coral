---
slug: frontmatter
type: concept
last_updated_commit: 19da4d6
confidence: 0.95
sources:
- crates/coral-core/src/frontmatter.rs
backlinks:
- wikilinks
- search
- runner-trait
status: draft
---

# Frontmatter

Every Coral wiki page begins with a YAML frontmatter block delimited by `---` fences. This block carries structured metadata that powers indexing, search ranking, lifecycle management, and bi-temporal queries. The `frontmatter` module in `coral-core` handles parsing, validation, and serialization.

## Schema

```rust
// crates/coral-core/src/frontmatter.rs

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Frontmatter {
    pub slug: String,
    #[serde(rename = "type")]
    pub page_type: PageType,
    pub last_updated_commit: String,
    pub confidence: Confidence,
    #[serde(default)]
    pub sources: Vec<String>,
    #[serde(default)]
    pub backlinks: Vec<String>,
    pub status: Status,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generated_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_to: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<String>,
    /// Catch-all for additional fields (BTreeMap for deterministic ordering)
    #[serde(flatten, default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, serde_yaml_ng::Value>,
}
```

## Page Types

The `PageType` enum determines which subdirectory a page lives in under `.wiki/`:

```rust
// crates/coral-core/src/frontmatter.rs

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PageType {
    Module, Concept, Entity, Flow, Decision, Synthesis,
    Operation, Source, Gap, Index, Log, Schema, Readme,
    Reference, Interface,
}
```

Each type maps to a specific content purpose: `Module` for code modules, `Concept` for architectural ideas, `Entity` for domain objects, `Flow` for process sequences, `Decision` for ADRs, `Interface` (v0.24) for API contracts linked to `.coral/contracts/`.

## Status Lifecycle

```rust
// crates/coral-core/src/frontmatter.rs

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Draft, Reviewed, Verified, Stale, Archived, Reference,
}
```

Pages progress from `Draft` (LLM-generated) through `Reviewed` (human-approved) to `Verified` (tested against source). `Stale` marks pages whose source files changed since last update.

## Confidence Score

```rust
// crates/coral-core/src/frontmatter.rs

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct Confidence(f64);

impl Confidence {
    pub fn try_new(v: f64) -> Result<Self> {
        if !v.is_finite() || !(0.0..=1.0).contains(&v) {
            return Err(CoralError::InvalidConfidence(v));
        }
        Ok(Self(v))
    }

    pub fn as_f64(self) -> f64 {
        self.0
    }
}
```

The `Confidence` newtype enforces the `[0.0, 1.0]` invariant at parse time. Values reflect reliability: `0.95` for direct source reading, `0.85` for inferred content. Invalid values (NaN, infinity, out-of-range) produce `CoralError::InvalidConfidence`.

## Bi-Temporal Fields (M2.16)

Three fields enable time-travel queries ("what did the wiki say about X at time T?"):

- `valid_from`: ISO-8601 timestamp when content became valid (inclusive)
- `valid_to`: ISO-8601 timestamp when content ceased to be valid (exclusive)
- `superseded_by`: slug of the replacement page

```rust
// crates/coral-core/src/frontmatter.rs

impl Frontmatter {
    pub fn is_current(&self) -> bool {
        self.valid_to.is_none() && self.superseded_by.is_none()
    }

    pub fn is_valid_at(&self, at: &str) -> bool {
        let after_from = match &self.valid_from {
            Some(from) => at >= from.as_str(),
            None => true,
        };
        let before_to = match &self.valid_to {
            Some(to) => at < to.as_str(),
            None => true,
        };
        after_from && before_to
    }
}
```

## Parsing and Serialization

```rust
// crates/coral-core/src/frontmatter.rs

/// Parses a Markdown document with YAML frontmatter.
/// Returns (Frontmatter, body) where body is the text AFTER the closing `---`.
pub fn parse(content: &str, path: impl Into<PathBuf>) -> Result<(Frontmatter, String)> { ... }

/// Serializes a Frontmatter + body back into a complete Markdown document.
/// Output format: `---\n{yaml}---\n\n{body}`
pub fn serialize(fm: &Frontmatter, body: &str) -> Result<String> {
    let yaml = serde_yaml_ng::to_string(fm)?;
    let mut out = String::with_capacity(yaml.len() + body.len() + 16);
    out.push_str("---\n");
    out.push_str(&yaml);
    out.push_str("---\n\n");
    out.push_str(body);
    Ok(out)
}
```

The parser validates the opening `---`, collects YAML lines until the closing `---`, and strips one canonical blank line between the fence and body. Errors: `MissingFrontmatter` (no opener), `UnterminatedFrontmatter` (no closing fence).

## Fast Path

`body_after_frontmatter(content: &str) -> String` skips YAML deserialization entirely when only the body text is needed (used by the walk cache). Handles both LF and CRLF line endings (v0.19.6 audit N3 fix for Windows-authored pages).

## Extra Fields

The `#[serde(flatten)]` catch-all `extra: BTreeMap<String, Value>` allows arbitrary additional YAML keys without schema changes. Supports per-project extensions (e.g. `audit: legal`, `priority: high`) while keeping the core schema stable. Empty `extra` maps are omitted from serialized output.

## Example YAML

```yaml
---
slug: order
type: module
last_updated_commit: abc123
confidence: 0.95
status: verified
sources:
  - src/order.rs
  - src/order_state.rs
backlinks:
  - flows/checkout
generated_at: 2026-04-30T10:00:00Z
valid_from: 2024-01-01T00:00:00Z
---
```

## See Also

- [[wikilinks]] -- the `backlinks` field is populated from reverse wikilink extraction
- [[search]] -- confidence and page type influence adaptive search routing
