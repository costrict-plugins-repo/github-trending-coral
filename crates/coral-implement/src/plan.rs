//! Implementation plan generation and execution.
//!
//! A plan is a structured representation of a code change: which files to
//! create/modify, what symbols to add, what tests to write. Plans are
//! generated from natural language descriptions grounded in existing
//! codebase patterns, and can be reviewed before execution.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A complete implementation plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImplementationPlan {
    /// Human-readable title
    pub title: String,
    /// Confidence that this plan is correct (0.0-1.0)
    pub confidence: f64,
    /// Wiki pages / code files that informed this plan
    pub grounded_in: PlanSources,
    /// Ordered steps to execute
    pub steps: Vec<PlanStep>,
    /// Impact analysis: what might break
    pub impact: PlanImpact,
    /// Known risks
    pub risks: Vec<String>,
}

/// Sources the plan was derived from.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanSources {
    /// Wiki page slugs consulted
    pub wiki: Vec<String>,
    /// Source files examined
    pub code: Vec<PathBuf>,
    /// Pattern instances used as templates
    pub patterns: Vec<PathBuf>,
}

/// A single step in the implementation plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanStep {
    /// File to create or modify
    pub file: PathBuf,
    /// Action to take
    pub action: StepAction,
    /// Human-readable description of what this step does
    pub description: String,
    /// Symbols affected (for modify actions)
    pub symbols_affected: Vec<String>,
    /// New dependencies required
    pub depends_on: Vec<String>,
    /// Test assertions (for test file creation)
    pub assertions: Vec<String>,
}

/// Action type for a plan step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StepAction {
    /// Create a new file
    Create,
    /// Modify an existing file
    Modify,
    /// Delete a file (rare, always flagged for review)
    Delete,
}

/// Predicted impact of executing the plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanImpact {
    /// Files that will be modified
    pub files_modified: usize,
    /// Files that will be created
    pub files_created: usize,
    /// New package dependencies
    pub new_dependencies: Vec<String>,
    /// New symbols being added
    pub symbols_added: Vec<String>,
    /// Existing symbols being modified
    pub symbols_modified: Vec<String>,
    /// Files that might be affected (from graph analysis)
    pub potentially_affected: Vec<PathBuf>,
}

/// Context provided to the plan generator (LLM).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanContext {
    /// The user's request in natural language
    pub request: String,
    /// Relevant code snippets found via search
    pub relevant_code: Vec<CodeSnippet>,
    /// Relevant wiki pages
    pub relevant_wiki: Vec<WikiSnippet>,
    /// Detected patterns that might apply
    pub applicable_patterns: Vec<PatternSummary>,
    /// Files that would be affected by changes
    pub affected_files: Vec<PathBuf>,
}

/// A code snippet with location metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeSnippet {
    pub file: PathBuf,
    pub symbol: Option<String>,
    pub content: String,
    pub start_line: u32,
    pub end_line: u32,
    pub relevance: f64,
}

/// A wiki page summary for context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WikiSnippet {
    pub slug: String,
    pub title: String,
    pub summary: String,
    pub confidence: f64,
}

/// Abbreviated pattern info for context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternSummary {
    pub name: String,
    pub location: String,
    pub shape: String,
    pub example_file: PathBuf,
}

