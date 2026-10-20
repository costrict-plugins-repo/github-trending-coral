//! Named plan storage — save/load/list implementation plans as YAML.
//!
//! Plans are stored under `.coral/plans/<name>.yaml` for later recall.
//! Names are sanitized to alphanumeric + hyphens only.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

use crate::plan::ImplementationPlan;

/// Sanitize a plan name to contain only lowercase alphanumeric characters
/// and hyphens. Whitespace is converted to hyphens, other characters
/// are stripped, and consecutive hyphens are collapsed.
fn sanitize_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        if c.is_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if c == '-' || c == '_' || c.is_whitespace() {
            // Collapse consecutive separators
            if !out.ends_with('-') {
                out.push('-');
            }
        }
        // other characters are dropped
    }
    // Trim leading/trailing hyphens
    out.trim_matches('-').to_string()
}

/// Return the directory where named plans are stored.
fn plans_dir(root: &Path) -> PathBuf {
    root.join(".coral").join("plans")
}

/// Save a plan to `.coral/plans/<name>.yaml`.
///
/// The name is sanitized (alphanumeric + hyphens). An empty or
/// all-punctuation name is rejected.
pub fn save_named_plan(root: &Path, name: &str, plan: &ImplementationPlan) -> Result<PathBuf> {
    let safe_name = sanitize_name(name);
    if safe_name.is_empty() {
        bail!("plan name is empty after sanitization (must contain alphanumeric characters)");
    }

    let dir = plans_dir(root);
    std::fs::create_dir_all(&dir).context("failed to create .coral/plans directory")?;

    let path = dir.join(format!("{safe_name}.yaml"));
    let yaml = serde_yaml::to_string(plan).context("failed to serialize plan to YAML")?;
    std::fs::write(&path, yaml).with_context(|| format!("failed to write plan {safe_name}"))?;

    Ok(path)
}

/// Load a named plan from `.coral/plans/<name>.yaml`.
///
/// The name is sanitized before lookup.
pub fn load_named_plan(root: &Path, name: &str) -> Result<ImplementationPlan> {
    let safe_name = sanitize_name(name);
    if safe_name.is_empty() {
        bail!("plan name is empty after sanitization");
    }

    let path = plans_dir(root).join(format!("{safe_name}.yaml"));
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("plan '{safe_name}' not found"))?;
    let plan: ImplementationPlan =
        serde_yaml::from_str(&content).with_context(|| format!("failed to parse plan '{safe_name}'"))?;
    Ok(plan)
}

/// List all saved plan names (without the `.yaml` extension).
///
/// Returns names sorted alphabetically.
pub fn list_plans(root: &Path) -> Result<Vec<String>> {
    let dir = plans_dir(root);
    if !dir.exists() {
        return Ok(vec![]);
    }

    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .context("failed to read .coral/plans directory")?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let name = entry.file_name().to_string_lossy().to_string();
            if name.ends_with(".yaml") {
                Some(name.strip_suffix(".yaml").unwrap().to_string())
            } else {
                None
            }
        })
        .collect();

    names.sort();
    Ok(names)
}

