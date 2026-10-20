use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::parser::{detect_language, parse_file, Language, ParsedFile};

/// Configuration for indexing a repository.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexConfig {
    /// Languages to index.
    pub languages: Vec<Language>,
    /// Glob patterns to exclude (e.g., "target/", "node_modules/").
    pub exclude_patterns: Vec<String>,
    /// Root directory of the repository.
    pub root: PathBuf,
}

/// A complete index of a repository's symbols.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoIndex {
    /// All parsed files in the index.
    pub files: Vec<ParsedFile>,
    /// Map of file path -> blake3 hash for incremental indexing.
    pub file_hashes: HashMap<PathBuf, String>,
}

impl RepoIndex {
    /// Create an empty index.
    pub fn new() -> Self {
        Self {
            files: Vec::new(),
            file_hashes: HashMap::new(),
        }
    }

    /// Total number of symbols across all files.
    pub fn symbol_count(&self) -> usize {
        self.files
            .iter()
            .map(|f| count_symbols_recursive(&f.symbols))
            .sum()
    }
}

impl Default for RepoIndex {
    fn default() -> Self {
        Self::new()
    }
}

fn count_symbols_recursive(symbols: &[crate::symbol::Symbol]) -> usize {
    symbols
        .iter()
        .map(|s| 1 + count_symbols_recursive(&s.children))
        .sum()
}

/// Index an entire repository according to the configuration.
///
/// Walks the file tree, filters by language and exclusion patterns,
/// and parses all matching files in parallel using rayon.
pub fn index_repo(config: &IndexConfig) -> Result<RepoIndex> {
    let files_to_parse = collect_files(config)?;

    tracing::info!(
        "Indexing {} files in {}",
        files_to_parse.len(),
        config.root.display()
    );

    let results: Vec<ParsedFile> = files_to_parse
        .par_iter()
        .filter_map(|path| {
            match parse_file(path) {
                Ok(parsed) => Some(parsed),
                Err(e) => {
                    tracing::warn!("Failed to parse {}: {}", path.display(), e);
                    None
                }
            }
        })
        .collect();

    let file_hashes: HashMap<PathBuf, String> = results
        .iter()
        .map(|f| (f.path.clone(), f.content_hash.clone()))
        .collect();

    Ok(RepoIndex {
        files: results,
        file_hashes,
    })
}

/// Incrementally index a repository, only re-parsing files whose content has changed.
///
/// Compares file hashes from the previous index against current file content.
/// Files with unchanged hashes are carried over from the previous index.
pub fn index_incremental(config: &IndexConfig, previous: &RepoIndex) -> Result<RepoIndex> {
    let files_to_check = collect_files(config)?;

    tracing::info!(
        "Incremental index: checking {} files against {} cached",
        files_to_check.len(),
        previous.file_hashes.len()
    );

    // Compute current hashes in parallel
    let current_hashes: Vec<(PathBuf, String)> = files_to_check
        .par_iter()
        .filter_map(|path| {
            let content = std::fs::read(path).ok()?;
            let hash = blake3::hash(&content).to_hex().to_string();
            Some((path.clone(), hash))
        })
        .collect();

    // Determine which files need re-parsing
    let mut changed_files = Vec::new();
    let mut unchanged_paths = Vec::new();

    for (path, hash) in &current_hashes {
        match previous.file_hashes.get(path) {
            Some(prev_hash) if prev_hash == hash => {
                unchanged_paths.push(path.clone());
            }
            _ => {
                changed_files.push(path.clone());
            }
        }
    }

    tracing::info!(
        "Incremental: {} unchanged, {} changed/new",
        unchanged_paths.len(),
        changed_files.len()
    );

    // Parse only changed files
    let mut new_files: Vec<ParsedFile> = changed_files
        .par_iter()
        .filter_map(|path| {
            match parse_file(path) {
                Ok(parsed) => Some(parsed),
                Err(e) => {
                    tracing::warn!("Failed to parse {}: {}", path.display(), e);
                    None
                }
            }
        })
        .collect();

    // Carry over unchanged files from previous index
    let unchanged_set: std::collections::HashSet<&PathBuf> = unchanged_paths.iter().collect();
    for prev_file in &previous.files {
        if unchanged_set.contains(&prev_file.path) {
            new_files.push(prev_file.clone());
        }
    }

    let file_hashes: HashMap<PathBuf, String> = new_files
        .iter()
        .map(|f| (f.path.clone(), f.content_hash.clone()))
        .collect();

    Ok(RepoIndex {
        files: new_files,
        file_hashes,
    })
}

/// Collect all source files matching the index configuration.
fn collect_files(config: &IndexConfig) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();

    for entry in WalkDir::new(&config.root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| !is_excluded(e.path(), &config.root, &config.exclude_patterns))
    {
        let entry = entry.context("failed to read directory entry")?;
        if !entry.file_type().is_file() {
            continue;
        }

        let path = entry.path();
        if let Some(lang) = detect_language(path)
            && config.languages.contains(&lang)
        {
            files.push(path.to_path_buf());
        }
    }

    Ok(files)
}

