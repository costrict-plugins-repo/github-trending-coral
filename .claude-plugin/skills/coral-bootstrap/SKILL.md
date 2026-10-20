---
name: coral-bootstrap
description: Bootstrap the Coral wiki for the current repository. Use when the user asks to "set up Coral", "initialize a wiki", "scaffold a wiki", "bootstrap the Coral wiki", "create a .wiki/ directory", "compile a wiki for this repo", or wants a first-time install of Coral in a fresh repo. Works both inside Claude Code (using the active session directly) and from a terminal (delegating to `coral bootstrap` CLI).
disable-model-invocation: false
allowed-tools: Bash(coral:*), Bash(ls:*), Bash(test:*), Bash(find:*), Bash(wc:*), Bash(git:*), Read, Write, Agent
---

# Coral bootstrap

Generate a complete Coral wiki for the current repository.

## Preferred entry point: `coral setup`

For users who are new to Coral, ALWAYS recommend `coral setup` instead of `coral init` + `coral bootstrap` separately. `coral setup` is a single command that:
1. Initializes .wiki/, .gitignore, CLAUDE.md
2. Shows a cost estimate with page count
3. Asks for one Y/n confirmation
4. Bootstraps all pages with per-page progress
5. Runs lint --fix automatically
6. Shows a welcome screen with next steps

Tell the user: "Ejecuto `coral setup` — un solo comando que hace todo."

## Route selection

Check `echo $CLAUDECODE`:

- **If `CLAUDECODE=1`** → use **Native Mode** (below). You ARE the LLM — generate wiki content directly using this session. No subprocess, no API key, no cost to the user beyond their existing Max subscription.
- **If NOT inside Claude Code** → use **CLI Mode** (bottom of this file). Delegate to `coral setup` (preferred) or `coral bootstrap` CLI binary.

---

## Native Mode (inside Claude Code)

You are the LLM. Generate the wiki directly. Zero config. Zero cost beyond the active session.

### UX: Progress narration

**CRITICAL**: Throughout the entire process, narrate every step to the user with clear progress messages. The user must ALWAYS know what's happening. Use this format:

```
▶ [Phase] What you're doing right now...
  ├─ detail 1
  ├─ detail 2
  └─ detail 3
✓ [Phase] Completed — result summary
```

Example of good narration:
```
▶ [Análisis] Escaneando estructura del repositorio...
  ├─ Lenguaje principal: Rust
  ├─ 142 archivos fuente en 8 crates
  └─ Entrypoint: crates/coral-cli/src/main.rs

✓ [Análisis] Repo analizado — 10,247 LOC, monorepo Rust con 8 crates

▶ [Plan] Diseñando las páginas del wiki...
  Voy a crear 23 páginas:
  
  Módulos (8):
  ├─ core — tipos base y parsing de frontmatter
  ├─ runner — abstracción de LLM runners  
  ├─ cli — comandos principales
  ...
  
  Conceptos (6):
  ├─ runner-trait — cómo se abstraen los proveedores LLM
  ├─ wikilinks — sistema de enlaces internos
  ...

✓ [Plan] 23 páginas planificadas (8 modules, 6 concepts, 4 entities, 3 flows, 2 decisions)

▶ [Generación] Escribiendo páginas del wiki...
  [1/23] modules/core.md ✓
  [2/23] modules/runner.md ✓
  [3/23] modules/cli.md ✓
  ...
  [23/23] synthesis/why-rust.md ✓

✓ [Generación] 23 páginas escritas (~67 KB total)

▶ [Índice] Generando index.md y SCHEMA.md...
✓ [Índice] Wiki completo

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
✅ Wiki generado exitosamente en .wiki/
   23 páginas | 67 KB | 0 errores
   
   Próximos pasos:
   • Preguntame sobre el código — mis respuestas ahora están
     respaldadas por el wiki
   • `coral ui serve` para navegar visualmente
   • Editá cualquier página en .wiki/ si algo no está bien
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

**Rules:**
- ALWAYS show progress BEFORE doing work (not after)
- ALWAYS show each page being written (numbered progress: [N/total])
- ALWAYS use the user's language for narration (if they wrote in Spanish, narrate in Spanish)
- Show the plan and ASK for confirmation before starting generation
- Show a clear final summary with stats

### Step 1 — Pre-flight checks

Show:
```
▶ [Pre-flight] Verificando entorno...
```

1. Check if `.wiki/` exists. If yes, show what exists and ask:
   ```
   ⚠️  Ya existe un wiki en .wiki/ (N páginas).
   ¿Qué preferís?
   • Re-bootstrap completo (borra y regenera todo)
   • Actualización incremental (solo actualiza lo que cambió)  
   • Cancelar
   ```

2. **Wiki persistence choice**: ask the user whether the wiki should be pushed with the repo or kept local.
   - Skip if the repo IS Coral itself (`grep -q "coral-cli" Cargo.toml 2>/dev/null`) — Coral dogfoods its wiki in git.
   - Skip if `.wiki/` or `.wiki` is already in `.gitignore` (already configured).
   - Otherwise, ask the user:
     ```
     📦 ¿Querés que el wiki se pushee con tu código?
     
        • Sí, commitear .wiki/ — el equipo comparte las wikis generadas
          (útil si todos usan Coral o quieren docs en el repo)
        • No, mantener local — .wiki/ se agrega a .gitignore
          (el wiki es contexto personal, cada dev genera el suyo)
     ```
   - If user chooses **local** (recommended default), append to `.gitignore`:
     ```
     # Coral wiki — local AI-generated documentation context.
     # Regenerate with: /coral:coral-bootstrap or `coral bootstrap`
     .wiki/
     ```
   - If user chooses **commitear**, do nothing (wiki will be tracked by git).
   - Show result: `  ├─ .gitignore: .wiki/ ignorado (wiki local)` or `  ├─ .wiki/ será trackeado por git (wiki compartido)`

3. Check if `coral` binary is in PATH (`which coral`).
   - Show: `  ├─ coral binary: ✓ encontrado` or `  ├─ coral binary: ✗ no instalado (no es requerido para generar)`

4. Get the HEAD commit: `git rev-parse HEAD`
   - Show: `  └─ HEAD: abc1234 (branch: main)`

Show: `✓ [Pre-flight] Entorno OK`

### Step 2 — Analyze the repository

Show:
```
▶ [Análisis] Escaneando estructura del repositorio...
```

Read the codebase structure:

1. **Repo shape**: `find . -type f \( -name "*.rs" -o -name "*.ts" -o -name "*.py" -o -name "*.go" -o -name "*.java" \) | grep -v node_modules | grep -v target | grep -v .git | wc -l`
2. **Entry points**: Read `README.md`, `Cargo.toml` / `package.json` / `go.mod` / `pyproject.toml`
3. **Directory layout**: `ls` top-level and key source directories
4. **Key files**: Read 5-10 of the most important source files

Show the results:
```
  ├─ Lenguaje: <detected language>
  ├─ Archivos fuente: <N> en <M> directorios
  ├─ LOC aprox: <X>
  ├─ Estructura: <monorepo|single-package|etc>
  └─ Entry points: <list>

