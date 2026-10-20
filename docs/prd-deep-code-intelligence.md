# PRD: Deep Code Intelligence & Implementation Assistance

**Version**: 1.0  
**Author**: Agustin Bajo  
**Date**: 2026-05-16  
**Status**: Draft  
**Target**: Coral v0.42–v0.44 (3 milestones)

---

## 1. Problem Statement

Coral's wiki is excellent for **conceptual understanding** (architecture, decisions, flows), but falls short in two critical areas:

1. **Shallow code knowledge** — Wiki pages summarize intent, but Coral doesn't deeply understand the actual code. It can't answer "what calls this function?", "what types flow through here?", or "what would break if I change this interface?"

2. **No implementation assistance** — Coral is read-only. It helps you *understand* code but doesn't help you *write* it. Users still need to context-switch to grep, LSP, or manual exploration when implementing.

### Current Limitations

| Capability | Current State | Gap |
|-----------|--------------|-----|
| Symbol extraction | Regex-based (30% coverage) | No type info, no call graph |
| Search | TF-IDF over wiki prose | No semantic search, no code search |
| Cross-file navigation | Manual `backlinks` in YAML | No automatic dependency resolution |
| Implementation help | None | Can't plan edits, can't verify |
| Context for LLMs | Static wiki pages | Not optimized for token budgets |
| Code freshness | `sources` field in frontmatter | No incremental AST diff |

### Competitive Landscape

| Tool | Code Understanding | Implementation | Local/Private |
|------|-------------------|----------------|---------------|
| Cursor | Embeddings + AST (Turbopuffer) | Composer (multi-file) | Cloud-indexed |
| Aider | PageRank repo map | Diff-based edits | Fully local |
| Sourcegraph Cody | SCIP (compiler-accurate) | Chat-based | Self-hosted option |
| Copilot Agent | Workspace tools | Issue-to-PR | Cloud |
| **Coral (today)** | Wiki summaries | None | Fully local |
| **Coral (target)** | AST + embeddings + graph | Plan-verify loop | Fully local |

---

## 2. Vision

Transform Coral from a **documentation layer** into a **code intelligence platform** that:

1. **Knows your code structurally** — AST-level understanding of every symbol, call, and type relationship
2. **Finds anything instantly** — Hybrid search (semantic + structural + keyword) returns the right code in <200ms
3. **Helps you implement** — Plan changes, generate code grounded in existing patterns, and verify correctness
4. **Stays local** — All indexing on your machine, all embeddings local or self-hosted

> *"Ask Coral what to change, and it shows you exactly where and how — then helps you do it."*

---

## 3. Architecture Overview

```
┌─────────────────────────────────────────────────────────────┐
│                        Coral CLI / MCP                        │
├─────────────────────────────────────────────────────────────┤
│                                                              │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌───────────┐  │
│  │   Wiki   │  │  Code    │  │  Search  │  │  Implement│  │
│  │  Layer   │  │  Graph   │  │  Engine  │  │  Engine   │  │
│  │ (exists) │  │  (NEW)   │  │  (NEW)   │  │  (NEW)    │  │
│  └────┬─────┘  └────┬─────┘  └────┬─────┘  └─────┬─────┘  │
│       │              │              │               │        │
│  ┌────┴──────────────┴──────────────┴───────────────┴────┐  │
│  │              Unified Index (.coral/index/)              │  │
│  │  ┌─────────┐ ┌──────────┐ ┌─────────┐ ┌───────────┐  │  │
│  │  │AST Store│ │Embeddings│ │Call Graph│ │ Type Index │  │  │
│  │  │(sled DB)│ │ (vecdb)  │ │(petgraph)│ │  (sled)   │  │  │
│  │  └─────────┘ └──────────┘ └─────────┘ └───────────┘  │  │
│  └────────────────────────────────────────────────────────┘  │
│                              │                               │
│  ┌───────────────────────────┴───────────────────────────┐  │
│  │              Tree-sitter Parser Layer                   │  │
│  │  Rust · TypeScript · Python · Go · Java · C/C++        │  │
│  └────────────────────────────────────────────────────────┘  │
│                                                              │
└─────────────────────────────────────────────────────────────┘
```

---

## 4. Milestone 1: Code Graph (v0.42)

**Goal**: Coral knows every symbol in your codebase and their relationships.

### 4.1 Tree-sitter AST Indexing

Replace the regex-based symbol extraction with full tree-sitter parsing.

