---
slug: stats
type: module
last_updated_commit: 19da4d6
confidence: 0.95
sources:
- crates/coral-stats/src/lib.rs
backlinks:
- cli
- core
- ui
status: draft
---

# stats

Wiki health dashboard and metrics computation. `coral-stats` aggregates page-level data into a structured `StatsReport` with confidence distributions, orphan detection, and staleness tracking.

## Responsibilities

- Compute aggregated wiki health metrics from a `&[Page]` slice
- Render metrics as Markdown or JSON
- Provide JSON Schema for downstream tooling validation
- Track confidence distributions in 4-bucket histograms
- Identify orphan candidate pages (no inbound links, excluding system types)

## StatsReport

```rust
// from crates/coral-stats/src/lib.rs
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StatsReport {
    pub total_pages: usize,
    pub by_type: BTreeMap<String, usize>,
    pub by_status: BTreeMap<String, usize>,
    pub confidence_avg: f64,
    pub confidence_min: f64,
    pub confidence_max: f64,
    pub low_confidence_count: usize,
    pub critical_low_confidence_count: usize,
    pub stale_count: usize,
    pub archived_count: usize,
    pub total_outbound_links: usize,
    pub orphan_candidates: Vec<String>,
    pub pages_without_sources_count: usize,
    pub oldest_commit_age_pages: Vec<String>,
    pub pages_by_confidence_bucket: BTreeMap<String, usize>,
}
```

## Core API

```rust
// from crates/coral-stats/src/lib.rs
impl StatsReport {
    pub fn new(pages: &[Page]) -> Self;
    pub fn as_markdown(&self) -> String;
    pub fn as_json(&self) -> Result<String, serde_json::Error>;
    pub fn json_schema() -> String;
}
```

## Confidence Buckets

Pages are bucketed into 4 ranges (all keys always present in output for stable JSON shape):

| Bucket | Range | Threshold |
|--------|-------|-----------|
| `"0.0-0.3"` | [0.0, 0.3) | Critical |
| `"0.3-0.6"` | [0.3, 0.6) | Low |
| `"0.6-0.8"` | [0.6, 0.8) | Medium |
| `"0.8-1.0"` | [0.8, 1.0] | High (closed at 1.0) |

## Orphan Detection

```rust
// from crates/coral-stats/src/lib.rs
fn is_system_type(t: PageType) -> bool {
    matches!(
        t,
        PageType::Index | PageType::Log | PageType::Schema
        | PageType::Readme | PageType::Reference
    )
}
```

System page types (Index, Log, Schema, Readme, Reference) are excluded from orphan detection. A page is an orphan candidate if its slug never appears in any other page's outbound links. Self-links count as inbound (so self-referencing pages are not orphans).

## Oldest Commit Tracking

The report includes the 5 pages with the oldest `last_updated_commit` (sorted lexicographically by commit then slug). This is a heuristic for staleness without requiring `git log` lookups.

## Performance Contract

The stats computation over 500 pages with 2 links each must complete in under 50ms (tested with a dedicated benchmark test).

## Schema Currency

The crate maintains a committed `docs/schemas/stats.schema.json` file that must match the live `json_schema()` output. A unit test enforces this invariant so downstream tooling (jq pipelines, dashboards) can validate against the contract.

## Cross-Crate Relationships

- [[core]] provides `Page`, `PageType`, `Status`, `Confidence` types
- [[cli]] delegates from `commands::stats`
- [[ui]] exposes stats via `/api/v1/stats` endpoint
