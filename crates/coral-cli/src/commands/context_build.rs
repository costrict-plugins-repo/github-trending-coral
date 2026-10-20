//! `coral context build --query <q> --budget <tokens>`
//!
//! Smart context loader: greedy fill of the `--budget` token target
//! by ranking wiki pages and code snippets via hybrid search (BM25 +
//! semantic + structural from coral-search), then expanding matched
//! symbols with caller/callee context from coral-graph. Output is a
//! single Markdown blob or JSON ready to paste into any prompt.
//!
//! v0.41: upgraded from wiki-only TF-IDF to hybrid search engine with
//! interleaved code chunks and graph context.

use anyhow::{Context, Result};
use clap::Args;
use coral_core::{search, walk};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::commands::common::resolve_project;

/// A single chunk of context included in the output, ranked by relevance.
#[derive(Debug, Clone)]
enum ContextChunk {
    WikiPage {
        slug: String,
        confidence: f64,
        body: String,
        score: f64,
    },
    CodeSnippet {
        path: PathBuf,
        symbol: Option<String>,
        snippet: String,
        line: Option<u32>,
        score: f64,
    },
    GraphContext {
        symbol: String,
        file: PathBuf,
        callers: Vec<String>,
        callees: Vec<String>,
    },
}

impl ContextChunk {
    /// Approximate token cost (4 chars/token heuristic).
    fn char_cost(&self) -> usize {
        match self {
            Self::WikiPage { body, slug, .. } => body.len() + slug.len() + 40,
            Self::CodeSnippet { snippet, path, symbol, .. } => {
                snippet.len() + path.to_string_lossy().len()
                    + symbol.as_ref().map(|s| s.len()).unwrap_or(0)
                    + 40
            }
            Self::GraphContext { symbol, callers, callees, .. } => {
                symbol.len()
                    + callers.iter().map(|c| c.len() + 4).sum::<usize>()
                    + callees.iter().map(|c| c.len() + 4).sum::<usize>()
                    + 60
            }
        }
    }

    /// Sort key: higher is better.
    fn score(&self) -> f64 {
        match self {
            Self::WikiPage { score, .. } => *score,
            Self::CodeSnippet { score, .. } => *score,
            // Graph context is supplementary — scored below search results
            // but above zero so it fills remaining budget.
            Self::GraphContext { .. } => 0.1,
        }
    }
}

#[derive(Args, Debug)]
pub struct ContextBuildArgs {
    /// The natural-language query that ranks pages.
    #[arg(long)]
    pub query: String,

    /// Token budget. Approximate: we use 4 chars/token as a heuristic
    /// (works well enough for English Markdown; the model will count
    /// more precisely on its end).
    #[arg(long, default_value_t = 50_000)]
    pub budget: usize,

    /// Output format. Markdown is the default (paste-friendly); JSON
    /// is for programmatic callers.
    #[arg(long, default_value = "markdown")]
    pub format: Format,

    /// How many seed pages to pull before expanding via backlinks.
    /// Default 8 covers most "explain X" queries; raise for broader
    /// surveys.
    #[arg(long, default_value_t = 8)]
    pub seeds: usize,

    /// Include code search results (default: true).
    #[arg(long, default_value_t = true)]
    pub include_code: bool,

    /// Include graph relationships for matched symbols (default: true).
    #[arg(long, default_value_t = true)]
    pub include_graph: bool,
}

#[derive(clap::ValueEnum, Clone, Debug)]
pub enum Format {
    Markdown,
    Json,
}

pub fn run(args: ContextBuildArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    let project = resolve_project(wiki_root)?;
    let project_root = &project.root;

    // Try hybrid search engine first; fall back to wiki-only TF-IDF.
    let search_index_path = project_root.join(".coral").join("search-index");
    let graph_path = project_root.join(".coral").join("graph.json");

    let has_search_index = search_index_path.join("keyword").join("meta.json").exists();

    if has_search_index {
        run_hybrid(&args, project_root, &search_index_path, &graph_path)
    } else {
        // Fall back to legacy wiki-only behavior.
        run_wiki_only(&args, &project)
    }
}

