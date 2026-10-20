---
slug: bootstrap-flow
type: flow
last_updated_commit: 19da4d6
confidence: 0.93
sources:
  - crates/coral-cli/src/commands/bootstrap/mod.rs
  - crates/coral-cli/src/commands/bootstrap/state.rs
  - crates/coral-cli/src/commands/bootstrap/estimate.rs
backlinks:
  - runner-trait
  - ingest-cycle
  - adr-multi-runner
  - cost-estimation
status: draft
---

# Bootstrap Flow

The bootstrap pipeline generates the initial wiki for a repository. It is the primary entry point for new Coral installations, executed as `coral bootstrap --apply`.

## Entry Point

```rust
// crates/coral-cli/src/commands/bootstrap/mod.rs

pub fn run(args: BootstrapArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    if args.from_symbols {
        return run_from_symbols(args, wiki_root);
    }
    let provider = super::runner_helper::resolve_provider(args.provider.as_deref())
        .map_err(|e| anyhow::anyhow!(e))?;
    let runner = super::runner_helper::make_runner(provider);
    run_with_runner_inner(args, wiki_root, runner.as_ref(), provider)
}
```

## Pipeline Steps

```
scan repo -> generate_plan (LLM) -> estimate cost -> confirm -> apply_pages (parallel) -> checkpoint -> write index
```

### Parallelism Constant

```rust
// crates/coral-cli/src/commands/bootstrap/mod.rs

/// v0.35.0 CP-1: maximum number of in-flight per-page LLM calls during
/// `coral bootstrap --apply`. Picked to fit comfortably under
/// Anthropic Sonnet's typical tier-1 cap (50 requests/minute)
const BOOTSTRAP_MAX_PARALLEL: usize = 4;
```

### Three-Phase Apply (apply_pages)

The core of bootstrap is a three-phase execution model that balances parallelism with sequential correctness:

```rust
// crates/coral-cli/src/commands/bootstrap/mod.rs (Phase B: parallel render)

type RenderResult = Result<(String, Option<coral_runner::TokenUsage>), RunnerError>;
let render_results: Vec<(usize, RenderResult)> = if to_render.is_empty() {
    Vec::new()
} else {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(BOOTSTRAP_MAX_PARALLEL)
        .thread_name(|i| format!("bootstrap-worker-{i}"))
        .build()
        .context("building bootstrap rayon thread pool")?;
    let indices = to_render.clone();
    pool.install(|| {
        indices
            .par_iter()
            .map(|&i| {
                let entry = &state.plan[i];
                let res = render_page_body(runner, entry, cwd);
                (i, res)
            })
            .collect()
    })
};
```

**Phase A (sequential):** Classify each plan entry into skip-completed (resume), unsupported action (mark Failed), inline-body (write immediately, zero cost), or needs-runner (queue for Phase B).

**Phase B (parallel, rayon):** A dedicated 4-thread pool calls `render_page_body()` for each queued entry. Each worker invokes [[adr-multi-runner]] via `Runner::run()` with a per-page prompt. Workers are pure -- they never touch shared state.

**Phase C (sequential):** Results are applied in plan order. For each rendered page: mid-flight `--max-cost` gate checks cumulative spend, SEC-06 injection check refuses pages containing prompt-injection markers, the page is built and written atomically, and the [[ingest-cycle]] index is updated.

### Mid-Flight Cost Gate

```rust
// crates/coral-cli/src/commands/bootstrap/mod.rs (Phase C)

if let Some(cap) = max_cost
    && state.cost_spent_usd + projected_page_cost > cap
{
    eprintln!(
        "Stopped at ${:.2} (cap ${cap:.2}). Run `coral bootstrap --resume` to continue.",
        state.cost_spent_usd
    );
    state.partial = true;
    state.save_atomic(root)?;
    partial = true;
    break;
}
```

## Resume Support

When `--resume` is passed, bootstrap loads the persisted checkpoint at `.wiki/.bootstrap-state.json`, skips the planner call entirely (reusing the saved plan verbatim), and re-tries every page that is NOT `Completed`:

```rust
// crates/coral-cli/src/commands/bootstrap/mod.rs

if args.resume {
    let state = BootstrapState::load(&root)?.ok_or_else(|| {
        anyhow::anyhow!(
            "--resume requires an existing checkpoint at {}; \
                 none found. Run `coral bootstrap --apply` first.",
            BootstrapState::path(&root).display()
        )
    })?;
    let _lock = BootstrapLock::acquire(&root)?;
    let resume_max_cost = args.max_cost.or(state.max_cost_usd);
    return apply_pages(&root, &cwd, runner, cost_provider, state, resume_max_cost);
}
```

## Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success -- all pages created |
| 1 | Failure -- runner error or all pages skipped |
| 2 | Partial -- `--max-cost` halted mid-flight; resume to continue |

## From-Symbols Path

An alternative `--from-symbols` mode bypasses the LLM entirely. It uses regex-based symbol extraction to discover structs, traits, and functions, groups them by module, and generates draft pages with `confidence: 0.4`. Useful for quick scaffolding without incurring API costs.
