//! `coral stale` — detect wiki pages whose source files have changed.
//!
//! For each wiki page with `sources:` entries in its frontmatter, checks
//! whether the referenced source files still exist, whether they have been
//! modified after the wiki page was last written, and (when a code index is
//! available) whether any public symbols mentioned in the wiki body have
//! been removed from the source.
//!
//! This is a best-effort heuristic: false positives are acceptable (the
//! user can verify and dismiss), but false negatives should be minimised.

use anyhow::{Context, Result};
use clap::Args;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use coral_core::walk;
use coral_index::{RepoIndex, Visibility, load_index};

// ── Public types ────────────────────────────────────────────────────

/// A wiki page flagged as potentially stale, with one or more reasons.
#[derive(Debug, Clone)]
pub struct StaleCheck {
    pub page_slug: String,
    pub page_path: PathBuf,
    pub reasons: Vec<StaleReason>,
}

/// Why a wiki page is considered stale.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StaleReason {
    /// A source file listed in `sources:` no longer exists on disk.
    SourceDeleted { path: String },
    /// A source file's mtime is newer than the wiki page's mtime.
    SourceModifiedAfterPage { path: String },
    /// A public symbol name that appears in the wiki body is no longer
    /// present in the source file's public symbol list (per the index).
    SymbolRemoved { path: String, symbol: String },
}

// ── CLI ─────────────────────────────────────────────────────────────

#[derive(Args, Debug)]
pub struct StaleArgs {
    /// Repository root directory (default: current directory).
    #[arg(long, default_value = ".")]
    pub root: PathBuf,
}

pub fn run(args: StaleArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    let root = args.root.canonicalize().unwrap_or(args.root.clone());
    let wiki = wiki_root
        .map(Path::to_path_buf)
        .unwrap_or_else(|| root.join(".wiki"));

    if !wiki.exists() {
        anyhow::bail!(
            "wiki root not found: {}. Run `coral init` first.",
            wiki.display()
        );
    }

    let index_path = root.join(".coral").join("index.json");
    let index = if index_path.exists() {
        match load_index(&index_path) {
            Ok(idx) => Some(idx),
            Err(e) => {
                eprintln!(
                    "warning: could not load code index at {}: {e:#}",
                    index_path.display()
                );
                None
            }
        }
    } else {
        None
    };

    let stale = detect_stale(&wiki, &root, index.as_ref())?;

    if stale.is_empty() {
        eprintln!("No stale wiki pages detected.");
        return Ok(ExitCode::SUCCESS);
    }

    eprintln!("{} wiki page(s) appear stale:\n", stale.len());
    for check in &stale {
        eprintln!("  {} ({})", check.page_slug, check.page_path.display());
        for reason in &check.reasons {
            match reason {
                StaleReason::SourceDeleted { path } => {
                    eprintln!("    - source deleted: {path}");
                }
                StaleReason::SourceModifiedAfterPage { path } => {
                    eprintln!("    - source modified after page: {path}");
                }
                StaleReason::SymbolRemoved { path, symbol } => {
                    eprintln!("    - symbol removed from {path}: {symbol}");
                }
            }
        }
    }

    // Exit 1 to signal stale pages were found (matches lint exit-code
    // contract: 1 = findings).
    Ok(ExitCode::from(1))
}

// ── Core detection logic ────────────────────────────────────────────