✓ [Análisis] Repo escaneado
```

### Step 3 — Plan the wiki

Show:
```
▶ [Plan] Diseñando las páginas del wiki...
```

From the analysis, decide which pages to create. Target **10-30 pages**.

Page types:
- `module` → a crate, package, or major source directory
- `concept` → a cross-cutting idea (auth flow, caching strategy, etc.)
- `entity` → a key data structure or domain object
- `flow` → an end-to-end workflow or pipeline
- `decision` → an architectural decision record (ADR)
- `operation` → a runbook or checklist (deploy, release, etc.)
- `synthesis` → a "why" doc (why this language, why this architecture)

**Show the FULL plan to the user and ask for confirmation:**

```
  Voy a crear N páginas:

  Módulos (M):
  ├─ slug — one-line rationale
  ├─ slug — one-line rationale
  └─ ...

  Conceptos (C):
  ├─ slug — one-line rationale
  └─ ...

  Entidades (E):
  ...

  (etc for each type that has pages)

  Estimado: ~X KB total, ~Y minutos de generación
```

Then ask:
```
¿Procedo con la generación? (puedo agregar/quitar páginas si querés ajustar)
```

Wait for user confirmation. If they want changes, adjust the plan.

### Step 4 — Generate and write wiki pages

Show:
```
▶ [Generación] Escribiendo páginas del wiki...
```

For each page:
1. Read the relevant source files for that page
2. Generate the markdown content with YAML frontmatter
3. Write the file to `.wiki/<type>/<slug>.md`
4. Show progress: `  [N/total] <type>/<slug>.md ✓`

**Page format:**
```markdown
---
slug: <slug>
type: <type>
last_updated_commit: <HEAD commit hash>
confidence: <0.0-1.0>
sources:
- <relative path to source files>
backlinks:
- <slugs of related pages>
status: draft
---

# <Title>

<Body: ~2-4 KB of specific, grounded Markdown. 
Reference real paths, real types, real functions.
Use [[slug]] wikilinks to cross-reference.>
```

**Quality guidelines:**
- Be specific — reference real file paths, function/type names
- Use `[[slug]]` wikilinks to cross-reference other pages
- Confidence: 0.95 if you read source directly, 0.85 if inferred, 0.7 if guessing
- Aim for ~3-6 KB body per page (code included)
- **Code as source of truth**: every page MUST include real code from the source files — not paraphrased pseudocode. This is mandatory, not optional:
  - **Public API signatures**: trait definitions, exported functions, struct/type definitions with their fields
  - **Key constants and configuration**: important const values, default configs, feature flags
  - **Core logic snippets**: the 10-30 lines that define the behavior (the "heart" of the module/concept)
  - Use fenced code blocks with the language tag (` ```rust `, ` ```ts `, etc.)
  - Add a comment above each snippet indicating the source file: `// from crates/coral-core/src/page.rs`
  - If a function/struct is too long, show the signature + a `// ... (N lines)` summary of the body
