---
slug: env
type: module
last_updated_commit: 19da4d6
confidence: 0.95
sources:
- crates/coral-env/src/lib.rs
backlinks:
- cli
- test
status: draft
---

# env

The environment orchestration layer. `coral-env` provides a pluggable backend trait (`EnvBackend`) for managing multi-service dev environments described in `coral.toml`, with Docker Compose as the MVP backend.

## Responsibilities

- Define the `EnvBackend` trait for environment lifecycle (up/down/status/logs/exec)
- Implement `ComposeBackend` (subprocess `docker compose` / `docker-compose` / `podman compose`)
- Provide `MockBackend` for deterministic testing
- Parse environment specifications from `coral.toml`
- Support healthchecks, chaos engineering, monitors, and devcontainer generation
- Model service states, published ports, and log streams

## EnvBackend Trait

```rust
// from crates/coral-env/src/lib.rs
pub trait EnvBackend: Send + Sync {
    fn name(&self) -> &'static str;
    fn up(&self, plan: &EnvPlan, opts: &UpOptions) -> EnvResult<EnvHandle>;
    fn down(&self, plan: &EnvPlan, opts: &DownOptions) -> EnvResult<()>;
    fn status(&self, plan: &EnvPlan) -> EnvResult<EnvStatus>;
    fn logs(&self, plan: &EnvPlan, service: &str, opts: &LogsOptions) -> EnvResult<Vec<LogLine>>;
    fn exec(
        &self,
        plan: &EnvPlan,
        service: &str,
        cmd: &[String],
        opts: &ExecOptions,
    ) -> EnvResult<ExecOutput>;

    fn capabilities(&self) -> EnvCapabilities {
        EnvCapabilities::default()
    }
}
```

## Options Types

```rust
// from crates/coral-env/src/lib.rs
#[derive(Debug, Clone, Default)]
pub struct UpOptions {
    pub services: Vec<String>,
    pub detach: bool,
    pub build: bool,
    pub watch: bool,
}

#[derive(Debug, Clone, Default)]
pub struct DownOptions {
    pub volumes: bool,
}

#[derive(Debug, Clone, Default)]
pub struct LogsOptions {
    pub follow: bool,
    pub tail: Option<usize>,
}

#[derive(Debug, Clone, Default)]
pub struct ExecOptions {
    pub working_dir: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct ExecOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}
```

## Capabilities

```rust
// from crates/coral-env/src/lib.rs
#[derive(Debug, Clone, Copy, Default)]
pub struct EnvCapabilities {
    pub watch: bool,
    pub exec: bool,
    pub logs_follow: bool,
    pub port_forward_explicit: bool,
    pub emit_devcontainer: bool,
}
```

## Module Exports

```rust
// from crates/coral-env/src/lib.rs
pub use devcontainer::{DevcontainerArtifact, DevcontainerOpts, render_devcontainer};
pub use error::{EnvError, EnvResult};
pub use mock::MockBackend;
pub use plan::{
    EnvHandle, EnvPlan, EnvStatus, HealthState, LogLine, LogStream,
    PublishedPort, ServiceSpecPlan, ServiceState, ServiceStatus,
};
pub use spec::{
    ChaosBackend, ChaosConfig, ChaosScenario, EnvironmentSpec, Healthcheck,
    HealthcheckKind, HealthcheckTiming, MonitorSpec, OnFailure,
    PropertyTestSpec, RealService, RecordedConfig, ServiceKind, ToxicKind,
};
```

## Submodules

| Module | Purpose |
|--------|---------|
| `compose` | Docker/Podman Compose backend implementation |
| `compose_yaml` | Compose YAML generation from spec |
| `devcontainer` | VS Code devcontainer.json generation |
| `error` | `EnvError` / `EnvResult` types |
| `healthcheck` | Liveness probe execution |
| `import` | Import existing compose files into coral.toml |
| `mock` | `MockBackend` for tests |
| `plan` | `EnvPlan`, `EnvHandle`, `EnvStatus` types |
| `spec` | `EnvironmentSpec`, `Healthcheck`, `ChaosConfig`, etc. |

## Design Notes

- Mirrors `coral_runner::Runner` deliberately: same `Send + Sync`, `thiserror` errors, `Mock*` pattern
- Future backends (`KindBackend`, `TiltBackend`, `K3dBackend`) live behind Cargo features
- Default dep tree stays slim (only Compose backend compiled by default)
- Chaos engineering (Toxiproxy sidecar) lives in the spec types; CLI drives the injection

## Cross-Crate Relationships

- [[cli]] delegates via `commands::up`, `commands::down`, `commands::env`, `commands::verify`
- [[test]] uses `EnvHandle` for test execution context
