---
slug: core
type: module
last_updated_commit: 19da4d6
confidence: 0.95
sources:
- crates/coral-core/src/lib.rs
- crates/coral-core/src/frontmatter.rs
- crates/coral-core/src/page.rs
- crates/coral-core/src/wikilinks.rs
- crates/coral-core/src/error.rs
backlinks:
- runner
- cli
- mcp
- lint
- stats
status: draft
---

# core

The foundational crate for the Coral wiki system. `coral-core` defines the page model (frontmatter + body), YAML serialization, wikilink extraction, error types, and all shared data structures consumed by every other crate in the workspace.

## Responsibilities

- Parse and serialize YAML frontmatter for wiki pages
- Define the `Page` type with disk persistence (atomic writes)
- Extract `\[[wikilinks]]` from Markdown bodies (respecting fences and escapes)
- Provide shared error types (`CoralError`) used across the workspace
- Expose curated re-exports of internal modules (index, embeddings, cache, narrative, symbols, etc.)
- Host the observability/progress macro system (v0.41)

## Public Module Surface

The crate underwent a SemVer audit (v0.35 ARCH-C1) that demoted ~48% of modules from `pub` to `pub(crate)` with curated `pub use` re-exports at the crate root.

```rust
// from crates/coral-core/src/lib.rs
pub mod atomic;
pub mod auth;
pub mod config;
pub mod cost;
pub mod error;
pub mod frontmatter;
pub mod gitdiff;
pub mod governance;
pub mod log;
pub mod observability;
pub mod page;
pub mod path;
pub mod project;
pub mod search;
pub mod slug;
pub mod walk;
```

Internal modules are re-exported selectively:

```rust
// from crates/coral-core/src/lib.rs
pub use crate::index::{IndexEntry, WikiIndex};
pub use crate::cache::WalkCache;
pub use crate::embeddings::EmbeddingsIndex;
pub use crate::wikilinks::extract as wikilinks_extract;
pub use crate::narrative::{PageDiff, diff_wiki_states, generate_narrative};
pub use crate::symbols::{Symbol, SymbolKind, extract_from_dir, find_symbols_for_slug};
pub use crate::gc::{
    analyze as gc_analyze, render_json as gc_render_json,
    render_markdown as gc_render_markdown,
};
```

## Key Types

### Frontmatter

```rust
// from crates/coral-core/src/frontmatter.rs
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Frontmatter {
    pub slug: String,
    #[serde(rename = "type")]
    pub page_type: PageType,
    pub last_updated_commit: String,
    pub confidence: Confidence,
    pub sources: Vec<String>,
    pub backlinks: Vec<String>,
    pub status: Status,
    pub generated_at: Option<String>,
    pub valid_from: Option<String>,
    pub valid_to: Option<String>,
    pub superseded_by: Option<String>,
    #[serde(flatten, default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, serde_yaml_ng::Value>,
}
```

### PageType Enum

```rust
// from crates/coral-core/src/frontmatter.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PageType {
    Module, Concept, Entity, Flow, Decision, Synthesis,
    Operation, Source, Gap, Index, Log, Schema, Readme,
    Reference, Interface,
}
```

### Page

```rust
// from crates/coral-core/src/page.rs
#[derive(Debug, Clone, PartialEq)]
pub struct Page {
    pub path: PathBuf,
    pub frontmatter: Frontmatter,
    pub body: String,
}

impl Page {
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self>;
    pub fn from_content(content: &str, path: impl Into<PathBuf>) -> Result<Self>;
    pub fn to_string(&self) -> Result<String>;
    pub fn write(&self) -> Result<()>;
    pub fn outbound_links(&self) -> Vec<String>;
    pub fn bump_last_commit(&mut self, commit: impl Into<String>);
    pub fn add_backlink(&mut self, slug: impl Into<String>);
    pub fn is_unreviewed_distilled(&self) -> bool;
}
```

### Wikilink Extraction

```rust
// from crates/coral-core/src/wikilinks.rs
/// Extracts unique wikilinks from `content` in document order.
/// Skips code fences, backslash-escaped links, empty targets.
/// Handles alias ([[target|alias]]) and anchor ([[target#anchor]]).
pub fn extract(content: &str) -> Vec<String> {
    let re = wikilink_re();
    let mut out: Vec<String> = Vec::new();
    let mut seen: AHashSet<String> = AHashSet::new();
    let mut inside_fence = false;
    // iterates lines, toggles fences, regex-captures targets
    // deduplicates preserving document order
}
```

### Error Types

```rust
// from crates/coral-core/src/error.rs
#[derive(Debug, Error)]
pub enum CoralError {
    #[error("io error at {path}: {source}")]
    Io { path: PathBuf, source: std::io::Error },
    #[error("yaml parse error: {0}")]
    Yaml(#[from] serde_yaml_ng::Error),
    #[error("frontmatter missing -- expected leading `---` block in {path}")]
    MissingFrontmatter { path: PathBuf },
    #[error("frontmatter unterminated -- expected closing `---` in {path}")]
    UnterminatedFrontmatter { path: PathBuf },
    #[error("invalid confidence value {0}: must be in [0.0, 1.0]")]
    InvalidConfidence(f64),
    #[error("git error: {0}")]
    Git(String),
    #[error("sqlite error: {0}")]
    Sqlite(String),
    #[error("manifest error: {0}")]
    Manifest(String),
}

pub type Result<T> = std::result::Result<T, CoralError>;
```

## Bi-Temporal Support (M2.16)

Pages support `valid_from` / `valid_to` / `superseded_by` fields for time-travel queries:

```rust
// from crates/coral-core/src/frontmatter.rs
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

## Frontmatter Parsing

```rust
// from crates/coral-core/src/frontmatter.rs
pub fn parse(content: &str, path: impl Into<PathBuf>) -> Result<(Frontmatter, String)>;
pub fn body_after_frontmatter(content: &str) -> String;
pub fn serialize(fm: &Frontmatter, body: &str) -> Result<String>;
```

The parser handles both LF and CRLF line endings (v0.19.6 audit N3).

## Cross-Crate Relationships

- [[runner]] uses `coral_core::observability::progress!` and `progress_enabled()`
- [[cli]] depends on nearly every public API for command implementations
- [[mcp]] uses `Page`, `Frontmatter`, and the walk/search modules
- [[lint]] consumes `Page` for structural checks
- [[stats]] computes metrics from `Page` slices