**What we index per file:**
```rust
struct FileIndex {
    path: PathBuf,
    language: Language,
    hash: Blake3Hash,          // content-addressed for incremental
    symbols: Vec<Symbol>,
    imports: Vec<Import>,
    exports: Vec<Export>,
}

struct Symbol {
    name: String,
    kind: SymbolKind,          // Function, Struct, Trait, Class, Interface, Enum, Const
    span: Span,                // line:col start..end
    signature: Option<String>, // full type signature
    visibility: Visibility,    // Public, Private, Crate, Module
    doc_comment: Option<String>,
    children: Vec<Symbol>,     // methods, fields, variants
}
```

**Languages (priority order):**
1. Rust, TypeScript/JavaScript, Python (P0 — cover 90% of users)
2. Go, Java, C/C++ (P1)
3. Ruby, Elixir, Swift, Kotlin (P2)

**Tree-sitter queries** — use `.scm` query files per language:
```scheme
;; Rust: extract function definitions
(function_item
  name: (identifier) @name
  parameters: (parameters) @params
  return_type: (type_identifier)? @return_type
  body: (block) @body) @definition.function
```

### 4.2 Call Graph & Reference Graph

Build a directed graph where:
- **Nodes** = symbols (functions, types, modules)
- **Edges** = relationships (calls, imports, implements, extends)

```rust
enum EdgeKind {
    Calls,           // function A calls function B
    Imports,         // module A imports from module B
    Implements,      // type A implements trait/interface B
    Extends,         // class A extends class B
    References,      // symbol A references symbol B (field access, type annotation)
    Contains,        // module A contains symbol B
    DependsOn,       // file A depends on file B (via imports)
}
```

**Storage**: `petgraph` in-memory + serialized to `.coral/index/graph.bincode`

**Commands:**
```bash
coral graph show <symbol>          # show call graph for a symbol
coral graph callers <symbol>       # who calls this?
coral graph callees <symbol>       # what does this call?
coral graph deps <file>            # file dependency tree
coral graph affected <file>        # what breaks if I change this file?
coral graph hot                    # most-referenced symbols (PageRank)
```

### 4.3 Incremental Indexing

Use content-addressed hashing (Blake3) per file:
- On `coral index` (or automatic on any `coral` command): hash every source file
- Only re-parse files whose hash changed since last index
- Merkle tree over the file list for O(1) "is anything stale?" check

**Performance target**: <2 seconds for a 50K LOC repo on M1 MacBook.

### 4.4 MCP Tools Exposed

```json
{
  "tools": [
    {"name": "symbol_lookup", "description": "Find a symbol by name, return its definition + signature + location"},
    {"name": "callers", "description": "List all callers of a symbol"},
    {"name": "callees", "description": "List all symbols called by a function"},
    {"name": "references", "description": "Find all references to a symbol"},
    {"name": "affected_by", "description": "List files/symbols affected by a change to a file"},
    {"name": "file_symbols", "description": "List all symbols defined in a file with signatures"}
  ]
}
```

### 4.5 Wiki Integration

When wiki pages reference `sources:`, Coral can now:
- Auto-populate `symbols:` frontmatter field with the actual symbols defined in those files
- Auto-detect when a source file's public API changed → flag wiki page as potentially stale
- Generate "API surface" sections in wiki pages automatically

---

## 5. Milestone 2: Smart Search (v0.43)

**Goal**: Find any code, concept, or pattern in <200ms with natural language.

### 5.1 Local Embeddings

**Model**: `voyage-code-3` (best code retrieval) or local alternative:
- **Cloud option**: Voyage API ($0.06/1M tokens, 1024 dims, 32K context)
- **Local option**: `nomic-embed-code-v1.5` (768 dims, runs on CPU via ONNX)
- **Config**: user chooses in `.coral/config.toml`:
  ```toml
  [search.embeddings]
  provider = "local"          # or "voyage", "openai"
  model = "nomic-embed-code"
  dimensions = 768
  ```

### 5.2 What Gets Embedded

Two indices with different chunk strategies:

| Index | Content | Chunk Size | Strategy |
|-------|---------|------------|----------|
| **Wiki** | `.wiki/*.md` pages | Whole page (most <2K tokens) | One embedding per page |
| **Code** | Source files | AST-aware (function/class boundaries) | One embedding per symbol + one per file summary |

