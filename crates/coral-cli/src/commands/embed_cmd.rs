//! `coral embed` — build or rebuild the search index.
//!
//! Builds the hybrid search index (keyword BM25 + semantic TF-IDF) from the
//! code index and wiki pages, optionally wiring in the code graph for
//! structural search. Supports real embeddings via `--provider`.

use anyhow::{Context, Result};
use clap::Args;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use coral_search::{SearchConfig, SearchEngine, SymbolEntry};

#[derive(Args, Debug)]
pub struct EmbedArgs {
    /// Root directory (defaults to current).
    #[arg(default_value = ".")]
    pub root: PathBuf,

    /// Force full rebuild of the search index.
    #[arg(long)]
    pub full: bool,

    /// Show embedding index health without rebuilding.
    #[arg(long)]
    pub status: bool,

    /// Embeddings provider override: "none" (TF-IDF fallback), "voyage", or "openai".
    #[arg(long, default_value = "none")]
    pub provider: String,
}

pub fn run(args: EmbedArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    let root = args.root.canonicalize().unwrap_or(args.root.clone());
    let wiki_root = wiki_root.unwrap_or(Path::new(".wiki"));
    let index_path = root.join(".coral").join("search-index");

    // --status: just show current index health and exit
    if args.status {
        return show_status(&index_path);
    }

    std::fs::create_dir_all(&index_path).context("failed to create search-index directory")?;

    eprintln!("\u{25b6} [Coral] Building search index...");

    let mut engine = if args.full {
        // Remove existing keyword index to force rebuild
        let keyword_path = index_path.join("keyword");
        if keyword_path.exists() {
            let _ = std::fs::remove_dir_all(&keyword_path);
        }
        let semantic_path = index_path.join("semantic.json");
        if semantic_path.exists() {
            let _ = std::fs::remove_file(&semantic_path);
        }
        let vectors_path = index_path.join("vectors.json");
        if vectors_path.exists() {
            let _ = std::fs::remove_file(&vectors_path);
        }
        SearchEngine::new(&index_path).context("failed to create search engine")?
    } else {
        SearchEngine::new(&index_path).context("failed to open search engine")?
    };

    // Load search config from .coral/config.toml if available
    let config = load_search_config(&root, &args.provider);
    engine.set_config(config.clone());

    let mut file_count: usize = 0;
    let mut symbol_count: usize = 0;
    let mut wiki_count: usize = 0;

    // Index code from coral-index if available
    let code_index_path = root.join(".coral").join("index.json");
    if code_index_path.exists() {
        let index =
            coral_index::load_index(&code_index_path).context("loading code index for search")?;
        for file in &index.files {
            let content = std::fs::read_to_string(&file.path).unwrap_or_default();
            let symbols: Vec<SymbolEntry> = file
                .symbols
                .iter()
                .map(|s| SymbolEntry {
                    name: s.name.clone(),
                    line: s.span.start_line,
                    snippet: s.signature.clone().unwrap_or_else(|| s.name.clone()),
                })
                .collect();
            symbol_count += symbols.len();
            let _ = engine.index_file(&file.path, &content, &symbols);
            file_count += 1;
        }
    } else {
        eprintln!("  (no code index found at .coral/index.json \u{2014} run `coral index` first)");
    }

    // Index wiki pages
    let wiki_path = root.join(wiki_root);
    if wiki_path.exists() {
        if let Ok(entries) = std::fs::read_dir(&wiki_path) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().map(|e| e == "md").unwrap_or(false) {
                    let content = std::fs::read_to_string(&path).unwrap_or_default();
                    let _ = engine.index_wiki_page(&path, &content);
                    wiki_count += 1;
                }
            }
        }
    }

    engine.build_semantic();

    // Wire graph path if available
    let graph_path = root.join(".coral").join("graph.json");
    let graph_available = graph_path.exists();
    if graph_available {
        engine.set_graph_path(graph_path);
    }

    // Compute real embeddings if provider != "none"
    let effective_provider = config.embeddings.provider.as_str();
    if effective_provider != "none" {
        eprintln!("  Embeddings provider: {effective_provider}");
        match create_embeddings_provider(&config) {
            Ok(provider) => {
                match engine.embed_with_provider(provider.as_ref()) {
                    Ok(()) => {
                        engine.save_vecstore(&index_path)?;
                        let status = engine.embedding_status();
                        eprintln!(
                            "  Vectors computed: {} entries ({}-dim, model: {})",
                            status.entry_count, status.dimensions, status.model
                        );
                    }
                    Err(e) => {
                        eprintln!("  \u{26a0}\u{fe0f} Embeddings failed: {e}");
                        eprintln!("  Falling back to TF-IDF semantic search.");
                    }
                }
            }
            Err(e) => {
                eprintln!("  \u{26a0}\u{fe0f} Cannot create embeddings provider: {e}");
                eprintln!("  Falling back to TF-IDF semantic search.");
            }
        }
    }

    eprintln!("\u{2713} [Coral] Search index built");
    eprintln!("  Files indexed:   {file_count}");
    eprintln!("  Symbols indexed: {symbol_count}");
    eprintln!("  Wiki pages:      {wiki_count}");
    if graph_available {
        eprintln!("  Structural:      graph wired (.coral/graph.json)");
    } else {
        eprintln!("  Structural:      no graph (run `coral graph build` to enable)");
    }

    Ok(ExitCode::SUCCESS)
}

