---
slug: lint
type: module
last_updated_commit: 19da4d6
confidence: 0.95
sources:
- crates/coral-lint/src/lib.rs
backlinks:
- cli
- core
status: draft
---

# lint

Structural and semantic wiki validation. `coral-lint` runs parallel checks against the wiki page graph, producing a sorted `LintReport` with categorized issues.

## Responsibilities

- Run structural checks in parallel via rayon
- Detect broken wikilinks, orphan pages, low confidence, stale status
- Verify git commit existence and source file presence (context-aware)
- Enforce trust-by-curation (block `reviewed: false` distilled pages)
- Produce machine-readable and human-readable lint reports

## Public API

```rust
// from crates/coral-lint/src/lib.rs
pub use report::{LintCode, LintIssue, LintReport, LintSeverity};

/// Runs all structural checks in parallel. Defaults repo_root to ".".
pub fn run_structural(pages: &[Page]) -> LintReport {
    run_structural_with_root(pages, Path::new("."))
}

/// Runs all structural checks in parallel against `pages`, using
/// `repo_root` for context-aware checks (commit-in-git, source-exists).
pub fn run_structural_with_root(pages: &[Page], repo_root: &Path) -> LintReport {
    // Pre-computes outbound_links once to avoid redundant regex extraction
    // Fans out via rayon across pure, link, and context checks
}
```

## Check Categories

### Pure Checks (operate on `&[Page]` only)

```rust
// from crates/coral-lint/src/lib.rs
let pure_checks: Vec<StructuralCheck> = vec![
    structural::check_low_confidence,
    structural::check_high_confidence_without_sources,
    structural::check_stale_status,
    structural::check_unknown_extra_field,
    structural::check_unreviewed_distilled,
];
```

### Link Checks (receive pre-computed outbound links)

```rust
// from crates/coral-lint/src/lib.rs
let link_checks: Vec<StructuralCheckWithLinks> = vec![
    structural::check_broken_wikilinks,
    structural::check_orphan_pages,
    structural::check_archived_linked_from_head,
];
```

### Context Checks (need filesystem/git access)

```rust
// from crates/coral-lint/src/lib.rs
let context_checks: Vec<StructuralCheckWithRoot> = vec![
    structural::check_commit_in_git,
    structural::check_source_exists,
];
```

## Check Function Signatures

```rust
// from crates/coral-lint/src/lib.rs
type StructuralCheck = fn(&[Page]) -> Vec<LintIssue>;
type StructuralCheckWithLinks = fn(&[Page], &[Vec<String>]) -> Vec<LintIssue>;
type StructuralCheckWithRoot = fn(&[Page], &Path) -> Vec<LintIssue>;
```

## Parallel Execution

All checks fan out via `rayon::par_iter()`:

```rust
// from crates/coral-lint/src/lib.rs
let pure_issues: Vec<LintIssue> = pure_checks
    .par_iter()
    .flat_map_iter(|check| check(pages))
    .collect();
let link_issues: Vec<LintIssue> = link_checks
    .par_iter()
    .flat_map_iter(|check| check(pages, &precomputed_links))
    .collect();
let context_issues: Vec<LintIssue> = context_checks
    .par_iter()
    .flat_map_iter(|check| check(pages, repo_root))
    .collect();

let mut issues = pure_issues;
issues.extend(link_issues);
issues.extend(context_issues);
LintReport::from_issues(issues)
```

## Module Structure

| Module | Purpose |
|--------|---------|
| `structural` | All individual check functions |
| `semantic` | LLM-augmented validation (future) |
| `report` | `LintReport`, `LintIssue`, `LintCode`, `LintSeverity` types |

## Trust-by-Curation (v0.20)

The `check_unreviewed_distilled` check is critical for the `coral session distill` workflow. It blocks any page with `reviewed: false` frontmatter AND a populated `source.runner` field from being committed cleanly. This ensures LLM-distilled output is always human-reviewed before entering the wiki.

## Cross-Crate Relationships

- [[core]] provides `Page` type and `outbound_links()` for link analysis
- [[cli]] delegates from `commands::lint` and uses in `commands::guarantee`
