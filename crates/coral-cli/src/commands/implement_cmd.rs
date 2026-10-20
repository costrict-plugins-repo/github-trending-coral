//! `coral implement` — implementation assistance (patterns, plan, sketch, impact, validate).

use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Args, Debug)]
pub struct ImplementArgs {
    #[command(subcommand)]
    pub command: ImplementCmd,
}

#[derive(Subcommand, Debug)]
pub enum ImplementCmd {
    /// Run the full implement loop: understand -> plan -> generate -> validate -> stage.
    Run {
        /// Natural language description of the change.
        description: String,
        /// Automatically apply staged changes (default: stage only).
        #[arg(long)]
        apply: bool,
        /// Use test-driven development mode.
        #[arg(long)]
        tdd: bool,
        /// Maximum retries for LLM.
        #[arg(long, default_value = "2")]
        retries: usize,
    },
    /// Apply previously staged changes from .coral/staging/.
    Apply,
    /// Show what's currently staged.
    Staged,
    /// Clear the staging area.
    ClearStaged,
    /// Detect recurring code patterns in the repository.
    Patterns {
        /// Root directory to scan.
        #[arg(default_value = ".")]
        root: PathBuf,
    },
    /// Generate an implementation plan prompt from a description.
    Plan {
        /// Natural language description of the change.
        description: String,
        /// Execute the plan via LLM (requires configured runner).
        #[arg(long)]
        execute: bool,
        /// Maximum retry attempts for LLM parsing failures.
        #[arg(long, default_value = "2")]
        retries: usize,
        /// Generate test stubs first (TDD mode): includes test-first
        /// guidance in the plan prompt.
        #[arg(long)]
        tdd: bool,
    },
    /// Generate a sketch (diff preview) prompt from a description.
    Sketch {
        /// Natural language description of the change.
        description: String,
        /// Execute the sketch via LLM (requires configured runner).
        #[arg(long)]
        execute: bool,
    },
    /// Analyze the impact of changing specific files.
    Impact {
        /// Files being changed.
        files: Vec<PathBuf>,
        /// Maximum dependency traversal depth.
        #[arg(long, default_value = "3")]
        depth: usize,
    },
    /// Validate changes: check files exist, compile, run tests.
    Validate {
        /// Files to validate.
        files: Vec<PathBuf>,
        /// Root directory (defaults to current directory).
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },
}

pub fn run(args: ImplementArgs, _wiki_root: Option<&Path>) -> Result<ExitCode> {
    match args.command {
        ImplementCmd::Run {
            description,
            apply,
            tdd,
            retries,
        } => run_orchestrate(&description, apply, tdd, retries),
        ImplementCmd::Apply => run_apply(),
        ImplementCmd::Staged => run_staged(),
        ImplementCmd::ClearStaged => run_clear_staged(),
        ImplementCmd::Patterns { root } => run_patterns(&root),
        ImplementCmd::Plan {
            description,
            execute,
            retries,
            tdd,
        } => run_plan(&description, execute, retries, tdd),
        ImplementCmd::Sketch {
            description,
            execute,
        } => run_sketch(&description, execute),
        ImplementCmd::Impact { files, depth } => run_impact(&files, depth),
        ImplementCmd::Validate { files, root } => run_validate(&root, &files),
    }
}

fn run_orchestrate(description: &str, apply: bool, tdd: bool, retries: usize) -> Result<ExitCode> {
    use coral_implement::orchestrate::{orchestrate, OrchestrateConfig, StderrProgress};

    let root = std::env::current_dir().context("failed to get current directory")?;

    let runner = super::runner_helper::default_runner();
    let config = OrchestrateConfig {
        root: root.clone(),
        max_retries: retries,
        tdd,
        auto_apply: apply,
        run_tests: tdd,
        max_test_retries: 2,
    };
    let progress = StderrProgress;

    let result = orchestrate(description, runner.as_ref(), &config, &progress)?;

    // Print final summary
    if let Some(ref plan_result) = result.plan {
        use coral_implement::plan::format_plan;
        eprintln!("\n{}", format_plan(&plan_result.plan));
    }

    if let Some(ref sketch_result) = result.sketch {
        if !sketch_result.sketch.diffs.is_empty() {
            use coral_implement::sketch::format_sketch;
            print!("{}", format_sketch(&sketch_result.sketch));
        }
    }

    if result.applied {
        eprintln!("\nChanges applied successfully.");
        eprintln!(
            "▶ [Coral] Consider running `coral index && coral stale` to update the code index and check wiki freshness."
        );
    } else if result.staged_path.is_some() {
        eprintln!("\nChanges staged. Review with `coral implement staged`, apply with `coral implement apply`.");
    }

    Ok(ExitCode::SUCCESS)
}

