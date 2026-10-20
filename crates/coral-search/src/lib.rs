//! Hybrid search engine for Coral wiki and code content.
//!
//! Provides BM25 keyword search (via tantivy) and semantic similarity search
//! (TF-IDF cosine as default fallback, real embeddings via `EmbeddingsProvider`
//! when configured). Results are fused using Reciprocal Rank Fusion (RRF) with
//! configurable per-source weights.

pub mod config;
pub mod engine;
pub mod fuzzy;
pub mod hybrid;
pub mod keyword;
pub mod semantic;
pub mod structural;
pub mod types;
pub mod vecstore;

// Re-exports for ergonomic use.
pub use config::{EmbeddingsConfig, HybridConfig, SearchConfig};
pub use engine::{EmbeddingStatus, FileEntry, SearchEngine, SymbolEntry, WikiEntry};
pub use fuzzy::{FuzzyMatch, TrigramIndex};
pub use hybrid::{reciprocal_rank_fusion, weighted_reciprocal_rank_fusion};
pub use keyword::{IndexDocument, KeywordIndex};
pub use semantic::{EmbeddingDocInfo, SemanticIndex};
pub use structural::structural_search;
pub use types::{DocType, SearchQuery, SearchResult, SearchSource};
pub use vecstore::VectorStore;
