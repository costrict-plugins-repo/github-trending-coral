//! LLM execution engine for implementation plans and sketches.
//!
//! Bridges `coral-implement`'s prompt generators (`plan`, `sketch`) with
//! `coral-runner`'s [`Runner`] trait so that `coral implement plan --execute`
//! can call an LLM and parse the structured result in one step.

use anyhow::{Context, Result};
use coral_runner::{Prompt, RunOutput, Runner, TokenUsage};

use crate::plan::{parse_plan, plan_system_prompt, plan_user_prompt, ImplementationPlan, PlanContext};
use crate::sketch::{parse_sketch, sketch_system_prompt, sketch_user_prompt, Sketch, SketchContext};

/// Result of executing a plan generation via LLM.
#[derive(Debug)]
pub struct PlanResult {
    /// The parsed implementation plan.
    pub plan: ImplementationPlan,
    /// Raw LLM output before parsing.
    pub raw_output: String,
    /// Token usage reported by the runner, if available.
    pub usage: Option<TokenUsage>,
}

/// Result of executing a sketch generation via LLM.
#[derive(Debug, Clone)]
pub struct SketchResult {
    /// The parsed sketch (set of file diffs).
    pub sketch: Sketch,
    /// Raw LLM output before parsing.
    pub raw_output: String,
    /// Token usage reported by the runner, if available.
    pub usage: Option<TokenUsage>,
}

/// Execute plan generation via LLM.
///
/// Builds a [`Prompt`] from the plan system/user prompts, sends it through
/// the provided [`Runner`], and parses the YAML response into an
/// [`ImplementationPlan`].
pub fn execute_plan(ctx: &PlanContext, runner: &dyn Runner) -> Result<PlanResult> {
    let prompt = Prompt {
        system: Some(plan_system_prompt()),
        user: plan_user_prompt(ctx),
        ..Default::default()
    };
    let output: RunOutput = runner.run(&prompt).context("LLM plan generation failed")?;
    let plan = parse_plan(&output.stdout).context("failed to parse plan from LLM output")?;
    Ok(PlanResult {
        plan,
        raw_output: output.stdout,
        usage: output.usage,
    })
}

/// Execute sketch generation via LLM.
///
/// Builds a [`Prompt`] from the sketch system/user prompts, sends it through
/// the provided [`Runner`], and parses the unified-diff response into a
/// [`Sketch`].
pub fn execute_sketch(
    description: &str,
    context: &[SketchContext],
    runner: &dyn Runner,
) -> Result<SketchResult> {
    let prompt = Prompt {
        system: Some(sketch_system_prompt()),
        user: sketch_user_prompt(description, context),
        ..Default::default()
    };
    let output: RunOutput = runner.run(&prompt).context("LLM sketch generation failed")?;
    let sketch = parse_sketch(description, &output.stdout);
    Ok(SketchResult {
        sketch,
        raw_output: output.stdout,
        usage: output.usage,
    })
}

/// Execute plan generation with a retry loop (up to `max_retries` attempts).
///
/// If the LLM output fails to parse as valid YAML, the prompt is re-sent
/// with an appended hint describing the parse error. This handles the common
/// case where the model outputs a slightly malformed YAML block on the first
/// attempt but self-corrects when shown the error.
pub fn execute_plan_with_retry(
    ctx: &PlanContext,
    runner: &dyn Runner,
    max_retries: usize,
) -> Result<PlanResult> {
    let mut last_error: Option<String> = None;

    for attempt in 0..=max_retries {
        let mut user = plan_user_prompt(ctx);

        if let Some(ref err) = last_error {
            user.push_str(&format!(
                "\n\n**Previous attempt failed to parse**: {err}\n\
                 Please output valid YAML matching the schema exactly."
            ));
        }

        let prompt = Prompt {
            system: Some(plan_system_prompt()),
            user,
            ..Default::default()
        };

        match runner.run(&prompt) {
            Ok(output) => match parse_plan(&output.stdout) {
                Ok(plan) => {
                    return Ok(PlanResult {
                        plan,
                        raw_output: output.stdout,
                        usage: output.usage,
                    });
                }
                Err(e) if attempt < max_retries => {
                    last_error = Some(e.to_string());
                    tracing::warn!(attempt, "plan parse failed, retrying: {e}");
                    continue;
                }
                Err(e) => {
                    return Err(e.context("plan parse failed after retries"));
                }
            },
            Err(e) => {
                return Err(e.into());
            }
        }
    }

    unreachable!("loop with 0..=max_retries always returns")
}

#[cfg(test)]
mod tests {
    use super::*;
    use coral_runner::MockRunner;
    use std::path::PathBuf;

    /// Sample valid YAML that `parse_plan` can handle.
    const VALID_PLAN_YAML: &str = r#"title: "Add caching"
confidence: 0.9
grounded_in:
  wiki: [cache-layer]
  code: [src/db.ts]
  patterns: []
steps:
  - file: src/cache.ts
    action: create
    description: "New cache module"
    symbols_affected: []
    depends_on: [redis]
    assertions: []
impact:
  files_modified: 0
  files_created: 1
  new_dependencies: [redis]
  symbols_added: [cache]
  symbols_modified: []
  potentially_affected: []
risks:
  - "Redis dependency"
"#;

