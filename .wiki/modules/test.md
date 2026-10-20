---
slug: test
type: module
last_updated_commit: 19da4d6
confidence: 0.95
sources:
- crates/coral-test/src/lib.rs
backlinks:
- cli
- env
status: draft
---

# test

The functional testing layer. `coral-test` provides a pluggable trait family for smoke, contract, property, recorded, event, trace, and browser tests against a running environment.

## Responsibilities

- Define the `TestRunner` trait for pluggable test execution
- Implement runners: Healthcheck, UserDefined, Hurl, Contract, Property, Recorded, Event, Trace, Browser
- Provide test orchestration with filtering and parallelism control
- Generate JUnit XML output for CI integration
- Compute test coverage against OpenAPI specs
- Detect performance regressions against baselines
- Perform semantic diff on OpenAPI/protobuf schemas for breaking changes
- Auto-discover test cases from OpenAPI specs (no LLM)

## TestRunner Trait

```rust
// from crates/coral-test/src/lib.rs
pub trait TestRunner: Send + Sync {
    fn name(&self) -> &'static str;
    fn supports(&self, kind: TestKind) -> bool;
    fn run(&self, case: &TestCase, env: &EnvHandle) -> TestResult<TestReport>;

    fn discover(&self, _project_root: &std::path::Path) -> TestResult<Vec<TestCase>> {
        Ok(Vec::new())
    }

    fn parallelism_hint(&self) -> ParallelismHint {
        ParallelismHint::Isolated
    }

    fn snapshot_dir(&self) -> Option<PathBuf> { None }
    fn supports_record(&self) -> bool { false }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParallelismHint {
    Isolated,
    Sequential,
    PerService,
}
```

## RunOptions

```rust
// from crates/coral-test/src/lib.rs
#[derive(Debug, Clone, Default)]
pub struct RunOptions {
    pub services: Vec<String>,
    pub tags: Vec<String>,
    pub kinds: Vec<TestKind>,
    pub update_snapshots: bool,
    pub parallelism: Option<usize>,
    pub timeout: Option<Duration>,
}
```

## Module Exports

```rust
// from crates/coral-test/src/lib.rs
pub use browser_runner::BrowserRunner;
pub use contract_check::{
    ContractReport, Finding as ContractFinding, FindingKind as ContractFindingKind,
    Severity as ContractSeverity, check_project as check_contracts,
};
pub use contract_runner::ContractRunner;
pub use coverage::{CoverageReport, Endpoint as CoverageEndpoint, compute_coverage};
pub use discover::{DiscoveredCase, discover_openapi_in_project};
pub use emit_k6::{EmitOutput, SkipNote, SkipReason, emit_k6};
pub use error::{TestError, TestResult};
pub use event_runner::EventRunner;
pub use healthcheck_runner::HealthcheckRunner;
pub use history::{FlakeEntry, TestRecord, append_records, compute_flakes, read_history};
pub use hurl_runner::HurlRunner;
pub use mock::MockTestRunner;
pub use orchestrator::{TestFilters, run_test_suite_filtered};
pub use perf::{PerfBaseline, PerfRegression, PerfReport, detect_regressions};
pub use property_runner::{PropertyRunner, PropertyTestCaseSpec, cases_from_property_specs};
pub use recorded_runner::{KeployTestCase, RecordedRunner, discover_recorded};
pub use report::{Evidence, JunitOutput, TestReport, TestStatus};
pub use semantic_diff::{BreakingChange, DiffSeverity, SemanticDiffResult, diff_openapi, diff_protobuf};
pub use spec::{TestCase, TestKind, TestSource, TestSpec};
pub use trace_runner::TraceRunner;
pub use user_defined_runner::UserDefinedRunner;
```

## Submodules

| Module | Purpose |
|--------|---------|
| `healthcheck_runner` | Liveness probes against running services |
| `user_defined_runner` | YAML-defined test cases |
| `hurl_runner` | Hurl HTTP test files |
| `contract_runner` | Interface contract validation |
| `contract_check` | Cross-repo drift detection |
| `property_runner` | Property-based testing from JSON Schema |
| `recorded_runner` | Keploy-style recorded traffic replay |
| `event_runner` | Async event/message testing |
| `trace_runner` | Distributed trace validation |
| `browser_runner` | Browser/UI tests |
| `coverage` | API endpoint coverage computation |
| `discover` | Auto-discover tests from OpenAPI specs |
| `emit_k6` | Generate k6 load test scripts |
| `history` | Test history ledger (JSONL), flake detection |
| `orchestrator` | Fan-out execution with filtering |
| `perf` | Performance baseline/regression detection |
| `semantic_diff` | OpenAPI/protobuf breaking change detection |
| `report` | `TestReport`, `JunitOutput`, `TestStatus` |
| `spec` | `TestCase`, `TestKind`, `TestSpec` |
| `mock` | `MockTestRunner` for tests |
| `walk_tests` | Discover test files on disk |

## Semantic Diff

```rust
// from crates/coral-test/src/lib.rs
pub use semantic_diff::{
    BreakingChange, DiffSeverity, SemanticDiffResult,
    diff_openapi, diff_protobuf, diff_schema,
};
```

Detects breaking changes in API schemas without requiring a running environment.

## Design Notes

- Mirrors `coral-env::EnvBackend` and `coral-runner::Runner` patterns: `Send + Sync`, `thiserror`, `Mock*`
- Test execution requires an `EnvHandle` from [[env]] (the running environment)
- The orchestration layer uses `rayon` for parallel-safe runners (`ParallelismHint::Isolated`)
- k6 script generation enables load testing workflows without a custom runner

## Cross-Crate Relationships

- [[env]] provides `EnvHandle` required by `TestRunner::run`
- [[cli]] delegates via `commands::test`, `commands::test_discover`, `commands::guarantee`