/// Delete a named plan. Returns true if the plan existed and was removed.
pub fn delete_named_plan(root: &Path, name: &str) -> Result<bool> {
    let safe_name = sanitize_name(name);
    if safe_name.is_empty() {
        bail!("plan name is empty after sanitization");
    }

    let path = plans_dir(root).join(format!("{safe_name}.yaml"));
    if path.exists() {
        std::fs::remove_file(&path)
            .with_context(|| format!("failed to delete plan '{safe_name}'"))?;
        Ok(true)
    } else {
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{PlanImpact, PlanSources, PlanStep, StepAction};

    fn sample_plan(title: &str) -> ImplementationPlan {
        ImplementationPlan {
            title: title.to_string(),
            confidence: 0.85,
            grounded_in: PlanSources {
                wiki: vec!["auth-middleware".into()],
                code: vec![PathBuf::from("src/lib.rs")],
                patterns: vec![],
            },
            steps: vec![PlanStep {
                file: PathBuf::from("src/cache.rs"),
                action: StepAction::Create,
                description: "New cache module".into(),
                symbols_affected: vec![],
                depends_on: vec![],
                assertions: vec!["should cache".into()],
            }],
            impact: PlanImpact {
                files_modified: 0,
                files_created: 1,
                new_dependencies: vec![],
                symbols_added: vec!["Cache".into()],
                symbols_modified: vec![],
                potentially_affected: vec![],
            },
            risks: vec!["memory usage".into()],
        }
    }

    #[test]
    fn sanitize_name_basic() {
        assert_eq!(sanitize_name("add-caching"), "add-caching");
        assert_eq!(sanitize_name("Add Caching"), "add-caching");
        assert_eq!(sanitize_name("my_cool_plan"), "my-cool-plan");
        assert_eq!(sanitize_name("plan!!!"), "plan");
        assert_eq!(sanitize_name("--leading--trailing--"), "leading-trailing");
        assert_eq!(sanitize_name(""), "");
        assert_eq!(sanitize_name("!!!"), "");
        assert_eq!(sanitize_name("a  b  c"), "a-b-c");
    }

    #[test]
    fn save_and_load_roundtrip() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        let plan = sample_plan("Add caching");

        let path = save_named_plan(root, "add-caching", &plan).unwrap();
        assert!(path.exists());
        assert!(path.to_string_lossy().contains("add-caching.yaml"));

        let loaded = load_named_plan(root, "add-caching").unwrap();
        assert_eq!(loaded.title, "Add caching");
        assert_eq!(loaded.confidence, 0.85);
        assert_eq!(loaded.steps.len(), 1);
        assert_eq!(loaded.steps[0].file, PathBuf::from("src/cache.rs"));
    }

    #[test]
    fn save_sanitizes_name() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        let plan = sample_plan("Rate Limiting");

        let path = save_named_plan(root, "Rate Limiting!", &plan).unwrap();
        assert!(path.to_string_lossy().contains("rate-limiting.yaml"));

        // Can load with original unsanitized name too
        let loaded = load_named_plan(root, "Rate Limiting!").unwrap();
        assert_eq!(loaded.title, "Rate Limiting");
    }

    #[test]
    fn save_rejects_empty_name() {
        let tmp = tempfile::TempDir::new().unwrap();
        let plan = sample_plan("test");
        let err = save_named_plan(tmp.path(), "!!!", &plan).unwrap_err();
        assert!(err.to_string().contains("empty after sanitization"));
    }

    #[test]
    fn load_nonexistent_plan_fails() {
        let tmp = tempfile::TempDir::new().unwrap();
        let err = load_named_plan(tmp.path(), "does-not-exist").unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    fn list_plans_empty_directory() {
        let tmp = tempfile::TempDir::new().unwrap();
        let names = list_plans(tmp.path()).unwrap();
        assert!(names.is_empty());
    }

    #[test]
    fn list_plans_returns_sorted_names() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();

        save_named_plan(root, "zeta-plan", &sample_plan("z")).unwrap();
        save_named_plan(root, "alpha-plan", &sample_plan("a")).unwrap();
        save_named_plan(root, "mid-plan", &sample_plan("m")).unwrap();

        let names = list_plans(root).unwrap();
        assert_eq!(names, vec!["alpha-plan", "mid-plan", "zeta-plan"]);
    }

    #[test]
    fn delete_named_plan_removes_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();

        save_named_plan(root, "to-delete", &sample_plan("del")).unwrap();
        assert!(delete_named_plan(root, "to-delete").unwrap());

        // Second delete returns false
        assert!(!delete_named_plan(root, "to-delete").unwrap());

        // List should be empty
        assert!(list_plans(root).unwrap().is_empty());
    }

    #[test]
    fn delete_nonexistent_returns_false() {
        let tmp = tempfile::TempDir::new().unwrap();
        assert!(!delete_named_plan(tmp.path(), "nope").unwrap());
    }

    #[test]
    fn list_ignores_non_yaml_files() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();

        save_named_plan(root, "real-plan", &sample_plan("real")).unwrap();
        // Create a non-yaml file in the plans dir
        let dir = root.join(".coral/plans");
        std::fs::write(dir.join("notes.txt"), "some notes").unwrap();

        let names = list_plans(root).unwrap();
        assert_eq!(names, vec!["real-plan"]);
    }
}