**AST-aware chunking** (pseudocode):
```
for each file:
  parse with tree-sitter
  for each top-level symbol (function, class, struct, impl):
    chunk = signature + doc_comment + body (truncated to 512 tokens)
    embed(chunk) → store with metadata {file, symbol, span}
  
  file_summary = imports + exported symbols (names + signatures)
  embed(file_summary) → store with metadata {file, type: "file_overview"}
```

### 5.3 Hybrid Search Engine

Three retrieval paths, fused with Reciprocal Rank Fusion (RRF, k=60):

```
Query: "how does authentication middleware validate tokens?"

Path 1: Semantic (embeddings)
  → auth_middleware.rs::validate_token() [0.92]
  → jwt.rs::decode_jwt() [0.87]
  → .wiki/auth-middleware.md [0.85]

Path 2: Structural (code graph)
  → symbols named *auth*, *token*, *validate* [0.80]
  → callers/callees of known auth functions [0.75]

Path 3: Keyword (BM25 over code + wiki)
  → files containing "authenticate", "middleware", "token" [0.70]

Fused (RRF k=60):
  1. auth_middleware.rs::validate_token()    [RRF: 0.048]
  2. .wiki/auth-middleware.md                [RRF: 0.045]
  3. jwt.rs::decode_jwt()                    [RRF: 0.042]
```

### 5.4 Commands

```bash
coral find "authentication flow"         # hybrid search (all 3 paths)
coral find --code "parse JWT token"      # code-only search
coral find --wiki "how auth works"       # wiki-only search
coral find --type function "validate*"   # structural search by symbol kind
coral find --callers "validate_token"    # graph-based search
```

### 5.5 MCP Integration

The existing `coral query` and `coral search` commands get upgraded silently:
- `coral query` now retrieves from embeddings + code graph (not just wiki TF-IDF)
- `coral search` becomes hybrid (was: TF-IDF only)
- New MCP tool: `semantic_search` for agents

### 5.6 Context Builder Enhancement

The existing `coral context-build` gets upgraded:
```bash
coral context-build "implement rate limiting for the API"
```

Output: a ranked, de-duped context window containing:
1. Relevant wiki pages (conceptual understanding)
2. Relevant source code (actual implementations)
3. Related types and interfaces (structural context)
4. Test examples (patterns to follow)

All within a configurable token budget (default: 8K tokens).

---

## 6. Milestone 3: Implementation Engine (v0.44)

**Goal**: Coral helps you write code, not just understand it.

### 6.1 `coral implement` Command

```bash
coral implement "add rate limiting to the API routes"
```

**Flow:**

```
┌─────────────────────────────────────────────────────────────┐
│ 1. UNDERSTAND                                                │
│    coral find "rate limiting" + "API routes"                 │
│    → Retrieve relevant code, wiki pages, patterns            │
│                                                              │
│ 2. PLAN                                                      │
│    LLM generates a structured plan:                          │
│    - Which files to modify                                   │
│    - What symbols to add/change                              │
│    - Expected behavior (as test assertions)                  │
│    - Dependencies needed                                     │
│                                                              │
│ 3. VERIFY PLAN                                               │
│    coral graph affected <files-in-plan>                      │
│    → Show user what else might break                         │
│    → Ask for confirmation                                    │
│                                                              │
│ 4. GENERATE                                                  │
│    Per-file code generation grounded in:                     │
│    - Existing code patterns (from embeddings)                │
│    - Type signatures (from code graph)                       │
│    - Wiki conventions (from wiki pages)                      │
│                                                              │
│ 5. VALIDATE                                                  │
│    - coral lint (wiki consistency)                           │
│    - compile check (if applicable)                           │
│    - test run (if tests exist)                               │
│    - coral graph affected (verify no unintended breakage)    │
│                                                              │
│ 6. REPORT                                                    │
│    Show diff, test results, affected graph                   │
└─────────────────────────────────────────────────────────────┘
```

### 6.2 Implementation Plan Format

