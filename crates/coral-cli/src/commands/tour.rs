//! `coral tour` — interactive guided walkthrough of Coral features.
//!
//! Shows the user 4-5 key features using their own repo as example.
//! Each step explains what the feature does, shows a real example,
//! and tells them the command to use it.
//!
//! Design: prints sections sequentially with a "Press Enter to continue"
//! between each. No LLM calls — purely local, instant.

use anyhow::Result;
use clap::Args;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Args, Debug, Default)]
pub struct TourArgs {
    /// Skip interactive pauses (print everything at once).
    #[arg(long)]
    pub no_pause: bool,
}

pub fn run(args: TourArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    let root: PathBuf = wiki_root
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(".wiki"));

    if !root.exists() {
        eprintln!("  ✗ No se encontró .wiki/ en este repositorio.");
        eprintln!("    Ejecuta primero: coral setup");
        return Ok(ExitCode::FAILURE);
    }

    let pages = collect_page_info(&root);
    let page_count = pages.len();

    // ─── Step 1: What is Coral? ──────────────────────────────────────
    println!();
    println!("  🪸 Tour de Coral");
    println!("  ═══════════════════════════════════════════════════");
    println!();
    println!("  Coral es una wiki viva de tu código. A diferencia de los READMEs");
    println!("  que se desactualizan, Coral mantiene documentación sincronizada con");
    println!("  tu código fuente usando LLMs.");
    println!();
    println!("  Tu wiki tiene {} páginas que cubren la arquitectura de este repo.", page_count);
    println!();

    pause_if_interactive(&args)?;

    // ─── Step 2: Wiki pages ──────────────────────────────────────────
    println!("  ━━ 1/4: Páginas del Wiki ━━");
    println!();
    println!("  Cada página documenta un concepto, módulo, o flujo de tu código.");
    println!("  Las páginas están en .wiki/ y tienen:");
    println!();
    println!("    • slug       — identificador único (ej: auth-middleware)");
    println!("    • type       — categoría (module, flow, entity, concept...)");
    println!("    • confidence — qué tan confiable es (0.0 – 1.0)");
    println!("    • sources    — archivos fuente que documenta");
    println!("    • backlinks  — otras páginas que la referencian");
    println!();

    // Show real examples from their wiki
    if let Some(sample) = pages.first() {
        println!("  Ejemplo de tu repo:");
        println!("  ┌──────────────────────────────────────");
        println!("  │ slug: {}", sample.slug);
        println!("  │ type: {}", sample.page_type);
        println!("  │ confidence: {}", sample.confidence);
        if !sample.sources.is_empty() {
            println!("  │ sources: {}", sample.sources.join(", "));
        }
        println!("  └──────────────────────────────────────");
    }
    println!();
    println!("  📁 Ubicación: {}/", root.display());
    println!();

    pause_if_interactive(&args)?;

    // ─── Step 3: Querying ────────────────────────────────────────────
    println!("  ━━ 2/4: Preguntas al Wiki ━━");
    println!();
    println!("  Puedes hacer preguntas sobre tu código y Coral responde");
    println!("  basándose en la wiki (no en grep ciego del código):");
    println!();
    println!("    coral query \"¿cómo funciona la autenticación?\"");
    println!("    coral query \"¿qué hace el módulo de tareas?\"");
    println!("    coral query \"¿cómo se conectan los servicios?\"");
    println!();
    println!("  En Claude Code, las preguntas se responden automáticamente");
    println!("  consultando la wiki primero (sin necesidad de comandos).");
    println!();

    pause_if_interactive(&args)?;

    // ─── Step 4: Keeping it fresh ────────────────────────────────────
    println!("  ━━ 3/4: Mantener la Wiki Actualizada ━━");
    println!();
    println!("  Cuando modificas código, la wiki puede quedar desactualizada.");
    println!("  Coral detecta esto automáticamente:");
    println!();
    println!("    coral lint          → detecta páginas stale o rotas");
    println!("    coral lint --fix    → auto-corrige formato + confianza");
    println!("    coral ingest        → actualiza páginas con cambios nuevos");
    println!();
    println!("  La confianza se ajusta automáticamente:");
    println!("    • Sube (+0.05/+0.10) cuando los sources existen en disco");
    println!("    • Baja (-0.20) cuando los sources fueron eliminados");
    println!();

    pause_if_interactive(&args)?;

    // ─── Step 5: Daily workflow ──────────────────────────────────────
    println!("  ━━ 4/4: Workflow Diario ━━");
    println!();
    println!("  Comandos que usarás cada día:");
    println!();
    println!("  ┌─────────────────────────────────────────────────────────────┐");
    println!("  │  coral status      │ Dashboard rápido: salud del wiki       │");
    println!("  │  coral query \"..\"  │ Preguntar sobre arquitectura           │");
    println!("  │  coral lint        │ Verificar consistencia                 │");
    println!("  │  coral search \"..\" │ Buscar en páginas del wiki             │");
    println!("  │  coral ui serve    │ WebUI visual en localhost:3838         │");
    println!("  └─────────────────────────────────────────────────────────────┘");
    println!();
    println!("  En Claude Code, todo esto es automático — Claude consulta la");
    println!("  wiki antes de responder y ejecuta lint después de editar código.");
    println!();
    println!("  ═══════════════════════════════════════════════════");
    println!("  ✓ Tour completado. ¡Prueba: coral query \"¿qué hace este repo?\"");
    println!();

    Ok(ExitCode::SUCCESS)
}

