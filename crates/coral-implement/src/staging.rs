//! Staging area — write generated code to .coral/staging/ for review.
//!
//! Generated changes land here as JSON metadata (not actual files) so the user
//! can inspect diffs and apply them explicitly with `coral implement apply`.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// A staged change set ready for review/apply.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StagedChanges {
    pub description: String,
    pub timestamp: String,
    pub files: Vec<StagedFile>,
    pub plan_summary: Option<String>,
}

/// A single file within a staged change set.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StagedFile {
    pub path: PathBuf,
    pub is_new: bool,
    pub diff: String,
    pub new_content: Option<String>,
}

/// Write a Sketch to the staging area. Returns the path to the staged JSON file.
pub fn stage_sketch(root: &Path, sketch: &crate::sketch::Sketch) -> Result<PathBuf> {
    let staging_dir = root.join(".coral").join("staging");
    std::fs::create_dir_all(&staging_dir)
        .context("failed to create .coral/staging directory")?;

    let changes = StagedChanges {
        description: sketch.description.clone(),
        timestamp: now_rfc3339(),
        files: sketch
            .diffs
            .iter()
            .map(|d| StagedFile {
                path: d.path.clone(),
                is_new: d.is_new,
                diff: d.diff.clone(),
                new_content: d.new_content.clone(),
            })
            .collect(),
        plan_summary: None,
    };

    let path = staging_dir.join("changes.json");
    let json = serde_json::to_string_pretty(&changes)
        .context("failed to serialize staged changes")?;
    std::fs::write(&path, json).context("failed to write staging file")?;
    Ok(path)
}

/// Load staged changes from disk.
pub fn load_staged(root: &Path) -> Result<StagedChanges> {
    let path = root.join(".coral").join("staging").join("changes.json");
    let content = std::fs::read_to_string(&path).context("no staged changes found")?;
    Ok(serde_json::from_str(&content).context("failed to parse staged changes")?)
}

/// Apply staged changes (write new/modified files to disk).
/// Returns the list of file paths that were written.
pub fn apply_staged(root: &Path, staged: &StagedChanges) -> Result<Vec<PathBuf>> {
    let mut applied = Vec::new();
    for file in &staged.files {
        if let Some(ref content) = file.new_content {
            let full_path = root.join(&file.path);
            if let Some(parent) = full_path.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create directory for {}", file.path.display()))?;
            }
            std::fs::write(&full_path, content)
                .with_context(|| format!("failed to write {}", file.path.display()))?;
            applied.push(file.path.clone());
        }
    }
    // Clean up staging
    let staging_path = root.join(".coral").join("staging").join("changes.json");
    let _ = std::fs::remove_file(staging_path);
    Ok(applied)
}

/// Clear the staging area entirely.
pub fn clear_staged(root: &Path) -> Result<()> {
    let staging_dir = root.join(".coral").join("staging");
    if staging_dir.exists() {
        std::fs::remove_dir_all(&staging_dir)
            .context("failed to remove staging directory")?;
    }
    Ok(())
}

/// Format staged changes for human display.
pub fn format_staged(staged: &StagedChanges) -> String {
    let mut out = String::new();
    out.push_str(&format!("Staged: {}\n", staged.description));
    out.push_str(&format!("  Time: {}\n", staged.timestamp));
    if let Some(ref plan) = staged.plan_summary {
        out.push_str(&format!("  Plan: {plan}\n"));
    }
    out.push_str(&format!("  Files ({}):\n", staged.files.len()));
    for f in &staged.files {
        let label = if f.is_new { "(new)" } else { "(modify)" };
        let has_content = if f.new_content.is_some() { " [content ready]" } else { "" };
        out.push_str(&format!("    {} {}{}\n", f.path.display(), label, has_content));
    }
    out
}

/// Simple RFC-3339 timestamp without pulling in chrono.
fn now_rfc3339() -> String {
    // Use std::time for a basic timestamp. For precision we use seconds since UNIX epoch.
    let duration = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs();
    // Format as ISO-8601 basic (good enough for staging metadata).
    // We convert to a rough date-time without pulling in chrono.
    let days = secs / 86400;
    let remaining = secs % 86400;
    let hours = remaining / 3600;
    let minutes = (remaining % 3600) / 60;
    let seconds = remaining % 60;

    // Days since 1970-01-01, convert to Y-M-D (simplified, no leap-second).
    let (year, month, day) = days_to_ymd(days);
    format!("{year:04}-{month:02}-{day:02}T{hours:02}:{minutes:02}:{seconds:02}Z")
}

