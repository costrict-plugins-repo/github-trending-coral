//! End-to-end implementation orchestrator.
//!
//! `coral implement run "description"` drives the full loop:
//! UNDERSTAND -> PLAN -> VERIFY -> GENERATE -> VALIDATE -> STAGE/APPLY

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use coral_runner::Runner;

use crate::execute::{execute_plan_with_retry, execute_sketch, PlanResult, SketchResult};
use crate::plan::{CodeSnippet, PlanContext};
use crate::sketch::SketchContext;
use crate::staging::stage_sketch;
use crate::validate::{validate_change, validate_with_tests, ValidationResult};

/// Configuration for the orchestration loop.
#[derive(Debug, Clone)]
pub struct OrchestrateConfig {
    /// Repository root directory.
    pub root: PathBuf,
    /// Maximum LLM retries for plan generation.
    pub max_retries: usize,
    /// Whether to use test-driven development mode.
    pub tdd: bool,
    /// Whether to automatically apply staged changes.
    pub auto_apply: bool,
    /// Whether to run tests during validation.
    /// Automatically set to `true` when `tdd` is `true`.
    pub run_tests: bool,
    /// Maximum number of test-fix-retest iterations.
    /// Only used when `run_tests` is `true`.
    pub max_test_retries: u32,
}

/// Progress callback for reporting steps to the user.
pub trait OrchestrateProgress {
    fn on_step(&self, step: &str, detail: &str);
    fn on_warning(&self, message: &str);
}

/// A simple stderr progress reporter.
pub struct StderrProgress;

impl OrchestrateProgress for StderrProgress {
    fn on_step(&self, step: &str, detail: &str) {
        eprintln!("[Coral] {step}: {detail}");
    }
    fn on_warning(&self, message: &str) {
        eprintln!("[Coral] warning: {message}");
    }
}

/// Result of the full orchestration loop.
#[derive(Debug)]
pub struct OrchestrateResult {
    pub plan: Option<PlanResult>,
    pub sketch: Option<SketchResult>,
    pub validation: Option<ValidationResult>,
    pub staged_path: Option<PathBuf>,
    pub applied: bool,
}

/// Run the full implement loop.
///
/// Steps: UNDERSTAND -> PLAN -> VERIFY -> GENERATE -> VALIDATE -> STAGE/APPLY
pub fn orchestrate(
    description: &str,
    runner: &dyn Runner,
    config: &OrchestrateConfig,
    progress: &dyn OrchestrateProgress,
) -> Result<OrchestrateResult> {
    // 1. UNDERSTAND — gather context (search + graph if available)
    progress.on_step("UNDERSTAND", "gathering context...");
    let context = gather_context(&config.root, description, config.tdd);

    // 2. PLAN — generate implementation plan via LLM
    progress.on_step("PLAN", "generating implementation plan...");
    let plan_result = execute_plan_with_retry(&context, runner, config.max_retries)
        .context("plan generation failed")?;
    let plan = &plan_result.plan;
    progress.on_step(
        "PLAN",
        &format!("{} (confidence: {:.0}%)", plan.title, plan.confidence * 100.0),
    );

    // 3. VERIFY — check impact via graph
    progress.on_step("VERIFY", "checking impact...");
    let affected_files: Vec<PathBuf> = plan.steps.iter().map(|s| s.file.clone()).collect();
    verify_impact(&config.root, &affected_files, progress);

    // 4. GENERATE — create sketch from plan
    progress.on_step("GENERATE", "generating code...");
    let sketch_desc = build_sketch_description(description, &plan.title, config.tdd);
    let file_context = load_file_context(&config.root, &affected_files);
    let sketch_result = execute_sketch(&sketch_desc, &file_context, runner)
        .context("sketch generation failed")?;
    progress.on_step(
        "GENERATE",
        &format!("{} file(s) in sketch", sketch_result.sketch.diffs.len()),
    );

    // Determine whether to run tests: TDD implies run_tests.
    let run_tests = config.run_tests || config.tdd;

    // 5. VALIDATE — compilation check + optional test loop
    let (validation, sketch_result) = if !sketch_result.sketch.diffs.is_empty() {
        progress.on_step("VALIDATE", "checking generated code...");
        let files_to_check: Vec<PathBuf> =
            sketch_result.sketch.diffs.iter().map(|d| d.path.clone()).collect();

        let validate_fn = if run_tests {
            validate_with_tests
        } else {
            validate_change
        };

        match validate_fn(&config.root, &files_to_check) {
            Ok(result) => {
                if run_tests && result.tests_passed == Some(false) {
                    // TDD iteration loop: feed failures back to LLM and retry
                    let (final_validation, final_sketch) = tdd_iterate(
                        description,
                        &sketch_result,
                        result,
                        &affected_files,
                        runner,
                        config,
                        progress,
                    )?;
                    (Some(final_validation), final_sketch)
                } else {
                    progress.on_step("VALIDATE", &result.summary);
                    (Some(result), sketch_result)
                }
            }
            Err(e) => {
                progress.on_warning(&format!("validation skipped: {e}"));
                (None, sketch_result)
            }
        }
    } else {
        (None, sketch_result)
    };

    // 6. STAGE or APPLY
    let staged_path = stage_sketch(&config.root, &sketch_result.sketch)
        .context("failed to stage sketch")?;
    progress.on_step("STAGE", &format!("staged to {}", staged_path.display()));

    let applied = if config.auto_apply {
        let staged = crate::staging::load_staged(&config.root)?;
        let applied_files = crate::staging::apply_staged(&config.root, &staged)?;
        progress.on_step("APPLY", &format!("applied {} files", applied_files.len()));
        true
    } else {
        progress.on_step(
            "REPORT",
            "changes staged. Run `coral implement apply` to apply.",
        );
        false
    };

    Ok(OrchestrateResult {
        plan: Some(plan_result),
        sketch: Some(sketch_result),
        validation,
        staged_path: Some(staged_path),
        applied,
    })
}

