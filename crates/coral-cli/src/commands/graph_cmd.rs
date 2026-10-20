//! `coral graph` — build and query the code dependency graph.

use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use coral_graph::{
    CodeGraph, EdgeKind, FileSymbols, GraphNode, NodeKind, SymbolInfo, build_graph, load_graph,
    load_graph_bin, save_graph, save_graph_bin,
};

#[derive(Args, Debug)]
pub struct GraphArgs {
    #[command(subcommand)]
    pub command: GraphCmd,
}

#[derive(Subcommand, Debug)]
pub enum GraphCmd {
    /// Build the code graph from the index.
    Build {
        /// Root directory (defaults to current).
        #[arg(default_value = ".")]
        root: PathBuf,
    },
    /// Show callers of a symbol.
    Callers {
        /// Symbol name to look up.
        symbol: String,
    },
    /// Show callees of a symbol.
    Callees {
        /// Symbol name to look up.
        symbol: String,
    },
    /// Show the hottest symbols (highest PageRank).
    Hot {
        /// Number of symbols to show.
        #[arg(long, default_value = "10")]
        limit: usize,
    },
    /// Show what files are affected if given files change.
    Impact {
        /// Files that are changing.
        files: Vec<PathBuf>,
        /// Traversal depth.
        #[arg(long, default_value = "3")]
        depth: usize,
    },
    /// Show the call graph around a symbol (callers + callees tree).
    Show {
        /// Symbol name to visualize.
        symbol: String,
        /// Output format: ascii (default) or mermaid.
        #[arg(long, default_value = "ascii")]
        format: String,
        /// Depth of traversal in each direction.
        #[arg(long, default_value = "2")]
        depth: usize,
    },
    /// Show the dependency tree for a file — which files this file imports/calls into.
    Deps {
        /// The file path to analyze.
        file: String,
        /// Maximum depth to traverse (default 3).
        #[arg(long, default_value_t = 3)]
        depth: usize,
        /// Output format: ascii (default) or json.
        #[arg(long, default_value = "ascii")]
        format: String,
    },
}