/// New hybrid path: uses coral-search engine + coral-graph.
fn run_hybrid(
    args: &ContextBuildArgs,
    project_root: &Path,
    search_index_path: &Path,
    graph_path: &Path,
) -> Result<ExitCode> {
    use coral_search::{SearchEngine, SearchQuery, SearchSource};

    let mut engine = SearchEngine::new(search_index_path)
        .context("failed to open search index")?;

    // Wire graph for structural search.
    if graph_path.exists() {
        engine.set_graph_path(graph_path.to_path_buf());
    }

    // Determine which sources to query.
    let mut sources = vec![SearchSource::Keyword, SearchSource::Semantic];
    if graph_path.exists() {
        sources.push(SearchSource::Structural);
    }

    let query = SearchQuery {
        text: args.query.clone(),
        limit: args.seeds * 4, // fetch more candidates than needed; budget trims
        sources,
        file_filter: None,
    };

    let results = engine.search(&query)?;

    // Convert search results into ContextChunks.
    let wiki_root = project_root.join(".wiki");
    let mut chunks: Vec<ContextChunk> = Vec::new();

    for r in &results {
        let is_wiki = r.path.starts_with(&wiki_root)
            || r.path.to_string_lossy().starts_with(".wiki")
            || r.path.extension().map(|e| e == "md").unwrap_or(false)
                && r.path.to_string_lossy().contains(".wiki");

        if is_wiki {
            // Load wiki page body and extract slug from filename.
            let slug = r
                .path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string();

            // Try to read the full page for body + confidence.
            let page_path = if r.path.is_absolute() {
                r.path.clone()
            } else {
                project_root.join(&r.path)
            };

            let body = std::fs::read_to_string(&page_path).unwrap_or_else(|_| r.snippet.clone());

            // Extract confidence from frontmatter if available.
            let confidence = extract_confidence(&body);

            chunks.push(ContextChunk::WikiPage {
                slug,
                confidence,
                body: strip_frontmatter(&body),
                score: r.score,
            });
        } else if args.include_code {
            // Code result — load snippet with surrounding context.
            let file_path = if r.path.is_absolute() {
                r.path.clone()
            } else {
                project_root.join(&r.path)
            };

            let snippet = if let Some(line) = r.line {
                extract_code_snippet(&file_path, line, 10)
            } else if !r.snippet.is_empty() {
                r.snippet.clone()
            } else {
                // Fall back to reading first N lines of the file.
                extract_code_snippet(&file_path, 1, 20)
            };

            chunks.push(ContextChunk::CodeSnippet {
                path: r.path.clone(),
                symbol: r.symbol.clone(),
                snippet,
                line: r.line,
                score: r.score,
            });
        }
    }

    // Add graph context for top matched symbols.
    if args.include_graph && graph_path.exists() {
        if let Ok(graph) = coral_graph::load_graph(graph_path) {
            // Collect unique symbols from top code results.
            let top_symbols: Vec<String> = results
                .iter()
                .filter_map(|r| r.symbol.clone())
                .take(5)
                .collect();

            for sym_name in &top_symbols {
                let nodes = graph.find_symbol(sym_name);
                for node in nodes.iter().take(1) {
                    let node_id = &node.id;
                    let callers: Vec<String> = graph
                        .callers(node_id)
                        .iter()
                        .take(5)
                        .map(|n| {
                            n.signature
                                .clone()
                                .unwrap_or_else(|| n.id.symbol.clone())
                        })
                        .collect();
                    let callees: Vec<String> = graph
                        .callees(node_id)
                        .iter()
                        .take(5)
                        .map(|n| {
                            n.signature
                                .clone()
                                .unwrap_or_else(|| n.id.symbol.clone())
                        })
                        .collect();

                    if !callers.is_empty() || !callees.is_empty() {
                        chunks.push(ContextChunk::GraphContext {
                            symbol: node.id.symbol.clone(),
                            file: node.file.clone(),
                            callers,
                            callees,
                        });
                    }
                }
            }
        }
    }

    // Sort by score descending, then fill under budget.
    chunks.sort_by(|a, b| {
        b.score()
            .partial_cmp(&a.score())
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let budget_chars = args.budget.saturating_mul(4);
    let mut chars_used = 0usize;
    let mut selected: Vec<ContextChunk> = Vec::new();

    for chunk in chunks {
        let cost = chunk.char_cost();
        if chars_used + cost > budget_chars {
            continue; // skip this chunk, try smaller ones
        }
        chars_used += cost;
        selected.push(chunk);
    }

    match args.format {
        Format::Markdown => print_mixed_markdown(&args.query, &selected, args.budget),
        Format::Json => print_mixed_json(&args.query, &selected)?,
    }

    Ok(ExitCode::SUCCESS)
}

/// Legacy wiki-only path: converts wiki pages into [`ContextChunk::WikiPage`]
/// items and reuses the shared hybrid output formatters.
fn run_wiki_only(
    args: &ContextBuildArgs,
    project: &coral_core::project::Project,
) -> Result<ExitCode> {
    let pages = walk::read_pages(project.wiki_root())
        .with_context(|| format!("walking wiki at {}", project.wiki_root().display()))?;
    if pages.is_empty() {
        anyhow::bail!(
            "no wiki pages found at {}; run `coral init` first",
            project.wiki_root().display()
        );
    }

    let ranking = rank_pages(&pages, &args.query);
    let mut selected_slugs: Vec<String> = ranking
        .iter()
        .take(args.seeds)
        .map(|(slug, _)| slug.clone())
        .collect();

    // BFS over backlinks until budget exhausts.
    let mut included = std::collections::BTreeSet::new();
    included.extend(selected_slugs.iter().cloned());
    let mut chars_used = 0usize;
    let budget_chars = args.budget.saturating_mul(4);

    let mut pending: std::collections::VecDeque<String> =
        selected_slugs.iter().cloned().collect();
    while let Some(slug) = pending.pop_front() {
        let page = match pages.iter().find(|p| p.frontmatter.slug == slug) {
            Some(p) => p,
            None => continue,
        };
        if chars_used + page.body.len() > budget_chars {
            included.remove(&slug);
            continue;
        }
        chars_used += page.body.len();
        for backlink in &page.frontmatter.backlinks {
            if !included.contains(backlink) {
                included.insert(backlink.clone());
                pending.push_back(backlink.clone());
                selected_slugs.push(backlink.clone());
            }
        }
    }

    // Build ranked page list, then convert to ContextChunks for the shared formatter.
    let mut result_pages: Vec<&coral_core::page::Page> = pages
        .iter()
        .filter(|p| included.contains(&p.frontmatter.slug))
        .collect();
    result_pages.sort_by(|a, b| {
        b.frontmatter
            .confidence
            .as_f64()
            .partial_cmp(&a.frontmatter.confidence.as_f64())
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.body.len().cmp(&b.body.len()))
    });

    // Convert wiki pages to ContextChunk::WikiPage, using confidence as score.
    let chunks: Vec<ContextChunk> = result_pages
        .iter()
        .map(|p| ContextChunk::WikiPage {
            slug: p.frontmatter.slug.clone(),
            confidence: p.frontmatter.confidence.as_f64(),
            body: p.body.clone(),
            score: p.frontmatter.confidence.as_f64(),
        })
        .collect();

    match args.format {
        Format::Markdown => print_mixed_markdown(&args.query, &chunks, args.budget),
        Format::Json => print_mixed_json(&args.query, &chunks)?,
    }

    Ok(ExitCode::SUCCESS)
}

// ─── Hybrid output formatters ───────────────────────────────────────

fn print_mixed_markdown(query: &str, chunks: &[ContextChunk], budget: usize) {
    println!("# Coral context for: \"{query}\"\n");

    let wiki_count = chunks
        .iter()
        .filter(|c| matches!(c, ContextChunk::WikiPage { .. }))
        .count();
    let code_count = chunks
        .iter()
        .filter(|c| matches!(c, ContextChunk::CodeSnippet { .. }))
        .count();
    let graph_count = chunks
        .iter()
        .filter(|c| matches!(c, ContextChunk::GraphContext { .. }))
        .count();

    println!(
        "_Loaded {} wiki page(s), {} code snippet(s), {} graph context(s) \
         under a budget of {} tokens (~4 chars/token)._\n",
        wiki_count, code_count, graph_count, budget
    );

    for chunk in chunks {
        match chunk {
            ContextChunk::WikiPage {
                slug,
                confidence,
                body,
                score,
            } => {
                println!("---\n");
                println!("## [wiki] {slug} (confidence={confidence:.2}, score={score:.3})");
                println!("{}\n", body.trim());
            }
            ContextChunk::CodeSnippet {
                path,
                symbol,
                snippet,
                line,
                score,
            } => {
                println!("---\n");
                let sym_label = symbol
                    .as_ref()
                    .map(|s| format!(" :: {s}"))
                    .unwrap_or_default();
                let line_label = line
                    .map(|l| format!(":{l}"))
                    .unwrap_or_default();
                println!(
                    "## [code] {}{}{} (score={:.3})",
                    path.display(),
                    line_label,
                    sym_label,
                    score
                );
                // Detect language for fenced code block.
                let lang = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("");
                println!("```{lang}");
                println!("{}", snippet.trim());
                println!("```\n");
            }
            ContextChunk::GraphContext {
                symbol,
                file,
                callers,
                callees,
            } => {
                println!("---\n");
                println!("## [graph] {symbol} ({})", file.display());
                if !callers.is_empty() {
                    println!("\n**Called by:**");
                    for c in callers {
                        println!("- `{c}`");
                    }
                }
                if !callees.is_empty() {
                    println!("\n**Calls:**");
                    for c in callees {
                        println!("- `{c}`");
                    }
                }
                println!();
            }
        }
    }
}

fn print_mixed_json(query: &str, chunks: &[ContextChunk]) -> Result<()> {
    let items: Vec<serde_json::Value> = chunks
        .iter()
        .map(|chunk| match chunk {
            ContextChunk::WikiPage {
                slug,
                confidence,
                body,
                score,
            } => serde_json::json!({
                "type": "wiki",
                "slug": slug,
                "confidence": confidence,
                "body": body,
                "score": score,
            }),
            ContextChunk::CodeSnippet {
                path,
                symbol,
                snippet,
                line,
                score,
            } => serde_json::json!({
                "type": "code",
                "path": path.to_string_lossy(),
                "symbol": symbol,
                "snippet": snippet,
                "line": line,
                "score": score,
            }),
            ContextChunk::GraphContext {
                symbol,
                file,
                callers,
                callees,
            } => serde_json::json!({
                "type": "graph",
                "symbol": symbol,
                "file": file.to_string_lossy(),
                "callers": callers,
                "callees": callees,
            }),
        })
        .collect();

    let json = serde_json::json!({
        "query": query,
        "chunks": items,
    });
    println!("{}", serde_json::to_string_pretty(&json)?);
    Ok(())
}

// ─── Helpers ────────────────────────────────────────────────────────

/// Rank pages using RRF hybrid search (BM25 + TF-IDF fusion).
/// Returns all pages ranked by relevance — the caller picks the top seeds.
fn rank_pages(pages: &[coral_core::page::Page], query: &str) -> Vec<(String, f64)> {
    let results = search::search_hybrid(pages, query, pages.len());
    results.into_iter().map(|r| (r.slug, r.score)).collect()
}

/// Extract a code snippet from a file: +-`context_lines` around `target_line`.
fn extract_code_snippet(path: &Path, target_line: u32, context_lines: u32) -> String {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return String::new(),
    };

    let lines: Vec<&str> = content.lines().collect();
    if lines.is_empty() {
        return String::new();
    }

    let target = target_line.saturating_sub(1) as usize; // 0-indexed
    let start = target.saturating_sub(context_lines as usize);
    let end = (target + context_lines as usize + 1).min(lines.len());

    lines[start..end].join("\n")
}

/// Extract confidence value from YAML frontmatter (best-effort).
fn extract_confidence(body: &str) -> f64 {
    if !body.starts_with("---") {
        return 0.5; // default when no frontmatter
    }
    // Find the closing `---`.
    let after_first = &body[3..];
    let end = match after_first.find("---") {
        Some(pos) => pos,
        None => return 0.5,
    };
    let frontmatter = &after_first[..end];

    for line in frontmatter.lines() {
        let trimmed = line.trim();
        if let Some(f) = trimmed
            .strip_prefix("confidence:")
            .and_then(|val| val.trim().parse::<f64>().ok())
        {
            return f;
        }
    }
    0.5
}

/// Strip YAML frontmatter from a wiki page body.
fn strip_frontmatter(body: &str) -> String {
    if !body.starts_with("---") {
        return body.to_string();
    }
    let after_first = &body[3..];
    match after_first.find("---") {
        Some(pos) => after_first[pos + 3..].trim_start().to_string(),
        None => body.to_string(),
    }
}
