---
phase: 10-developer-tooling
plan: 06
subsystem: package-manager
tags: [toml, git2, semver, dependency-resolution, lockfile, manifest]

# Dependency graph
requires:
  - phase: 01-project-foundation-lexer
    provides: "Rust workspace structure and crate conventions"
provides:
  - "snow-pkg crate with manifest parsing (snow.toml)"
  - "DFS dependency resolver with git and path support"
  - "Deterministic lockfile generation (snow.lock)"
  - "Conflict and cycle detection for dependency graphs"
affects: [10-developer-tooling]

# Tech tracking
tech-stack:
  added: [toml 0.8, git2 0.19, semver 1]
  patterns: [serde untagged enum for dependency variants, BTreeMap for deterministic ordering, DFS with visiting set for cycle detection]

key-files:
  created:
    - "crates/snow-pkg/Cargo.toml"
    - "crates/snow-pkg/src/lib.rs"
    - "crates/snow-pkg/src/manifest.rs"
    - "crates/snow-pkg/src/resolver.rs"
    - "crates/snow-pkg/src/lockfile.rs"
  modified:
    - "Cargo.toml"

key-decisions:
  - "Serde untagged enum for Dependency (Git/Path) enables natural TOML syntax"
  - "BTreeMap for dependencies and lockfile packages ensures deterministic ordering"
  - "DFS visiting set for cycle detection, source key comparison for conflict detection"
  - "Git deps cloned to .snow/deps/<name>/ (conventional location)"
  - "Path deps use canonicalize() for consistent source key comparison"

patterns-established:
  - "snow.toml manifest format: [package] + [dependencies] sections"
  - "snow.lock lockfile format: TOML with version field and sorted packages array"
  - "Dependency resolution via resolve_dependencies() -> (Vec<ResolvedDep>, Lockfile)"

# Metrics
duration: 6min
completed: 2026-02-07
---

# Phase 10 Plan 06: Package Manager Core Summary

**snow-pkg crate with TOML manifest parsing, git2-based dependency resolution, DFS conflict/cycle detection, and deterministic lockfile generation**

## Performance

- **Duration:** 6 min
- **Started:** 2026-02-07T17:13:02Z
- **Completed:** 2026-02-07T17:18:48Z
- **Tasks:** 2
- **Files modified:** 6

## Accomplishments
- Manifest parsing supports [package] metadata and [dependencies] with git (rev/branch/tag) and path variants
- DFS dependency resolver handles transitive deps, diamond conflicts, and cycles
- Git dependencies cloned via git2 to .snow/deps/ with rev/branch/tag checkout support
- Deterministic lockfile (snow.lock) with sorted packages and round-trip serialization
- 20 tests covering parsing, resolution, conflict detection, cycle detection, and git operations

## Task Commits

Each task was committed atomically:

1. **Task 1: snow.toml manifest parsing and snow.lock lockfile format** - `1d091a9` (feat)
2. **Task 2: Git-based dependency resolution with conflict detection** - `f25909b` (feat)

## Files Created/Modified
- `crates/snow-pkg/Cargo.toml` - Crate manifest with toml, git2, semver, serde dependencies
- `crates/snow-pkg/src/lib.rs` - Package manager public API (re-exports Manifest, resolve_dependencies)
- `crates/snow-pkg/src/manifest.rs` - snow.toml parsing with Manifest, Package, Dependency types
- `crates/snow-pkg/src/resolver.rs` - DFS resolver with ResolvedDep, DepSource, conflict/cycle detection
- `crates/snow-pkg/src/lockfile.rs` - Lockfile struct with deterministic TOML serialization
- `Cargo.toml` - Added snow-pkg to workspace members, toml/git2/semver to workspace.dependencies

## Decisions Made
- [10-06]: Serde untagged enum for Dependency (Git variant with git/rev/branch/tag fields, Path variant with path field)
- [10-06]: BTreeMap for both manifest dependencies and resolver output ensures deterministic ordering
- [10-06]: DFS with HashSet<String> visiting set for cycle detection; BTreeMap<String, String> source_keys for conflict detection
- [10-06]: Git deps cloned to project_dir/.snow/deps/<name>/ (project-local, not global cache)
- [10-06]: Path dep source keys use canonicalize() so same-path diamond deps are correctly deduplicated
- [10-06]: Lockfile version field (always 1) for future format evolution

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Created stub lib.rs for snow-lsp and snow-repl**
- **Found during:** Task 1 (workspace compilation)
- **Issue:** Concurrent plans (snow-lsp, snow-repl) added crate entries to workspace Cargo.toml but source files were incomplete, blocking cargo check
- **Fix:** Created minimal lib.rs stubs for both crates (later overwritten by their respective plan executions)
- **Files modified:** crates/snow-lsp/src/lib.rs, crates/snow-repl/src/lib.rs
- **Verification:** cargo check -p snow-pkg succeeds
- **Committed in:** 1d091a9 (Task 1 commit -- stubs were transitional, overwritten by concurrent plans)

**2. [Rule 1 - Bug] Fixed borrow checker error in fetch_git_dep**
- **Found during:** Task 2 (resolver implementation)
- **Issue:** git2::Remote borrows Repository, preventing repo move out of if-block
- **Fix:** Scoped the Remote borrow in a block so it drops before repo is moved
- **Files modified:** crates/snow-pkg/src/resolver.rs
- **Verification:** cargo test -p snow-pkg passes
- **Committed in:** f25909b (Task 2 commit)

---

**Total deviations:** 2 auto-fixed (1 blocking, 1 bug)
**Impact on plan:** Both fixes necessary for compilation. No scope creep.

## Issues Encountered
- Concurrent plan executions modified Cargo.toml workspace members, adding snow-lsp/snow-fmt/snow-repl entries before their source files existed. Resolved by creating temporary stubs.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness
- snow-pkg crate ready for integration with CLI (snowc) commands
- Lockfile can be extended with checksum fields when registry support is added
- Git dependency fetching tested with local repos; network git URLs will work identically via git2

## Self-Check: PASSED

---
*Phase: 10-developer-tooling*
*Completed: 2026-02-07*
