//! Pattern detection — analyze a codebase to find recurring structural patterns.
//!
//! A "pattern" is a repeating file/symbol structure that appears 3+ times in
//! the codebase. Examples:
//! - Middleware pattern: `src/middleware/*.ts` all export a function with (req, res, next)
//! - Route pattern: `src/routes/*.ts` all create a Router and export it
//! - Test pattern: `tests/*.test.ts` mirror source file structure
//!
//! Patterns are detected heuristically by grouping files by directory and
//! analyzing their symbol structure for commonalities.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Minimum instances needed to declare something a "pattern".
pub const MIN_PATTERN_INSTANCES: usize = 3;

/// A detected code pattern in the repository.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pattern {
    /// Human-readable name (e.g., "middleware", "route", "model")
    pub name: String,
    /// Directory where instances live (e.g., "src/middleware/")
    pub location: PathBuf,
    /// Glob pattern matching instances (e.g., "src/middleware/*.ts")
    pub glob: String,
    /// Common structural elements across all instances
    pub shape: PatternShape,
    /// Files that match this pattern
    pub instances: Vec<PathBuf>,
    /// Confidence that this is a real pattern (0.0-1.0)
    pub confidence: f64,
}

/// The structural shape common to all pattern instances.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternShape {
    /// Common exported symbol kinds (e.g., ["function"], ["class"])
    pub exports: Vec<String>,
    /// Common import patterns (e.g., "express.Router")
    pub common_imports: Vec<String>,
    /// Naming convention (e.g., "kebab-case", "PascalCase")
    pub naming: Option<String>,
    /// Associated test location pattern (e.g., "tests/{name}.test.ts")
    pub test_pattern: Option<String>,
    /// Registration point (e.g., "app.ts imports and uses this")
    pub registration: Option<String>,
}

/// A simplified symbol for pattern analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileShape {
    pub path: PathBuf,
    pub exports: Vec<ExportedSymbol>,
    pub imports: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportedSymbol {
    pub name: String,
    pub kind: String, // "function", "class", "const", etc.
    pub signature: Option<String>,
}

/// Detect patterns in a set of file shapes.
///
/// Groups files by parent directory, then checks if files in the same
/// directory share structural similarities (same export kinds, same imports).
pub fn detect_patterns(files: &[FileShape]) -> Vec<Pattern> {
    // Group files by parent directory
    let mut by_dir: HashMap<PathBuf, Vec<&FileShape>> = HashMap::new();
    for f in files {
        if let Some(parent) = f.path.parent() {
            by_dir.entry(parent.to_path_buf()).or_default().push(f);
        }
    }

    let mut patterns = Vec::new();

    for (dir, dir_files) in &by_dir {
        // Need at least MIN_PATTERN_INSTANCES to declare a pattern
        if dir_files.len() < MIN_PATTERN_INSTANCES {
            continue;
        }

        // Find common export kinds across all files in this directory
        let common_exports = find_common_exports(dir_files);
        if common_exports.is_empty() {
            continue;
        }

        // Find common imports
        let common_imports = find_common_imports(dir_files);

        // Detect naming convention
        let naming = detect_naming_convention(dir_files);

        // Detect associated test pattern
        let test_pattern = detect_test_pattern(dir_files, files);

        // Derive pattern name from directory name
        let name = dir
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "unnamed".to_string());

        // Derive glob
        let ext = dir_files
            .first()
            .and_then(|f| f.path.extension())
            .map(|e| format!("*.{}", e.to_string_lossy()))
            .unwrap_or_else(|| "*".to_string());
        let glob = format!("{}/{}", dir.display(), ext);

        let instances: Vec<PathBuf> = dir_files.iter().map(|f| f.path.clone()).collect();
        let instance_count = instances.len();

        // Confidence based on how many common elements we found
        let confidence = calculate_pattern_confidence(
            instance_count,
            &common_exports,
            &common_imports,
        );

        patterns.push(Pattern {
            name,
            location: dir.clone(),
            glob,
            shape: PatternShape {
                exports: common_exports,
                common_imports,
                naming,
                test_pattern,
                registration: None,
            },
            instances,
            confidence,
        });
    }

    // Sort by confidence descending
    patterns.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap_or(std::cmp::Ordering::Equal));
    patterns
}

