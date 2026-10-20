---
slug: wiki-index
type: entity
last_updated_commit: 19da4d6
confidence: 0.95
sources:
- crates/coral-core/src/index.rs
backlinks:
- page
- bootstrap-flow
- ingest-cycle
- cli
status: draft
---

# WikiIndex

The `WikiIndex` struct is the master catalog of all wiki pages in a Coral repository. It tracks every page's slug, type, path, confidence, status, and last-updated commit. The index is persisted as `.wiki/index.md` — a Markdown file with its own frontmatter (containing a `last_commit` anchor) and a body formatted as a Markdown table.

## Struct definitions

```rust
// crates/coral-core/src/index.rs

/// Single entry in the wiki index.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IndexEntry {
    pub slug: String,
    pub page_type: PageType,
    pub path: String,
    pub confidence: Confidence,
    pub status: Status,
    pub last_updated_commit: String,
}

/// Wiki index — catalog of all pages + last_commit anchor for incremental ingest.
#[derive(Debug, Clone, PartialEq)]
pub struct WikiIndex {
    pub last_commit: String,
    pub generated_at: DateTime<Utc>,
    pub entries: Vec<IndexEntry>,
}
```

The private frontmatter shape used for serialization:

```rust
// crates/coral-core/src/index.rs

#[derive(Debug, Serialize, Deserialize)]
struct IndexFrontmatter {
    last_commit: String,
    generated_at: DateTime<Utc>,
}
```

## On-disk format

The `.wiki/index.md` file uses a custom frontmatter shape (separate from the page `Frontmatter`):

```markdown
---
last_commit: abc123
generated_at: 2026-05-01T12:00:00+00:00
---

# Wiki index

| Type | Slug | Path | Confidence | Status | Last commit |
|------|------|------|------------|--------|-------------|
| entity | page | entities/page.md | 0.95 | draft | 19da4d6 |
| module | runner | modules/runner.md | 0.85 | reviewed | abc123 |
```

Entries are sorted by page type key (alphabetical) then slug within each type group. This guarantees deterministic diffs when the index is committed to version control.

## Key methods

### Construction and parsing

```rust
// crates/coral-core/src/index.rs

impl WikiIndex {
    pub fn new(last_commit: impl Into<String>) -> Self {
        Self {
            last_commit: last_commit.into(),
            generated_at: Utc::now(),
            entries: Vec::new(),
        }
    }

    /// Parse from the contents of `.wiki/index.md`.
    /// Format: frontmatter with `last_commit` + `generated_at`,
    /// body is a Markdown table.
    pub fn parse(content: &str) -> Result<Self> { ... }

    /// Serialize to a Markdown document suitable for `.wiki/index.md`.
    pub fn to_string(&self) -> Result<String> { ... }
}
```

### CRUD operations

```rust
// crates/coral-core/src/index.rs

impl WikiIndex {
    /// Insert or update an entry by slug. Idempotent.
    pub fn upsert(&mut self, entry: IndexEntry) {
        if let Some(existing) = self.entries.iter_mut().find(|e| e.slug == entry.slug) {
            *existing = entry;
        } else {
            self.entries.push(entry);
        }
    }

    /// Find an entry by slug.
    pub fn find(&self, slug: &str) -> Option<&IndexEntry> {
        self.entries.iter().find(|e| e.slug == slug)
    }

    /// Set the last_commit and bump generated_at to `Utc::now()`.
    pub fn bump_last_commit(&mut self, commit: impl Into<String>) {
        self.last_commit = commit.into();
        self.generated_at = Utc::now();
    }
}
```

## Incremental ingest via last_commit

The `last_commit` field serves as a cursor for incremental updates. When `coral ingest` runs without an explicit `--from` flag, it reads `last_commit` from the index and computes `git diff <last_commit>..HEAD` to determine which source files changed since the last wiki update. After updating, `bump_last_commit` advances the cursor and timestamps the operation.

## Table parsing logic

The `parse_table` helper extracts entries from the Markdown table body:

1. Splits each line on `|` separators
2. Drops leading/trailing empty cells produced by `|a|b|` format
3. Skips the header row and separator row (all dashes/colons)
4. Parses each data row's 6 cells into an `IndexEntry`
5. Confidence is parsed as `f64` and validated via `Confidence::try_new`
6. Page type and status are deserialized from their string keys via `serde_yaml_ng`

The parser tolerates rows with fewer than 6 columns (skips them) and correctly identifies separator rows.

## Serialization details

The `to_string()` method:

```rust
// crates/coral-core/src/index.rs (simplified)

pub fn to_string(&self) -> Result<String> {
    let mut out = String::new();
    out.push_str("---\n");
    out.push_str(&format!("last_commit: {}\n", self.last_commit));
    out.push_str(&format!("generated_at: {}\n", self.generated_at.to_rfc3339()));
    out.push_str("---\n\n");
    out.push_str("# Wiki index\n\n");
    out.push_str("| Type | Slug | Path | Confidence | Status | Last commit |\n");
    out.push_str("|------|------|------|------------|--------|-------------|\n");

    let mut sorted = self.entries.clone();
    sorted.sort_by(|a, b| {
        page_type_key(&a.page_type)
            .cmp(page_type_key(&b.page_type))
            .then_with(|| a.slug.cmp(&b.slug))
    });
    for e in &sorted {
        out.push_str(&format!(
            "| {} | {} | {} | {:.2} | {} | {} |\n",
            page_type_key(&e.page_type), e.slug, e.path,
            e.confidence.as_f64(), status_key(&e.status), e.last_updated_commit,
        ));
    }
    Ok(out)
}
```

## Page type keys

The index uses these canonical type strings for sorting and display:

| Variant | Key |
|---------|-----|
| `Module` | `module` |
| `Concept` | `concept` |
| `Entity` | `entity` |
| `Flow` | `flow` |
| `Decision` | `decision` |
| `Synthesis` | `synthesis` |
| `Operation` | `operation` |
| `Source` | `source` |
| `Gap` | `gap` |
| `Index` | `index` |
| `Log` | `log` |
| `Schema` | `schema` |
| `Readme` | `readme` |
| `Reference` | `reference` |
| `Interface` | `interface` |

## Relationships

- Each entry corresponds to a [[page]] on disk
- Rebuilt at the end of [[bootstrap-flow]] and [[ingest-cycle]]
- Consumed by the [[cli]] for status reporting and page enumeration
- Uses `PageType`, `Confidence`, and `Status` from the [[frontmatter]] module
