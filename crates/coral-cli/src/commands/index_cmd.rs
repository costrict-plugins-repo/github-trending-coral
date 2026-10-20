//! `coral index` — build or update the code index (tree-sitter AST + symbols).

use anyhow::{Context, Result};
use clap::Args;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use coral_index::{IndexConfig, Language, index_incremental, index_repo, load_index, save_index};

/// Check if the code index is stale and re-index if needed.
/// Returns `Ok(())` silently if the index is fresh or doesn't exist yet.
/// Only runs for commands that benefit from a fresh index.
pub fn auto_index_if_stale(project_root: &Path) -> Result<()> {
    let index_path = project_root.join(".coral/index.json");
    if !index_path.exists() {
        return Ok(()); // No index yet — don't create one automatically
    }

    // Compare index mtime against source file mtimes (sample first 50 files).
    let index_mtime = match std::fs::metadata(&index_path).and_then(|m| m.modified()) {
        Ok(mt) => mt,
        Err(_) => return Ok(()), // Can't stat index — skip
    };

    let prev = match load_index(&index_path) {
        Ok(idx) => idx,
        Err(_) => return Ok(()), // Corrupt index — skip, don't block the command
    };

    // Sample the first 50 indexed files and check if any source is newer.
    let any_stale = prev.files.iter().take(50).any(|f| {
        let src = if f.path.is_absolute() {
            f.path.clone()
        } else {
            project_root.join(&f.path)
        };
        std::fs::metadata(&src)
            .and_then(|m| m.modified())
            .map(|src_mt| src_mt > index_mtime)
            .unwrap_or(false)
    });

    if !any_stale {
        return Ok(());
    }

    eprintln!("▶ [Coral] Auto-indexing (index stale)...");

    let config = IndexConfig {
        root: project_root.to_path_buf(),
        languages: vec![
            Language::Rust,
            Language::TypeScript,
            Language::JavaScript,
            Language::Python,
            Language::Go,
            Language::Java,
            Language::C,
            Language::Cpp,
        ],
        exclude_patterns: vec![
            "target/".into(),
            "node_modules/".into(),
            ".git/".into(),
            "__pycache__/".into(),
            "dist/".into(),
            "build/".into(),
        ],
    };

    let idx = index_incremental(&config, &prev).context("auto-index incremental failed")?;
    let store_path = project_root.join(".coral/index.json");
    save_index(&idx, &store_path).context("auto-index save failed")?;
    eprintln!(
        "✓ [Coral] Auto-indexed: {} files, {} symbols",
        idx.files.len(),
        idx.symbol_count()
    );

    Ok(())
}

#[derive(Args, Debug)]
pub struct IndexArgs {
    /// Root directory to index (defaults to current directory).
    #[arg(default_value = ".")]
    pub root: PathBuf,

    /// Force full reindex (ignore incremental cache).
    #[arg(long)]
    pub full: bool,

    /// Print symbol summary after indexing.
    #[arg(long)]
    pub summary: bool,
}

pub fn run(args: IndexArgs, _wiki_root: Option<&Path>) -> Result<ExitCode> {
    let root = args.root.canonicalize().unwrap_or(args.root.clone());
    let store_path = root.join(".coral").join("index.json");

    let config = IndexConfig {
        root: root.clone(),
        languages: vec![Language::Rust, Language::TypeScript, Language::JavaScript, Language::Python, Language::Go, Language::Java, Language::C, Language::Cpp],
        exclude_patterns: vec![
            "target/".into(),
            "node_modules/".into(),
            ".git/".into(),
            "__pycache__/".into(),
            "dist/".into(),
            "build/".into(),
        ],
    };

    let index = if args.full || !store_path.exists() {
        eprintln!("▶ [Coral] Indexing repository (full scan)...");
        let idx = index_repo(&config).context("failed to index repository")?;
        eprintln!("✓ [Coral] Indexed {} files, {} symbols",
            idx.files.len(),
            idx.symbol_count()
        );
        idx
    } else {
        eprintln!("▶ [Coral] Incremental index update...");
        let prev = load_index(&store_path).context("failed to load previous index")?;
        let idx = index_incremental(&config, &prev).context("incremental index failed")?;
        eprintln!("✓ [Coral] Updated index: {} files, {} symbols",
            idx.files.len(),
            idx.symbol_count()
        );
        idx
    };

    // Save
    std::fs::create_dir_all(store_path.parent().unwrap_or(Path::new(".")))
        .context("failed to create .coral directory")?;
    save_index(&index, &store_path).context("failed to save index")?;
    eprintln!("  Saved to {}", store_path.display());

    if args.summary {
        eprintln!("\n━━ Symbol Summary ━━");
        let mut lang_counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
        for f in &index.files {
            let lang = match f.language {
                Language::Rust => "rust",
                Language::TypeScript => "typescript",
                Language::JavaScript => "javascript",
                Language::Python => "python",
                Language::Go => "go",
                Language::Java => "java",
                Language::C => "c",
                Language::Cpp => "cpp",
            };
            *lang_counts.entry(lang).or_default() += f.symbols.len();
        }
        let mut langs: Vec<_> = lang_counts.into_iter().collect();
        langs.sort_by(|a, b| b.1.cmp(&a.1));
        for (lang, count) in &langs {
            eprintln!("  {lang}: {count} symbols");
        }
    }

    Ok(ExitCode::SUCCESS)
}
