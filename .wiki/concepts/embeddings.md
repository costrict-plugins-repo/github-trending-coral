---
slug: embeddings
type: concept
last_updated_commit: 19da4d6
confidence: 0.95
sources:
- crates/coral-core/src/embeddings.rs
- crates/coral-core/src/embeddings_sqlite.rs
- crates/coral-core/src/lib.rs
backlinks:
- search
status: draft
---

# Embeddings

Coral's semantic search relies on vector embeddings produced by external providers and stored locally. The `EmbeddingsIndex` in `coral-core` manages a per-wiki cache of slug-to-vector mappings with cosine-similarity search, freshness tracking, and atomic persistence.

## Index Structure

```rust
// crates/coral-core/src/embeddings.rs

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EmbeddingsIndex {
    #[serde(default = "default_version")]
    pub version: u32,
    /// Provider id, e.g. "voyage-3". Mismatched providers => stale index.
    pub provider: String,
    /// Embedding dimension (e.g. 1024 for voyage-3).
    pub dim: usize,
    /// slug -> IndexedVector
    pub entries: BTreeMap<String, IndexedVector>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexedVector {
    pub mtime_secs: i64,
    pub vector: Vec<f32>,
}
```

Storage path: `<wiki_root>/.coral-embeddings.json`. The file is schema-versioned (`SCHEMA_VERSION = 1`); loading a mismatched version resets to empty. Auto-ignored by `coral init`'s `.gitignore`.

## Core Operations

### Upsert with Dimension Validation

```rust
// crates/coral-core/src/embeddings.rs

pub fn upsert(&mut self, slug: impl Into<String>, mtime_secs: i64, vector: Vec<f32>) {
    if vector.len() != self.dim {
        tracing::warn!(%slug, expected = self.dim, actual = vector.len(),
            "embeddings::upsert refused: dim mismatch");
        return;
    }
    self.entries.insert(slug.into(), IndexedVector { mtime_secs, vector });
}
```

Dimension mismatch is refused with a `tracing::warn` (v0.19.5 audit M2) rather than silently corrupting the cache.

### Cosine Similarity Search

```rust
// crates/coral-core/src/embeddings.rs

pub fn search(&self, query_vec: &[f32], limit: usize) -> Vec<(String, f32)> {
    if query_vec.len() != self.dim || limit == 0 || self.entries.is_empty() {
        return vec![];
    }
    let q_norm = norm(query_vec);
    if q_norm == 0.0 { return vec![]; }
    let mut scored: Vec<(String, f32)> = self.entries.iter()
        .filter_map(|(slug, iv)| {
            let v_norm = norm(&iv.vector);
            if v_norm == 0.0 { return None; }
            let dot: f32 = query_vec.iter().zip(iv.vector.iter())
                .map(|(a, b)| a * b).sum();
            Some((slug.clone(), dot / (q_norm * v_norm)))
        })
        .collect();
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(limit);
    scored
}

fn norm(v: &[f32]) -> f32 {
    v.iter().map(|x| x * x).sum::<f32>().sqrt()
}
```

Brute-force cosine similarity over all entries. Zero-vector queries and zero-norm entries are excluded.

### Freshness and Pruning

```rust
// crates/coral-core/src/embeddings.rs

/// Returns true if the entry's mtime matches `mtime_secs`.
pub fn is_fresh(&self, slug: &str, mtime_secs: i64) -> bool {
    self.entries.get(slug).is_some_and(|e| e.mtime_secs == mtime_secs)
}

/// Drop entries whose slug is no longer in `live_slugs`.
pub fn prune(&mut self, live_slugs: &HashSet<String>) -> usize {
    let before = self.entries.len();
    self.entries.retain(|k, _| live_slugs.contains(k));
    before - self.entries.len()
}
```

`is_fresh` enables incremental re-embedding: only pages whose file mtime changed get re-embedded. `prune` garbage-collects vectors for deleted pages.

## Persistence

```rust
// crates/coral-core/src/embeddings.rs

pub fn save(&self, wiki_root: &Path) -> Result<PathBuf> {
    let path = wiki_root.join(Self::FILENAME);
    let content = serde_json::to_string_pretty(self)?;
    // Atomic write: temp-file + rename so concurrent readers never
    // observe a torn JSON file.
    crate::atomic::atomic_write_string(&path, &content)?;
    Ok(path)
}

pub fn load(wiki_root: &Path) -> Result<Self> {
    // ... reads file, checks schema version, resets on mismatch ...
}
```

The atomic write pattern (tempfile + rename) ensures concurrent `coral search` readers never observe a partially-written JSON file.

## SQLite Backend

`crates/coral-core/src/embeddings_sqlite.rs` provides `SqliteEmbeddingsIndex` as an opt-in alternative for large wikis (~5000+ pages) where JSON parse overhead becomes noticeable:

- Table: `embeddings(slug TEXT PRIMARY KEY, mtime_secs INTEGER, vector BLOB)`
- Table: `meta(key TEXT PRIMARY KEY, value TEXT)` for provider/dim/schema
- Vectors stored as little-endian f32 BLOBs
- Cosine similarity computed in Rust (no C extension dependency)
- Selected via `CORAL_EMBEDDINGS_BACKEND` env var

The SQLite file lives at `<wiki_root>/.coral-embeddings.sqlite`. Re-exported as `coral_core::SqliteEmbeddingsIndex` and `coral_core::SQLITE_FILENAME`.

## Provider Integration

The default provider is Voyage AI `voyage-3` (1024 dimensions). The embedding provider trait and implementations live in `coral-runner` (separate from the storage layer in `coral-core`). The search command orchestrates: embed query -> cosine search -> return ranked slugs.

## Re-exports

```rust
// crates/coral-core/src/lib.rs

pub use crate::embeddings::EmbeddingsIndex;
pub use crate::embeddings_sqlite::{SQLITE_FILENAME, SqliteEmbeddingsIndex};
```

## See Also

- [[search]] -- combines embedding cosine scores with BM25/TF-IDF via Reciprocal Rank Fusion
