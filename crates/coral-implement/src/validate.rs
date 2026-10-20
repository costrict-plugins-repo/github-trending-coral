//! Change validation — lint, compile check, test.
//!
//! After an implementation plan or sketch is generated, `validate_change`
//! runs a configurable set of checks against the modified files:
//! file existence, compilation, and test compilation. Results are
//! returned as structured data for programmatic consumption.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Aggregate result of all validation checks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationResult {
    /// Whether all checks passed.
    pub passed: bool,
    /// Individual check results.
    pub checks: Vec<CheckResult>,
    /// Human-readable summary.
    pub summary: String,
    /// Raw test output (only populated by `validate_with_tests`).
    pub test_output: Option<String>,
    /// Whether all tests passed (only populated by `validate_with_tests`).
    pub tests_passed: Option<bool>,
}

/// Result of a single validation check.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckResult {
    /// Check identifier (e.g. "files_exist", "cargo_check").
    pub name: String,
    /// Whether this check passed.
    pub passed: bool,
    /// Check output or error message.
    pub output: String,
}

/// Maximum characters of command output to retain per check.
/// Prevents huge payloads from compilation errors flooding the result.
const MAX_OUTPUT_LEN: usize = 2000;

/// Validate changes in a project directory.
///
/// Runs a configurable set of checks based on the file extensions present:
/// 1. **File existence**: all referenced files must exist on disk.
/// 2. **Compilation** (Rust): `cargo check --message-format=short`
/// 3. **Compilation** (TypeScript): `npx tsc --noEmit`
/// 4. **Test compilation** (Rust): `cargo test --no-run --message-format=short`
pub fn validate_change(root: &Path, files: &[PathBuf]) -> Result<ValidationResult> {
    let mut checks = Vec::new();

    // Check 1: All referenced files exist
    let mut missing = Vec::new();
    for f in files {
        let full = root.join(f);
        if !full.exists() {
            missing.push(f.display().to_string());
        }
    }
    checks.push(CheckResult {
        name: "files_exist".into(),
        passed: missing.is_empty(),
        output: if missing.is_empty() {
            "all files exist".into()
        } else {
            format!("missing: {}", missing.join(", "))
        },
    });

    // Detect language from file extensions
    let has_rust = files
        .iter()
        .any(|f| f.extension().map(|e| e == "rs").unwrap_or(false));
    let has_ts = files
        .iter()
        .any(|f| f.extension().map(|e| e == "ts" || e == "tsx").unwrap_or(false));

    // Check 2: Compilation
    if has_rust {
        let compile_check = run_command(root, "cargo", &["check", "--message-format=short"]);
        checks.push(compile_check);
    }
    if has_ts {
        let tsc_check = run_command(root, "npx", &["tsc", "--noEmit"]);
        checks.push(tsc_check);
    }

    // Check 3: Test compilation (Rust only — verifies tests compile without running them)
    if has_rust {
        let test_check = run_command(root, "cargo", &["test", "--no-run", "--message-format=short"]);
        checks.push(test_check);
    }

    let all_passed = checks.iter().all(|c| c.passed);
    let summary = if all_passed {
        "all checks passed".into()
    } else {
        let failed: Vec<&str> = checks
            .iter()
            .filter(|c| !c.passed)
            .map(|c| c.name.as_str())
            .collect();
        format!("failed checks: {}", failed.join(", "))
    };

    Ok(ValidationResult {
        passed: all_passed,
        checks,
        summary,
        test_output: None,
        tests_passed: None,
    })
}

/// Validate changes AND run tests.
///
/// Runs `validate_change` first; if all compilation checks pass,
/// additionally runs the project's test suite:
/// - **Rust**: `cargo test --no-fail-fast`
/// - **TypeScript/Node**: `npm test`
///
/// Test output and pass/fail status are captured in the returned
/// [`ValidationResult`]'s `test_output` and `tests_passed` fields.
pub fn validate_with_tests(root: &Path, files: &[PathBuf]) -> Result<ValidationResult> {
    let mut result = validate_change(root, files)?;

    // Only run tests if compilation checks passed
    if !result.passed {
        return Ok(result);
    }

    let has_rust = files
        .iter()
        .any(|f| f.extension().map(|e| e == "rs").unwrap_or(false));
    let has_ts = files
        .iter()
        .any(|f| f.extension().map(|e| e == "ts" || e == "tsx").unwrap_or(false));
    let has_js = files
        .iter()
        .any(|f| f.extension().map(|e| e == "js" || e == "jsx").unwrap_or(false));

    // Detect project type from project root if file extensions don't help
    let has_cargo_toml = root.join("Cargo.toml").exists();
    let has_package_json = root.join("package.json").exists();

    let test_check = if has_rust || has_cargo_toml {
        Some(run_command(root, "cargo", &["test", "--no-fail-fast"]))
    } else if has_ts || has_js || has_package_json {
        Some(run_command(root, "npm", &["test"]))
    } else {
        None
    };

    if let Some(check) = test_check {
        let tests_passed = check.passed;
        let test_output = check.output.clone();
        result.checks.push(check);

        result.test_output = Some(test_output);
        result.tests_passed = Some(tests_passed);

        if !tests_passed {
            result.passed = false;
            let failed: Vec<&str> = result
                .checks
                .iter()
                .filter(|c| !c.passed)
                .map(|c| c.name.as_str())
                .collect();
            result.summary = format!("failed checks: {}", failed.join(", "));
        }
    }

    Ok(result)
}

