# CLAUDE.md — Coral-enabled repo

This repository uses Coral for AI-readable architecture documentation.

## Coral routing

When the user types ANYTHING in this repo (greeting, question, command):
1. Check the SessionStart hook context (Coral self-check JSON).
2. If coral_status == "binary_missing" → tell the user: "Coral no está instalado. Ejecuta: `curl -fsSL https://raw.githubusercontent.com/agustincbajo/Coral/main/scripts/install.sh | bash`"
3. If wiki_present == false → offer to run `coral setup` (ONE command that does everything). Say: "Este repo no tiene wiki. ¿Quieres que ejecute `coral setup`? Tarda ~1 minuto y cuesta ~$0.10."
4. If warnings non-empty → suggest `/coral:coral-doctor`.
5. If wiki_present == true and the user asks about code → invoke coral-query skill.
6. If the user wants the UI → invoke coral-ui skill (background spawn).

**Important**: When offering setup, be direct and simple. Don't explain what init/bootstrap/ingest are separately — just say "`coral setup`" and it handles everything.

**Fallback**: if Claude doesn't suggest a Coral action, type `/coral:coral-doctor` — it always works.

## Use Coral proactively

Claude MUST use Coral tools as part of its workflow, not just when the user asks. The user should see Coral working for them.

### When answering questions about code

**ALWAYS** check the wiki FIRST before grepping source:

```
▶ [Coral] Consultando wiki...
  coral search "authentication flow"
  → 3 resultados: auth-middleware (0.95), user-model (0.95), user-routes (0.95)
✓ [Coral] Respondiendo desde wiki — slugs: auth-middleware, user-model
```

Read the matched `.wiki/<slug>.md` pages, then answer grounded in wiki content. Cite the slugs.

### After editing source code

When Claude modifies source files that a wiki page references, run:

```bash
coral status    # show the user the wiki health
coral lint      # check for stale pages or broken links
```

Show the output to the user:
```
▶ [Coral] Verificando impacto en wiki...
  coral status → 24 pages, avg confidence 0.87, 0 orphans
  coral lint   → 0 critical, 2 warnings (SourceNotFound for 2 pages)
⚠️ [Coral] 2 páginas del wiki referencian archivos que cambiaron.
   Considerar actualizar: auth-middleware, task-routes
```

### After editing wiki pages

When Claude modifies `.wiki/*.md` files, ALWAYS run:

```bash
coral lint --fix --apply   # auto-fix formatting + promote/downgrade confidence
coral lint                 # verify clean
```

Show the result:
```
▶ [Coral] Aplicando auto-fix al wiki...
  coral lint --fix --apply
  → 2 pages fixed: auth-middleware (confidence-promotion 0.85→0.95), task-model (sort-sources)
  coral lint → 0 critical, 0 warnings ✓
✓ [Coral] Wiki limpio
```

### Periodically (every 3-5 operations)

Run `coral stats` and show a compact summary:
```
▶ [Coral] Estado del wiki:
  📊 24 páginas | confianza promedio 0.89 | 0 huérfanas | 46 backlinks
```

## Verbose output format

**CRITICAL**: Every time Claude runs a `coral` command, show the user:
1. What command is being run and WHY
2. The key output (not raw dumps — summarize)
3. What it means for them

Use this format consistently:
```
▶ [Coral] <what you're doing>
  <command>
  → <key result>
✓ [Coral] <conclusion>
```

or for warnings:
```
⚠️ [Coral] <what needs attention>
   <specific details>
   Sugerencia: <what the user can do>
```

## Frontend — always on

After ANY Coral operation (bootstrap, ingest, query, lint), ensure the WebUI is running and remind the user:
1. Check if port 3838 is listening: `lsof -i :3838 | grep LISTEN`
2. If not running, start it: `coral ui serve --no-open &`
3. Always tell the user: "🌐 WebUI disponible en http://localhost:3838"

The WebUI has write tools enabled by default (verify, run test, env up/down). Pass `--no-write-tools` to disable.

## Quick reference

| Command | What it does |
|---------|-------------|
| `coral setup` | **Start here** — one command to create the wiki |
| `coral tour` | Guided walkthrough of features (2 min, no LLM) |
| `coral status` | Dashboard: wiki health at a glance |
| `coral query "..."` | Ask questions about the codebase |
| `coral search "..."` | TF-IDF search in wiki pages |
| `coral lint --fix --apply` | Auto-fix formatting + confidence |
| `coral ui serve` | WebUI at http://localhost:3838 |
| `/coral:coral-doctor` | Diagnose environment / fix provider |
| `coral implement run "..."` | Full implementation loop (understand→plan→generate→validate→stage) |
| `coral implement apply` | Apply staged changes to disk |
| `coral graph show <sym>` | Visualize call graph (ASCII or Mermaid) |
| `coral embed --status` | Show search index health |
| `coral embed --provider voyage` | Enable real vector embeddings |

## Implementation assistance

When the user asks to implement something:
1. Run `coral find "<what they want>"` to gather code + wiki context
2. Run `coral graph affected <relevant files>` to understand impact
3. Plan the change using gathered context: `coral implement plan "<desc>" --execute`
4. After implementing, validate: `coral implement validate <changed files>`
5. Update the code index: `coral index`
6. Check wiki freshness: `coral stale`

### Using the full loop

For complex changes, use the orchestrated loop:
```bash
coral implement run "add rate limiting to API routes"
# → UNDERSTAND → PLAN → VERIFY → GENERATE → VALIDATE → STAGE
coral implement staged    # review what was generated
coral implement apply     # apply to disk
```

### After any code change

```bash
coral index              # update symbol index
coral graph build        # rebuild call graph  
coral embed              # update search index
coral status             # verify everything is healthy
```