pub fn run(args: GraphArgs, _wiki_root: Option<&Path>) -> Result<ExitCode> {
    let graph_path = PathBuf::from(".coral/graph.json");
    let graph_bin_path = PathBuf::from(".coral/graph.bin");

    match args.command {
        GraphCmd::Build { root } => {
            let index_path = root.join(".coral/index.json");
            if !index_path.exists() {
                anyhow::bail!("No index found at {}. Run `coral index` first.", index_path.display());
            }
            let index = coral_index::load_index(&index_path)
                .context("failed to load index")?;

            eprintln!("▶ [Coral] Building code graph...");

            // Convert index ParsedFiles into FileSymbols for the graph builder
            let file_symbols: Vec<FileSymbols> = index.files.iter().map(|f| {
                let symbols = convert_symbols(&f.symbols);
                let references = f.references.iter().map(|r| coral_graph::ReferenceInfo {
                    from_symbol: r.from_symbol.clone(),
                    to_symbol: r.to_symbol.clone(),
                    kind: EdgeKind::Calls,
                }).collect();
                FileSymbols {
                    path: f.path.clone(),
                    symbols,
                    imports: vec![],
                    references,
                }
            }).collect();

            let graph = build_graph(&file_symbols);
            std::fs::create_dir_all(".coral").context("failed to create .coral")?;
            save_graph(&graph, &graph_path).context("failed to save graph")?;
            save_graph_bin(&graph, &graph_bin_path).context("failed to save graph bincode")?;

            eprintln!("✓ [Coral] Graph built: {} nodes, {} edges",
                graph.node_count(), graph.edge_count());
            eprintln!("  Saved to {} + {}", graph_path.display(), graph_bin_path.display());
        }
        GraphCmd::Callers { symbol } => {
            let graph = load_graph_prefer_bin(&graph_bin_path, &graph_path)
                .context("No graph found. Run `coral graph build` first.")?;
            let nodes = graph.find_symbol(&symbol);
            if nodes.is_empty() {
                eprintln!("Symbol '{symbol}' not found in graph.");
                return Ok(ExitCode::FAILURE);
            }
            for node in &nodes {
                let callers = graph.callers(&node.id);
                if callers.is_empty() {
                    eprintln!("No callers found for '{}' (in {})", node.id.symbol, node.file.display());
                } else {
                    eprintln!("Callers of '{}' ({}):", node.id.symbol, node.file.display());
                    for c in &callers {
                        eprintln!("  ← {} ({}:{})", c.id.symbol, c.file.display(), c.line);
                    }
                }
            }
        }
        GraphCmd::Callees { symbol } => {
            let graph = load_graph_prefer_bin(&graph_bin_path, &graph_path)
                .context("No graph found. Run `coral graph build` first.")?;
            let nodes = graph.find_symbol(&symbol);
            if nodes.is_empty() {
                eprintln!("Symbol '{symbol}' not found in graph.");
                return Ok(ExitCode::FAILURE);
            }
            for node in &nodes {
                let callees = graph.callees(&node.id);
                if callees.is_empty() {
                    eprintln!("No callees found for '{}' (in {})", node.id.symbol, node.file.display());
                } else {
                    eprintln!("Callees of '{}' ({}):", node.id.symbol, node.file.display());
                    for c in &callees {
                        eprintln!("  → {} ({}:{})", c.id.symbol, c.file.display(), c.line);
                    }
                }
            }
        }
        GraphCmd::Hot { limit } => {
            let graph = load_graph_prefer_bin(&graph_bin_path, &graph_path)
                .context("No graph found. Run `coral graph build` first.")?;
            let hot = graph.hot_symbols(limit);
            eprintln!("━━ Hottest Symbols (PageRank) ━━");
            for (i, (node, score)) in hot.iter().enumerate() {
                eprintln!("  {}. {} ({}:{}) — score: {:.4}",
                    i + 1, node.id.symbol, node.file.display(), node.line, score);
            }
        }
        GraphCmd::Impact { files, depth } => {
            let graph = load_graph_prefer_bin(&graph_bin_path, &graph_path)
                .context("No graph found. Run `coral graph build` first.")?;
            eprintln!("━━ Impact Analysis ━━\n");
            for file in &files {
                let affected = graph.affected_by(file, depth);
                if affected.is_empty() {
                    eprintln!("  {} → no downstream dependents", file.display());
                } else {
                    eprintln!("  {} → {} affected files:", file.display(), affected.len());
                    for a in affected.iter().take(20) {
                        eprintln!("    ⇢ {}", a.display());
                    }
                    if affected.len() > 20 {
                        eprintln!("    ... and {} more", affected.len() - 20);
                    }
                }
            }
        }
        GraphCmd::Show { symbol, format, depth } => {
            let graph = load_graph_prefer_bin(&graph_bin_path, &graph_path)
                .context("No graph found. Run `coral graph build` first.")?;
            let nodes = graph.find_symbol(&symbol);
            if nodes.is_empty() {
                eprintln!("Symbol '{symbol}' not found in graph.");
                return Ok(ExitCode::FAILURE);
            }
            match format.as_str() {
                "mermaid" => print_mermaid(&graph, &nodes, depth),
                _ => print_ascii(&graph, &nodes, depth),
            }
        }
        GraphCmd::Deps { file, depth, format } => {
            let graph = load_graph_prefer_bin(&graph_bin_path, &graph_path)
                .context("No graph found. Run `coral graph build` first.")?;
            let file_path_buf = PathBuf::from(&file);

            // Find all symbols in the given file
            let file_symbols: Vec<&GraphNode> = graph.node_map.iter()
                .filter(|(id, _)| id.file == file_path_buf)
                .map(|(_, &idx)| &graph.graph[idx])
                .collect();

            if file_symbols.is_empty() {
                eprintln!("No symbols found in '{file}'. Check the path matches the indexed file.");
                return Ok(ExitCode::FAILURE);
            }

            // Build the dependency tree: file -> set of files it depends on
            let deps_tree = build_file_deps_tree(&graph, &file_path_buf, depth);

            match format.as_str() {
                "json" => print_deps_json(&file, &deps_tree),
                _ => print_deps_ascii(&file, &deps_tree, "", true),
            }
        }
    }

    Ok(ExitCode::SUCCESS)
}

