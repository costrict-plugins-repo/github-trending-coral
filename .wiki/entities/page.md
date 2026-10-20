---
slug: page
type: entity
last_updated_commit: 19da4d6
confidence: 0.95
sources:
- crates/coral-core/src/page.rs
- crates/coral-core/src/frontmatter.rs
backlinks:
- frontmatter
- wikilinks
- wiki-index
- bootstrap-flow
- ingest-cycle
status: draft
---

# Page

The `Page` struct is the fundamental unit of persistence in Coral's wiki system. Each page is a Markdown file consisting of YAML frontmatter metadata plus a freeform body, stored under `.wiki/` in the repository.

## Struct definition

```rust
// crates/coral-core/src/page.rs

/// A wiki page parsed from disk (or memory) — frontmatter + body + path.
#[derive(Debug, Clone, PartialEq)]
pub struct Page {
    pub path: PathBuf,
    pub frontmatter: Frontmatter,
    pub body: String,
}
```

The page delegates its metadata to [[frontmatter]], which carries all YAML fields:

```rust
// crates/coral-core/src/frontmatter.rs

/// Frontmatter — YAML metadata block at the top of every wiki page.
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
    #[serde(flatten, default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, serde_yaml_ng::Value>,
}
```

## Lifecycle states

Pages transition through these statuses (defined by the `Status` enum):

| Status | Meaning |
|--------|---------|
| `draft` | Initial LLM-generated or hand-written content, not yet reviewed |
| `reviewed` | A human has confirmed factual accuracy |
| `verified` | Content verified against source code (e.g., by CI) |
| `stale` | Marked outdated — source files have drifted |
| `archived` | Retired content, retained for history |
| `reference` | Externally sourced material, not expected to drift |

## Confidence scale

The `Confidence` newtype wraps an `f64` validated to `[0.0, 1.0]`:

```rust
// crates/coral-core/src/frontmatter.rs

/// Newtype for confidence values, validated to be in [0.0, 1.0].
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

Confidence levels in practice:
- **0.95** — directly read from source code
- **0.80** — inferred from tests or documentation
- **0.50-0.70** — LLM-distilled without human review

## Key methods

### Parsing and persistence

```rust
// crates/coral-core/src/page.rs

impl Page {
    /// Parses a page from disk. Returns `CoralError::Io` on read failure
    /// (path captured), `CoralError::MissingFrontmatter` / `UnterminatedFrontmatter`
    /// for malformed pages, `CoralError::Yaml` on YAML errors.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let content = fs::read_to_string(path).map_err(|source| CoralError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        Self::from_content(&content, path.to_path_buf())
    }

    /// Parses a page from raw content + a path (path used only for error reporting).
    pub fn from_content(content: &str, path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let (frontmatter, body) = parse_fm(content, path.clone())?;
        Ok(Self { path, frontmatter, body })
    }

    /// Serializes back to a Markdown string suitable for writing to disk.
    pub fn to_string(&self) -> Result<String> {
        serialize_fm(&self.frontmatter, &self.body)
    }

    /// Writes the page to its `path` atomically (temp-file + rename).
    /// Creates parent dirs if missing. Race-free against concurrent readers.
    pub fn write(&self) -> Result<()> {
        let content = self.to_string()?;
        crate::atomic::atomic_write_string(&self.path, &content)
    }
}
```

### Link graph extraction

```rust
// crates/coral-core/src/page.rs

impl Page {
    /// Returns wikilinks discovered in the body (and in `backlinks` field, deduplicated).
    /// The result is sorted alphabetically + deduplicated for stable output.
    pub fn outbound_links(&self) -> Vec<String> {
        let mut all: Vec<String> = wikilinks::extract(&self.body);
        for bl in &self.frontmatter.backlinks {
            all.push(bl.clone());
        }
        all.sort();
        all.dedup();
        all
    }
}
```

Wikilinks inside fenced code blocks are explicitly excluded from extraction by the [[wikilinks]] module.

### Mutation helpers

```rust
// crates/coral-core/src/page.rs

impl Page {
    /// Bumps `last_updated_commit` to `commit` (string sha).
    /// Mutates the page in place; caller persists with `write()`.
    pub fn bump_last_commit(&mut self, commit: impl Into<String>) {
        self.frontmatter.last_updated_commit = commit.into();
    }

    /// Inserts a backlink entry if not already present. Idempotent.
    pub fn add_backlink(&mut self, slug: impl Into<String>) {
        let slug = slug.into();
        if !self.frontmatter.backlinks.contains(&slug) {
            self.frontmatter.backlinks.push(slug);
        }
    }
}
```

### Review gate

```rust
// crates/coral-core/src/page.rs

impl Page {
    /// Returns `true` for pages whose frontmatter declares `reviewed: false`
    /// AND carries a populated `source.runner` field — i.e. LLM-distilled
    /// output that no human has signed off on.
    pub fn is_unreviewed_distilled(&self) -> bool { ... }
}
```

This method is consumed by both `coral_lint::structural::check_unreviewed_distilled` and the MCP `WikiResourceProvider` to suppress unreviewed LLM output from being served as authoritative wiki content. It defensively accepts both YAML boolean (`reviewed: false`) and string (`reviewed: "false"` / `reviewed: "no"`) forms.

## Bi-temporal support

Pages support temporal validity windows via `valid_from` and `valid_to` fields in frontmatter. The `Frontmatter::is_valid_at(at)` method enables queries like "what did the wiki say about X at time T?" The `superseded_by` field marks a page as replaced by another slug, and `Frontmatter::is_current()` returns false when either `valid_to` or `superseded_by` is set.

## Relationships

- Stored and indexed by [[wiki-index]]
- Generated by the runner pipeline using prompt templates
- Persisted atomically via `crate::atomic::atomic_write_string`
- Wikilinks extracted by the [[wikilinks]] module
- Used throughout [[bootstrap-flow]] and [[ingest-cycle]]