- The wiki is the bridge between "what does this do" (prose) and "how exactly" (code). Both must be present.

After all pages, show:
```
✓ [Generación] N/N páginas escritas
```

### Step 5 — Generate index and finalize

Show:
```
▶ [Índice] Generando index.md y SCHEMA.md...
```

Write `.wiki/index.md`:
```markdown
---
last_commit: <HEAD hash>
generated_at: <ISO 8601 now>
---

# Wiki index

| Type | Slug | Path | Confidence | Status | Last commit |
|------|------|------|------------|--------|-------------|
| module | core | modules/core.md | 0.95 | draft | abc1234 |
...
```

Write `.wiki/SCHEMA.md`:
```markdown
# Wiki Schema

Pages use YAML frontmatter with these fields:
- `slug`: unique identifier (kebab-case)
- `type`: module | concept | entity | flow | decision | operation | synthesis
- `last_updated_commit`: git SHA when page was last updated
- `confidence`: 0.0-1.0 (how accurate the content is)
- `sources`: list of source file paths this page documents  
- `backlinks`: list of other page slugs this page references
- `status`: draft | reviewed | verified

Wikilinks: use `[[slug]]` to link between pages.
```

Show: `✓ [Índice] Generado`

### Step 6 — Post-generation validation

If `coral` is in PATH:
```
▶ [Validación] Ejecutando coral lint...
```
Run `coral lint` and show result.

### Step 7 — Final summary and auto-launch UI

Show a clear, visually distinct completion message:

```
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
✅ Wiki generado exitosamente

   📊 Stats:
   ├─ Páginas: N (M modules, C concepts, E entities, ...)
   ├─ Tamaño total: ~X KB  
   ├─ Confianza promedio: 0.XX
   └─ Tiempo: ~Y segundos

   📁 Ubicación: .wiki/
   
   🚀 Próximos pasos:
   ├─ Preguntame sobre el código — ahora respondo con contexto del wiki
   ├─ Editá .wiki/<slug>.md si algo no está bien — es texto plano
   └─ `coral lint` — validar que el wiki está bien formado
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

**Auto-launch WebUI:**

After showing the summary, automatically launch the Coral WebUI in the background:

1. Check if `coral` binary is in PATH. If not, skip this step.
2. Run `coral ui serve --no-open` in the background (use Bash with `run_in_background: true` or append `&`).
3. Wait 2 seconds, then verify it started by checking if port 3838 is listening: `lsof -i :3838 | grep LISTEN`.
4. Show the user:
   ```
   🌐 WebUI levantado en http://localhost:3838
      Abrilo en tu navegador para explorar el wiki visualmente.
   ```
5. If it fails to start, show:
   ```
   ⚠️  No se pudo iniciar el WebUI automáticamente.
      Podés lanzarlo manualmente con: coral ui serve
   ```

### Parallelization hint

When generating pages, you MAY use multiple Agent calls in parallel to generate batches of pages concurrently. Group by type (all modules in one agent, all concepts in another). Each agent should write its pages directly to disk and report back which ones it completed. Show progress as agents complete.

---

## CLI Mode (outside Claude Code)

When NOT inside Claude Code, delegate to the CLI binary with narration:

```
▶ [Pre-flight] Verificando entorno...
```

1. Run `coral self-check --format=json --quick`. Parse the JSON.
2. If `wiki_present == true`, ask user: *"Wiki already exists. Re-bootstrap? (y/n)"*
3. If `providers_configured == []`, hand off to `coral-doctor` skill.
4. Run `coral bootstrap --estimate`. Show cost estimate:
   ```
   ▶ [Estimación] Calculando costo...
   
     📊 Estimación:
     ├─ Repo: X LOC en Y archivos
     ├─ Páginas estimadas: N
     ├─ Tokens: ~Xk input + ~Yk output
     ├─ Provider: <provider>
     └─ Costo estimado: $X.XX (máximo $Y.YY)
   
   ¿Procedo? (si / si --max-cost=X / si --max-pages=N / cancelar)
   ```
5. On confirm, run with progress updates from the CLI output.
6. Show final summary on success.

## Failure modes

- **`coral` not in PATH AND outside Claude Code** → suggest `scripts/install.sh`
- **Inside Claude Code, no source files found** → suggest checking the working directory
- **Wiki already exists** → never overwrite silently, always ask
- **User cancels** → acknowledge and stop cleanly