// ─── Show subcommand helpers ───────────────────────────────────────────────

/// A tree node used for recursive caller/callee collection.
struct TreeNode {
    symbol: String,
    file: String,
    line: u32,
    children: Vec<TreeNode>,
}

/// Recursively collect callers up to `depth`.
fn collect_callers(
    graph: &CodeGraph,
    node: &GraphNode,
    depth: usize,
    visited: &mut HashSet<String>,
) -> Vec<TreeNode> {
    if depth == 0 {
        return Vec::new();
    }
    let callers = graph.callers(&node.id);
    let mut result = Vec::new();
    for caller in callers {
        let key = format!("{}::{}", caller.file.display(), caller.id.symbol);
        if visited.contains(&key) {
            continue;
        }
        visited.insert(key);
        let children = collect_callers(graph, caller, depth - 1, visited);
        result.push(TreeNode {
            symbol: caller.id.symbol.clone(),
            file: caller.file.display().to_string(),
            line: caller.line,
            children,
        });
    }
    result
}

/// Recursively collect callees up to `depth`.
fn collect_callees(
    graph: &CodeGraph,
    node: &GraphNode,
    depth: usize,
    visited: &mut HashSet<String>,
) -> Vec<TreeNode> {
    if depth == 0 {
        return Vec::new();
    }
    let callees = graph.callees(&node.id);
    let mut result = Vec::new();
    for callee in callees {
        let key = format!("{}::{}", callee.file.display(), callee.id.symbol);
        if visited.contains(&key) {
            continue;
        }
        visited.insert(key);
        let children = collect_callees(graph, callee, depth - 1, visited);
        result.push(TreeNode {
            symbol: callee.id.symbol.clone(),
            file: callee.file.display().to_string(),
            line: callee.line,
            children,
        });
    }
    result
}

/// Print a caller tree in ASCII with indentation.
fn print_caller_tree(nodes: &[TreeNode], indent: usize) {
    for node in nodes {
        // Print children first (they are further up the call chain)
        print_caller_tree(&node.children, indent + 2);
        let pad = " ".repeat(indent);
        println!("{pad}← {} ({}:{})", node.symbol, node.file, node.line);
    }
}

/// Print a callee tree in ASCII with indentation.
fn print_callee_tree(nodes: &[TreeNode], indent: usize) {
    for node in nodes {
        let pad = " ".repeat(indent);
        println!("{pad}→ {} ({}:{})", node.symbol, node.file, node.line);
        print_callee_tree(&node.children, indent + 2);
    }
}

/// Print the ASCII call-graph visualization.
fn print_ascii(graph: &CodeGraph, nodes: &[&GraphNode], depth: usize) {
    for node in nodes {
        let mut visited = HashSet::new();
        let key = format!("{}::{}", node.file.display(), node.id.symbol);
        visited.insert(key);

        let callers = collect_callers(graph, node, depth, &mut visited);
        let callees = collect_callees(graph, node, depth, &mut visited);

        println!("\n━━ Call Graph: {} ━━\n", node.id.symbol);

        println!("  Callers (who calls this):");
        if callers.is_empty() {
            println!("    (none)");
        } else {
            print_caller_tree(&callers, 4);
        }

        println!();
        println!("  ▶ {} ({}:{})", node.id.symbol, node.file.display(), node.line);
        if let Some(sig) = &node.signature {
            println!("    {sig}");
        }
        println!();

        println!("  Callees (what this calls):");
        if callees.is_empty() {
            println!("    (none)");
        } else {
            print_callee_tree(&callees, 4);
        }
        println!();
    }
}

/// Emit edges for the Mermaid diagram from a caller tree.
fn mermaid_caller_edges(
    nodes: &[TreeNode],
    target_symbol: &str,
    edges: &mut Vec<String>,
    node_labels: &mut Vec<(String, String)>,
) {
    for node in nodes {
        let label = format!(
            "{}[\"{}\\n<small>{}:{}</small>\"]",
            sanitize_mermaid_id(&node.symbol),
            node.symbol,
            node.file,
            node.line,
        );
        node_labels.push((sanitize_mermaid_id(&node.symbol), label));
        edges.push(format!(
            "    {} --> {}",
            sanitize_mermaid_id(&node.symbol),
            sanitize_mermaid_id(target_symbol),
        ));
        // Recurse: children are callers of this node
        mermaid_caller_edges(&node.children, &node.symbol, edges, node_labels);
    }
}

