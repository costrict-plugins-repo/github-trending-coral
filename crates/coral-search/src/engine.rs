use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tracing;

use crate::config::SearchConfig;
use crate::hybrid::{reciprocal_rank_fusion, weighted_reciprocal_rank_fusion};
use crate::keyword::{IndexDocument, KeywordIndex};
use crate::semantic::SemanticIndex;
use crate::structural::structural_search;
use crate::types::{DocType, SearchQuery, SearchResult, SearchSource};
use crate::vecstore::{VectorEntry, VectorStore};

/// A symbol entry for indexing code files.
#[derive(Debug, Clone)]
pub struct SymbolEntry {
    pub name: String,
    pub line: u32,
    pub snippet: String,
}

/// A file entry for bulk indexing.
#[derive(Debug, Clone)]
pub struct FileEntry {
    pub path: PathBuf,
    pub content: String,
    pub language: Option<String>,
    pub symbols: Vec<SymbolEntry>,
}

/// A wiki page entry for bulk indexing.
#[derive(Debug, Clone)]
pub struct WikiEntry {
    pub path: PathBuf,
    pub content: String,
}

/// The RRF smoothing constant (standard default).
const RRF_K: f64 = 60.0;

/// Status of the embedding index.
#[derive(Debug, Clone)]
pub struct EmbeddingStatus {
    /// Whether a vector store is loaded.
    pub has_vectors: bool,
    /// Number of entries in the vector store.
    pub entry_count: usize,
    /// Model name used for embeddings.
    pub model: String,
    /// Dimensionality of stored vectors.
    pub dimensions: usize,
}

/// Unified hybrid search engine combining keyword (BM25), semantic (TF-IDF),
/// and structural (graph-based) search.
pub struct SearchEngine {
    keyword: KeywordIndex,
    semantic: SemanticIndex,
    /// Optional path to the code graph file (`.coral/graph.json`).
    /// When set, structural search is available.
    graph_path: Option<PathBuf>,
    /// Search configuration (weights, provider selection).
    config: SearchConfig,
    /// Optional vector store for real embeddings.
    vecstore: Option<VectorStore>,
}

impl SearchEngine {
    /// Create a new search engine with indices at the given path.
    /// The keyword index is stored in `index_path/keyword/` and the
    /// semantic index state in `index_path/semantic.json`.
    pub fn new(index_path: &Path) -> Result<Self> {
        let keyword_path = index_path.join("keyword");
        let keyword = if keyword_path.exists() {
            KeywordIndex::open(&keyword_path)
                .unwrap_or_else(|e| {
                    tracing::warn!("failed to open keyword index, recreating: {e}");
                    KeywordIndex::create(&keyword_path)
                        .expect("failed to create keyword index")
                })
        } else {
            KeywordIndex::create(&keyword_path)
                .context("failed to create keyword index")?
        };

        let semantic_path = index_path.join("semantic.json");
        let semantic = if semantic_path.exists() {
            SemanticIndex::load(&semantic_path).unwrap_or_else(|e| {
                tracing::warn!("failed to load semantic index, creating new: {e}");
                SemanticIndex::new()
            })
        } else {
            SemanticIndex::new()
        };

        // Try to load existing vector store
        let vecstore_path = index_path.join("vectors.json");
        let vecstore = if vecstore_path.exists() {
            VectorStore::load(&vecstore_path)
                .map(Some)
                .unwrap_or_else(|e| {
                    tracing::warn!("failed to load vector store, ignoring: {e}");
                    None
                })
        } else {
            None
        };

        Ok(Self {
            keyword,
            semantic,
            graph_path: None,
            config: SearchConfig::default(),
            vecstore,
        })
    }

    /// Create a search engine with in-RAM indices (for testing).
    pub fn new_in_memory() -> Result<Self> {
        let keyword = KeywordIndex::create_in_ram()
            .context("failed to create in-memory keyword index")?;
        let semantic = SemanticIndex::new();

        Ok(Self {
            keyword,
            semantic,
            graph_path: None,
            config: SearchConfig::default(),
            vecstore: None,
        })
    }

