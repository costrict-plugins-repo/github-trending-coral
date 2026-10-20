---
slug: cli
type: module
last_updated_commit: 19da4d6
confidence: 0.95
sources:
- crates/coral-cli/src/main.rs
- crates/coral-cli/src/commands/mod.rs
- crates/coral-cli/src/commands/runner_helper.rs
backlinks:
- core
- runner
- mcp
- lint
- stats
- session
- env
- test
- ui
status: draft
---

# cli

The user-facing command-line interface. `coral-cli` is the `coral` binary that dispatches to 40+ subcommands for wiki management, environment orchestration, testing, and agent integration.

## Responsibilities

- Parse CLI arguments via `clap` (derive-based)
- Dispatch to command modules (init, bootstrap, query, lint, ingest, etc.)
- Construct the appropriate `Runner` implementation based on `--provider` flag
- Manage exit codes with a documented contract (0=clean, 1=findings, 2=usage, 3=internal)
- Use `mimalloc` as global allocator for performance

## CLI Structure

```rust
// from crates/coral-cli/src/main.rs
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[derive(Parser, Debug)]
#[command(
    name = "coral",
    version,
    about = "Karpathy-style LLM Wiki maintainer for Git repos."
)]
struct Cli {
    #[arg(long, global = true)]
    wiki_root: Option<PathBuf>,
    #[arg(long, global = true)]
    quiet: bool,
    #[arg(long, global = true)]
    verbose: bool,
    #[command(subcommand)]
    command: Cmd,
}
```

## Subcommands (Selected)

```rust
// from crates/coral-cli/src/main.rs
#[derive(Subcommand, Debug)]
enum Cmd {
    Init(commands::init::InitArgs),
    Bootstrap(commands::bootstrap::BootstrapArgs),
    Ingest(commands::ingest::IngestArgs),
    Query(commands::query::QueryArgs),
    Lint(commands::lint::LintArgs),
    Consolidate(commands::consolidate::ConsolidateArgs),
    Stats(commands::stats::StatsArgs),
    Search(commands::search::SearchArgs),
    Export(commands::export::ExportArgs),
    Mcp(commands::mcp::McpArgs),
    Session(commands::session::SessionArgs),
    Contract(commands::contract::ContractArgs),
    Project(commands::project::ProjectArgs),
    Up(commands::up::UpArgs),
    Down(commands::down::DownArgs),
    Test(commands::test::TestArgs),
    Guarantee(commands::guarantee::GuaranteeArgs),
    #[cfg(feature = "ui")]
    Ui(commands::ui::UiArgs),
    Doctor(commands::doctor::DoctorArgs),
    SelfCheck(commands::self_check::SelfCheckArgs),
    SelfUpgrade(commands::self_upgrade::SelfUpgradeArgs),
    // ... 40+ total subcommands
}
```

## Provider Resolution (runner_helper)

```rust
// from crates/coral-cli/src/commands/runner_helper.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProviderName {
    #[default]
    Claude,
    Gemini,
    Local,
    Http,
    HostSampling,
    AnthropicApi,
}

impl std::str::FromStr for ProviderName {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "claude" => Ok(Self::Claude),
            "gemini" => Ok(Self::Gemini),
            "local" | "llama" | "llama.cpp" => Ok(Self::Local),
            "http" => Ok(Self::Http),
            "host-sampling" | "host_sampling" => Ok(Self::HostSampling),
            "anthropic-api" | "anthropic_api" | "anthropic" => Ok(Self::AnthropicApi),
            other => Err(format!("unknown provider: {other}")),
        }
    }
}
```

The `make_runner()` function resolves credentials from `[provider.*]` in `.coral/config.toml`, environment variables, and CLI flags to construct the appropriate [[runner]] implementation.

## Exit Code Contract

```rust
// from crates/coral-cli/src/commands/mod.rs
pub mod exit_codes {
    pub const CLEAN: u8 = 0;
    pub const FINDINGS: u8 = 1;
    pub const USAGE: u8 = 2;
    pub const INTERNAL: u8 = 3;

    pub fn map_internal_err(result: anyhow::Result<ExitCode>) -> (ExitCode, Option<anyhow::Error>) {
        match result {
            Ok(code) => (code, None),
            Err(e) => (ExitCode::from(INTERNAL), Some(e)),
        }
    }
}
```

## Command Modules

The `commands/` directory contains one module per subcommand:

| Module | Purpose |
|--------|---------|
| `init` | Create `.wiki/` structure |
| `bootstrap` | Full wiki compilation from HEAD (LLM) |
| `ingest` | Incremental update from last commit (LLM) |
| `query` | Natural language wiki query (LLM) |
| `lint` | Structural + semantic wiki checks |
| `search` | TF-IDF full-text search |
| `stats` | Health dashboard |
| `mcp` | Start the MCP server |
| `session` | Agent transcript capture/distill |
| `project` | Multi-repo management |
| `up`/`down`/`env` | Dev environment lifecycle |
| `test` | Functional test orchestration |
| `contract` | Interface drift detection |
| `doctor` | Diagnostic/wizard flow |
| `self_upgrade` | In-place binary upgrade |
| `self_check` | SessionStart hook probe (JSON output) |

## Key Environment Variables

| Variable | Purpose |
|----------|---------|
| `CORAL_HTTP_ENDPOINT` | Chat-completions URL for `--provider http` |
| `CORAL_HTTP_API_KEY` | Bearer token for HTTP provider |
| `ANTHROPIC_API_KEY` | Forwarded to `claude` subprocess |
| `GEMINI_API_KEY` | Forwarded to `gemini` subprocess |
| `CORAL_QUIET` | Suppress progress output |
| `CORAL_VERBOSE` | Force verbose output |

## Cross-Crate Relationships

- [[core]] provides page model, config, frontmatter, walk, search
- [[runner]] provides LLM invocation (constructed via `runner_helper`)
- [[lint]] provides structural checks (delegated from `commands::lint`)
- [[stats]] provides health metrics (delegated from `commands::stats`)
- [[mcp]] provides MCP server (delegated from `commands::mcp`)
- [[session]] provides transcript capture/distill
- [[env]] provides environment backend (compose)
- [[test]] provides test orchestration
- [[ui]] provides WebUI server (feature-gated)
