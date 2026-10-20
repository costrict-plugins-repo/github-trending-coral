//! `coral setup` — single-command onboarding for new users.
//!
//! Combines `coral init` + cost estimation + confirmation + `coral bootstrap --apply`
//! + welcome screen into one guided flow. The goal: a developer who just
//! cloned a repo and has `coral` on PATH can type `coral setup` and be
//! productive in under 2 minutes.
//!
//! Design goals (v0.41 onboarding rework):
//! - Zero prerequisite knowledge (the user doesn't need to know about init/bootstrap/ingest)
//! - Continuous feedback (progress per phase, per page, final summary)
//! - Graceful degradation (if LLM fails, fall back to from-symbols)
//! - Single confirmation point (show estimate, one Y/n prompt)

use anyhow::{Context, Result};
use clap::Args;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use super::bootstrap::BootstrapArgs;
use super::init::InitArgs;

#[derive(Args, Debug)]
pub struct SetupArgs {
    /// Skip the confirmation prompt (auto-accept cost estimate).
    #[arg(long)]
    pub yes: bool,
    /// LLM provider override (default: auto-detect).
    #[arg(long)]
    pub provider: Option<String>,
    /// Model override.
    #[arg(long)]
    pub model: Option<String>,
    /// Maximum cost in USD (abort if estimate exceeds this).
    #[arg(long, value_name = "USD")]
    pub max_cost: Option<f64>,
    /// Limit to N pages (for large repos).
    #[arg(long, value_name = "N")]
    pub max_pages: Option<usize>,
}