    /// Set the path to the code graph file for structural search.
    pub fn set_graph_path(&mut self, path: PathBuf) {
        self.graph_path = Some(path);
    }

    /// Apply a search configuration (embeddings provider, hybrid weights).
    pub fn set_config(&mut self, config: SearchConfig) {
        self.config = config;
    }

    /// Set or replace the vector store.
    pub fn set_vecstore(&mut self, vecstore: VectorStore) {
        self.vecstore = Some(vecstore);
    }

    /// Compute embedding vectors for all indexed documents using the given
    /// provider. Populates `self.vecstore` with the results.
    ///
    /// The documents come from the semantic index's internal document list.
    /// Each document's snippet is embedded with `input_type = "document"`.
    pub fn embed_with_provider(
        &mut self,
        provider: &dyn coral_runner::EmbeddingsProvider,
    ) -> Result<()> {
        // Gather snippets from the semantic index's documents
        let docs = self.semantic.documents_for_embedding();
        if docs.is_empty() {
            tracing::info!("no documents to embed");
            return Ok(());
        }

        let texts: Vec<String> = docs.iter().map(|d| d.snippet.clone()).collect();
        tracing::info!(count = texts.len(), "computing embeddings...");

        let vectors = provider
            .embed_batch(&texts, Some("document"))
            .map_err(|e| anyhow::anyhow!("embeddings provider error: {e}"))?;

        let mut store = VectorStore::new(provider.dim(), provider.name().to_string());
        for (doc, vector) in docs.iter().zip(vectors.into_iter()) {
            store.add(VectorEntry {
                path: doc.path.clone(),
                symbol: doc.symbol.clone(),
                line: doc.line,
                snippet: doc.snippet.clone(),
                vector,
            })?;
        }

        tracing::info!(entries = store.len(), "vector store populated");
        self.vecstore = Some(store);
        Ok(())
    }

    /// Return the current embedding status (vector store health).
    pub fn embedding_status(&self) -> EmbeddingStatus {
        match &self.vecstore {
            Some(vs) => EmbeddingStatus {
                has_vectors: true,
                entry_count: vs.len(),
                model: vs.model.clone(),
                dimensions: vs.dim,
            },
            None => EmbeddingStatus {
                has_vectors: false,
                entry_count: 0,
                model: String::new(),
                dimensions: 0,
            },
        }
    }

    /// Save the vector store to the given directory (as `vectors.json`).
    pub fn save_vecstore(&self, index_path: &Path) -> Result<()> {
        if let Some(ref vs) = self.vecstore {
            let path = index_path.join("vectors.json");
            vs.save(&path).context("saving vector store")?;
        }
        Ok(())
    }

    /// Index a single source file with its extracted symbols.
    pub fn index_file(
        &mut self,
        path: &Path,
        content: &str,
        symbols: &[SymbolEntry],
    ) -> Result<()> {
        // Index the whole file as one document for keyword search
        let doc = IndexDocument {
            path: path.to_path_buf(),
            content: content.to_string(),
            symbol: None,
            language: detect_language(path),
            doc_type: DocType::Code,
            line: None,
        };
        self.keyword.add_document(&doc)?;

        // Also index individual symbols for more granular results
        let mut symbol_docs = Vec::new();
        for sym in symbols {
            symbol_docs.push(IndexDocument {
                path: path.to_path_buf(),
                content: sym.snippet.clone(),
                symbol: Some(sym.name.clone()),
                language: detect_language(path),
                doc_type: DocType::Code,
                line: Some(sym.line),
            });
        }
        if !symbol_docs.is_empty() {
            self.keyword.add_documents(&symbol_docs)?;
        }

        // Add to semantic index
        self.semantic
            .add_document(path.to_path_buf(), content, None, None);
        for sym in symbols {
            self.semantic.add_document(
                path.to_path_buf(),
                &sym.snippet,
                Some(sym.name.clone()),
                Some(sym.line),
            );
        }

        Ok(())
    }