fn run_apply() -> Result<ExitCode> {
    use coral_implement::staging::{apply_staged, load_staged};

    let root = std::env::current_dir().context("failed to get current directory")?;

    let staged = load_staged(&root)?;
    eprintln!("Applying staged changes: {}", staged.description);

    let applied = apply_staged(&root, &staged)?;
    if applied.is_empty() {
        eprintln!("No files with content to apply (only diffs without new_content).");
    } else {
        eprintln!("Applied {} file(s):", applied.len());
        for f in &applied {
            eprintln!("  + {}", f.display());
        }
    }

    // Post-apply: check if any applied files are referenced by wiki pages
    // and suggest updating the index + checking for stale pages.
    post_apply_wiki_hint(&root, &applied);

    Ok(ExitCode::SUCCESS)
}

/// After applying staged changes, check if a code index exists and
/// whether any of the changed files intersect with wiki page sources.
/// Prints actionable suggestions to keep the wiki in sync.
fn post_apply_wiki_hint(root: &Path, applied_files: &[PathBuf]) {
    if applied_files.is_empty() {
        return;
    }

    let index_path = root.join(".coral/index.json");
    let wiki_path = root.join(".wiki");

    if index_path.exists() {
        // Check if any applied files have wiki pages referencing them.
        let has_wiki_refs = if wiki_path.exists() {
            check_wiki_references_changed_files(root, &wiki_path, applied_files)
        } else {
            false
        };

        if has_wiki_refs {
            eprintln!(
                "\n⚠️ [Coral] Some changed files are referenced by wiki pages."
            );
            eprintln!(
                "   Consider running: `coral index && coral stale` to update symbol references."
            );
        } else {
            eprintln!(
                "\n▶ [Coral] Consider running `coral index` to update the code index."
            );
        }
    }
}

/// Check whether any wiki pages reference the changed files in their
/// `sources:` frontmatter. Returns `true` if at least one match is found.
fn check_wiki_references_changed_files(
    repo_root: &Path,
    wiki_root: &Path,
    changed_files: &[PathBuf],
) -> bool {
    let pages = match coral_core::walk::read_pages(wiki_root) {
        Ok(p) => p,
        Err(_) => return false,
    };

    for page in &pages {
        for src in &page.frontmatter.sources {
            let src_abs = repo_root.join(src);
            for changed in changed_files {
                let changed_abs = if changed.is_absolute() {
                    changed.clone()
                } else {
                    repo_root.join(changed)
                };
                if src_abs == changed_abs || src == &changed.display().to_string() {
                    return true;
                }
            }
        }
    }

    false
}