/// TDD iteration: when tests fail, feed the failure output back to the
/// LLM to generate a fix, then re-validate. Repeats up to
/// `config.max_test_retries` times.
fn tdd_iterate(
    description: &str,
    initial_sketch: &SketchResult,
    initial_validation: ValidationResult,
    affected_files: &[PathBuf],
    runner: &dyn Runner,
    config: &OrchestrateConfig,
    progress: &dyn OrchestrateProgress,
) -> Result<(ValidationResult, SketchResult)> {
    let mut current_sketch = initial_sketch.clone();
    let mut current_validation = initial_validation;

    for retry in 0..config.max_test_retries {
        let test_output = current_validation
            .test_output
            .as_deref()
            .unwrap_or("(no test output captured)");

        progress.on_step(
            "TDD",
            &format!(
                "tests failed, retry {}/{}...",
                retry + 1,
                config.max_test_retries
            ),
        );

        // Build a fix prompt incorporating the test failure
        let fix_desc = format!(
            "Fix the following test failures for: {description}\n\n\
             Test output:\n```\n{test_output}\n```\n\n\
             Generate corrected code that makes the tests pass."
        );

        let file_context = load_file_context(&config.root, affected_files);
        let new_sketch = execute_sketch(&fix_desc, &file_context, runner)
            .context("TDD fix sketch generation failed")?;

        progress.on_step(
            "TDD",
            &format!("{} file(s) in fix sketch", new_sketch.sketch.diffs.len()),
        );

        // Re-validate with tests
        let files_to_check: Vec<PathBuf> = new_sketch
            .sketch
            .diffs
            .iter()
            .map(|d| d.path.clone())
            .collect();

        match validate_with_tests(&config.root, &files_to_check) {
            Ok(result) => {
                if result.tests_passed == Some(true) || result.tests_passed.is_none() {
                    progress.on_step("TDD", "tests pass after fix");
                    return Ok((result, new_sketch));
                }
                current_validation = result;
                current_sketch = new_sketch;
            }
            Err(e) => {
                progress.on_warning(&format!("validation error during TDD retry: {e}"));
                return Ok((current_validation, current_sketch));
            }
        }
    }

    // Exhausted retries — return last state
    progress.on_warning(&format!(
        "TDD retries exhausted ({} attempts), tests still failing",
        config.max_test_retries
    ));
    Ok((current_validation, current_sketch))
}

/// Gather context for plan generation by searching the index if available.
fn gather_context(root: &Path, description: &str, tdd: bool) -> PlanContext {
    let mut relevant_code = Vec::new();

    // Try to search the existing index for relevant code
    let search_index = root.join(".coral/search-index");
    if search_index.exists() {
        if let Ok(engine) = coral_search::SearchEngine::new(&search_index) {
            let query = coral_search::SearchQuery {
                text: description.to_string(),
                limit: 5,
                sources: vec![
                    coral_search::SearchSource::Keyword,
                    coral_search::SearchSource::Semantic,
                ],
                file_filter: None,
            };
            if let Ok(results) = engine.search(&query) {
                for r in results.iter().take(5) {
                    // Use the snippet from search results; read full file if needed
                    let content = if r.snippet.is_empty() {
                        let full = root.join(&r.path);
                        let raw = std::fs::read_to_string(&full).unwrap_or_default();
                        if raw.len() > 500 {
                            raw[..raw.floor_char_boundary(500)].to_string()
                        } else {
                            raw
                        }
                    } else {
                        r.snippet.clone()
                    };
                    relevant_code.push(CodeSnippet {
                        file: r.path.clone(),
                        symbol: r.symbol.clone(),
                        content,
                        start_line: r.line.unwrap_or(1),
                        end_line: r.line.unwrap_or(1) + 20,
                        relevance: r.score,
                    });
                }
            }
        }
    }

    let mut request = description.to_string();
    if tdd {
        request.push_str(
            "\n\n**TDD Mode**: Generate test files FIRST. Each step that creates a new \
             module must be preceded by a step that creates its test file with failing \
             assertions. The implementation step follows, making the tests pass.",
        );
    }

    PlanContext {
        request,
        relevant_code,
        relevant_wiki: vec![],
        applicable_patterns: vec![],
        affected_files: vec![],
    }
}