/// Find export kinds that appear in >70% of files in the group.
fn find_common_exports(files: &[&FileShape]) -> Vec<String> {
    let total = files.len();
    let mut kind_counts: HashMap<String, usize> = HashMap::new();

    for f in files {
        // Deduplicate kinds per file
        let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for exp in &f.exports {
            if seen.insert(&exp.kind) {
                *kind_counts.entry(exp.kind.clone()).or_default() += 1;
            }
        }
    }

    kind_counts
        .into_iter()
        .filter(|(_, count)| *count as f64 / total as f64 > 0.7)
        .map(|(kind, _)| kind)
        .collect()
}

/// Find imports that appear in >50% of files in the group.
fn find_common_imports(files: &[&FileShape]) -> Vec<String> {
    let total = files.len();
    let mut import_counts: HashMap<String, usize> = HashMap::new();

    for f in files {
        let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for imp in &f.imports {
            if seen.insert(imp) {
                *import_counts.entry(imp.clone()).or_default() += 1;
            }
        }
    }

    import_counts
        .into_iter()
        .filter(|(_, count)| *count as f64 / total as f64 > 0.5)
        .map(|(imp, _)| imp)
        .collect()
}

/// Detect naming convention from file names.
fn detect_naming_convention(files: &[&FileShape]) -> Option<String> {
    let names: Vec<String> = files
        .iter()
        .filter_map(|f| f.path.file_stem().map(|s| s.to_string_lossy().to_string()))
        .collect();

    if names.is_empty() {
        return None;
    }

    let kebab_count = names.iter().filter(|n| n.contains('-')).count();
    let snake_count = names.iter().filter(|n| n.contains('_')).count();
    let pascal_count = names
        .iter()
        .filter(|n| n.chars().next().map(|c| c.is_uppercase()).unwrap_or(false))
        .count();

    let total = names.len();
    if kebab_count as f64 / total as f64 > 0.7 {
        Some("kebab-case".to_string())
    } else if snake_count as f64 / total as f64 > 0.7 {
        Some("snake_case".to_string())
    } else if pascal_count as f64 / total as f64 > 0.7 {
        Some("PascalCase".to_string())
    } else {
        None
    }
}

/// Try to detect if there's a parallel test directory structure.
fn detect_test_pattern(dir_files: &[&FileShape], all_files: &[FileShape]) -> Option<String> {
    // Check common test locations
    let test_dirs = ["tests/", "test/", "__tests__/", "spec/"];
    let sample_name = dir_files
        .first()
        .and_then(|f| f.path.file_stem())
        .map(|s| s.to_string_lossy().to_string())?;

    for test_dir in &test_dirs {
        let test_suffixes = [".test", ".spec", "_test", "_spec"];
        for suffix in &test_suffixes {
            let test_pattern_candidate = format!("{}{}{}", test_dir, sample_name, suffix);
            let matches = all_files
                .iter()
                .filter(|f| {
                    f.path
                        .to_string_lossy()
                        .contains(&test_pattern_candidate.replace(&sample_name, ""))
                })
                .count();
            if matches >= 2 {
                let ext = dir_files
                    .first()
                    .and_then(|f| f.path.extension())
                    .map(|e| format!(".{}", e.to_string_lossy()))
                    .unwrap_or_default();
                return Some(format!("{}{{name}}{}{}", test_dir, suffix, ext));
            }
        }
    }
    None
}

/// Calculate confidence based on structural consistency.
fn calculate_pattern_confidence(
    instance_count: usize,
    common_exports: &[String],
    common_imports: &[String],
) -> f64 {
    let mut score = 0.0;

    // More instances = higher confidence
    score += (instance_count as f64 / 10.0).min(0.4);

    // Having common exports is a strong signal
    if !common_exports.is_empty() {
        score += 0.3;
    }

    // Common imports add confidence
    if !common_imports.is_empty() {
        score += 0.2;
    }

    // Base confidence for meeting the minimum threshold
    score += 0.1;

    score.min(1.0)
}

