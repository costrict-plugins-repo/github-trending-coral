//! `coral find` — hybrid search across code and wiki.

use anyhow::{Context, Result};
use clap::Args;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use coral_graph::{load_graph, load_graph_bin, CodeGraph};
use coral_index::SymbolKind;
use coral_search::{SearchEngine, SearchQuery, SearchSource, SymbolEntry};

#[derive(Args, Debug)]
pub struct FindArgs {
    /// Search query.
    pub query: String,

    /// Maximum number of results.
    #[arg(long, default_value = "10")]
    pub limit: usize,

    /// Search source filter: keyword, semantic, or all (default).
    #[arg(long, default_value = "all")]
    pub source: String,

    /// Only search code files (exclude wiki).
    #[arg(long)]
    pub code_only: bool,

    /// Only search wiki pages.
    #[arg(long)]
    pub wiki_only: bool,

    /// Filter by symbol kind: fn, struct, enum, trait, interface, class, method, const.
    #[arg(long = "type", name = "type")]
    pub symbol_type: Option<String>,

    /// Show callers of the matched symbol (integrates graph data).
    #[arg(long)]
    pub callers: bool,
}

pub fn run(args: FindArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    let wiki_root = wiki_root.unwrap_or(Path::new(".wiki"));
    let index_path = PathBuf::from(".coral/search-index");

    // Build or load engine
    let mut engine = SearchEngine::new(&index_path)
        .context("failed to initialize search engine")?;

    // If search index is freshly created, populate it
    let keyword_path = index_path.join("keyword");
    let needs_indexing = !keyword_path.join("meta.json").exists();

    if needs_indexing {
        eprintln!("▶ [Coral] Building search index...");

        // Index wiki pages
        if wiki_root.exists() && !args.code_only {
            if let Ok(entries) = std::fs::read_dir(wiki_root) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().map(|e| e == "md").unwrap_or(false) {
                        let content = std::fs::read_to_string(&path).unwrap_or_default();
                        let _ = engine.index_wiki_page(&path, &content);
                    }
                }
            }
        }

        // Index code from coral-index if available
        let code_index_path = PathBuf::from(".coral/index.json");
        if code_index_path.exists() && !args.wiki_only {
            let index = coral_index::load_index(&code_index_path)
                .context("loading code index for search")?;
            for file in &index.files {
                let content = std::fs::read_to_string(&file.path).unwrap_or_default();
                let symbols: Vec<SymbolEntry> = file.symbols.iter().map(|s| SymbolEntry {
                    name: s.name.clone(),
                    line: s.span.start_line,
                    snippet: s.signature.clone().unwrap_or_else(|| s.name.clone()),
                }).collect();
                let _ = engine.index_file(&file.path, &content, &symbols);
            }
        }

        engine.build_semantic();
        eprintln!("✓ [Coral] Search index ready");
    }

    // Wire graph path for structural search
    let graph_path = PathBuf::from(".coral/graph.json");
    if graph_path.exists() {
        engine.set_graph_path(graph_path);
    }

    // Build the query
    let sources = match args.source.as_str() {
        "keyword" => vec![SearchSource::Keyword],
        "semantic" => vec![SearchSource::Semantic],
        "structural" => vec![SearchSource::Structural],
        _ => vec![
            SearchSource::Keyword,
            SearchSource::Semantic,
            SearchSource::Structural,
        ],
    };

    let file_filter = if args.wiki_only {
        Some("*.md".to_string())
    } else {
        None
    };

    let query = SearchQuery {
        text: args.query.clone(),
        limit: args.limit,
        sources,
        file_filter,
    };

    let mut results = engine.search(&query)?;

    // --code-only: filter to common code extensions. We can't pass a
    // comma-separated glob because `simple_glob_match` treats the entire
    // string as one pattern. Instead, filter the results post-search.
    if args.code_only {
        let code_exts = ["rs", "ts", "py", "js", "tsx", "jsx", "go", "c", "cpp", "h", "java", "rb", "sh"];
        results.retain(|r| {
            r.path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|ext| code_exts.contains(&ext))
        });
    }

    // Apply --type filter if specified
    let results = if let Some(ref type_filter) = args.symbol_type {
        filter_by_symbol_type(results, type_filter)
    } else {
        results
    };

    // Optionally load the call graph for --callers
    let call_graph = if args.callers {
        load_call_graph()
    } else {
        None
    };

    if args.callers && call_graph.is_none() {
        eprintln!("\u{26a0}\u{fe0f} No call graph found. Run `coral graph build` to enable --callers.");
    }

    if results.is_empty() {
        eprintln!("No results for '{}'.", args.query);
    } else {
        eprintln!("━━ Results for '{}' ({} found) ━━\n", args.query, results.len());
        for (i, r) in results.iter().enumerate() {
            let source_label = match r.source {
                SearchSource::Keyword => "BM25",
                SearchSource::Semantic => "TF-IDF",
                SearchSource::Structural => "Graph",
            };
            eprintln!("  {}. {} [{}] (score: {:.3})",
                i + 1,
                r.path.display(),
                source_label,
                r.score
            );
            if let Some(ref sym) = r.symbol {
                eprintln!("     symbol: {sym}");
            }
            if !r.snippet.is_empty() {
                let snippet = if r.snippet.len() > 100 {
                    format!("{}...", &r.snippet[..r.snippet.floor_char_boundary(100)])
                } else {
                    r.snippet.clone()
                };
                eprintln!("     {snippet}");
            }

            // Show callers if --callers and graph is available
            if let (true, Some(graph), Some(sym_name)) = (args.callers, &call_graph, &r.symbol) {
                let matching_nodes = graph.find_symbol(sym_name);
                let mut printed_any = false;
                for node in &matching_nodes {
                    let callers = graph.callers(&node.id);
                    if !callers.is_empty() {
                        if !printed_any {
                            eprintln!("     Called by:");
                            printed_any = true;
                        }
                        for c in &callers {
                            eprintln!("       \u{2192} {} ({}:{})", c.id.symbol, c.file.display(), c.line);
                        }
                    }
                }
            }

            eprintln!();
        }
    }

    Ok(ExitCode::SUCCESS)
}