/// Run a command and capture its result as a [`CheckResult`].
fn run_command(root: &Path, cmd: &str, args: &[&str]) -> CheckResult {
    let name = format!("{}_{}", cmd, args.first().unwrap_or(&""));
    match Command::new(cmd).args(args).current_dir(root).output() {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let combined = format!("{stdout}{stderr}");
            // Truncate output to avoid huge payloads
            let truncated = if combined.len() > MAX_OUTPUT_LEN {
                format!("{}...(truncated)", &combined[..combined.floor_char_boundary(MAX_OUTPUT_LEN)])
            } else {
                combined
            };
            CheckResult {
                name,
                passed: output.status.success(),
                output: truncated,
            }
        }
        Err(e) => CheckResult {
            name,
            passed: false,
            output: format!("command not found or failed to execute: {e}"),
        },
    }
}

/// Format a [`ValidationResult`] for human display.
pub fn format_validation(result: &ValidationResult) -> String {
    let mut out = String::new();
    let icon = if result.passed { "PASS" } else { "FAIL" };
    out.push_str(&format!("━━ Validation: {icon} ━━\n\n"));

    for check in &result.checks {
        let status = if check.passed { "ok" } else { "FAIL" };
        out.push_str(&format!("  [{status}] {}\n", check.name));
        if !check.passed && !check.output.is_empty() {
            // Show first few lines of failure output
            for line in check.output.lines().take(10) {
                out.push_str(&format!("       {line}\n"));
            }
        }
    }

    out.push_str(&format!("\n{}\n", result.summary));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn validate_all_files_exist() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path();

        // Create test files
        fs::write(dir.join("a.txt"), "hello").unwrap();
        fs::write(dir.join("b.txt"), "world").unwrap();

        let files = vec![PathBuf::from("a.txt"), PathBuf::from("b.txt")];
        let result = validate_change(dir, &files).unwrap();

        assert!(result.checks[0].passed, "files_exist should pass");
        assert_eq!(result.checks[0].name, "files_exist");
        assert_eq!(result.checks[0].output, "all files exist");
    }

    #[test]
    fn validate_missing_files_detected() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path();

        // Create only one of two expected files
        fs::write(dir.join("exists.txt"), "hello").unwrap();

        let files = vec![
            PathBuf::from("exists.txt"),
            PathBuf::from("missing.txt"),
        ];
        let result = validate_change(dir, &files).unwrap();

        assert!(!result.checks[0].passed, "files_exist should fail");
        assert!(
            result.checks[0].output.contains("missing.txt"),
            "output should name the missing file"
        );
    }

    #[test]
    fn validate_empty_file_list() {
        let tmp = tempfile::TempDir::new().unwrap();
        let files: Vec<PathBuf> = vec![];
        let result = validate_change(tmp.path(), &files).unwrap();

        // Only the files_exist check, which passes vacuously
        assert_eq!(result.checks.len(), 1);
        assert!(result.passed);
        assert_eq!(result.summary, "all checks passed");
    }

    #[test]
    fn validate_non_code_files_skip_compilation() {
        let tmp = tempfile::TempDir::new().unwrap();
        fs::write(tmp.path().join("readme.md"), "# Hello").unwrap();

        let files = vec![PathBuf::from("readme.md")];
        let result = validate_change(tmp.path(), &files).unwrap();

        // Only files_exist check — no .rs or .ts means no compilation checks
        assert_eq!(result.checks.len(), 1);
        assert_eq!(result.checks[0].name, "files_exist");
    }

    #[test]
    fn format_validation_shows_pass() {
        let result = ValidationResult {
            passed: true,
            checks: vec![CheckResult {
                name: "files_exist".into(),
                passed: true,
                output: "all files exist".into(),
            }],
            summary: "all checks passed".into(),
            test_output: None,
            tests_passed: None,
        };
        let formatted = format_validation(&result);
        assert!(formatted.contains("PASS"));
        assert!(formatted.contains("[ok] files_exist"));
    }

    #[test]
    fn format_validation_shows_failure_details() {
        let result = ValidationResult {
            passed: false,
            checks: vec![
                CheckResult {
                    name: "files_exist".into(),
                    passed: false,
                    output: "missing: foo.rs".into(),
                },
            ],
            summary: "failed checks: files_exist".into(),
            test_output: None,
            tests_passed: None,
        };
        let formatted = format_validation(&result);
        assert!(formatted.contains("FAIL"));
        assert!(formatted.contains("missing: foo.rs"));
    }

    #[test]
    fn validate_change_has_no_test_fields() {
        let tmp = tempfile::TempDir::new().unwrap();
        let files: Vec<PathBuf> = vec![];
        let result = validate_change(tmp.path(), &files).unwrap();
        assert!(result.test_output.is_none());
        assert!(result.tests_passed.is_none());
    }

    #[test]
    fn run_command_captures_nonexistent_binary() {
        let tmp = tempfile::TempDir::new().unwrap();
        let check = run_command(tmp.path(), "nonexistent-binary-xyz-12345", &["--version"]);
        assert!(!check.passed);
        assert!(
            check.output.contains("command not found") || check.output.contains("failed to execute"),
            "should report command failure: {}",
            check.output
        );
    }
}
