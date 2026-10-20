//! Impact analysis — predict what breaks when files change.
//!
//! Uses the code graph to find all downstream dependents of a set of
//! files or symbols. This helps users understand the blast radius of
//! a proposed change before executing it.

use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};

/// Result of an impact analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImpactReport {
    /// Files directly changed (the input)
    pub changed_files: Vec<PathBuf>,
    /// Files that directly depend on changed files (depth 1)
    pub direct_dependents: Vec<PathBuf>,
    /// Files transitively affected (depth 2+)
    pub transitive_dependents: Vec<PathBuf>,
    /// Symbols that might need updating
    pub affected_symbols: Vec<AffectedSymbol>,
    /// Test files that should be re-run
    pub test_files: Vec<PathBuf>,
    /// Total blast radius (unique affected files)
    pub total_affected: usize,
}

/// A symbol affected by the change.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AffectedSymbol {
    pub name: String,
    pub file: PathBuf,
    pub reason: AffectReason,
}

/// Why a symbol is affected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AffectReason {
    /// Directly calls a changed symbol
    CallsChanged,
    /// Imports from a changed file
    ImportsChanged,
    /// Implements a changed interface/trait
    ImplementsChanged,
    /// Type-references a changed type
    ReferencesChanged,
}

/// Simplified dependency edge for impact analysis.
/// The graph crate provides the full graph; this module works with
/// a simpler adjacency list representation for BFS traversal.
#[derive(Debug, Clone)]
pub struct DependencyInfo {
    /// file → files that depend on it (reverse dependency)
    pub reverse_deps: std::collections::HashMap<PathBuf, Vec<PathBuf>>,
    /// file → symbols defined in it
    pub file_symbols: std::collections::HashMap<PathBuf, Vec<String>>,
    /// symbol → files that reference it
    pub symbol_refs: std::collections::HashMap<String, Vec<PathBuf>>,
    /// Known test file paths
    pub test_files: HashSet<PathBuf>,
}

/// Compute impact analysis for a set of changed files.
pub fn analyze_impact(
    changed_files: &[PathBuf],
    deps: &DependencyInfo,
    max_depth: usize,
) -> ImpactReport {
    let changed_set: HashSet<&PathBuf> = changed_files.iter().collect();
    let mut visited: HashSet<PathBuf> = HashSet::new();
    let mut direct_dependents: Vec<PathBuf> = Vec::new();
    let mut transitive_dependents: Vec<PathBuf> = Vec::new();
    let mut affected_symbols: Vec<AffectedSymbol> = Vec::new();
    let mut test_files: Vec<PathBuf> = Vec::new();

    // BFS from changed files through reverse dependencies
    let mut queue: VecDeque<(PathBuf, usize)> = VecDeque::new();

    for file in changed_files {
        visited.insert(file.clone());
        if let Some(dependents) = deps.reverse_deps.get(file) {
            for dep in dependents {
                if !visited.contains(dep) {
                    queue.push_back((dep.clone(), 1));
                    visited.insert(dep.clone());
                }
            }
        }
    }

    while let Some((file, depth)) = queue.pop_front() {
        if depth == 1 {
            direct_dependents.push(file.clone());
        } else {
            transitive_dependents.push(file.clone());
        }

        // Check if it's a test file
        if deps.test_files.contains(&file) || is_test_file(&file) {
            test_files.push(file.clone());
        }

        // Continue BFS if within max depth
        if depth < max_depth {
            if let Some(dependents) = deps.reverse_deps.get(&file) {
                for dep in dependents {
                    if !visited.contains(dep) {
                        queue.push_back((dep.clone(), depth + 1));
                        visited.insert(dep.clone());
                    }
                }
            }
        }
    }

    // Find affected symbols: symbols in changed files that are referenced elsewhere
    for file in changed_files {
        if let Some(symbols) = deps.file_symbols.get(file) {
            for sym in symbols {
                if let Some(referencing_files) = deps.symbol_refs.get(sym) {
                    for ref_file in referencing_files {
                        if !changed_set.contains(ref_file) {
                            affected_symbols.push(AffectedSymbol {
                                name: sym.clone(),
                                file: ref_file.clone(),
                                reason: AffectReason::ReferencesChanged,
                            });
                        }
                    }
                }
            }
        }
    }

    // Also check test files related to changed files
    for file in changed_files {
        let test_candidates = find_test_file_candidates(file);
        for candidate in test_candidates {
            if deps.test_files.contains(&candidate) || candidate.exists() {
                if !test_files.contains(&candidate) {
                    test_files.push(candidate);
                }
            }
        }
    }

    let total_affected = direct_dependents.len() + transitive_dependents.len();

    ImpactReport {
        changed_files: changed_files.to_vec(),
        direct_dependents,
        transitive_dependents,
        affected_symbols,
        test_files,
        total_affected,
    }
}

/// Heuristic: is this file likely a test file?
fn is_test_file(path: &Path) -> bool {
    let s = path.to_string_lossy();
    s.contains("test") || s.contains("spec") || s.contains("_test.") || s.contains(".test.")
}