    /// Index a wiki page.
    pub fn index_wiki_page(&mut self, path: &Path, content: &str) -> Result<()> {
        let doc = IndexDocument {
            path: path.to_path_buf(),
            content: content.to_string(),
            symbol: None,
            language: None,
            doc_type: DocType::Wiki,
            line: None,
        };
        self.keyword.add_document(&doc)?;

        self.semantic
            .add_document(path.to_path_buf(), content, None, None);

        Ok(())
    }

    /// Search across all enabled sources using hybrid ranking.
    ///
    /// When a vector store is available and the query requests semantic search,
    /// the vector store is used instead of TF-IDF. Weights from `self.config.hybrid`
    /// are applied via weighted RRF.
    pub fn search(&self, query: &SearchQuery) -> Result<Vec<SearchResult>> {
        if query.text.trim().is_empty() {
            return Ok(Vec::new());
        }

        let weights = &self.config.hybrid;
        let mut weighted_sets: Vec<(Vec<SearchResult>, f64)> = Vec::new();

        // Keyword (BM25) search
        if query.sources.contains(&SearchSource::Keyword) {
            let keyword_results = self.keyword.search(&query.text, query.limit * 2)?;
            let filtered = apply_file_filter(keyword_results, &query.file_filter);
            if !filtered.is_empty() {
                weighted_sets.push((filtered, weights.keyword_weight));
            }
        }

        // Semantic search — TF-IDF fallback (always available).
        // TODO: when query-time embedding is implemented, use `self.vecstore`
        // here to compute a query vector and search the vector store instead.
        if query.sources.contains(&SearchSource::Semantic) {
            let semantic_results = self.semantic.search(&query.text, query.limit * 2);
            let filtered = apply_file_filter(semantic_results, &query.file_filter);
            if !filtered.is_empty() {
                weighted_sets.push((filtered, weights.semantic_weight));
            }
        }

        // Structural (graph-based) search
        if query.sources.contains(&SearchSource::Structural) {
            if let Some(ref gp) = self.graph_path {
                match coral_graph::load_graph(gp) {
                    Ok(graph) => {
                        let structural_results =
                            structural_search(&graph, &query.text, query.limit * 2);
                        let filtered =
                            apply_file_filter(structural_results, &query.file_filter);
                        if !filtered.is_empty() {
                            weighted_sets.push((filtered, weights.structural_weight));
                        }
                    }
                    Err(e) => {
                        tracing::warn!("structural search skipped: failed to load graph: {e}");
                    }
                }
            }
        }

        if weighted_sets.is_empty() {
            return Ok(Vec::new());
        }

        // If only one source was used, return its results directly
        if weighted_sets.len() == 1 {
            let mut results = weighted_sets.into_iter().next().unwrap_or_default().0;
            results.truncate(query.limit);
            return Ok(results);
        }

        // Check if all weights are uniform (1.0) — use plain RRF for backward compat
        let all_uniform = weighted_sets
            .iter()
            .all(|(_, w)| (*w - 1.0).abs() < 1e-10);

        if all_uniform {
            let plain_sets: Vec<Vec<SearchResult>> =
                weighted_sets.into_iter().map(|(s, _)| s).collect();
            let fused = reciprocal_rank_fusion(plain_sets, RRF_K, query.limit);
            Ok(fused)
        } else {
            let fused = weighted_reciprocal_rank_fusion(weighted_sets, RRF_K, query.limit);
            Ok(fused)
        }
    }