pub fn run(args: SetupArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    let start = Instant::now();
    let cwd = std::env::current_dir().context("getting cwd")?;

    // ──────────────────────────────────────────────────────────────────
    // Phase 1: Welcome banner
    // ──────────────────────────────────────────────────────────────────
    print_banner();

    // ──────────────────────────────────────────────────────────────────
    // Phase 2: Init (.wiki/ structure, .gitignore, CLAUDE.md)
    // ──────────────────────────────────────────────────────────────────
    eprintln!("\n{}", phase_header(1, "Preparando repositorio"));

    let root: PathBuf = wiki_root
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(".wiki"));

    let already_initialized = root.exists() && root.join("SCHEMA.md").exists();

    if already_initialized {
        eprintln!("  ✓ .wiki/ ya existe — saltando inicialización");
    } else {
        eprintln!("  ▶ Creando .wiki/, .gitignore, CLAUDE.md...");
        let init_args = InitArgs {
            force: false,
            yes: true,
            provider: args.provider.clone(),
        };
        super::init::run(init_args, wiki_root)?;
        eprintln!("  ✓ Repositorio preparado");
    }

    // ──────────────────────────────────────────────────────────────────
    // Phase 3: Scan + Estimate
    // ──────────────────────────────────────────────────────────────────
    eprintln!("\n{}", phase_header(2, "Analizando código fuente"));
    eprintln!("  ▶ Escaneando archivos...");

    // Count files for the user
    let (loc, file_count) = repo_size_quick(&cwd);
    eprintln!("  ✓ {} archivos encontrados ({} LOC)", file_count, format_number(loc));

    // ──────────────────────────────────────────────────────────────────
    // Phase 4: Estimate + Confirm
    // ──────────────────────────────────────────────────────────────────
    eprintln!("\n{}", phase_header(3, "Estimación de costos"));

    // Heuristic estimate (without running the planner LLM call)
    let estimated_pages = estimate_page_count(file_count);
    let estimated_pages = args.max_pages.map(|n| n.min(estimated_pages)).unwrap_or(estimated_pages);
    let (est_cost_low, est_cost_high) = estimate_cost_range(estimated_pages);
    let est_time_secs = estimated_pages as u64 * 5; // ~5s per page avg

    eprintln!();
    eprintln!("  ┌─────────────────────────────────────────────┐");
    eprintln!("  │  📊 Resumen del proyecto                    │");
    eprintln!("  ├─────────────────────────────────────────────┤");
    eprintln!("  │  Archivos:  {:>6}                          │", file_count);
    eprintln!("  │  LOC:       {:>6}                          │", format_number(loc));
    eprintln!("  │  Páginas:   ~{:<5}                          │", estimated_pages);
    eprintln!("  │  Costo:     ${:.2} – ${:.2} USD             │", est_cost_low, est_cost_high);
    eprintln!("  │  Tiempo:    ~{:<3} segundos                  │", est_time_secs);
    eprintln!("  └─────────────────────────────────────────────┘");
    eprintln!();

    // Pre-flight cost gate
    if let Some(max) = args.max_cost {
        if est_cost_high > max {
            eprintln!(
                "  ✗ Estimación (${:.2}) excede --max-cost (${:.2}). Abortando.",
                est_cost_high, max
            );
            eprintln!("    Sugerencia: usa --max-pages={} para reducir el costo.",
                (max / est_cost_high * estimated_pages as f64) as usize
            );
            return Ok(ExitCode::FAILURE);
        }
    }

    // Confirmation prompt
    if !args.yes {
        eprint!("  ¿Continuar? [Y/n] ");
        io::stderr().flush()?;
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let answer = input.trim().to_lowercase();
        if !answer.is_empty() && answer != "y" && answer != "yes" && answer != "si" && answer != "sí" {
            eprintln!("\n  Cancelado. Puedes ejecutar más tarde:");
            eprintln!("    coral setup --yes");
            return Ok(ExitCode::SUCCESS);
        }
    }

    // ──────────────────────────────────────────────────────────────────
    // Phase 5: Bootstrap with progress
    // ──────────────────────────────────────────────────────────────────
    eprintln!("\n{}", phase_header(4, "Generando wiki"));
    eprintln!("  ▶ Llamando al LLM para planificar páginas...");

    let bootstrap_args = BootstrapArgs {
        model: args.model,
        provider: args.provider,
        dry_run: false,
        apply: true,
        from_symbols: false,
        path: None,
        estimate: false,
        max_cost: args.max_cost,
        resume: false,
        max_pages: args.max_pages,
    };

    // Set an env var so bootstrap knows to emit progress lines
    // SAFETY: single-threaded at this point in the flow
    unsafe { std::env::set_var("CORAL_SETUP_PROGRESS", "1") };

    let bootstrap_result = super::bootstrap::run(bootstrap_args, wiki_root);

    unsafe { std::env::remove_var("CORAL_SETUP_PROGRESS") };

    match bootstrap_result {
        Ok(code) if code == ExitCode::SUCCESS => {
            eprintln!("  ✓ Wiki generado exitosamente");
        }
        Ok(code) => {
            // Partial completion (max-cost reached) — still show welcome
            eprintln!("  ⚠ Bootstrap parcial (límite de costo alcanzado)");
            eprintln!("    Ejecuta `coral bootstrap --resume` para continuar.");
            // Fall through to welcome
            let elapsed = start.elapsed();
            print_welcome(&root, elapsed, true);
            return Ok(code);
        }
        Err(e) => {
            eprintln!("  ✗ Error durante bootstrap: {e}");
            eprintln!();
            eprintln!("  Alternativa rápida (sin LLM, calidad inferior):");
            eprintln!("    coral bootstrap --from-symbols --apply");
            eprintln!();
            eprintln!("  O diagnostica el problema:");
            eprintln!("    coral doctor");
            return Err(e);
        }
    }

    // ──────────────────────────────────────────────────────────────────
    // Phase 6: Post-bootstrap lint fix
    // ──────────────────────────────────────────────────────────────────
    eprintln!("\n{}", phase_header(5, "Limpieza automática"));
    eprintln!("  ▶ Ejecutando lint --fix...");

    let lint_args = super::lint::LintArgs {
        fix: true,
        apply: true,
        ..Default::default()
    };
    let _ = super::lint::run(lint_args, wiki_root);
    eprintln!("  ✓ Wiki formateado y confianza calibrada");

    // ──────────────────────────────────────────────────────────────────
    // Phase 7: Welcome screen
    // ──────────────────────────────────────────────────────────────────
    let elapsed = start.elapsed();
    print_welcome(&root, elapsed, false);

    Ok(ExitCode::SUCCESS)
}

// ═══════════════════════════════════════════════════════════════════════
// Helpers
// ═══════════════════════════════════════════════════════════════════════

fn print_banner() {
    eprintln!();
    eprintln!("  🪸 Coral Setup");
    eprintln!("  ─────────────────────────────────────────────");
    eprintln!("  Voy a crear una wiki de tu código fuente que");
    eprintln!("  te permite hacer preguntas sobre la arquitectura");
    eprintln!("  y mantener documentación viva.");
}

fn phase_header(n: u8, title: &str) -> String {
    format!("  ━━ Paso {n}: {title} ━━")
}

