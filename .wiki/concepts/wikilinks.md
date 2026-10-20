---
slug: wikilinks
type: concept
last_updated_commit: 19da4d6
confidence: 0.95
sources:
- crates/coral-core/src/wikilinks.rs
- crates/coral-core/src/lib.rs
backlinks:
- frontmatter
- search
status: draft
---

# Wikilinks

Wikilinks are Coral's primary cross-referencing mechanism. The syntax follows the Obsidian/wiki convention: \[[target]] creates a link to another page by slug. The extraction system parses these from Markdown page bodies and powers backlink computation, graph traversal, and link validation in `coral lint`.

## Syntax

The supported forms are:

- \[[slug]] -- simple link to a page
- \[[slug|display text]] -- link with a display alias (alias is stripped; target is `slug`)
- \[[slug#anchor]] -- link with a section anchor (anchor is stripped; target is `slug`)
- \[[slug#anchor|alias]] -- both anchor and alias

Escaped wikilinks use a leading backslash (`\[[escaped]]`) and are treated as literal text, not links.

## Extraction Algorithm

The public entry point is `extract(content: &str) -> Vec<String>`:

```rust
// crates/coral-core/src/wikilinks.rs

fn wikilink_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\[\[([^\]\n]+)\]\]").expect("valid wikilink regex"))
}

/// Extracts unique wikilinks from `content` in document order.
///
/// Wikilinks inside fenced code blocks are ignored.
/// Wikilinks with a leading backslash are ignored.
/// Empty targets are ignored.
/// Duplicates are removed but document order is preserved.
pub fn extract(content: &str) -> Vec<String> {
    let re = wikilink_re();
    let mut out: Vec<String> = Vec::new();
    let mut seen: AHashSet<String> = AHashSet::new();
    let mut inside_fence = false;
    let bytes = content.as_bytes();

    let mut line_start: usize = 0;
    for line in content.split_inclusive('\n') {
        let trimmed = line.trim_start();

        // Toggle fence on lines starting with ```
        if trimmed.starts_with("```") {
            inside_fence = !inside_fence;
            line_start += line.len();
            continue;
        }
        if inside_fence {
            line_start += line.len();
            continue;
        }

        for cap in re.captures_iter(line) {
            let Some(m) = cap.get(0) else { continue };
            let abs_start = line_start + m.start();
            // Escape check: backslash immediately before [[
            if abs_start > 0 && bytes[abs_start - 1] == b'\\' {
                continue;
            }
            // ... alias/anchor stripping, deduplication ...
        }
        line_start += line.len();
    }
    out
}
```

## Key Behaviors

1. **Fenced code block exclusion**: Lines between triple-backtick fences are skipped entirely, so code examples containing wikilink syntax do not produce false links.

2. **Backslash escaping**: A `\` immediately before `[[` suppresses extraction. This is how wiki pages can document wikilink syntax without creating broken links.

3. **Deduplication with order preservation**: Uses `AHashSet` for O(1) membership checks while appending to a `Vec` to maintain document order.

4. **Alias and anchor stripping**: The pipe `|` separates target from display text; `#` separates target from section anchor. Only the target portion is returned.

## Escaped Pipe Handling (Issue #27)

`\|` inside a link body is treated as a literal pipe character in the target slug (Obsidian semantics). Implementation uses a sentinel character substitution:

```rust
// crates/coral-core/src/wikilinks.rs

const PIPE_SENTINEL: char = '\u{1f}';
let escaped = target_raw.replace(r"\|", &PIPE_SENTINEL.to_string());
// Strip alias (after unescaped `|`) and anchor (after `#`)
let mut target: &str = &escaped;
if let Some(idx) = target.find('|') {
    target = &target[..idx];
}
if let Some(idx) = target.find('#') {
    target = &target[..idx];
}
// Restore escaped pipes back to literal `|`
let target = target.trim().replace(PIPE_SENTINEL, "|");
```

After pipe processing, any remaining backslash in the resulting slug causes rejection (keeps the slug allowlist strict).

## Public API

The extraction function is re-exported at crate root for cross-crate use:

```rust
// crates/coral-core/src/lib.rs

pub use crate::wikilinks::extract as wikilinks_extract;
```

Callers in `coral-cli` (consolidate, lint, bootstrap) and integration tests import via `coral_core::wikilinks_extract`.

## Performance

The regex is compiled once via `OnceLock<Regex>` and reused across all calls. The `AHashSet` provides faster hashing than the standard `HashMap` for the deduplication pass. For typical wiki pages (2-4 KB body, 3-8 wikilinks), extraction completes in single-digit microseconds.

## Integration Points

- **`coral lint`**: validates that every extracted wikilink target resolves to an existing page slug (dangling link detection).
- **`coral consolidate`**: uses extracted wikilinks to compute the backlink graph and detect orphan pages.
- **[[frontmatter]]**: the `backlinks` field is populated from the reverse wikilink graph.
- **[[search]]**: wikilink targets influence adaptive search routing (linked pages are considered related via backlinks).

## Edge Cases

| Input | Result |
|-------|--------|
| `\[[escaped]]` | Empty (escaped) |
| `\[[]]` | Empty (blank target) |
| `\[[   ]]` | Empty (whitespace-only) |
| Wikilink inside ` ``` ` fence | Skipped |
| `\[[a\|b]]` | Target: `a|b` |
| `\[[a|alias]]` | Target: `a` |
| `\[[slug#section]]` | Target: `slug` |
| `\[[a\b]]` | Rejected (stray backslash) |
| Multi-line `\[[...]]` | Cannot match (regex excludes `\n`) |