/// Format patterns for display.
pub fn format_patterns(patterns: &[Pattern]) -> String {
    let mut out = String::new();
    out.push_str("Detected patterns:\n\n");

    for p in patterns {
        out.push_str(&format!(
            "  {} ({} instances, confidence: {:.0}%):\n",
            p.name,
            p.instances.len(),
            p.confidence * 100.0
        ));
        out.push_str(&format!("    Location: {}\n", p.glob));
        if !p.shape.exports.is_empty() {
            out.push_str(&format!(
                "    Shape: exports [{}]\n",
                p.shape.exports.join(", ")
            ));
        }
        if !p.shape.common_imports.is_empty() {
            out.push_str(&format!(
                "    Common imports: {}\n",
                p.shape.common_imports.join(", ")
            ));
        }
        if let Some(naming) = &p.shape.naming {
            out.push_str(&format!("    Naming: {naming}\n"));
        }
        if let Some(test) = &p.shape.test_pattern {
            out.push_str(&format!("    Tests: {test}\n"));
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_middleware_files() -> Vec<FileShape> {
        let names = ["auth", "cors", "rate-limit", "logging"];
        names
            .iter()
            .map(|name| FileShape {
                path: PathBuf::from(format!("src/middleware/{name}.ts")),
                exports: vec![ExportedSymbol {
                    name: format!("{name}Middleware"),
                    kind: "function".to_string(),
                    signature: Some("(req, res, next) => void".to_string()),
                }],
                imports: vec!["express".to_string()],
            })
            .collect()
    }

    #[test]
    fn detects_middleware_pattern() {
        let files = make_middleware_files();
        let patterns = detect_patterns(&files);
        assert!(!patterns.is_empty());
        assert_eq!(patterns[0].name, "middleware");
        assert_eq!(patterns[0].instances.len(), 4);
        assert!(patterns[0].shape.exports.contains(&"function".to_string()));
    }

    #[test]
    fn requires_minimum_instances() {
        let files = vec![
            FileShape {
                path: PathBuf::from("src/utils/a.ts"),
                exports: vec![ExportedSymbol {
                    name: "a".into(),
                    kind: "function".into(),
                    signature: None,
                }],
                imports: vec![],
            },
            FileShape {
                path: PathBuf::from("src/utils/b.ts"),
                exports: vec![ExportedSymbol {
                    name: "b".into(),
                    kind: "function".into(),
                    signature: None,
                }],
                imports: vec![],
            },
        ];
        let patterns = detect_patterns(&files);
        assert!(patterns.is_empty(), "2 files should not trigger a pattern");
    }

    #[test]
    fn detects_naming_convention() {
        // Only 1/4 files has a hyphen, so below 70% threshold — returns None
        let binding = make_middleware_files();
        let files_ref: Vec<&FileShape> = binding.iter().collect();
        let naming = detect_naming_convention(&files_ref);
        assert_eq!(naming, None);

        // Test with all kebab-case files
        let kebab_files: Vec<FileShape> = ["rate-limit", "cors-handler", "error-handler", "request-logger"]
            .iter()
            .map(|name| FileShape {
                path: PathBuf::from(format!("src/middleware/{name}.ts")),
                exports: vec![],
                imports: vec![],
            })
            .collect();
        let kebab_ref: Vec<&FileShape> = kebab_files.iter().collect();
        let naming = detect_naming_convention(&kebab_ref);
        assert_eq!(naming, Some("kebab-case".to_string()));
    }

    #[test]
    fn confidence_increases_with_instances() {
        let c3 = calculate_pattern_confidence(3, &["function".to_string()], &[]);
        let c10 = calculate_pattern_confidence(10, &["function".to_string()], &[]);
        assert!(c10 > c3);
    }

    #[test]
    fn format_patterns_produces_readable_output() {
        let files = make_middleware_files();
        let patterns = detect_patterns(&files);
        let output = format_patterns(&patterns);
        assert!(output.contains("middleware"));
        assert!(output.contains("4 instances"));
        assert!(output.contains("function"));
    }
}