/// Emit edges for the Mermaid diagram from a callee tree.
fn mermaid_callee_edges(
    nodes: &[TreeNode],
    source_symbol: &str,
    edges: &mut Vec<String>,
    node_labels: &mut Vec<(String, String)>,
) {
    for node in nodes {
        let label = format!(
            "{}[\"{}\\n<small>{}:{}</small>\"]",
            sanitize_mermaid_id(&node.symbol),
            node.symbol,
            node.file,
            node.line,
        );
        node_labels.push((sanitize_mermaid_id(&node.symbol), label));
        edges.push(format!(
            "    {} --> {}",
            sanitize_mermaid_id(source_symbol),
            sanitize_mermaid_id(&node.symbol),
        ));
        // Recurse: children are callees of this node
        mermaid_callee_edges(&node.children, &node.symbol, edges, node_labels);
    }
}

/// Sanitize a symbol name for use as a Mermaid node ID.
fn sanitize_mermaid_id(s: &str) -> String {
    s.replace(|c: char| !c.is_alphanumeric() && c != '_', "_")
}

/// Print the Mermaid call-graph visualization.
fn print_mermaid(graph: &CodeGraph, nodes: &[&GraphNode], depth: usize) {
    for node in nodes {
        let mut visited = HashSet::new();
        let key = format!("{}::{}", node.file.display(), node.id.symbol);
        visited.insert(key);

        let callers = collect_callers(graph, node, depth, &mut visited);
        let callees = collect_callees(graph, node, depth, &mut visited);

        let mut edges: Vec<String> = Vec::new();
        let mut node_labels: Vec<(String, String)> = Vec::new();

        // Root node label
        let root_id = sanitize_mermaid_id(&node.id.symbol);
        let root_label = format!(
            "{root_id}[\"{}\\n<small>{}:{}</small>\"]",
            node.id.symbol,
            node.file.display(),
            node.line,
        );
        node_labels.push((root_id, root_label));

        mermaid_caller_edges(&callers, &node.id.symbol, &mut edges, &mut node_labels);
        mermaid_callee_edges(&callees, &node.id.symbol, &mut edges, &mut node_labels);

        // Deduplicate node labels by ID
        let mut seen_ids: HashSet<String> = HashSet::new();
        let unique_labels: Vec<&(String, String)> = node_labels
            .iter()
            .filter(|(id, _)| seen_ids.insert(id.clone()))
            .collect();

        println!("graph TD");
        for (_, label) in &unique_labels {
            println!("    {label}");
        }
        for edge in &edges {
            println!("{edge}");
        }
        println!();
    }
}

// ─── Deps subcommand helpers ──────────────────────────────────────────────

/// A recursive file dependency tree node.
struct FileDepsNode {
    file: String,
    deps: Vec<FileDepsNode>,
}

/// Build a tree of file dependencies: for the given file, find all files its
/// symbols call into, then recurse up to `depth`.
fn build_file_deps_tree(
    graph: &CodeGraph,
    file: &Path,
    depth: usize,
) -> Vec<FileDepsNode> {
    let mut visited = HashSet::new();
    visited.insert(file.to_path_buf());
    collect_file_deps(graph, file, depth, &mut visited)
}

/// Recursively collect file-level dependencies.
fn collect_file_deps(
    graph: &CodeGraph,
    file: &Path,
    depth: usize,
    visited: &mut HashSet<PathBuf>,
) -> Vec<FileDepsNode> {
    if depth == 0 {
        return Vec::new();
    }

    // Find all symbols in this file
    let file_symbol_ids: Vec<_> = graph.node_map.iter()
        .filter(|(id, _)| id.file == file)
        .map(|(id, _)| id.clone())
        .collect();

    // For each symbol, find its callees and map to their files
    let mut dep_files = BTreeSet::new();
    for sym_id in &file_symbol_ids {
        for callee in graph.callees(sym_id) {
            if callee.file != file {
                dep_files.insert(callee.file.clone());
            }
        }
    }

    let mut result = Vec::new();
    for dep_file in dep_files {
        let children = if visited.insert(dep_file.clone()) {
            collect_file_deps(graph, &dep_file, depth - 1, visited)
        } else {
            Vec::new() // already visited — avoid cycles
        };
        result.push(FileDepsNode {
            file: dep_file.display().to_string(),
            deps: children,
        });
    }

    result
}

