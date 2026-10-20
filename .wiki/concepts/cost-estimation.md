---
slug: cost-estimation
type: concept
last_updated_commit: 19da4d6
confidence: 0.95
sources:
- crates/coral-core/src/cost.rs
backlinks:
- runner-trait
status: draft
---

# Cost Estimation

The cost estimation module prevents surprise bills during `coral bootstrap` by projecting LLM spend before any inference calls run. It provides two pure, side-effect-free functions: a per-page token heuristic and a pricing lookup.

## CostEstimate Structure

```rust
// crates/coral-core/src/cost.rs

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CostEstimate {
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Mid-point USD estimate (provider price list x token counts).
    pub usd_estimate: f64,
    /// estimate x 1.25 in M1 (+/-25% margin).
    pub usd_upper_bound: f64,
    /// Pinned at 25 (percent) for M1.
    pub margin_of_error_pct: u8,
}

impl CostEstimate {
    pub fn zero() -> Self { ... }  // Identity for sums
}
```

The `usd_upper_bound` is the conservative ceiling that `coral bootstrap --max-cost` gates on (not the mid-point estimate).

## Provider Enum

```rust
// crates/coral-core/src/cost.rs

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Provider {
    Claude,  // Anthropic Sonnet 4.5
    Gemini,  // Google 2.0 Flash
    Http,    // Generic OpenAI-compatible (cost unknown)
    Local,   // llama.cpp / Ollama ($0)
}

impl Provider {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Claude => "claude-sonnet-4-5",
            Self::Gemini => "gemini-2.0-flash",
            Self::Http => "http",
            Self::Local => "local",
        }
    }
}
```

## Token Estimation Heuristic

```rust
// crates/coral-core/src/cost.rs

#[derive(Debug, Clone, Copy)]
pub struct PlanEntryEstimate {
    pub body_len_chars: usize,
    pub rationale_len_chars: usize,
}

pub fn estimate_tokens_for_entry(entry: &PlanEntryEstimate) -> (u64, u64) {
    const BASE_INPUT_TOKENS: u64 = 1500;  // system prompt + context framing
    const AVG_OUTPUT_TOKENS: u64 = 800;   // typical page generation length
    let extra_input = ((entry.body_len_chars + entry.rationale_len_chars) as u64).div_ceil(4);
    let input = BASE_INPUT_TOKENS.saturating_add(extra_input);
    (input, AVG_OUTPUT_TOKENS)
}
```

- **Base input (1500 tokens)**: covers the bootstrap prompt template (~6 KB)
- **Rationale/body chars / 4**: rough char-to-token conversion
- **Output (800 tokens)**: calibrated from v0.33.0 dogfood runs (average page ~3 KB)

## Pricing Model

```rust
// crates/coral-core/src/cost.rs

pub fn estimate_cost_from_tokens(
    input_tokens: u64,
    output_tokens: u64,
    provider: Provider,
) -> CostEstimate {
    let (input_rate_per_mtok, output_rate_per_mtok) = match provider {
        Provider::Claude => (3.0, 15.0),    // $3/MTok in, $15/MTok out
        Provider::Gemini => (0.10, 0.40),   // $0.10/MTok in, $0.40/MTok out
        Provider::Http | Provider::Local => (0.0, 0.0),
    };
    let usd_estimate = (input_tokens as f64) * input_rate_per_mtok / 1_000_000.0
        + (output_tokens as f64) * output_rate_per_mtok / 1_000_000.0;
    CostEstimate {
        input_tokens,
        output_tokens,
        usd_estimate,
        usd_upper_bound: usd_estimate * 1.25,
        margin_of_error_pct: 25,
    }
}
```

Pricing (pinned at v0.34.0, May 2026):

| Provider | Input $/MTok | Output $/MTok | Notes |
|----------|-------------|--------------|-------|
| Claude (Sonnet 4.5) | $3.00 | $15.00 | Public list price |
| Gemini (2.0 Flash) | $0.10 | $0.40 | ~30x cheaper |
| Http | $0.00 | $0.00 | User-configured; cost unknown |
| Local | $0.00 | $0.00 | Runs on user hardware |

## --max-cost Abort Mechanism

During `coral bootstrap --apply --max-cost=$N`:

1. **Pre-flight**: runs `plan_cost_estimate` and aborts if `usd_upper_bound > N` before any LLM call.
2. **Mid-flight**: after each page generation, accumulates real `TokenUsage` from [[runner-trait]] (when available) or falls back to the heuristic estimate. If cumulative cost would exceed the budget, `RunnerError::BudgetExceeded` is returned and the bootstrap halts gracefully with a partial result.

## Design Decisions

- **Hardcoded pricing** (no live API ping) for offline/air-gapped environments. A price drop is a one-line change.
- **No prompt-caching model in M1**: the conservative no-caching upper bound is reported. The skill message tells users real cost can be 30-50% lower. M2 will recalibrate once 30+ real runs are collected.
- **Local/Http at $0**: useful for "you'd use ~X tokens" diagnostics without misleading cost projections.
- **Upper bound = estimate x 1.25**: the +/-25% margin is the conservative spread until calibration data is available.
- **`CostEstimate::zero()`**: identity element for plan rollups when the plan is empty.

## Real Token Usage (v0.34.0+)

When the [[runner-trait]] returns `Some(TokenUsage)` (e.g. `ClaudeRunner` parsing `--output-format=json`), mid-flight cost tracking uses real numbers including `cache_read_tokens` (0.1x rate) and `cache_write_tokens` (1.25x rate). Runners that cannot extract structured usage (e.g. `LocalRunner`) return `None`, and the system falls back to heuristic estimates with a one-time warning.

## See Also

- [[runner-trait]] -- provides `TokenUsage` for real cost tracking