/// Walk every wiki page and check its `sources:` entries for staleness.
///
/// `wiki_root` is the `.wiki/` directory; `repo_root` is the repository
/// root (parent of `.wiki/`). `index` is optionally loaded from
/// `.coral/index.json`.
pub fn detect_stale(
    wiki_root: &Path,
    repo_root: &Path,
    index: Option<&RepoIndex>,
) -> Result<Vec<StaleCheck>> {
    let pages = walk::read_pages(wiki_root)
        .with_context(|| format!("reading pages from {}", wiki_root.display()))?;

    let mut stale_pages = Vec::new();

    for page in &pages {
        if page.frontmatter.sources.is_empty() {
            continue;
        }

        let page_mtime = file_mtime(&page.path);

        let mut reasons = Vec::new();

        for src in &page.frontmatter.sources {
            let src_abs = repo_root.join(src);

            // 1. Source deleted?
            if !src_abs.exists() {
                reasons.push(StaleReason::SourceDeleted {
                    path: src.clone(),
                });
                continue; // no further checks possible
            }

            // 2. Source modified after wiki page?
            if let (Some(page_mt), Some(src_mt)) = (page_mtime, file_mtime(&src_abs)) {
                if src_mt > page_mt {
                    reasons.push(StaleReason::SourceModifiedAfterPage {
                        path: src.clone(),
                    });
                }
            }

            // 3. Symbol drift (requires index).
            if let Some(idx) = index {
                check_symbol_drift(idx, src, &page.body, repo_root, &mut reasons);
            }
        }

        if !reasons.is_empty() {
            stale_pages.push(StaleCheck {
                page_slug: page.frontmatter.slug.clone(),
                page_path: page.path.clone(),
                reasons,
            });
        }
    }

    Ok(stale_pages)
}

/// Convert stale-detection results into `coral_lint::LintIssue` entries
/// so `coral lint` can include them in its report. Uses `StaleStatus` for
/// mtime-based and symbol-drift findings (the `SourceNotFound` code is
/// already emitted by the structural lint for deleted files, so we skip
/// `SourceDeleted` here to avoid duplicates).
pub fn stale_checks_to_lint_issues(checks: &[StaleCheck]) -> Vec<coral_lint::LintIssue> {
    let mut issues = Vec::new();
    for check in checks {
        for reason in &check.reasons {
            match reason {
                StaleReason::SourceDeleted { .. } => {
                    // Already covered by coral_lint structural SourceNotFound.
                }
                StaleReason::SourceModifiedAfterPage { path } => {
                    issues.push(coral_lint::LintIssue {
                        code: coral_lint::LintCode::StaleStatus,
                        severity: coral_lint::LintSeverity::Warning,
                        page: Some(check.page_path.clone()),
                        message: format!(
                            "Source '{}' was modified after wiki page '{}' was last written",
                            path, check.page_slug
                        ),
                        context: Some(path.clone()),
                    });
                }
                StaleReason::SymbolRemoved { path, symbol } => {
                    issues.push(coral_lint::LintIssue {
                        code: coral_lint::LintCode::StaleStatus,
                        severity: coral_lint::LintSeverity::Warning,
                        page: Some(check.page_path.clone()),
                        message: format!(
                            "Symbol `{}` referenced in wiki page '{}' is no longer a public symbol in '{}'",
                            symbol, check.page_slug, path
                        ),
                        context: Some(format!("{path}::{symbol}")),
                    });
                }
            }
        }
    }
    issues
}

/// Returns the mtime of `path` as seconds-since-epoch, or `None` on error.
fn file_mtime(path: &Path) -> Option<i64> {
    std::fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
}