/// Print the dependency tree in ASCII format.
fn print_deps_ascii(file: &str, deps: &[FileDepsNode], prefix: &str, is_root: bool) {
    if is_root {
        println!("{file}");
        for (i, dep) in deps.iter().enumerate() {
            let is_last = i == deps.len() - 1;
            let connector = if is_last { "\u{2514}\u{2500}\u{2500} " } else { "\u{251c}\u{2500}\u{2500} " };
            let child_prefix = if is_last { "    " } else { "\u{2502}   " };
            println!("{prefix}{connector}{}", dep.file);
            print_deps_ascii(&dep.file, &dep.deps, &format!("{prefix}{child_prefix}"), false);
        }
    } else {
        for (i, dep) in deps.iter().enumerate() {
            let is_last = i == deps.len() - 1;
            let connector = if is_last { "\u{2514}\u{2500}\u{2500} " } else { "\u{251c}\u{2500}\u{2500} " };
            let child_prefix = if is_last { "    " } else { "\u{2502}   " };
            println!("{prefix}{connector}{}", dep.file);
            print_deps_ascii(&dep.file, &dep.deps, &format!("{prefix}{child_prefix}"), false);
        }
    }
}

/// Print the dependency tree in JSON format.
fn print_deps_json(file: &str, deps: &[FileDepsNode]) {
    let json = deps_to_json_value(file, deps);
    // json! macro output is always serializable
    #[allow(clippy::unwrap_used, reason = "serde_json::Value is infallibly serialisable")]
    let pretty = serde_json::to_string_pretty(&json).unwrap();
    println!("{pretty}");
}

/// Convert the deps tree into a serde_json::Value.
fn deps_to_json_value(file: &str, deps: &[FileDepsNode]) -> serde_json::Value {
    let dep_values: Vec<serde_json::Value> = deps.iter()
        .map(|d| deps_to_json_value(&d.file, &d.deps))
        .collect();
    serde_json::json!({
        "file": file,
        "deps": dep_values,
    })
}

/// Load graph preferring bincode if available, falling back to JSON.
fn load_graph_prefer_bin(bin_path: &Path, json_path: &Path) -> Result<CodeGraph> {
    if bin_path.exists() {
        load_graph_bin(bin_path)
    } else {
        load_graph(json_path)
    }
}

/// Convert coral-index Symbols to coral-graph SymbolInfo.
fn convert_symbols(symbols: &[coral_index::Symbol]) -> Vec<SymbolInfo> {
    symbols.iter().map(|s| {
        let kind = match s.kind {
            coral_index::SymbolKind::Function => NodeKind::Function,
            coral_index::SymbolKind::Struct => NodeKind::Struct,
            coral_index::SymbolKind::Enum => NodeKind::Enum,
            coral_index::SymbolKind::Trait => NodeKind::Trait,
            coral_index::SymbolKind::Interface => NodeKind::Interface,
            coral_index::SymbolKind::Class => NodeKind::Class,
            coral_index::SymbolKind::Const => NodeKind::Const,
            coral_index::SymbolKind::Static => NodeKind::Const,
            coral_index::SymbolKind::TypeAlias => NodeKind::TypeAlias,
            coral_index::SymbolKind::Module => NodeKind::Module,
            coral_index::SymbolKind::Method => NodeKind::Method,
            coral_index::SymbolKind::Field => NodeKind::Const,
        };
        SymbolInfo {
            name: s.name.clone(),
            kind,
            signature: s.signature.clone(),
            line: s.span.start_line,
            children: convert_symbols(&s.children),
        }
    }).collect()
}