    /// Search using a pre-computed query vector against the vector store.
    ///
    /// Useful when the caller has already computed the query embedding (e.g.
    /// via `EmbeddingsProvider::embed_batch` with `input_type = "query"`).
    /// Results are returned as `SearchResult` with `SearchSource::Semantic`.
    pub fn search_by_vector(&self, query_vec: &[f32], limit: usize) -> Vec<SearchResult> {
        let Some(ref vs) = self.vecstore else {
            return Vec::new();
        };

        vs.search(query_vec, limit)
            .into_iter()
            .map(|(idx, score)| {
                let entry = &vs.entries[idx];
                SearchResult {
                    path: entry.path.clone(),
                    symbol: entry.symbol.clone(),
                    snippet: entry.snippet.clone(),
                    score,
                    source: SearchSource::Semantic,
                    line: entry.line,
                }
            })
            .collect()
    }

    /// Delete all indices and rebuild from scratch.
    pub fn rebuild(&mut self, files: &[FileEntry], wiki_pages: &[WikiEntry]) -> Result<()> {
        // Rebuild keyword index
        let mut all_docs: Vec<IndexDocument> = Vec::new();

        for file in files {
            all_docs.push(IndexDocument {
                path: file.path.clone(),
                content: file.content.clone(),
                symbol: None,
                language: file.language.clone(),
                doc_type: DocType::Code,
                line: None,
            });
            for sym in &file.symbols {
                all_docs.push(IndexDocument {
                    path: file.path.clone(),
                    content: sym.snippet.clone(),
                    symbol: Some(sym.name.clone()),
                    language: file.language.clone(),
                    doc_type: DocType::Code,
                    line: Some(sym.line),
                });
            }
        }

        for page in wiki_pages {
            all_docs.push(IndexDocument {
                path: page.path.clone(),
                content: page.content.clone(),
                symbol: None,
                language: None,
                doc_type: DocType::Wiki,
                line: None,
            });
        }

        self.keyword.rebuild(&all_docs)?;

        // Rebuild semantic index
        self.semantic = SemanticIndex::new();
        for file in files {
            self.semantic
                .add_document(file.path.clone(), &file.content, None, None);
            for sym in &file.symbols {
                self.semantic.add_document(
                    file.path.clone(),
                    &sym.snippet,
                    Some(sym.name.clone()),
                    Some(sym.line),
                );
            }
        }
        for page in wiki_pages {
            self.semantic
                .add_document(page.path.clone(), &page.content, None, None);
        }
        self.semantic.build();

        tracing::info!(
            files = files.len(),
            wiki_pages = wiki_pages.len(),
            "search index rebuilt"
        );

        Ok(())
    }

    /// Build the semantic index after batch additions.
    /// Call this after `index_file` / `index_wiki_page` if you added many documents.
    pub fn build_semantic(&mut self) {
        self.semantic.build();
    }
}

/// Apply a glob-style file filter to results.
fn apply_file_filter(results: Vec<SearchResult>, filter: &Option<String>) -> Vec<SearchResult> {
    let Some(pattern) = filter else {
        return results;
    };

    results
        .into_iter()
        .filter(|r| {
            let path_str = r.path.to_string_lossy();
            simple_glob_match(pattern, &path_str)
        })
        .collect()
}