/// Show the current embedding index status without rebuilding.
fn show_status(index_path: &Path) -> Result<ExitCode> {
    eprintln!("\u{25b6} [Coral] Embedding index status:");

    let vectors_path = index_path.join("vectors.json");
    if vectors_path.exists() {
        let store = coral_search::VectorStore::load(&vectors_path)
            .context("loading vector store for status")?;

        let modified = std::fs::metadata(&vectors_path)
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.elapsed().ok())
            .map(|elapsed| {
                let secs = elapsed.as_secs();
                if secs < 60 {
                    format!("{secs}s ago")
                } else if secs < 3600 {
                    format!("{}m ago", secs / 60)
                } else if secs < 86400 {
                    format!("{}h ago", secs / 3600)
                } else {
                    format!("{}d ago", secs / 86400)
                }
            })
            .unwrap_or_else(|| "unknown".to_string());

        eprintln!("  Entries:      {}", store.len());
        eprintln!("  Model:        {}", store.model);
        eprintln!("  Dimensions:   {}", store.dim);
        eprintln!("  Last updated: {modified}");
    } else {
        eprintln!("  No vector store found (using TF-IDF fallback).");
        eprintln!("  Run `coral embed --provider voyage` to compute real embeddings.");
    }

    let semantic_path = index_path.join("semantic.json");
    if semantic_path.exists() {
        eprintln!("  TF-IDF index:  present");
    } else {
        eprintln!("  TF-IDF index:  not built");
    }

    let keyword_path = index_path.join("keyword");
    if keyword_path.exists() {
        eprintln!("  Keyword index: present");
    } else {
        eprintln!("  Keyword index: not built");
    }

    Ok(ExitCode::SUCCESS)
}

/// Load search configuration from `.coral/config.toml`, applying CLI overrides.
fn load_search_config(root: &Path, provider_override: &str) -> SearchConfig {
    let config_path = root.join(".coral").join("config.toml");
    let mut config = if config_path.exists() {
        std::fs::read_to_string(&config_path)
            .ok()
            .and_then(|content| {
                // Try to parse the [search] section
                #[derive(serde::Deserialize, Default)]
                #[serde(default)]
                struct CoralConfig {
                    search: SearchConfig,
                }
                toml::from_str::<CoralConfig>(&content).ok()
            })
            .map(|c| c.search)
            .unwrap_or_default()
    } else {
        SearchConfig::default()
    };

    // CLI --provider overrides config file
    if provider_override != "none" {
        config.embeddings.provider = provider_override.to_string();
        // Set sensible defaults if model/dim not configured
        if config.embeddings.model.is_empty() {
            config.embeddings.model = match provider_override {
                "voyage" => coral_runner::DEFAULT_VOYAGE_MODEL.to_string(),
                "openai" => coral_runner::DEFAULT_OPENAI_MODEL.to_string(),
                _ => String::new(),
            };
        }
        if config.embeddings.dimensions == 0 {
            config.embeddings.dimensions = match provider_override {
                "voyage" => coral_runner::DEFAULT_VOYAGE_DIM,
                "openai" => coral_runner::DEFAULT_OPENAI_DIM,
                _ => 0,
            };
        }
    }

    config
}

/// Build an embeddings provider from explicit parameters.
///
/// This is the canonical constructor for embeddings providers throughout
/// the CLI. Both `embed` and `search` commands should use this to avoid
/// duplicating provider wiring logic.
///
/// # Arguments
/// - `provider_name` — `"voyage"` or `"openai"`.
/// - `model` — Model name override; pass `None` to use the provider's default.
/// - `api_key_env` — Environment variable name that holds the API key
///   (e.g. `"VOYAGE_API_KEY"`). The function reads the key from the environment.
/// - `dimensions` — Embedding dimensions override; pass `0` or `None` for default.
pub fn build_embeddings_provider(
    provider_name: &str,
    model: Option<&str>,
    api_key_env: &str,
    dimensions: Option<usize>,
) -> Result<Box<dyn coral_runner::EmbeddingsProvider>> {
    let key = std::env::var(api_key_env)
        .with_context(|| format!("{api_key_env} not set. Required for {provider_name} embeddings."))?;

    match provider_name {
        "voyage" => {
            let model = model
                .filter(|m| !m.is_empty())
                .map(String::from)
                .unwrap_or_else(|| coral_runner::DEFAULT_VOYAGE_MODEL.to_string());
            let dim = dimensions
                .filter(|&d| d != 0)
                .unwrap_or(coral_runner::DEFAULT_VOYAGE_DIM);
            Ok(Box::new(coral_runner::VoyageProvider::new(key, model, dim)))
        }
        "openai" => {
            let model = model
                .filter(|m| !m.is_empty())
                .map(String::from)
                .unwrap_or_else(|| coral_runner::DEFAULT_OPENAI_MODEL.to_string());
            let dim = dimensions
                .filter(|&d| d != 0)
                .unwrap_or(coral_runner::DEFAULT_OPENAI_DIM);
            Ok(Box::new(coral_runner::OpenAIProvider::new(key, model, dim)))
        }
        other => anyhow::bail!("unknown embeddings provider: {other:?}. Use \"voyage\" or \"openai\"."),
    }
}

/// Create an embeddings provider from a [`SearchConfig`].
///
/// Thin wrapper around [`build_embeddings_provider`] that extracts the
/// relevant fields from the config struct.
fn create_embeddings_provider(
    config: &SearchConfig,
) -> Result<Box<dyn coral_runner::EmbeddingsProvider>> {
    let provider = config.embeddings.provider.as_str();
    let api_key_env = match provider {
        "voyage" => "VOYAGE_API_KEY",
        "openai" => "OPENAI_API_KEY",
        other => anyhow::bail!("unknown embeddings provider: {other:?}. Use \"voyage\" or \"openai\"."),
    };
    let model = Some(config.embeddings.model.as_str());
    let dimensions = Some(config.embeddings.dimensions);
    build_embeddings_provider(provider, model, api_key_env, dimensions)
}