fn print_welcome(root: &Path, elapsed: std::time::Duration, partial: bool) {
    // Count pages in the wiki
    let page_count = count_wiki_pages(root);
    let secs = elapsed.as_secs();

    eprintln!();
    eprintln!("  ┏━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓");
    eprintln!("  ┃  🎉 ¡Wiki listo!                              ┃");
    eprintln!("  ┣━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┫");
    eprintln!("  ┃                                               ┃");
    eprintln!("  ┃  📄 {} páginas generadas                    ┃", format!("{:>3}", page_count));
    eprintln!("  ┃  ⏱️  {} segundos                             ┃", format!("{:>3}", secs));
    if partial {
        eprintln!("  ┃  ⚠️  Parcial (usa --resume para completar)   ┃");
    }
    eprintln!("  ┃                                               ┃");
    eprintln!("  ┣━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┫");
    eprintln!("  ┃  Próximos pasos:                              ┃");
    eprintln!("  ┃                                               ┃");
    eprintln!("  ┃  1. coral status     → ver resumen del wiki   ┃");
    eprintln!("  ┃  2. coral tour       → tour guiado (2 min)    ┃");
    eprintln!("  ┃  3. coral query \"..\" → preguntar al wiki      ┃");
    eprintln!("  ┃                                               ┃");
    eprintln!("  ┃  🌐 WebUI: coral ui serve                     ┃");
    eprintln!("  ┃     → http://localhost:3838                   ┃");
    eprintln!("  ┃                                               ┃");
    eprintln!("  ┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛");
    eprintln!();
}

fn count_wiki_pages(root: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(root) else {
        return 0;
    };
    let mut count = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && path.extension().map(|e| e == "md").unwrap_or(false) {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            if name != "SCHEMA.md" && name != "index.md" && name != "log.md" {
                count += 1;
            }
        }
        // Also count subdirectory pages
        if path.is_dir() {
            if let Ok(sub) = std::fs::read_dir(&path) {
                count += sub
                    .flatten()
                    .filter(|e| {
                        e.path().extension().map(|ext| ext == "md").unwrap_or(false)
                    })
                    .count();
            }
        }
    }
    count
}

fn repo_size_quick(cwd: &Path) -> (usize, usize) {
    let extensions = ["rs", "ts", "tsx", "js", "jsx", "py", "go", "java", "rb", "ex", "exs", "c", "cpp", "h", "hpp", "cs", "swift", "kt"];
    let mut loc = 0usize;
    let mut files = 0usize;
    walk_source_files(cwd, &extensions, &mut |path| {
        files += 1;
        if let Ok(content) = std::fs::read_to_string(path) {
            loc += content.lines().count();
        }
    });
    (loc, files)
}

fn walk_source_files(dir: &Path, extensions: &[&str], callback: &mut dyn FnMut(&Path)) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
        // Skip hidden dirs, node_modules, target, vendor, etc.
        if name.starts_with('.') || name == "node_modules" || name == "target"
            || name == "vendor" || name == "dist" || name == "build"
            || name == "__pycache__" || name == ".git"
        {
            continue;
        }
        if path.is_dir() {
            walk_source_files(&path, extensions, callback);
        } else if let Some(ext) = path.extension() {
            let ext_str = ext.to_string_lossy();
            if extensions.iter().any(|e| *e == ext_str.as_ref()) {
                callback(&path);
            }
        }
    }
}

fn estimate_page_count(file_count: usize) -> usize {
    // Heuristic: ~1 page per 3-5 source files, capped at 30 for initial bootstrap
    let raw = (file_count as f64 / 4.0).ceil() as usize;
    raw.clamp(5, 30)
}

fn estimate_cost_range(pages: usize) -> (f64, f64) {
    // Based on Sonnet pricing: ~1500 input tokens + ~800 output tokens per page
    // Plus a planner call (~3000 input + ~2000 output)
    let input_per_page: f64 = 1500.0;
    let output_per_page: f64 = 800.0;
    let planner_input: f64 = 3000.0;
    let planner_output: f64 = 2000.0;

    let total_input = planner_input + (pages as f64 * input_per_page);
    let total_output = planner_output + (pages as f64 * output_per_page);

    // Claude Sonnet pricing: $3/MTok input, $15/MTok output
    let cost = (total_input / 1_000_000.0 * 3.0) + (total_output / 1_000_000.0 * 15.0);
    let low = cost * 0.7; // Could be cheaper with caching
    let high = cost * 1.25; // Upper bound

    (low, high)
}

fn format_number(n: usize) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_page_count_small_repo() {
        assert_eq!(estimate_page_count(4), 5); // min 5
        assert_eq!(estimate_page_count(8), 5); // ceil(8/4) = 2, clamped to 5
    }

    #[test]
    fn estimate_page_count_medium_repo() {
        assert_eq!(estimate_page_count(40), 10);
        assert_eq!(estimate_page_count(60), 15);
    }

    #[test]
    fn estimate_page_count_large_repo() {
        assert_eq!(estimate_page_count(200), 30); // capped at 30
        assert_eq!(estimate_page_count(1000), 30);
    }

    #[test]
    fn estimate_cost_range_produces_valid_range() {
        let (low, high) = estimate_cost_range(10);
        assert!(low > 0.0);
        assert!(high > low);
        assert!(high < 1.0); // 10 pages should be well under $1
    }

    #[test]
    fn format_number_works() {
        assert_eq!(format_number(500), "500");
        assert_eq!(format_number(1500), "1.5k");
        assert_eq!(format_number(1_500_000), "1.5M");
    }
}