```yaml
# .coral/plans/rate-limiting.yaml (auto-generated, human-reviewable)
plan:
  title: "Add rate limiting to API routes"
  confidence: 0.85
  grounded_in:
    wiki: [api-routes, auth-middleware, express-app]
    code: [src/routes/tasks.ts, src/middleware/auth.ts]
    patterns: [src/middleware/cors.ts]  # existing middleware pattern

  steps:
    - file: src/middleware/rate-limit.ts
      action: create
      description: "New middleware using sliding window algorithm"
      depends_on: [express, redis]  # new dependencies
      
    - file: src/routes/tasks.ts
      action: modify
      symbols_affected: [router]
      description: "Apply rate limiter before route handlers"
      
    - file: src/routes/users.ts
      action: modify
      symbols_affected: [router]
      description: "Apply stricter limit on auth endpoints"

    - file: tests/rate-limit.test.ts
      action: create
      description: "Integration tests for rate limiting"
      assertions:
        - "returns 429 after 100 requests in 1 minute"
        - "resets counter after window expires"
        - "applies per-IP limiting"

  impact:
    files_modified: 3
    files_created: 2
    new_dependencies: [redis]
    symbols_added: [rateLimiter, RateLimitConfig, createRateLimiter]
    symbols_modified: [router (tasks), router (users)]
    
  risks:
    - "Redis dependency adds operational complexity"
    - "Per-IP limiting may not work behind load balancer (consider X-Forwarded-For)"
```

### 6.3 Pattern Learning

Coral observes patterns in the existing codebase and enforces them:

```bash
coral patterns                   # list detected patterns
coral patterns show middleware   # show the middleware pattern
```

Example output:
```
Detected patterns:

  middleware (5 instances):
    Location: src/middleware/*.ts
    Shape: export function <name>(req, res, next) { ... next() }
    Tests: tests/middleware/<name>.test.ts
    Registration: app.use(<name>) in src/app.ts
    
  routes (4 instances):
    Location: src/routes/*.ts
    Shape: const router = Router(); ... export default router
    Tests: tests/routes/<name>.test.ts
    Registration: app.use('/<plural>', <name>Router) in src/app.ts
```

When generating new code, Coral follows these patterns automatically.

### 6.4 Test-Driven Implementation