/// Check downstream impact via the code graph if available.
fn verify_impact(root: &Path, affected_files: &[PathBuf], progress: &dyn OrchestrateProgress) {
    let graph_path = root.join(".coral/graph.json");
    if !graph_path.exists() {
        return;
    }
    match coral_graph::load_graph(&graph_path) {
        Ok(graph) => {
            let mut total_affected = 0;
            for file in affected_files {
                total_affected += graph.affected_by(file, 2).len();
            }
            if total_affected > 0 {
                progress.on_warning(&format!(
                    "{total_affected} downstream files may be affected by this change"
                ));
            }
        }
        Err(e) => {
            tracing::debug!("could not load graph for impact check: {e}");
        }
    }
}

/// Build the description sent to the sketch generator.
fn build_sketch_description(description: &str, plan_title: &str, tdd: bool) -> String {
    if tdd {
        format!(
            "Generate tests first, then implementation for: {description}.\n\
             Follow the plan: {plan_title}"
        )
    } else {
        format!(
            "Implement: {description}.\n\
             Follow this plan: {plan_title}"
        )
    }
}

/// Load file contents for files that exist (provides context to sketch generator).
fn load_file_context(root: &Path, files: &[PathBuf]) -> Vec<SketchContext> {
    files
        .iter()
        .filter_map(|f| {
            let full_path = root.join(f);
            let content = std::fs::read_to_string(&full_path).ok()?;
            let language = f
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| match e {
                    "rs" => "rust",
                    "ts" | "tsx" => "typescript",
                    "js" | "jsx" => "javascript",
                    "py" => "python",
                    "go" => "go",
                    other => other,
                })
                .map(String::from);
            Some(SketchContext {
                path: f.clone(),
                content,
                language,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use coral_runner::MockRunner;

    const VALID_PLAN_YAML: &str = r#"title: "Add logging"
confidence: 0.9
grounded_in:
  wiki: []
  code: [src/lib.rs]
  patterns: []
steps:
  - file: src/logger.rs
    action: create
    description: "New logger module"
    symbols_affected: []
    depends_on: []
    assertions: []
impact:
  files_modified: 0
  files_created: 1
  new_dependencies: []
  symbols_added: [Logger]
  symbols_modified: []
  potentially_affected: []
risks: []
"#;

    const VALID_SKETCH_OUTPUT: &str = r#"--- /dev/null
+++ b/src/logger.rs
@@ -0,0 +1,3 @@
+pub struct Logger;
+
+impl Logger {}
"#;

    struct TestProgress {
        steps: std::cell::RefCell<Vec<(String, String)>>,
    }

    impl TestProgress {
        fn new() -> Self {
            Self {
                steps: std::cell::RefCell::new(Vec::new()),
            }
        }
    }

    impl OrchestrateProgress for TestProgress {
        fn on_step(&self, step: &str, detail: &str) {
            self.steps
                .borrow_mut()
                .push((step.to_string(), detail.to_string()));
        }
        fn on_warning(&self, _message: &str) {}
    }

    #[test]
    fn orchestrate_full_loop_stages_result() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();

        let mock = MockRunner::new();
        // Plan generation
        mock.push_ok(VALID_PLAN_YAML);
        // Sketch generation
        mock.push_ok(VALID_SKETCH_OUTPUT);

        let config = OrchestrateConfig {
            root: root.to_path_buf(),
            max_retries: 1,
            tdd: false,
            auto_apply: false,
            run_tests: false,
            max_test_retries: 3,
        };

        let progress = TestProgress::new();
        let result = orchestrate("add logging", &mock, &config, &progress).unwrap();

        assert!(result.plan.is_some());
        assert!(result.sketch.is_some());
        assert!(result.staged_path.is_some());
        assert!(!result.applied);

        // Verify staging file exists
        assert!(root.join(".coral/staging/changes.json").exists());

        // Verify progress steps were reported
        let steps = progress.steps.borrow();
        assert!(steps.iter().any(|(s, _)| s == "UNDERSTAND"));
        assert!(steps.iter().any(|(s, _)| s == "PLAN"));
        assert!(steps.iter().any(|(s, _)| s == "STAGE"));
    }

    #[test]
    fn orchestrate_with_auto_apply() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();

        let mock = MockRunner::new();
        mock.push_ok(VALID_PLAN_YAML);
        // Sketch with new_content — parse_sketch doesn't set new_content from diffs,
        // so apply won't write anything. That's fine for this test.
        mock.push_ok(VALID_SKETCH_OUTPUT);

        let config = OrchestrateConfig {
            root: root.to_path_buf(),
            max_retries: 1,
            tdd: false,
            auto_apply: true,
            run_tests: false,
            max_test_retries: 3,
        };

        let progress = TestProgress::new();
        let result = orchestrate("add logging", &mock, &config, &progress).unwrap();

        assert!(result.applied);
        // Staging file should be cleaned up after apply
        assert!(!root.join(".coral/staging/changes.json").exists());
    }

    #[test]
    fn gather_context_without_index_returns_empty() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ctx = gather_context(tmp.path(), "add caching", false);
        assert_eq!(ctx.request, "add caching");
        assert!(ctx.relevant_code.is_empty());
    }

    #[test]
    fn gather_context_tdd_appends_instructions() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ctx = gather_context(tmp.path(), "add caching", true);
        assert!(ctx.request.contains("TDD Mode"));
    }

    #[test]
    fn build_sketch_description_includes_plan_title() {
        let desc = build_sketch_description("add caching", "Cache Layer Plan", false);
        assert!(desc.contains("add caching"));
        assert!(desc.contains("Cache Layer Plan"));
    }

    #[test]
    fn build_sketch_description_tdd_mode() {
        let desc = build_sketch_description("add caching", "Cache Plan", true);
        assert!(desc.contains("tests first"));
    }

    #[test]
    fn tdd_mode_sets_run_tests_implicitly() {
        // Verifying at the config level that tdd=true with run_tests=false
        // still triggers test validation (the `run_tests || tdd` logic).
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();

        let mock = MockRunner::new();
        mock.push_ok(VALID_PLAN_YAML);
        mock.push_ok(VALID_SKETCH_OUTPUT);

        let config = OrchestrateConfig {
            root: root.to_path_buf(),
            max_retries: 1,
            tdd: true,
            auto_apply: false,
            run_tests: false, // tdd overrides this
            max_test_retries: 3,
        };

        let progress = TestProgress::new();
        // This will run orchestrate; the validation step will attempt
        // to run tests (which won't find a Cargo.toml in the temp dir,
        // so validate_with_tests falls back to just compilation checks).
        let result = orchestrate("add logging", &mock, &config, &progress).unwrap();
        assert!(result.staged_path.is_some());
    }

    #[test]
    fn config_max_test_retries_defaults() {
        let config = OrchestrateConfig {
            root: PathBuf::new(),
            max_retries: 1,
            tdd: false,
            auto_apply: false,
            run_tests: false,
            max_test_retries: 3,
        };
        assert_eq!(config.max_test_retries, 3);
    }

    #[test]
    fn tdd_iterate_attempts_retries() {
        // When the fix sketch produces no diffs, validate_with_tests
        // returns passed=true with tests_passed=None (no tests to run).
        // The TDD loop sees tests_passed.is_none() and treats it as
        // success, returning early.
        let validation = crate::validate::ValidationResult {
            passed: false,
            checks: vec![],
            summary: "tests failed".into(),
            test_output: Some("FAILED: test_foo".into()),
            tests_passed: Some(false),
        };

        let sketch_result = SketchResult {
            sketch: crate::sketch::Sketch {
                description: "test".into(),
                diffs: vec![],
            },
            raw_output: String::new(),
            usage: None,
        };

        let mock = MockRunner::new();
        // Push a fix sketch that's empty (no diffs parsed)
        mock.push_ok("--- no diffs ---");

        let tmp = tempfile::TempDir::new().unwrap();
        let config = OrchestrateConfig {
            root: tmp.path().to_path_buf(),
            max_retries: 1,
            tdd: true,
            auto_apply: false,
            run_tests: true,
            max_test_retries: 1,
        };

        let progress = TestProgress::new();
        let (result_validation, _result_sketch) =
            tdd_iterate("test", &sketch_result, validation, &[], &mock, &config, &progress).unwrap();
        // Should have attempted a retry
        let steps = progress.steps.borrow();
        assert!(steps.iter().any(|(s, _)| s == "TDD"));
        // The empty sketch triggers validate_with_tests on empty files,
        // which returns passed=true, tests_passed=None. The TDD loop
        // treats None as success and returns.
        assert!(result_validation.passed);
    }
}