fn run_staged() -> Result<ExitCode> {
    use coral_implement::staging::{format_staged, load_staged};

    let root = std::env::current_dir().context("failed to get current directory")?;

    match load_staged(&root) {
        Ok(staged) => {
            print!("{}", format_staged(&staged));
            Ok(ExitCode::SUCCESS)
        }
        Err(_) => {
            eprintln!("No staged changes. Run `coral implement run \"description\"` to generate.");
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn run_clear_staged() -> Result<ExitCode> {
    use coral_implement::staging::clear_staged;

    let root = std::env::current_dir().context("failed to get current directory")?;
    clear_staged(&root)?;
    eprintln!("Staging area cleared.");
    Ok(ExitCode::SUCCESS)
}

fn run_patterns(root: &Path) -> Result<ExitCode> {
    use coral_implement::patterns::{detect_patterns, format_patterns, ExportedSymbol, FileShape};

    let index_path = root.join(".coral/index.json");
    if !index_path.exists() {
        anyhow::bail!("No code index found. Run `coral index` first.");
    }

    let index = coral_index::load_index(&index_path)
        .context("failed to load index")?;

    // Convert index symbols to FileShapes for pattern detection
    let file_shapes: Vec<FileShape> = index.files.iter().map(|f| {
        FileShape {
            path: f.path.clone(),
            exports: f.symbols.iter().map(|s| ExportedSymbol {
                name: s.name.clone(),
                kind: s.kind.to_string(),
                signature: s.signature.clone(),
            }).collect(),
            imports: f.imports.clone(),
        }
    }).collect();

    let patterns = detect_patterns(&file_shapes);
    if patterns.is_empty() {
        eprintln!("No recurring patterns detected (need 3+ files with shared structure in same directory).");
    } else {
        eprint!("{}", format_patterns(&patterns));
    }

    Ok(ExitCode::SUCCESS)
}

fn run_plan(description: &str, execute: bool, retries: usize, tdd: bool) -> Result<ExitCode> {
    use coral_implement::plan::{plan_system_prompt, plan_user_prompt, PlanContext};

    let mut request = description.to_string();
    if tdd {
        request.push_str(
            "\n\n**TDD Mode**: Generate test files FIRST. Each step that creates a new \
             module must be preceded by a step that creates its test file with failing \
             assertions. The implementation step follows, making the tests pass.",
        );
    }

    let ctx = PlanContext {
        request,
        relevant_code: vec![],
        relevant_wiki: vec![],
        applicable_patterns: vec![],
        affected_files: vec![],
    };

    if execute {
        use coral_implement::execute::execute_plan_with_retry;
        use coral_implement::plan::format_plan;

        eprintln!("Executing plan via LLM (max retries: {retries})...\n");

        let runner = super::runner_helper::default_runner();
        let result = execute_plan_with_retry(&ctx, runner.as_ref(), retries)?;

        // Print the formatted plan
        print!("{}", format_plan(&result.plan));

        // Print usage info if available
        if let Some(usage) = result.usage {
            eprintln!(
                "\nTokens: {} input, {} output",
                usage.input_tokens, usage.output_tokens
            );
        }
    } else {
        eprintln!("━━ Implementation Plan Context ━━\n");
        eprintln!("Pass this to your LLM to generate an implementation plan:\n");

        println!("--- system ---\n{}\n", plan_system_prompt());
        println!("--- user ---\n{}", plan_user_prompt(&ctx));

        eprintln!(
            "\nTip: Use `coral find \"{}\"` first to gather relevant code context.",
            description
        );
        eprintln!("Tip: Use `--execute` to have the LLM generate the plan directly.");
    }

    Ok(ExitCode::SUCCESS)
}

fn run_sketch(description: &str, execute: bool) -> Result<ExitCode> {
    use coral_implement::sketch::{sketch_system_prompt, sketch_user_prompt, SketchContext};

    let context: Vec<SketchContext> = vec![];

    if execute {
        use coral_implement::execute::execute_sketch;
        use coral_implement::sketch::format_sketch;

        eprintln!("Executing sketch via LLM...\n");

        let runner = super::runner_helper::default_runner();
        let result = execute_sketch(description, &context, runner.as_ref())?;

        // Print the formatted sketch
        print!("{}", format_sketch(&result.sketch));

        if result.sketch.diffs.is_empty() {
            eprintln!("\nWarning: LLM output contained no parseable diffs.");
        }

        // Print usage info if available
        if let Some(usage) = result.usage {
            eprintln!(
                "\nTokens: {} input, {} output",
                usage.input_tokens, usage.output_tokens
            );
        }
    } else {
        eprintln!("━━ Sketch Context ━━\n");
        eprintln!("Pass this to your LLM to generate a diff preview:\n");

        println!("--- system ---\n{}\n", sketch_system_prompt());
        println!(
            "--- user ---\n{}",
            sketch_user_prompt(description, &context)
        );

        eprintln!(
            "\nTip: Use `coral find \"{}\"` first to gather file context.",
            description
        );
        eprintln!("Tip: Use `--execute` to have the LLM generate the sketch directly.");
    }

    Ok(ExitCode::SUCCESS)
}

fn run_impact(files: &[PathBuf], depth: usize) -> Result<ExitCode> {
    let graph_path = PathBuf::from(".coral/graph.json");
    if !graph_path.exists() {
        anyhow::bail!("No code graph found. Run `coral graph build` first.");
    }

    let graph = coral_graph::load_graph(&graph_path)
        .context("failed to load graph")?;

    eprintln!("━━ Impact Analysis ━━\n");
    eprintln!("Changed files:");
    for f in files {
        eprintln!("  • {}", f.display());
    }

    let mut total_affected = 0;
    for file in files {
        let affected = graph.affected_by(file, depth);
        if !affected.is_empty() {
            eprintln!("\n  {} → {} downstream files:", file.display(), affected.len());
            for a in affected.iter().take(15) {
                eprintln!("    ⇢ {}", a.display());
            }
            if affected.len() > 15 {
                eprintln!("    ... and {} more", affected.len() - 15);
            }
            total_affected += affected.len();
        }
    }

    if total_affected == 0 {
        eprintln!("\n  No downstream dependents found.");
    } else {
        eprintln!("\nTotal blast radius: {} files", total_affected);
    }

    Ok(ExitCode::SUCCESS)
}

fn run_validate(root: &Path, files: &[PathBuf]) -> Result<ExitCode> {
    use coral_implement::validate::{format_validation, validate_change};

    eprintln!("━━ Validating changes ━━\n");

    let result = validate_change(root, files)?;
    eprint!("{}", format_validation(&result));

    if result.passed {
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(ExitCode::from(1))
    }
}
