---
slug: release-checklist
type: operation
last_updated_commit: 19da4d6
confidence: 0.92
sources:
  - scripts/release.sh
  - scripts/release-gh.sh
  - Cargo.toml
backlinks:
  - adr-rust
  - bootstrap-flow
status: draft
---

# Release Checklist

How to release a new version of Coral. The workspace uses semantic versioning with a single version number shared across all crates.

## Workspace Version

```toml
# Cargo.toml (workspace root)

[workspace.package]
version = "0.40.2"
```

All crates in `crates/*` inherit this version. Bumping the workspace version bumps every crate simultaneously.

## Tooling: scripts/release.sh

The release script has three subcommands:

```bash
# scripts/release.sh

# Three subcommands:
#   release.sh preflight     Pre-release hook (validates CHANGELOG + runs CI)
#   release.sh bump <X.Y.Z>  Wraps cargo release (no tag, no push)
#   release.sh tag <X.Y.Z>   Validates HEAD, tags, and pushes
```

### Preflight

Invoked as the `cargo-release` pre-release hook. Reads `$NEW_VERSION` (exported by cargo-release). Asserts a `## [<v>] - <today>` heading exists in CHANGELOG.md, rewrites the link-footer, then runs `ci-locally.sh`:

```bash
# scripts/release.sh — cmd_preflight()

local heading="## [$version] - $today"
if ! grep -qE "^## \[...\] - $today\$" CHANGELOG.md; then
    err "preflight: CHANGELOG.md is missing the heading: $heading"
    return 3
fi
ok "CHANGELOG section '$heading' present"
rewrite_changelog_footer "$version"
"$ci_script"
```

The preflight caches results per HEAD SHA to avoid re-running the test suite once per workspace crate (9 packages):

```bash
# scripts/release.sh — preflight caching

local marker="${TMPDIR:-/tmp}/coral-preflight-${version}-${head_sha}.pass"
if [[ -z "${CORAL_PREFLIGHT_FORCE:-}" && -f "$marker" ]]; then
    ok "ci-locally.sh already passed in this cargo-release run; skipping"
    return 0
fi
```

### Bump

```bash
# scripts/release.sh — cmd_bump()

# Wraps `cargo release X.Y.Z --no-tag --no-push --execute`.
# Bumps every workspace-member version + writes the release commit.
# Tag/push deferred to `release.sh tag` after tester sign-off.
```

### Tag

Validates HEAD's commit subject matches `release(vX.Y.Z):`, then creates an annotated tag and pushes:

```bash
# scripts/release.sh — cmd_tag() (logical flow)
# 1. Validate HEAD subject starts with "release(vX.Y.Z):"
# 2. cargo release tag X.Y.Z --execute
# 3. cargo release push --execute
# The tag-push triggers .github/workflows/release.yml
```

## GitHub Release (scripts/release-gh.sh)

Run AFTER the binary build completes. Extracts the CHANGELOG section for the version, derives the title from the bold span, and creates or updates the GH release:

```bash
# scripts/release-gh.sh (logical flow)
# 1. Extract CHANGELOG section between version headers
# 2. Derive title from bold span
# 3. gh release edit/create with extracted notes
```

## Typical Flow

1. Hand-write `## [X.Y.Z] - YYYY-MM-DD` section under `## [Unreleased]` in CHANGELOG.md
2. `scripts/release.sh bump X.Y.Z`
3. Tester validates the bump commit
4. `scripts/release.sh tag X.Y.Z`
5. Wait for `.github/workflows/release.yml` to build binaries
6. `scripts/release-gh.sh vX.Y.Z` -- creates the GH release with notes

## Commit Conventions

From the git log:
- `release(vX.Y.Z): bump version` -- the version-bump commit
- `ci(release): sync plugin manifests to vX.Y.Z [skip ci]` -- automated manifest sync
- `docs(readme): vX.Y.Z update` -- README updates for the new version

## Pre-release Checks

- `cargo clippy --workspace` passes with no warnings
- `cargo test --workspace` (1900+ tests)
- `cargo build --release` compiles successfully
- CHANGELOG covers all user-visible changes
- README version badges are correct