/// Pause and wait for Enter (unless --no-pause).
fn pause_if_interactive(args: &TourArgs) -> Result<()> {
    if args.no_pause {
        return Ok(());
    }
    eprint!("  [Enter para continuar] ");
    io::stderr().flush()?;
    let stdin = io::stdin();
    let mut line = String::new();
    stdin.lock().read_line(&mut line)?;
    println!();
    Ok(())
}

/// Minimal page info extracted from frontmatter.
struct PageInfo {
    slug: String,
    page_type: String,
    confidence: String,
    sources: Vec<String>,
}

/// Read basic info from wiki pages for the tour examples.
fn collect_page_info(root: &Path) -> Vec<PageInfo> {
    let mut pages = Vec::new();
    collect_pages_from_dir(root, &mut pages);
    // Sort by confidence descending so we show the best examples
    pages.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap_or(std::cmp::Ordering::Equal));
    pages
}

fn collect_pages_from_dir(dir: &Path, pages: &mut Vec<PageInfo>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_pages_from_dir(&path, pages);
        } else if path.extension().map(|e| e == "md").unwrap_or(false) {
            let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
            if name == "SCHEMA.md" || name == "index.md" || name == "log.md" {
                continue;
            }
            if let Some(info) = parse_page_frontmatter(&path) {
                pages.push(info);
            }
        }
    }
}

fn parse_page_frontmatter(path: &Path) -> Option<PageInfo> {
    let content = std::fs::read_to_string(path).ok()?;
    if !content.starts_with("---") {
        return None;
    }
    let end = content[3..].find("---")?;
    let yaml = &content[3..3 + end];

    let slug = extract_yaml_value(yaml, "slug")?;
    let page_type = extract_yaml_value(yaml, "type").unwrap_or_else(|| "unknown".into());
    let confidence = extract_yaml_value(yaml, "confidence").unwrap_or_else(|| "0.5".into());
    let sources = extract_yaml_list(yaml, "sources");

    Some(PageInfo {
        slug,
        page_type,
        confidence,
        sources,
    })
}

fn extract_yaml_value(yaml: &str, key: &str) -> Option<String> {
    for line in yaml.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix(&format!("{key}:")) {
            let val = rest.trim().trim_matches('"').trim_matches('\'');
            if !val.is_empty() {
                return Some(val.to_string());
            }
        }
    }
    None
}

fn extract_yaml_list(yaml: &str, key: &str) -> Vec<String> {
    let mut in_list = false;
    let mut items = Vec::new();
    for line in yaml.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with(&format!("{key}:")) {
            // Check inline list
            if let Some(rest) = trimmed.strip_prefix(&format!("{key}:")) {
                let rest = rest.trim();
                if rest.starts_with('[') && rest.ends_with(']') {
                    // Inline list
                    let inner = &rest[1..rest.len() - 1];
                    for item in inner.split(',') {
                        let item = item.trim().trim_matches('"').trim_matches('\'');
                        if !item.is_empty() {
                            items.push(item.to_string());
                        }
                    }
                    return items;
                }
            }
            in_list = true;
            continue;
        }
        if in_list {
            if trimmed.starts_with("- ") {
                let val = trimmed[2..].trim().trim_matches('"').trim_matches('\'');
                items.push(val.to_string());
            } else if !trimmed.is_empty() && !trimmed.starts_with('#') {
                break; // End of list
            }
        }
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_yaml_value_basic() {
        let yaml = "slug: auth-middleware\ntype: module\nconfidence: 0.95\n";
        assert_eq!(extract_yaml_value(yaml, "slug"), Some("auth-middleware".into()));
        assert_eq!(extract_yaml_value(yaml, "type"), Some("module".into()));
        assert_eq!(extract_yaml_value(yaml, "confidence"), Some("0.95".into()));
        assert_eq!(extract_yaml_value(yaml, "missing"), None);
    }

    #[test]
    fn extract_yaml_list_inline() {
        let yaml = "sources: [\"src/auth.ts\", \"src/jwt.ts\"]\n";
        let list = extract_yaml_list(yaml, "sources");
        assert_eq!(list, vec!["src/auth.ts", "src/jwt.ts"]);
    }

    #[test]
    fn extract_yaml_list_multiline() {
        let yaml = "sources:\n  - src/auth.ts\n  - src/jwt.ts\nbacklinks:\n";
        let list = extract_yaml_list(yaml, "sources");
        assert_eq!(list, vec!["src/auth.ts", "src/jwt.ts"]);
    }
}