    fn sample_ctx() -> PlanContext {
        PlanContext {
            request: "add caching".to_string(),
            relevant_code: vec![],
            relevant_wiki: vec![],
            applicable_patterns: vec![],
            affected_files: vec![],
        }
    }

    #[test]
    fn execute_plan_parses_valid_yaml_from_mock() {
        let mock = MockRunner::new();
        mock.push_ok(VALID_PLAN_YAML);

        let result = execute_plan(&sample_ctx(), &mock).expect("should succeed");
        assert_eq!(result.plan.title, "Add caching");
        assert_eq!(result.plan.steps.len(), 1);
        assert!(!result.raw_output.is_empty());
    }

    #[test]
    fn execute_plan_propagates_runner_error() {
        let mock = MockRunner::new();
        mock.push_err(coral_runner::RunnerError::NotFound);

        let err = execute_plan(&sample_ctx(), &mock).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("plan generation failed") || msg.contains("not found"),
            "error should mention failure: {msg}"
        );
    }

    #[test]
    fn execute_plan_fails_on_invalid_yaml() {
        let mock = MockRunner::new();
        mock.push_ok("this is not yaml at all { broken }}}");

        let err = execute_plan(&sample_ctx(), &mock).unwrap_err();
        assert!(
            err.to_string().contains("failed to parse plan"),
            "should report parse failure: {}",
            err
        );
    }

    #[test]
    fn execute_plan_with_retry_succeeds_on_second_attempt() {
        let mock = MockRunner::new();
        // First attempt: invalid YAML
        mock.push_ok("not valid yaml");
        // Second attempt: valid YAML
        mock.push_ok(VALID_PLAN_YAML);

        let result =
            execute_plan_with_retry(&sample_ctx(), &mock, 1).expect("should succeed on retry");
        assert_eq!(result.plan.title, "Add caching");

        // Verify two calls were made
        let calls = mock.calls();
        assert_eq!(calls.len(), 2);
        // Second call should contain the error hint
        assert!(
            calls[1].user.contains("Previous attempt failed to parse"),
            "retry prompt should include parse error hint"
        );
    }

    #[test]
    fn execute_plan_with_retry_fails_after_exhausting_retries() {
        let mock = MockRunner::new();
        mock.push_ok("bad yaml 1");
        mock.push_ok("bad yaml 2");
        mock.push_ok("bad yaml 3");

        let err = execute_plan_with_retry(&sample_ctx(), &mock, 2).unwrap_err();
        assert!(
            err.to_string().contains("parse failed after retries"),
            "should report exhausted retries: {}",
            err
        );
        // Should have made 3 calls (attempt 0, 1, 2)
        assert_eq!(mock.calls().len(), 3);
    }

    #[test]
    fn execute_plan_with_retry_zero_retries_behaves_like_single_attempt() {
        let mock = MockRunner::new();
        mock.push_ok(VALID_PLAN_YAML);

        let result =
            execute_plan_with_retry(&sample_ctx(), &mock, 0).expect("should succeed");
        assert_eq!(result.plan.title, "Add caching");
        assert_eq!(mock.calls().len(), 1);
    }

    #[test]
    fn execute_sketch_parses_diff_output() {
        let diff_output = r#"--- a/src/auth.ts
+++ b/src/auth.ts
@@ -10,6 +10,8 @@ function validate() {
   const token = req.headers.authorization;
+  logger.info("Validating token");
   if (!token) {
+    logger.warn("No token provided");
     return res.status(401).json({ error: "unauthorized" });
   }
"#;
        let mock = MockRunner::new();
        mock.push_ok(diff_output);

        let context: Vec<SketchContext> = vec![];
        let result =
            execute_sketch("add logging", &context, &mock).expect("should succeed");
        assert_eq!(result.sketch.description, "add logging");
        assert_eq!(result.sketch.diffs.len(), 1);
        assert_eq!(
            result.sketch.diffs[0].path,
            PathBuf::from("src/auth.ts")
        );
    }

    #[test]
    fn execute_sketch_propagates_runner_error() {
        let mock = MockRunner::new();
        mock.push_err(coral_runner::RunnerError::NotFound);

        let context: Vec<SketchContext> = vec![];
        let err = execute_sketch("add logging", &context, &mock).unwrap_err();
        assert!(
            err.to_string().contains("sketch generation failed") || err.to_string().contains("not found"),
            "error should mention failure: {}",
            err
        );
    }

    #[test]
    fn execute_plan_propagates_usage_when_present() {
        let mock = MockRunner::new();
        let usage = TokenUsage {
            input_tokens: 100,
            output_tokens: 50,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
        };
        mock.push_ok_with_usage(VALID_PLAN_YAML, usage);

        let result = execute_plan(&sample_ctx(), &mock).expect("should succeed");
        let got_usage = result.usage.expect("usage should be propagated");
        assert_eq!(got_usage.input_tokens, 100);
        assert_eq!(got_usage.output_tokens, 50);
    }
}