/// Check whether any public symbols from the indexed file that are
/// mentioned in the wiki body have disappeared from the current index.
fn check_symbol_drift(
    index: &RepoIndex,
    source_rel: &str,
    body: &str,
    repo_root: &Path,
    reasons: &mut Vec<StaleReason>,
) {
    // Find the parsed file in the index. The index stores paths relative
    // to the repo root (or absolute); try both forms.
    let source_path = Path::new(source_rel);
    let source_abs = repo_root.join(source_rel);

    let parsed = index.files.iter().find(|f| {
        f.path == source_path || f.path == source_abs
    });

    let Some(parsed) = parsed else {
        return; // file not in index — skip symbol checks
    };

    // Collect all public symbol names from the indexed file (including
    // children, e.g. methods on an impl block).
    let mut public_symbols = Vec::new();
    collect_public_symbols(&parsed.symbols, &mut public_symbols);

    // For each symbol name mentioned in the body that is NOT in the
    // current public symbols, flag as removed. We only check names
    // that appear as whole words (bounded by non-alphanumeric chars)
    // to reduce false positives.
    //
    // Strategy: we don't have the *old* symbol list, so we can't do a
    // true diff. Instead, we look for body text that references symbol
    // names that no longer exist. This catches the case where a public
    // function was renamed or removed: the wiki still mentions the old
    // name, but the index no longer has it.
    //
    // To avoid noise from common English words that happen to match
    // short symbol names, we only consider symbols with names >= 3 chars.
    let current_names: std::collections::HashSet<String> = public_symbols
        .iter()
        .map(|s| s.to_lowercase())
        .collect();

    // We look for identifiers in the body that look like code references
    // (backtick-wrapped) and check if any match removed symbols.
    // This is more precise than scanning the entire body text.
    for cap in extract_backtick_identifiers(body) {
        let cap_lower = cap.to_lowercase();
        if cap.len() >= 3 && !current_names.contains(&cap_lower) {
            // Only flag if it looks like it *could* have been a symbol
            // name (alphanumeric + underscores, starts with letter/underscore).
            if is_plausible_symbol_name(&cap) {
                // Check that at least some of the CURRENT symbols share
                // a prefix or the same "module" — this reduces noise from
                // random backtick code in the body that never was a symbol
                // in this file. We skip this extra gate for now and rely
                // on the backtick heuristic being selective enough.
                reasons.push(StaleReason::SymbolRemoved {
                    path: source_rel.to_string(),
                    symbol: cap.to_string(),
                });
            }
        }
    }
}

/// Recursively collect public symbol names from a symbol tree.
fn collect_public_symbols(symbols: &[coral_index::Symbol], out: &mut Vec<String>) {
    for sym in symbols {
        if sym.visibility == Visibility::Public {
            out.push(sym.name.clone());
        }
        collect_public_symbols(&sym.children, out);
    }
}

/// Extract identifiers wrapped in backticks from markdown text.
/// Returns the content inside each single-backtick span (not code fences).
fn extract_backtick_identifiers(text: &str) -> Vec<&str> {
    let mut results = Vec::new();
    let mut in_fence = false;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        // Extract single-backtick spans from this line.
        let mut rest = line;
        while let Some(start) = rest.find('`') {
            let after_tick = &rest[start + 1..];
            // Skip double backticks (code fence fragments).
            if after_tick.starts_with('`') {
                rest = &after_tick[1..];
                continue;
            }
            if let Some(end) = after_tick.find('`') {
                let content = &after_tick[..end];
                // Only single-token identifiers (no spaces).
                let content = content.trim();
                if !content.is_empty() && !content.contains(' ') {
                    results.push(content);
                }
                rest = &after_tick[end + 1..];
            } else {
                break;
            }
        }
    }
    results
}

