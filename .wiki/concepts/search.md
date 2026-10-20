---
slug: search
type: concept
last_updated_commit: 19da4d6
confidence: 0.95
sources:
- crates/coral-core/src/search.rs
- crates/coral-core/src/search_index.rs
- crates/coral-core/src/lib.rs
backlinks:
- embeddings
- wikilinks
- frontmatter
status: draft
---

# Search

The search module provides full-text ranking over wiki page collections using TF-IDF, BM25, and hybrid Reciprocal Rank Fusion (RRF). Combined with [[embeddings]] for semantic search, this powers `coral query`, `coral search`, and the MCP `search` tool.

## Algorithms

### TF-IDF (`search`)

Classic term-frequency / inverse-document-frequency with `sqrt(N)` length normalization:

```rust
// crates/coral-core/src/search.rs

pub fn search(pages: &[Page], query: &str, limit: usize) -> Vec<SearchResult> {
    let query_tokens = tokenize(query);
    // ... compute document frequencies, then per-doc:
    let mut score = 0.0;
    for q in &query_tokens {
        if let Some(&count) = tf.get(q) {
            let df_count = *df.get(q).unwrap_or(&1) as f64;
            let idf = ((n_docs + 1.0) / (df_count + 1.0)).ln() + 1.0;
            score += (count as f64) * idf;
        }
    }
    let final_score = score / (tokens.len() as f64).sqrt();
    // ...
}
```

### BM25 (`search_bm25`)

Okapi BM25 with canonical Robertson/Sparck-Jones constants:

```rust
// crates/coral-core/src/search.rs

pub const BM25_K1: f64 = 1.5;  // term-frequency saturation
pub const BM25_B: f64 = 0.75;  // length-normalization (0=disabled, 1=full)
```

Score formula: `score(D,Q) = sum_q IDF(q) * (tf(q,D) * (k1+1)) / (tf(q,D) + k1 * (1 - b + b * |D|/avgdl))`

IDF uses the BM25 variant clamped at zero so very common terms (appearing in > N/2 docs) don't push scores negative:

```rust
// crates/coral-core/src/search.rs

let raw = ((n_docs - n_q + 0.5) / (n_q + 0.5) + 1.0).ln();
(q, raw.max(0.0))  // Clamp at 0
```

### Hybrid RRF (`search_hybrid`)

Reciprocal Rank Fusion combines BM25 and TF-IDF:

```rust
// crates/coral-core/src/search.rs

/// RRF constant k=60 (Cormack, Clarke & Buettcher 2009).
pub const RRF_K: f64 = 60.0;

pub fn search_hybrid(pages: &[Page], query: &str, limit: usize) -> Vec<SearchResult> {
    let bm25_results = search_bm25(pages, query, pages.len());
    let tfidf_results = search(pages, query, pages.len());
    // Accumulate: score(d) = sum(1 / (k + rank_i(d)))
    for (rank, r) in bm25_results.iter().enumerate() {
        *rrf_scores.entry(r.slug.as_str()).or_insert(0.0) += 1.0 / (RRF_K + (rank+1) as f64);
    }
    // ... same for tfidf_results, then sort and truncate ...
}
```

Pages appearing in both result sets get boosted (two RRF contributions). This is the recommended function for `coral query` and MCP tool calls.

### Adaptive Routing (`search_adaptive`)

Classifies queries into `Entity` or `Synthesis` levels:

```rust
// crates/coral-core/src/search.rs

pub enum QueryLevel {
    Entity,     // targets a specific known slug
    Synthesis,  // cross-cutting flow/relationship
}
```

Classification signals:
- **Entity**: short query (<=4 tokens), matches a known slug, patterns like "what is X"
- **Synthesis**: flow/process words ("how", "flow", "end-to-end", "between", "across"), multiple entities, longer queries (>6 tokens)

Entity queries do exact slug match (score 1.0) plus direct backlink expansion (score 0.8). Synthesis queries delegate to `search_hybrid`.

## Tokenization

Shared tokenizer for all search functions:

```rust
// crates/coral-core/src/search.rs

fn tokenize(text: &str) -> Vec<String> {
    let sw = stopwords();
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() > 1)
        .filter(|t| !sw.contains(t))
        .map(String::from)
        .collect()
}
```

Stopword list covers English and Spanish: "the", "and", "of", "is", "el", "la", "de", "y", "en", "que", etc.

## SearchResult

```rust
// crates/coral-core/src/search.rs

#[derive(Debug, Clone, PartialEq)]
pub struct SearchResult {
    pub slug: String,
    pub score: f64,
    pub snippet: String,
}
```

## Snippet Generation

`build_snippet` finds the first query-token occurrence in the page body and extracts a window around it (40 chars before, up to `max_len` total). Uses `floor_char_boundary` / `ceil_char_boundary` to avoid panics on multi-byte UTF-8 characters near window edges.

## Persisted Search Index

`search_with_index` (re-exported from `crates/coral-core/src/search_index.rs`) provides a persisted BM25 index at `.coral/search-index.bin`:

- Content hash (SHA-256) detects corpus changes
- Exclusive file lock for concurrent safety
- Atomic save (tmpfile + rename)
- Falls back to rebuild when hash mismatches

## See Also

- [[embeddings]] -- semantic cosine-similarity search combined via RRF
- [[wikilinks]] -- backlinks used by adaptive search for entity expansion
- [[frontmatter]] -- slug and page type used for query routing