/// Convert days since epoch to (year, month, day). Simple civil calendar conversion.
fn days_to_ymd(days: u64) -> (u64, u64, u64) {
    // Algorithm from http://howardhinnant.github.io/date_algorithms.html
    let z = days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sketch::{FileDiff, Sketch};

    fn sample_sketch() -> Sketch {
        Sketch {
            description: "add logging".to_string(),
            diffs: vec![
                FileDiff {
                    path: PathBuf::from("src/auth.rs"),
                    is_new: false,
                    diff: "@@ -1,3 +1,4 @@\n+use log::info;\n".to_string(),
                    new_content: Some("use log::info;\nfn auth() {}\n".to_string()),
                },
                FileDiff {
                    path: PathBuf::from("src/new_module.rs"),
                    is_new: true,
                    diff: "@@ -0,0 +1,3 @@\n+pub fn hello() {}\n".to_string(),
                    new_content: Some("pub fn hello() {}\n".to_string()),
                },
            ],
        }
    }

    #[test]
    fn stage_and_load_roundtrip() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();

        let sketch = sample_sketch();
        let path = stage_sketch(root, &sketch).unwrap();
        assert!(path.exists());

        let loaded = load_staged(root).unwrap();
        assert_eq!(loaded.description, "add logging");
        assert_eq!(loaded.files.len(), 2);
        assert_eq!(loaded.files[0].path, PathBuf::from("src/auth.rs"));
        assert!(!loaded.files[0].is_new);
        assert!(loaded.files[1].is_new);
    }

    #[test]
    fn apply_staged_writes_files() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();

        let sketch = sample_sketch();
        stage_sketch(root, &sketch).unwrap();
        let loaded = load_staged(root).unwrap();
        let applied = apply_staged(root, &loaded).unwrap();

        assert_eq!(applied.len(), 2);
        let content = std::fs::read_to_string(root.join("src/auth.rs")).unwrap();
        assert!(content.contains("use log::info;"));
        let content2 = std::fs::read_to_string(root.join("src/new_module.rs")).unwrap();
        assert!(content2.contains("pub fn hello()"));

        // Staging file should be cleaned up
        assert!(!root.join(".coral/staging/changes.json").exists());
    }

    #[test]
    fn clear_staged_removes_directory() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();

        let sketch = sample_sketch();
        stage_sketch(root, &sketch).unwrap();
        assert!(root.join(".coral/staging").exists());

        clear_staged(root).unwrap();
        assert!(!root.join(".coral/staging").exists());
    }

    #[test]
    fn load_staged_fails_when_nothing_staged() {
        let tmp = tempfile::TempDir::new().unwrap();
        let err = load_staged(tmp.path()).unwrap_err();
        assert!(err.to_string().contains("no staged changes"));
    }

    #[test]
    fn format_staged_is_readable() {
        let staged = StagedChanges {
            description: "add caching".into(),
            timestamp: "2026-01-15T10:30:00Z".into(),
            files: vec![StagedFile {
                path: PathBuf::from("src/cache.rs"),
                is_new: true,
                diff: String::new(),
                new_content: Some("pub fn cache() {}".into()),
            }],
            plan_summary: Some("single-step plan".into()),
        };
        let out = format_staged(&staged);
        assert!(out.contains("add caching"));
        assert!(out.contains("src/cache.rs"));
        assert!(out.contains("(new)"));
        assert!(out.contains("[content ready]"));
    }

    #[test]
    fn now_rfc3339_is_valid_format() {
        let ts = now_rfc3339();
        // Should match YYYY-MM-DDTHH:MM:SSZ pattern
        assert_eq!(ts.len(), 20);
        assert!(ts.ends_with('Z'));
        assert_eq!(&ts[4..5], "-");
        assert_eq!(&ts[7..8], "-");
        assert_eq!(&ts[10..11], "T");
    }
}