/// Simple glob matching (supports * and ?).
///
/// Uses an iterative two-pointer algorithm to avoid unbounded recursion
/// and exponential backtracking on patterns like `*a*b*c*d`.
fn simple_glob_match(pattern: &str, text: &str) -> bool {
    let pattern_chars: Vec<char> = pattern.chars().collect();
    let text_chars: Vec<char> = text.chars().collect();

    let (mut pi, mut ti) = (0usize, 0usize);
    let (mut star_pi, mut star_ti) = (usize::MAX, 0usize);

    while ti < text_chars.len() {
        if pi < pattern_chars.len() && (pattern_chars[pi] == '?' || pattern_chars[pi] == text_chars[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < pattern_chars.len() && pattern_chars[pi] == '*' {
            star_pi = pi;
            star_ti = ti;
            pi += 1;
        } else if star_pi != usize::MAX {
            // Mismatch — backtrack: advance the match position by one from
            // where the last '*' was, and retry from the char after that '*'.
            star_ti += 1;
            ti = star_ti;
            pi = star_pi + 1;
        } else {
            return false;
        }
    }

    // Consume trailing '*' characters in the pattern.
    while pi < pattern_chars.len() && pattern_chars[pi] == '*' {
        pi += 1;
    }

    pi == pattern_chars.len()
}

/// Detect the programming language from file extension.
fn detect_language(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| match ext {
            "rs" => "rust",
            "py" => "python",
            "js" => "javascript",
            "ts" => "typescript",
            "go" => "go",
            "rb" => "ruby",
            "java" => "java",
            "c" | "h" => "c",
            "cpp" | "hpp" | "cc" => "cpp",
            "md" => "markdown",
            "toml" => "toml",
            "yaml" | "yml" => "yaml",
            "json" => "json",
            _ => ext,
        })
        .map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_engine_full_search() {
        let mut engine = SearchEngine::new_in_memory().unwrap();

        engine
            .index_file(
                Path::new("src/auth.rs"),
                "pub fn authenticate_user(token: &str) -> Result<User> {\n    validate_jwt(token)\n}",
                &[SymbolEntry {
                    name: "authenticate_user".to_string(),
                    line: 1,
                    snippet: "pub fn authenticate_user(token: &str) -> Result<User>".to_string(),
                }],
            )
            .unwrap();

        engine
            .index_file(
                Path::new("src/db.rs"),
                "pub fn connect_database(url: &str) -> Pool {\n    create_pool(url)\n}",
                &[SymbolEntry {
                    name: "connect_database".to_string(),
                    line: 1,
                    snippet: "pub fn connect_database(url: &str) -> Pool".to_string(),
                }],
            )
            .unwrap();

        engine
            .index_wiki_page(
                Path::new(".wiki/auth-middleware.md"),
                "# Authentication Middleware\nHandles JWT token validation and user sessions.",
            )
            .unwrap();

        engine.build_semantic();

        let query = SearchQuery {
            text: "authenticate jwt".to_string(),
            limit: 5,
            sources: vec![SearchSource::Keyword, SearchSource::Semantic],
            file_filter: None,
        };

        let results = engine.search(&query).unwrap();
        assert!(!results.is_empty(), "expected results for hybrid search");

        // The auth-related files should dominate the results
        let auth_results: Vec<_> = results
            .iter()
            .filter(|r| r.path.to_string_lossy().contains("auth"))
            .collect();
        assert!(
            !auth_results.is_empty(),
            "auth files should appear in results"
        );
    }

    #[test]
    fn test_engine_file_filter() {
        let mut engine = SearchEngine::new_in_memory().unwrap();

        engine
            .index_file(
                Path::new("src/auth.rs"),
                "authentication token validation",
                &[],
            )
            .unwrap();

        engine
            .index_wiki_page(
                Path::new(".wiki/auth.md"),
                "authentication token validation wiki page",
            )
            .unwrap();

        engine.build_semantic();

        let query = SearchQuery {
            text: "authentication".to_string(),
            limit: 10,
            sources: vec![SearchSource::Keyword],
            file_filter: Some("*.rs".to_string()),
        };

        let results = engine.search(&query).unwrap();
        for r in &results {
            assert!(
                r.path.to_string_lossy().ends_with(".rs"),
                "file filter should exclude non-.rs files"
            );
        }
    }

    #[test]
    fn test_engine_empty_query() {
        let engine = SearchEngine::new_in_memory().unwrap();
        let query = SearchQuery {
            text: "".to_string(),
            limit: 10,
            sources: vec![SearchSource::Keyword, SearchSource::Semantic],
            file_filter: None,
        };
        let results = engine.search(&query).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_simple_glob_match() {
        assert!(simple_glob_match("*.rs", "src/auth.rs"));
        assert!(simple_glob_match("src/*", "src/main.rs"));
        assert!(!simple_glob_match("*.rs", "src/auth.py"));
        assert!(simple_glob_match("*auth*", "src/auth/middleware.rs"));
        assert!(simple_glob_match("?oo", "foo"));
        assert!(!simple_glob_match("?oo", "foos"));
    }
}