/// Check if a path should be excluded based on patterns.
fn is_excluded(path: &Path, root: &Path, patterns: &[String]) -> bool {
    let relative = path.strip_prefix(root).unwrap_or(path);
    let rel_str = relative.to_string_lossy();

    for pattern in patterns {
        let pat = pattern.trim_end_matches('/');
        // Simple prefix/contains matching
        if rel_str.starts_with(pat) || rel_str.contains(&format!("/{pat}/")) {
            return true;
        }
        // Check if any path component matches
        for component in relative.components() {
            if component.as_os_str().to_string_lossy() == pat {
                return true;
            }
        }
    }

    // Always exclude common directories
    let always_exclude = [
        ".git",
        "node_modules",
        "target",
        "__pycache__",
        ".venv",
        "venv",
        "dist",
        "build",
    ];
    for component in relative.components() {
        let comp_str = component.as_os_str().to_string_lossy();
        if always_exclude.contains(&comp_str.as_ref()) {
            return true;
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn setup_test_repo() -> TempDir {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        // Create some source files
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(
            root.join("src/main.rs"),
            "pub fn main() { println!(\"hello\"); }\n",
        )
        .unwrap();
        fs::write(
            root.join("src/lib.rs"),
            "pub struct App {}\nimpl App { pub fn new() -> Self { Self {} } }\n",
        )
        .unwrap();

        // Create a Python file
        fs::write(
            root.join("app.py"),
            "def hello():\n    \"\"\"Say hello.\"\"\"\n    print('hello')\n",
        )
        .unwrap();

        // Create an excluded directory
        fs::create_dir_all(root.join("target/debug")).unwrap();
        fs::write(root.join("target/debug/main.rs"), "fn ignored() {}\n").unwrap();

        dir
    }

    #[test]
    fn test_index_repo_basic() {
        let dir = setup_test_repo();
        let config = IndexConfig {
            languages: vec![Language::Rust, Language::Python],
            exclude_patterns: vec![],
            root: dir.path().to_path_buf(),
        };

        let index = index_repo(&config).unwrap();
        // Should find main.rs, lib.rs, app.py (not target/debug/main.rs)
        assert_eq!(index.files.len(), 3);
        assert!(index.symbol_count() > 0);
    }

    #[test]
    fn test_index_repo_excludes_target() {
        let dir = setup_test_repo();
        let config = IndexConfig {
            languages: vec![Language::Rust],
            exclude_patterns: vec![],
            root: dir.path().to_path_buf(),
        };

        let index = index_repo(&config).unwrap();
        // Should NOT include target/debug/main.rs
        for file in &index.files {
            assert!(
                !file.path.to_string_lossy().contains("target"),
                "Should exclude target directory"
            );
        }
    }

    #[test]
    fn test_index_incremental_unchanged() {
        let dir = setup_test_repo();
        let config = IndexConfig {
            languages: vec![Language::Rust],
            exclude_patterns: vec![],
            root: dir.path().to_path_buf(),
        };

        let first_index = index_repo(&config).unwrap();
        let second_index = index_incremental(&config, &first_index).unwrap();

        // Same files, same hashes
        assert_eq!(first_index.files.len(), second_index.files.len());
        for (path, hash) in &second_index.file_hashes {
            assert_eq!(first_index.file_hashes.get(path), Some(hash));
        }
    }

    #[test]
    fn test_index_incremental_detects_changes() {
        let dir = setup_test_repo();
        let config = IndexConfig {
            languages: vec![Language::Rust],
            exclude_patterns: vec![],
            root: dir.path().to_path_buf(),
        };

        let first_index = index_repo(&config).unwrap();

        // Modify a file
        fs::write(
            dir.path().join("src/main.rs"),
            "pub fn main() { println!(\"changed\"); }\npub fn other() {}\n",
        )
        .unwrap();

        let second_index = index_incremental(&config, &first_index).unwrap();

        // The file should have been re-parsed with a different hash
        let main_path = dir.path().join("src/main.rs");
        assert_ne!(
            first_index.file_hashes.get(&main_path),
            second_index.file_hashes.get(&main_path)
        );
    }

    #[test]
    fn test_is_excluded_patterns() {
        let root = Path::new("/repo");
        let patterns = vec!["vendor".to_string(), "generated".to_string()];

        assert!(is_excluded(Path::new("/repo/vendor"), root, &patterns));
        assert!(is_excluded(
            Path::new("/repo/src/generated"),
            root,
            &patterns
        ));
        assert!(!is_excluded(Path::new("/repo/src/main.rs"), root, &patterns));
    }
}