/// Generate the system prompt for plan generation.
pub fn plan_system_prompt() -> String {
    r#"You are a code implementation planner. Given a user's request and relevant context from their codebase (code snippets, wiki pages, detected patterns), generate a structured implementation plan.

Your plan must:
1. Follow existing patterns in the codebase (if a pattern exists for the type of change, follow it exactly)
2. Include test files that verify the new behavior
3. Minimize the number of files modified
4. List all new dependencies explicitly
5. Identify symbols that will be added or modified

Output the plan as YAML matching this schema:

```yaml
title: "Short description of the change"
confidence: 0.85  # how confident you are this plan is correct
grounded_in:
  wiki: [slug1, slug2]
  code: [path/to/file.ts]
  patterns: [path/to/pattern-example.ts]
steps:
  - file: path/to/new-file.ts
    action: create
    description: "What this file does"
    symbols_affected: []
    depends_on: [package-name]
    assertions: []
  - file: path/to/existing.ts
    action: modify
    description: "Add import and register new module"
    symbols_affected: [app, router]
    depends_on: []
    assertions: []
  - file: tests/new-file.test.ts
    action: create
    description: "Tests for the new functionality"
    symbols_affected: []
    depends_on: []
    assertions:
      - "should handle the happy path"
      - "should return 400 on invalid input"
      - "should be idempotent"
impact:
  files_modified: 1
  files_created: 2
  new_dependencies: [package-name]
  symbols_added: [newFunction, NewType]
  symbols_modified: [app]
  potentially_affected: [path/to/consumer.ts]
risks:
  - "Potential risk or consideration"
```"#.to_string()
}

/// Generate the user prompt for plan generation from context.
pub fn plan_user_prompt(ctx: &PlanContext) -> String {
    let mut prompt = format!("## Request\n\n{}\n\n", ctx.request);

    if !ctx.relevant_code.is_empty() {
        prompt.push_str("## Relevant Code\n\n");
        for snippet in &ctx.relevant_code {
            prompt.push_str(&format!(
                "### {} (lines {}-{})\n```\n{}\n```\n\n",
                snippet.file.display(),
                snippet.start_line,
                snippet.end_line,
                snippet.content
            ));
        }
    }

    if !ctx.relevant_wiki.is_empty() {
        prompt.push_str("## Relevant Wiki Pages\n\n");
        for wiki in &ctx.relevant_wiki {
            prompt.push_str(&format!(
                "- **{}** (confidence: {:.0}%): {}\n",
                wiki.slug,
                wiki.confidence * 100.0,
                wiki.summary
            ));
        }
        prompt.push('\n');
    }

    if !ctx.applicable_patterns.is_empty() {
        prompt.push_str("## Applicable Patterns\n\n");
        prompt.push_str("Follow these patterns for consistency:\n\n");
        for pattern in &ctx.applicable_patterns {
            prompt.push_str(&format!(
                "- **{}** at `{}`: {}\n  Example: `{}`\n",
                pattern.name,
                pattern.location,
                pattern.shape,
                pattern.example_file.display()
            ));
        }
        prompt.push('\n');
    }

    if !ctx.affected_files.is_empty() {
        prompt.push_str("## Files That Might Be Affected\n\n");
        for f in &ctx.affected_files {
            prompt.push_str(&format!("- {}\n", f.display()));
        }
        prompt.push('\n');
    }

    prompt.push_str("## Output\n\nGenerate the implementation plan as YAML.\n");
    prompt
}

/// Parse a plan from YAML (LLM output).
pub fn parse_plan(yaml_str: &str) -> anyhow::Result<ImplementationPlan> {
    // Strip markdown code fences if present
    let clean = yaml_str
        .trim()
        .strip_prefix("```yaml")
        .or_else(|| yaml_str.trim().strip_prefix("```"))
        .unwrap_or(yaml_str.trim());
    let clean = clean.strip_suffix("```").unwrap_or(clean).trim();

    let plan: ImplementationPlan = serde_yaml::from_str(clean)?;
    Ok(plan)
}