/// Generate candidate test file paths for a source file.
fn find_test_file_candidates(source: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    let stem = source.file_stem().unwrap_or_default().to_string_lossy();
    let ext = source
        .extension()
        .map(|e| e.to_string_lossy().to_string())
        .unwrap_or_default();

    // Common test file naming patterns
    let patterns = [
        format!("tests/{stem}.test.{ext}"),
        format!("tests/{stem}_test.{ext}"),
        format!("test/{stem}.test.{ext}"),
        format!("__tests__/{stem}.test.{ext}"),
        format!("{stem}_test.{ext}"),
    ];

    for p in &patterns {
        candidates.push(PathBuf::from(p));
    }

    // Also check sibling test file
    if let Some(parent) = source.parent() {
        candidates.push(parent.join(format!("{stem}.test.{ext}")));
        candidates.push(parent.join(format!("{stem}_test.{ext}")));
    }

    candidates
}

/// Format an impact report for display.
pub fn format_impact(report: &ImpactReport) -> String {
    let mut out = String::new();
    out.push_str("━━ Impact Analysis ━━\n\n");

    out.push_str("Changed files:\n");
    for f in &report.changed_files {
        out.push_str(&format!("  • {}\n", f.display()));
    }

    if !report.direct_dependents.is_empty() {
        out.push_str(&format!(
            "\nDirect dependents ({}):\n",
            report.direct_dependents.len()
        ));
        for f in &report.direct_dependents {
            out.push_str(&format!("  → {}\n", f.display()));
        }
    }

    if !report.transitive_dependents.is_empty() {
        out.push_str(&format!(
            "\nTransitive dependents ({}):\n",
            report.transitive_dependents.len()
        ));
        for f in report.transitive_dependents.iter().take(10) {
            out.push_str(&format!("  ⇢ {}\n", f.display()));
        }
        if report.transitive_dependents.len() > 10 {
            out.push_str(&format!(
                "  ... and {} more\n",
                report.transitive_dependents.len() - 10
            ));
        }
    }

    if !report.affected_symbols.is_empty() {
        out.push_str(&format!(
            "\nAffected symbols ({}):\n",
            report.affected_symbols.len()
        ));
        for sym in report.affected_symbols.iter().take(10) {
            out.push_str(&format!("  ⚠ {} in {}\n", sym.name, sym.file.display()));
        }
    }

    if !report.test_files.is_empty() {
        out.push_str(&format!("\nTests to re-run ({}):\n", report.test_files.len()));
        for f in &report.test_files {
            out.push_str(&format!("  🧪 {}\n", f.display()));
        }
    }

    out.push_str(&format!(
        "\nTotal blast radius: {} files\n",
        report.total_affected
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn sample_deps() -> DependencyInfo {
        let mut reverse_deps = HashMap::new();
        reverse_deps.insert(
            PathBuf::from("src/auth.ts"),
            vec![
                PathBuf::from("src/routes/tasks.ts"),
                PathBuf::from("src/routes/users.ts"),
            ],
        );
        reverse_deps.insert(
            PathBuf::from("src/routes/tasks.ts"),
            vec![PathBuf::from("src/app.ts")],
        );

        let mut file_symbols = HashMap::new();
        file_symbols.insert(
            PathBuf::from("src/auth.ts"),
            vec!["authenticate".to_string(), "generateToken".to_string()],
        );

        let mut symbol_refs = HashMap::new();
        symbol_refs.insert(
            "authenticate".to_string(),
            vec![
                PathBuf::from("src/routes/tasks.ts"),
                PathBuf::from("src/routes/users.ts"),
            ],
        );

        let mut test_files = HashSet::new();
        test_files.insert(PathBuf::from("tests/auth.test.ts"));

        DependencyInfo {
            reverse_deps,
            file_symbols,
            symbol_refs,
            test_files,
        }
    }

    #[test]
    fn finds_direct_dependents() {
        let deps = sample_deps();
        let report = analyze_impact(&[PathBuf::from("src/auth.ts")], &deps, 2);
        assert_eq!(report.direct_dependents.len(), 2);
        assert!(report
            .direct_dependents
            .contains(&PathBuf::from("src/routes/tasks.ts")));
    }

    #[test]
    fn finds_transitive_dependents() {
        let deps = sample_deps();
        let report = analyze_impact(&[PathBuf::from("src/auth.ts")], &deps, 2);
        assert!(report
            .transitive_dependents
            .contains(&PathBuf::from("src/app.ts")));
    }

    #[test]
    fn finds_affected_symbols() {
        let deps = sample_deps();
        let report = analyze_impact(&[PathBuf::from("src/auth.ts")], &deps, 2);
        assert!(!report.affected_symbols.is_empty());
        assert!(report.affected_symbols.iter().any(|s| s.name == "authenticate"));
    }

    #[test]
    fn respects_max_depth() {
        let deps = sample_deps();
        let report = analyze_impact(&[PathBuf::from("src/auth.ts")], &deps, 1);
        // At depth 1, only direct dependents are found
        assert!(!report
            .transitive_dependents
            .contains(&PathBuf::from("src/app.ts")));
    }

    #[test]
    fn format_report_readable() {
        let deps = sample_deps();
        let report = analyze_impact(&[PathBuf::from("src/auth.ts")], &deps, 2);
        let formatted = format_impact(&report);
        assert!(formatted.contains("Impact Analysis"));
        assert!(formatted.contains("src/auth.ts"));
        assert!(formatted.contains("Direct dependents"));
    }
}
