# Wiki Schema

Pages use YAML frontmatter with these fields:

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `slug` | string | yes | Unique identifier (kebab-case) |
| `type` | enum | yes | module \| concept \| entity \| flow \| decision \| operation \| synthesis |
| `last_updated_commit` | string | yes | Git SHA when page was last updated |
| `confidence` | float | yes | 0.0-1.0 — how accurate the content is |
| `sources` | list | no | Source file paths this page documents |
| `backlinks` | list | no | Other page slugs this page references |
| `status` | enum | yes | draft \| reviewed \| verified |

## Page types

- **module** — a crate, package, or major source directory
- **concept** — a cross-cutting idea (auth flow, caching strategy, etc.)
- **entity** — a key data structure or domain object
- **flow** — an end-to-end workflow or pipeline
- **decision** — an architectural decision record (ADR)
- **operation** — a runbook or checklist (deploy, release, etc.)
- **synthesis** — a "why" doc (why this language, why this architecture)

## Wikilinks

Use `[[slug]]` to link between pages. Example: `[[runner-trait]]` links to `concepts/runner-trait.md`.

Backlinks are bidirectional — if page A links to page B with `[[B]]`, then B's frontmatter should list A in its `backlinks` array.

## Confidence scale

| Range | Meaning |
|-------|---------|
| 0.95+ | Content verified against source code |
| 0.85-0.94 | High confidence, inferred from structure |
| 0.70-0.84 | Moderate confidence, some assumptions |
| < 0.70 | Low confidence, needs human review |

## File layout

```
.wiki/
├── index.md          ← table of all pages
├── SCHEMA.md         ← this file
├── modules/          ← crate/package documentation
├── concepts/         ← cross-cutting ideas
├── entities/         ← key data structures
├── flows/            ← end-to-end workflows
├── decisions/        ← ADRs
├── operations/       ← runbooks
└── synthesis/        ← "why" docs
```
