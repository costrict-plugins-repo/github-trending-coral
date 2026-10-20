---
slug: ingest-cycle
type: flow
last_updated_commit: 19da4d6
confidence: 0.93
sources:
  - crates/coral-cli/src/commands/ingest.rs
backlinks:
  - bootstrap-flow
  - runner-trait
  - adr-multi-runner
status: draft
---

# Ingest Cycle

Incremental ingestion keeps the wiki fresh as the codebase evolves. Rather than re-bootstrapping the entire wiki, `coral ingest --apply` identifies what changed since the last update and surgically creates, updates, or retires only the affected pages.

## Entry Point

```rust
// crates/coral-cli/src/commands/ingest.rs

pub fn run_with_runner(
    args: IngestArgs,
    wiki_root: Option<&Path>,
    runner: &dyn Runner,
) -> Result<ExitCode> {
    let root: PathBuf = wiki_root
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(".wiki"));
    // ...
    let from = match args.from {
        Some(f) => f,
        None => {
            let idx_path = root.join("index.md");
            let idx_content = read_index_md_capped(&idx_path)?;
            let idx = WikiIndex::parse(&idx_content)?;
            idx.last_commit
        }
    };
    let range = format!("{from}..{head}");
    let entries = match gitdiff::run(&cwd, &range) {
        Ok(entries) => entries,
        Err(e) => { /* fallback with warning */ Vec::new() }
    };
    // ...
}
```

## Safety Cap

The index is capped at 32 MiB to prevent OOM on corrupted files:

```rust
// crates/coral-cli/src/commands/ingest.rs

const MAX_INDEX_BYTES: u64 = 32 * 1024 * 1024;

fn read_index_md_capped(path: &Path) -> Result<String> {
    if let Ok(meta) = std::fs::metadata(path)
        && meta.len() > MAX_INDEX_BYTES
    {
        anyhow::bail!(
            ".wiki/index.md exceeds 32 MiB cap ({} bytes); refusing to read {}",
            meta.len(),
            path.display()
        );
    }
    std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))
}
```

## Apply Loop (Create / Update / Retire)

```rust
// crates/coral-cli/src/commands/ingest.rs

for entry in &plan.plan {
    match entry.action {
        Action::Create => {
            let page = match build_page(entry, &head, &root) {
                Ok(p) => p,
                Err(e) => { warnings.push(...); continue; }
            };
            page.write()?;
            upserts.push(IndexEntry { /* ... */ });
            created += 1;
        }
        Action::Update => {
            let path = match locate_page(&root, &entry.slug) {
                Some(p) => p,
                None => { warnings.push(...); continue; }
            };
            let mut page = Page::from_file(&path)?;
            page.bump_last_commit(head.clone());
            page.write()?;
            updated += 1;
        }
        Action::Retire => {
            let mut page = Page::from_file(&path)?;
            page.frontmatter.status = Status::Stale;
            page.write()?;
            retired += 1;
        }
    }
}
```

## Atomic Index Update Under Exclusive Lock

The read-modify-write of `.wiki/index.md` happens entirely within an exclusive file lock to prevent lost-update races when concurrent `coral ingest --apply` invocations interleave (v0.19.5 audit fix):

```rust
// crates/coral-cli/src/commands/ingest.rs

coral_core::atomic::with_exclusive_lock(&idx_path, || {
    if let Ok(meta) = std::fs::metadata(&idx_path)
        && meta.len() > MAX_INDEX_BYTES
    {
        return Err(/* cap error */);
    }
    let raw = std::fs::read_to_string(&idx_path).map_err(/* ... */)?;
    let mut index = WikiIndex::parse(&raw)?;
    for u in &upserts {
        index.upsert(u.clone());
    }
    index.bump_last_commit(head.clone());
    coral_core::atomic::atomic_write_string(&idx_path, &index.to_string()?)
}).context("writing .wiki/index.md")?;
```

## Log Append

```rust
// crates/coral-cli/src/commands/ingest.rs

let summary = format!("range {range}: {created} created, {updated} updated, {retired} retired");
WikiLog::append_atomic(&log_path, "ingest", &summary)?;
```

## PDF Docs Ingestion (--include-docs)

An optional `--include-docs` flag scans a docs directory for PDF files and ingests them as `Reference` pages with `confidence: 0.3`. Requires `pdftotext` (poppler-utils). Text is extracted and truncated to 10,000 characters. Slugs derive from filenames with a `ref-` prefix.

## Cost Comparison to Bootstrap

Ingest is significantly cheaper than [[bootstrap-flow]] because only one LLM call is made (the plan), update/retire actions never invoke the LLM (purely local mutations), and newly created pages often get inline bodies from the planner.
