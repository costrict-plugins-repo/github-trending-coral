use std::path::Path;

use anyhow::{Context, Result};

use crate::indexer::RepoIndex;

/// Save a `RepoIndex` to disk as JSON.
///
/// Writes to the specified path (typically `.coral/index/symbols.json`).
/// Creates parent directories if they don't exist.
pub fn save_index(index: &RepoIndex, path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create directory: {}", parent.display()))?;
    }

    let json = serde_json::to_string_pretty(index)
        .context("failed to serialize index to JSON")?;

    std::fs::write(path, json)
        .with_context(|| format!("failed to write index to: {}", path.display()))?;

    tracing::info!("Saved index with {} files to {}", index.files.len(), path.display());
    Ok(())
}

/// Load a `RepoIndex` from a JSON file on disk.
pub fn load_index(path: &Path) -> Result<RepoIndex> {
    let json = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read index from: {}", path.display()))?;

    let index: RepoIndex = serde_json::from_str(&json)
        .with_context(|| format!("failed to deserialize index from: {}", path.display()))?;

    tracing::info!(
        "Loaded index with {} files from {}",
        index.files.len(),
        path.display()
    );
    Ok(index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::indexer::IndexConfig;
    use crate::parser::Language;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_save_and_load_roundtrip() {
        let dir = TempDir::new().unwrap();
        let repo_dir = dir.path().join("repo");
        fs::create_dir_all(&repo_dir).unwrap();
        fs::write(
            repo_dir.join("main.rs"),
            "pub fn hello() { println!(\"hi\"); }\n",
        )
        .unwrap();

        let config = IndexConfig {
            languages: vec![Language::Rust],
            exclude_patterns: vec![],
            root: repo_dir,
        };

        let index = crate::indexer::index_repo(&config).unwrap();
        let index_path = dir.path().join(".coral/index/symbols.json");

        // Save
        save_index(&index, &index_path).unwrap();
        assert!(index_path.exists());

        // Load
        let loaded = load_index(&index_path).unwrap();
        assert_eq!(loaded.files.len(), index.files.len());
        assert_eq!(loaded.file_hashes.len(), index.file_hashes.len());
    }

    #[test]
    fn test_save_creates_directories() {
        let dir = TempDir::new().unwrap();
        let deep_path = dir.path().join("a/b/c/index.json");

        let index = RepoIndex::new();
        save_index(&index, &deep_path).unwrap();
        assert!(deep_path.exists());
    }

    #[test]
    fn test_load_nonexistent_returns_error() {
        let result = load_index(Path::new("/nonexistent/path/index.json"));
        assert!(result.is_err());
    }
}
