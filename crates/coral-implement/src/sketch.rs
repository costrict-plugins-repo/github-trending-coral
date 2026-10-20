//! Sketch — lightweight diff preview for small changes.
//!
//! `coral sketch "add logging to the auth middleware"` generates a
//! unified diff showing what would change, without writing to disk.
//! The user can review and apply with `--apply`.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A sketch is a set of file diffs that preview a change.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sketch {
    /// What the sketch implements
    pub description: String,
    /// Individual file changes
    pub diffs: Vec<FileDiff>,
}

/// A diff for a single file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileDiff {
    /// File path (relative to repo root)
    pub path: PathBuf,
    /// Whether this is a new file or modification
    pub is_new: bool,
    /// The unified diff content
    pub diff: String,
    /// Full new content (for new files) or None (for modifications)
    pub new_content: Option<String>,
}

/// Generate the system prompt for sketch generation.
pub fn sketch_system_prompt() -> String {
    r#"You are a code sketch generator. Given a user's request and relevant code context, generate a minimal unified diff showing the exact changes needed.

Rules:
1. Make the MINIMUM changes needed to fulfill the request
2. Follow the existing code style exactly
3. Do not add unnecessary imports or dependencies
4. Output one unified diff per file affected

Format your output as:

```diff
--- a/path/to/file.ts
+++ b/path/to/file.ts
@@ -10,6 +10,8 @@ function existing() {
   existing line
+  new line
+  another new line
   existing line
```

For new files, use /dev/null as the source:
```diff
--- /dev/null
+++ b/path/to/new-file.ts
@@ -0,0 +1,10 @@
+line 1
+line 2
```

Only output diffs. No explanation outside the diff blocks."#.to_string()
}

/// Generate the user prompt for sketch generation.
pub fn sketch_user_prompt(description: &str, context: &[SketchContext]) -> String {
    let mut prompt = format!("## Change requested\n\n{description}\n\n");

    if !context.is_empty() {
        prompt.push_str("## Current code\n\n");
        for ctx in context {
            prompt.push_str(&format!(
                "### {}\n```{}\n{}\n```\n\n",
                ctx.path.display(),
                ctx.language.as_deref().unwrap_or(""),
                ctx.content
            ));
        }
    }

    prompt.push_str("## Output\n\nGenerate the minimal unified diff.\n");
    prompt
}

/// Context for sketch generation — current file contents.
#[derive(Debug, Clone)]
pub struct SketchContext {
    pub path: PathBuf,
    pub content: String,
    pub language: Option<String>,
}

/// Parse unified diffs from LLM output into a Sketch.
pub fn parse_sketch(description: &str, output: &str) -> Sketch {
    let mut diffs = Vec::new();
    let mut current_path: Option<PathBuf> = None;
    let mut current_diff = String::new();
    let mut current_is_new = false;
    let mut next_is_new = false;

    for line in output.lines() {
        if line.starts_with("+++ b/") {
            // Start of a new file diff — save previous if exists
            if let Some(path) = current_path.take() {
                if !current_diff.is_empty() {
                    diffs.push(FileDiff {
                        path,
                        is_new: current_is_new,
                        diff: current_diff.clone(),
                        new_content: None,
                    });
                }
            }
            current_path = Some(PathBuf::from(&line[6..]));
            current_diff.clear();
            current_is_new = next_is_new;
            next_is_new = false;
        } else if line.starts_with("--- /dev/null") {
            next_is_new = true;
        } else if line.starts_with("--- a/") {
            next_is_new = false;
        } else if line.starts_with("```") {
            // Skip markdown code fences
            continue;
        } else if current_path.is_some() {
            if !current_diff.is_empty() || line.starts_with("@@") || line.starts_with('+') || line.starts_with('-') || line.starts_with(' ') {
                current_diff.push_str(line);
                current_diff.push('\n');
            }
        }
    }

    // Save last diff
    if let Some(path) = current_path {
        if !current_diff.is_empty() {
            diffs.push(FileDiff {
                path,
                is_new: current_is_new,
                diff: current_diff,
                new_content: None,
            });
        }
    }

    Sketch {
        description: description.to_string(),
        diffs,
    }
}

/// Format a sketch for display.
pub fn format_sketch(sketch: &Sketch) -> String {
    let mut out = String::new();
    out.push_str(&format!("━━ Sketch: {} ━━\n\n", sketch.description));

    for diff in &sketch.diffs {
        let label = if diff.is_new { "(new file)" } else { "(modify)" };
        out.push_str(&format!("  {} {}\n", diff.path.display(), label));
    }
    out.push('\n');

    for diff in &sketch.diffs {
        let source = if diff.is_new {
            "/dev/null".to_string()
        } else {
            diff.path.to_string_lossy().to_string()
        };
        out.push_str(&format!("--- {source}\n"));
        out.push_str(&format!("+++ {}\n", diff.path.display()));
        out.push_str(&diff.diff);
        out.push('\n');
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_diff() {
        let output = r#"```diff
--- a/src/auth.ts
+++ b/src/auth.ts
@@ -10,6 +10,8 @@ function validate() {
   const token = req.headers.authorization;
+  logger.info("Validating token");
   if (!token) {
+    logger.warn("No token provided");
     return res.status(401).json({ error: "unauthorized" });
   }
```"#;
        let sketch = parse_sketch("add logging", output);
        assert_eq!(sketch.diffs.len(), 1);
        assert_eq!(sketch.diffs[0].path, PathBuf::from("src/auth.ts"));
        assert!(!sketch.diffs[0].is_new);
        assert!(sketch.diffs[0].diff.contains("logger.info"));
    }

    #[test]
    fn parse_new_file_diff() {
        let output = r#"--- /dev/null
+++ b/src/middleware/cache.ts
@@ -0,0 +1,5 @@
+import { Redis } from 'redis';
+
+export function cacheMiddleware() {
+  // ...
+}
"#;
        let sketch = parse_sketch("add cache", output);
        assert_eq!(sketch.diffs.len(), 1);
        assert!(sketch.diffs[0].is_new);
        assert_eq!(
            sketch.diffs[0].path,
            PathBuf::from("src/middleware/cache.ts")
        );
    }

    #[test]
    fn parse_multi_file_diff() {
        let output = r#"--- a/src/routes.ts
+++ b/src/routes.ts
@@ -1,3 +1,4 @@
 import { auth } from './middleware/auth';
+import { cache } from './middleware/cache';

--- /dev/null
+++ b/src/middleware/cache.ts
@@ -0,0 +1,3 @@
+export function cache() {
+  // cache impl
+}
"#;
        let sketch = parse_sketch("add cache", output);
        assert_eq!(sketch.diffs.len(), 2);
        assert!(!sketch.diffs[0].is_new);
        assert!(sketch.diffs[1].is_new);
    }

    #[test]
    fn format_sketch_readable() {
        let sketch = Sketch {
            description: "add logging".into(),
            diffs: vec![FileDiff {
                path: PathBuf::from("src/auth.ts"),
                is_new: false,
                diff: "@@ -10,6 +10,7 @@\n+  logger.info(\"ok\");\n".into(),
                new_content: None,
            }],
        };
        let out = format_sketch(&sketch);
        assert!(out.contains("Sketch: add logging"));
        assert!(out.contains("src/auth.ts (modify)"));
    }
}