When `coral implement` creates new code:
1. **Generate tests FIRST** (from the plan's `assertions`)
2. **Generate implementation** (constrained to pass the tests)
3. **Run tests** to verify
4. **Iterate** if tests fail (up to 3 attempts)

```bash
coral implement --tdd "add rate limiting"
# Generates tests first, shows them, then implements to pass them
```

### 6.5 `coral sketch` — Lightweight Alternative

For smaller changes that don't need the full plan-verify loop:

```bash
coral sketch "add a logger.warn call when rate limit is exceeded"
# → Shows a unified diff preview (no file writes)
# → User can accept with `coral sketch --apply`
```

### 6.6 MCP Tools for Implementation

```json
{
  "tools": [
    {"name": "plan_change", "description": "Generate an implementation plan for a described change"},
    {"name": "affected_analysis", "description": "Show what would break if files are modified"},
    {"name": "generate_code", "description": "Generate code for a plan step, grounded in codebase patterns"},
    {"name": "detect_patterns", "description": "List coding patterns detected in the repo"},
    {"name": "validate_change", "description": "Run lint + compile + test on proposed changes"}
  ]
}
```

---

## 7. Storage & Performance

### 7.1 Index Storage (`.coral/index/`)

```
.coral/index/
├── ast/                    # tree-sitter parse results (sled DB)
│   ├── symbols.sled       # symbol → definition + metadata
│   └── files.sled         # file hash → parsed symbols
├── graph/
│   ├── calls.bincode      # call graph (petgraph serialized)
│   └── deps.bincode       # file dependency graph
├── embeddings/
│   ├── code.vecdb         # code chunk embeddings
│   ├── wiki.vecdb         # wiki page embeddings
│   └── metadata.json      # model, dimensions, last_indexed
├── search/
│   ├── bm25.tantivy       # keyword index (tantivy)
│   └── trigrams.fst       # for fuzzy symbol search
└── manifest.json          # index version, file count, last_full_index
```

### 7.2 Performance Targets

| Operation | Target | Strategy |
|-----------|--------|----------|
| Full index (50K LOC) | <10s | Parallel tree-sitter + rayon |
| Incremental index | <500ms | Content-addressed hashing |
| Symbol lookup | <5ms | sled B-tree |
| Hybrid search | <200ms | Parallel retrieval + RRF fusion |
| Graph query (callers) | <50ms | petgraph traversal |
| Embedding generation (local) | ~30 tok/s | ONNX runtime |
| Embedding generation (API) | ~1000 tok/s | Batched Voyage API calls |

### 7.3 Index Size Estimates

| Repo Size | AST Index | Embeddings | Graph | Total |
|-----------|-----------|------------|-------|-------|
| 10K LOC | ~2 MB | ~5 MB | ~500 KB | ~8 MB |
| 50K LOC | ~10 MB | ~25 MB | ~2 MB | ~37 MB |
| 200K LOC | ~40 MB | ~100 MB | ~8 MB | ~148 MB |

---

## 8. Configuration

```toml
# .coral/config.toml additions

[index]
# Languages to index (auto-detected if omitted)
languages = ["rust", "typescript", "python"]
# Directories to exclude from indexing
exclude = ["vendor/", "generated/", "*.pb.go"]
# Auto-index on every coral command (default: true)
auto_index = true

[search]
# Embedding provider
[search.embeddings]
provider = "local"              # "local" | "voyage" | "openai" | "none"
model = "nomic-embed-code"      # for local
# model = "voyage-code-3"       # for voyage
dimensions = 768

[search.hybrid]
# Weight for each retrieval path in RRF fusion
semantic_weight = 1.0
structural_weight = 0.8
keyword_weight = 0.6

[implement]
# Enable test-driven implementation by default
tdd = true
# Max retries when generated code fails tests
max_retries = 3
# Auto-run tests after generating code
auto_test = true
# Pattern detection sensitivity
pattern_min_instances = 3       # need 3+ instances to declare a "pattern"
```

---

## 9. CLI Surface

### New Commands

```bash
# Code Graph (M1)
coral index                     # Build/update the code index
coral index --full              # Force full re-index
coral graph show <symbol>       # Visualize call graph
coral graph callers <symbol>    # Who calls this?
coral graph callees <symbol>    # What does this call?
coral graph affected <file>     # Impact analysis
coral graph hot                 # Most-referenced symbols

# Smart Search (M2)
coral find "<query>"            # Hybrid search (semantic + structural + keyword)
coral find --code "<query>"     # Code-only
coral find --type fn "<name>"   # By symbol kind
coral embed                     # Build/update embeddings
coral embed --status            # Show embedding index health

# Implementation (M3)
coral implement "<description>" # Full plan-verify-generate loop
coral implement --plan-only     # Generate plan without executing
coral sketch "<description>"    # Quick diff preview
coral patterns                  # Show detected code patterns
coral validate                  # Lint + compile + test on current state
```

### Enhanced Existing Commands

```bash
coral query "..."               # Now uses embeddings + code graph (not just wiki)
coral search "..."              # Now hybrid (was: TF-IDF only)
coral context-build "..."       # Now includes ranked code chunks
coral status                    # Shows index health alongside wiki health
```

---

## 10. Integration with Claude Code

### Auto-indexing Hook

The `SessionStart` hook already runs `coral self-check`. Extend it to:
1. Check if index is stale (file hashes changed since last index)
2. If stale: trigger background incremental index
3. Report index health in self-check JSON

### Proactive Implementation Assistance

When Claude Code detects an implementation request:
1. Claude calls `coral find` to gather context
2. Claude calls `plan_change` MCP tool to generate a plan
3. Claude shows the plan to the user
4. On approval, Claude executes the plan using standard file editing
5. Claude calls `validate_change` to verify

This makes Claude Code's implementation loop **grounded in the codebase** rather than generating from general knowledge.

### CLAUDE.md Additions

```markdown
## Implementation assistance

When the user asks to implement something:
1. Run `coral find "<what they want>"` to gather context
2. Run `coral graph affected <relevant files>` to understand impact
3. Plan the change using gathered context
4. After implementing, run `coral validate` to verify
5. Run `coral index` to update the code graph
```

---

## 11. Dependencies & Tech Choices

| Component | Library | Rationale |
|-----------|---------|-----------|
| AST parsing | `tree-sitter` (Rust bindings) | 40+ languages, error-tolerant, incremental |
| Graph | `petgraph` | Standard Rust graph library, serializable |
| Embeddings (local) | `ort` (ONNX Runtime) | Run models locally without Python |
| Embeddings (cloud) | `voyage-code-3` API | Best code retrieval quality |
| Vector storage | Custom (flat search + HNSW for >10K vectors) | No external dependency |
| Keyword search | `tantivy` | Rust-native full-text search (Lucene-like) |
| KV storage | `sled` | Embedded, ACID, zero-config |
| Hashing | `blake3` | Fastest content-addressed hashing |
| Serialization | `bincode` | Compact binary for graph serialization |

### New crates in the workspace:

```
crates/
├── coral-index/          # Tree-sitter parsing + symbol store
├── coral-graph/          # Call graph + dependency graph + PageRank
├── coral-search/         # Hybrid search engine (embeddings + BM25 + structural)
└── coral-implement/      # Plan generation + pattern detection + code generation
```

---

## 12. Rollout Plan

### M1: Code Graph (v0.42) — 4 weeks

| Week | Deliverable |
|------|-------------|
| 1 | `coral-index` crate: tree-sitter parsing for Rust + TS + Python |
| 2 | `coral-graph` crate: call graph + dependency graph from parsed symbols |
| 3 | CLI commands: `coral index`, `coral graph show/callers/callees/affected` |
| 4 | MCP tools + wiki integration (auto-detect stale pages from AST diff) |

**Success metric**: `coral graph callers authenticate` returns correct results in <50ms on a 50K LOC repo.

### M2: Smart Search (v0.43) — 4 weeks

| Week | Deliverable |
|------|-------------|
| 1 | `coral-search` crate: tantivy keyword index + BM25 |
| 2 | Local embeddings via ONNX (nomic-embed-code) + vector store |
| 3 | Hybrid search with RRF fusion + `coral find` command |
| 4 | Upgrade `coral query` / `coral search` / `coral context-build` |

**Success metric**: `coral find "authentication middleware"` returns the correct file in top-3 results, <200ms, on a repo the user has never queried before.

### M3: Implementation Engine (v0.44) — 6 weeks

| Week | Deliverable |
|------|-------------|
| 1 | Pattern detection engine (analyze existing code structure) |
| 2 | `coral sketch` (lightweight diff preview) |
| 3 | `coral implement --plan-only` (plan generation grounded in context) |
| 4 | Full implement loop (plan → generate → test → verify) |
| 5 | Test-driven mode (`--tdd`) + iteration loop |
| 6 | MCP tools + Claude Code integration + CLAUDE.md update |

**Success metric**: `coral implement "add input validation to the create-user endpoint"` generates correct, pattern-consistent code that passes existing tests on first try 60%+ of the time.

---

## 13. Risks & Mitigations

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Tree-sitter queries incomplete for edge cases | Medium | Low | Fallback to regex for unsupported patterns |
| Local embeddings too slow on CPU | Medium | Medium | Offer Voyage API as fast alternative |
| Graph too large for memory (>500K LOC) | Low | High | Incremental loading + LRU eviction |
| Generated code introduces bugs | High | Medium | Always validate with tests + compile check |
| Index storage bloats `.coral/` | Medium | Low | Configurable exclusions + periodic GC |
| Users confused by too many commands | Medium | Medium | `coral setup` auto-indexes; `coral find` is the one search command |

---

## 14. Non-Goals (this PRD)

- **Cloud-hosted index** — Coral stays local-first. No data leaves the machine.
- **Real-time IDE integration** — LSP server mode deferred to v0.45+.
- **Multi-language type unification** — We don't merge TypeScript types with Rust types across a polyglot repo.
- **Automated PR generation** — `coral implement` generates code locally; pushing/PR creation is the user's choice.
- **Training custom models** — We use off-the-shelf embeddings and LLMs.

---

## 15. Success Criteria

| Metric | Target | How Measured |
|--------|--------|--------------|
| Index build time (50K LOC) | <10s | CI benchmark |
| Incremental index | <500ms | CI benchmark |
| Search accuracy (top-3 recall) | >80% | Manual eval on 5 repos |
| Implementation success rate | >60% first-try | Manual eval (code passes tests) |
| Pattern detection accuracy | >90% | Unit tests with known patterns |
| User adoption (setup→first query) | <3 min | E2E test timing |
| Binary size increase | <5 MB | CI check |

---

## 16. Open Questions

1. **Should embeddings be opt-in or opt-out?** Local embeddings add ~10s to first index. Cloud embeddings have a (tiny) cost. Proposal: opt-in with `coral embed` command, but `coral find` degrades gracefully without them.

2. **How deep should the call graph go?** 1 level (direct callers/callees) is cheap. Transitive closure is expensive. Proposal: default 2 levels, configurable with `--depth`.

3. **Should `coral implement` modify files directly or generate patches?** Proposal: generate to a staging area (`.coral/staging/`), show diff, user accepts with `--apply`.

4. **Should the code index be committed to git?** Proposal: no (`.coral/index/` in `.gitignore`). It's derivable from source and machine-specific (embedding model may differ across team).

5. **Integration with existing LSP?** Tree-sitter gives us 80% of what we need without a running language server. Full LSP integration (type-aware refactoring) deferred to v0.45. Trade-off: tree-sitter is universal and instant; LSP is precise but requires project compilation.