/// Returns true if the string looks like it could be a code symbol name
/// (starts with a letter or underscore, contains only alphanumerics and
/// underscores/colons — colons for `Type::method` patterns).
fn is_plausible_symbol_name(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == ':')
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Helper: create a minimal wiki page file with the given sources and body.
    fn write_wiki_page(dir: &Path, slug: &str, sources: &[&str], body: &str) {
        let sources_yaml = if sources.is_empty() {
            "sources: []".to_string()
        } else {
            let entries: Vec<String> = sources.iter().map(|s| format!("  - {s}")).collect();
            format!("sources:\n{}", entries.join("\n"))
        };
        let content = format!(
            "---\nslug: {slug}\ntype: module\nlast_updated_commit: abc\nconfidence: 0.8\nstatus: draft\n{sources_yaml}\n---\n\n{body}\n"
        );
        fs::write(dir.join(format!("{slug}.md")), content).expect("write wiki page");
    }

    #[test]
    fn source_deleted_is_detected() {
        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path().join(".wiki");
        fs::create_dir_all(&wiki).unwrap();

        // Wiki page references a source that does not exist.
        write_wiki_page(&wiki, "my-mod", &["src/gone.rs"], "# My Module\n\nSome docs.");

        let stale = detect_stale(&wiki, tmp.path(), None).unwrap();
        assert_eq!(stale.len(), 1, "expected 1 stale page, got {}", stale.len());
        assert_eq!(stale[0].page_slug, "my-mod");
        assert_eq!(stale[0].reasons.len(), 1);
        assert_eq!(
            stale[0].reasons[0],
            StaleReason::SourceDeleted {
                path: "src/gone.rs".to_string()
            }
        );
    }

    #[test]
    fn source_modified_after_page_is_detected() {
        use std::fs::FileTimes;

        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path().join(".wiki");
        let src_dir = tmp.path().join("src");
        fs::create_dir_all(&wiki).unwrap();
        fs::create_dir_all(&src_dir).unwrap();

        // Create source file and wiki page.
        let src_path = src_dir.join("lib.rs");
        fs::write(&src_path, "pub fn hello() {}").unwrap();
        write_wiki_page(&wiki, "my-mod", &["src/lib.rs"], "# My Module");

        // Set the wiki page mtime to the past (epoch + 1_000_000s) so
        // the source file (current mtime) is guaranteed to be newer.
        let wiki_page_path = wiki.join("my-mod.md");
        let past = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000);
        let times = FileTimes::new().set_modified(past);
        let f = fs::File::options()
            .write(true)
            .open(&wiki_page_path)
            .unwrap();
        f.set_times(times).unwrap();

        let stale = detect_stale(&wiki, tmp.path(), None).unwrap();
        assert_eq!(stale.len(), 1, "expected 1 stale page, got {}", stale.len());
        assert_eq!(stale[0].page_slug, "my-mod");
        assert!(
            stale[0]
                .reasons
                .iter()
                .any(|r| matches!(r, StaleReason::SourceModifiedAfterPage { .. })),
            "expected SourceModifiedAfterPage, got {:?}",
            stale[0].reasons
        );
    }

    #[test]
    fn no_staleness_when_sources_empty() {
        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path().join(".wiki");
        fs::create_dir_all(&wiki).unwrap();

        write_wiki_page(&wiki, "clean-mod", &[], "# Clean Module");

        let stale = detect_stale(&wiki, tmp.path(), None).unwrap();
        assert!(stale.is_empty(), "pages with no sources should not be stale");
    }

    #[test]
    fn no_staleness_when_source_older_than_page() {
        use std::fs::FileTimes;

        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path().join(".wiki");
        let src_dir = tmp.path().join("src");
        fs::create_dir_all(&wiki).unwrap();
        fs::create_dir_all(&src_dir).unwrap();

        // Create source file and set its mtime to the distant past.
        let src_path = src_dir.join("lib.rs");
        fs::write(&src_path, "pub fn hello() {}").unwrap();
        let past = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000);
        let times = FileTimes::new().set_modified(past);
        let f = fs::File::options().write(true).open(&src_path).unwrap();
        f.set_times(times).unwrap();

        // Create wiki page (gets current mtime, which is later).
        write_wiki_page(&wiki, "fresh-mod", &["src/lib.rs"], "# Fresh Module");

        let stale = detect_stale(&wiki, tmp.path(), None).unwrap();
        assert!(
            stale.is_empty(),
            "source older than page should not be flagged stale, got {:?}",
            stale
        );
    }

    #[test]
    fn extract_backtick_identifiers_works() {
        let text = "Use `hello_world` to greet. Also see `MyStruct`.\n\n```\n`ignored`\n```\n";
        let ids = extract_backtick_identifiers(text);
        assert_eq!(ids, vec!["hello_world", "MyStruct"]);
    }

    #[test]
    fn is_plausible_symbol_name_accepts_valid() {
        assert!(is_plausible_symbol_name("hello"));
        assert!(is_plausible_symbol_name("MyStruct"));
        assert!(is_plausible_symbol_name("_private"));
        assert!(is_plausible_symbol_name("Type::method"));
    }

    #[test]
    fn is_plausible_symbol_name_rejects_invalid() {
        assert!(!is_plausible_symbol_name("123"));
        assert!(!is_plausible_symbol_name("hello world"));
        assert!(!is_plausible_symbol_name(""));
        assert!(!is_plausible_symbol_name("-flag"));
    }
}