/// Format a plan for human display.
pub fn format_plan(plan: &ImplementationPlan) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "━━ Implementation Plan: {} ━━\n",
        plan.title
    ));
    out.push_str(&format!(
        "Confidence: {:.0}%\n\n",
        plan.confidence * 100.0
    ));

    out.push_str("Steps:\n");
    for (i, step) in plan.steps.iter().enumerate() {
        let action_icon = match step.action {
            StepAction::Create => "+",
            StepAction::Modify => "~",
            StepAction::Delete => "-",
        };
        out.push_str(&format!(
            "  {}. [{}] {} — {}\n",
            i + 1,
            action_icon,
            step.file.display(),
            step.description
        ));
        if !step.symbols_affected.is_empty() {
            out.push_str(&format!(
                "     Symbols: {}\n",
                step.symbols_affected.join(", ")
            ));
        }
        if !step.assertions.is_empty() {
            out.push_str("     Tests:\n");
            for a in &step.assertions {
                out.push_str(&format!("       • {a}\n"));
            }
        }
    }

    out.push_str(&format!(
        "\nImpact: {} files modified, {} created",
        plan.impact.files_modified, plan.impact.files_created
    ));
    if !plan.impact.new_dependencies.is_empty() {
        out.push_str(&format!(
            ", new deps: [{}]",
            plan.impact.new_dependencies.join(", ")
        ));
    }
    out.push('\n');

    if !plan.risks.is_empty() {
        out.push_str("\nRisks:\n");
        for r in &plan.risks {
            out.push_str(&format!("  ⚠ {r}\n"));
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_PLAN_YAML: &str = r#"
title: "Add rate limiting to API routes"
confidence: 0.85
grounded_in:
  wiki: [api-routes, auth-middleware]
  code: [src/middleware/auth.ts]
  patterns: [src/middleware/cors.ts]
steps:
  - file: src/middleware/rate-limit.ts
    action: create
    description: "New middleware using sliding window"
    symbols_affected: []
    depends_on: [redis]
    assertions: []
  - file: src/routes/tasks.ts
    action: modify
    description: "Apply rate limiter"
    symbols_affected: [router]
    depends_on: []
    assertions: []
  - file: tests/rate-limit.test.ts
    action: create
    description: "Integration tests"
    symbols_affected: []
    depends_on: []
    assertions:
      - "returns 429 after 100 requests"
      - "resets after window expires"
impact:
  files_modified: 1
  files_created: 2
  new_dependencies: [redis]
  symbols_added: [rateLimiter, RateLimitConfig]
  symbols_modified: [router]
  potentially_affected: [src/app.ts]
risks:
  - "Redis adds operational complexity"
"#;

    #[test]
    fn parse_plan_from_yaml() {
        let plan = parse_plan(SAMPLE_PLAN_YAML).unwrap();
        assert_eq!(plan.title, "Add rate limiting to API routes");
        assert_eq!(plan.confidence, 0.85);
        assert_eq!(plan.steps.len(), 3);
        assert_eq!(plan.steps[0].action, StepAction::Create);
        assert_eq!(plan.steps[1].action, StepAction::Modify);
        assert_eq!(plan.impact.new_dependencies, vec!["redis"]);
    }

    #[test]
    fn parse_plan_with_code_fences() {
        let fenced = format!("```yaml\n{}\n```", SAMPLE_PLAN_YAML.trim());
        let plan = parse_plan(&fenced).unwrap();
        assert_eq!(plan.title, "Add rate limiting to API routes");
    }

    #[test]
    fn format_plan_readable() {
        let plan = parse_plan(SAMPLE_PLAN_YAML).unwrap();
        let formatted = format_plan(&plan);
        assert!(formatted.contains("Add rate limiting"));
        assert!(formatted.contains("[+] src/middleware/rate-limit.ts"));
        assert!(formatted.contains("[~] src/routes/tasks.ts"));
        assert!(formatted.contains("redis"));
    }

    #[test]
    fn plan_prompts_include_context() {
        let ctx = PlanContext {
            request: "add caching".to_string(),
            relevant_code: vec![CodeSnippet {
                file: PathBuf::from("src/db.ts"),
                symbol: Some("query".into()),
                content: "function query() {}".into(),
                start_line: 1,
                end_line: 1,
                relevance: 0.9,
            }],
            relevant_wiki: vec![],
            applicable_patterns: vec![],
            affected_files: vec![],
        };
        let prompt = plan_user_prompt(&ctx);
        assert!(prompt.contains("add caching"));
        assert!(prompt.contains("src/db.ts"));
        assert!(prompt.contains("function query()"));
    }
}