/// Parse a `--type` flag value into a `SymbolKind`.
fn parse_symbol_kind(s: &str) -> Option<SymbolKind> {
    match s.to_lowercase().as_str() {
        "fn" | "function" => Some(SymbolKind::Function),
        "struct" => Some(SymbolKind::Struct),
        "enum" => Some(SymbolKind::Enum),
        "trait" => Some(SymbolKind::Trait),
        "interface" => Some(SymbolKind::Interface),
        "class" => Some(SymbolKind::Class),
        "method" => Some(SymbolKind::Method),
        "const" => Some(SymbolKind::Const),
        _ => None,
    }
}

/// Filter search results by symbol kind using the code index.
///
/// Loads `.coral/index.json` and cross-references each result's symbol name
/// against the indexed symbols. Results without a symbol name are excluded
/// when a type filter is active.
fn filter_by_symbol_type(
    results: Vec<coral_search::SearchResult>,
    type_filter: &str,
) -> Vec<coral_search::SearchResult> {
    let Some(target_kind) = parse_symbol_kind(type_filter) else {
        eprintln!(
            "warning: unknown --type '{}'. Valid: fn, struct, enum, trait, interface, class, method, const",
            type_filter
        );
        return results;
    };

    let code_index_path = PathBuf::from(".coral/index.json");
    let Ok(index) = coral_index::load_index(&code_index_path) else {
        eprintln!(
            "warning: could not load .coral/index.json for --type filtering. Run `coral index` first."
        );
        return results;
    };

    // Build a lookup: symbol_name -> Vec<SymbolKind> (a name can appear in
    // multiple files with the same kind, or theoretically different kinds).
    let mut symbol_kinds: std::collections::HashMap<String, Vec<SymbolKind>> =
        std::collections::HashMap::new();
    for file in &index.files {
        collect_symbol_kinds(&file.symbols, &mut symbol_kinds);
    }

    results
        .into_iter()
        .filter(|r| {
            let Some(ref sym_name) = r.symbol else {
                return false;
            };
            symbol_kinds
                .get(sym_name)
                .is_some_and(|kinds| kinds.contains(&target_kind))
        })
        .collect()
}

/// Recursively collect symbol names and their kinds from the index.
fn collect_symbol_kinds(
    symbols: &[coral_index::Symbol],
    map: &mut std::collections::HashMap<String, Vec<SymbolKind>>,
) {
    for sym in symbols {
        map.entry(sym.name.clone())
            .or_default()
            .push(sym.kind);
        if !sym.children.is_empty() {
            collect_symbol_kinds(&sym.children, map);
        }
    }
}

/// Load the call graph, preferring bincode if available, falling back to JSON.
fn load_call_graph() -> Option<CodeGraph> {
    let bin_path = PathBuf::from(".coral/graph.bin");
    let json_path = PathBuf::from(".coral/graph.json");

    if bin_path.exists() {
        if let Ok(graph) = load_graph_bin(&bin_path) {
            return Some(graph);
        }
    }

    if json_path.exists() {
        return load_graph(&json_path).ok();
    }

    None
}
