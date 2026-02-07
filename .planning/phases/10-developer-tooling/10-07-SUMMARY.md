---
phase: 10-developer-tooling
plan: 07
subsystem: cli-package-management
tags: [scaffold, init, deps, lockfile, package-manager]

dependency-graph:
  requires: ["10-06"]
  provides: ["snowc-init-command", "snowc-deps-command", "project-scaffolding"]
  affects: ["10-09", "10-10"]

tech-stack:
  added: []
  patterns: ["CLI subcommand dispatch", "filesystem scaffolding", "lockfile freshness check"]

key-files:
  created:
    - crates/snow-pkg/src/scaffold.rs
  modified:
    - crates/snow-pkg/src/lib.rs
    - crates/snowc/src/main.rs
    - crates/snowc/Cargo.toml

decisions:
  - id: "10-07-01"
    description: "Lockfile freshness uses filesystem mtime comparison (manifest vs lockfile)"
    rationale: "Simple, no hashing needed -- if manifest modified after lockfile, re-resolve"

metrics:
  duration: "5min"
  completed: "2026-02-07"
---

# Phase 10 Plan 07: snowc init and deps Subcommands Summary

**One-liner:** Project scaffolding via `snowc init` and dependency resolution via `snowc deps` with lockfile freshness detection.

## What Was Done

### Task 1: Project scaffolding (snowc init)

Added `crates/snow-pkg/src/scaffold.rs` with `scaffold_project(name, dir)` function that creates a standard Snow project directory structure:

```
<name>/
  snow.toml    -- [package] name + version + empty [dependencies]
  main.snow    -- fn main() with IO.puts hello world
```

Error handling for existing directories ("Directory '<name>' already exists").

Re-exported `scaffold_project` from `snow-pkg` lib.rs.

4 unit tests:
- Directory structure creation
- Valid manifest (parseable by Manifest::from_str)
- main.snow content verification
- Error on existing directory

### Task 2: snowc init and snowc deps subcommands

Added `Init` and `Deps` variants to the snowc `Commands` enum:
- `Init { name: String }` -- creates project in current directory
- `Deps { dir: PathBuf }` -- defaults to current directory

Init handler calls `snow_pkg::scaffold_project(&name, &dir)`.

Deps handler (`deps_command`):
1. Checks for snow.toml existence
2. Checks lockfile freshness (mtime comparison: if manifest modified <= lockfile modified, skip)
3. Calls `snow_pkg::resolve_dependencies(dir)` for actual resolution
4. Writes snow.lock
5. Prints summary: "Resolved N dependencies" / "No dependencies" / "Dependencies up to date"

Added `snow-pkg` dependency to snowc Cargo.toml.

## Verification Results

- `cargo test -p snow-pkg`: 24 tests pass (20 existing + 4 new scaffold tests)
- `cargo test -p snowc`: 62 tests pass across all test suites
- `snowc init my-project` creates correct directory with snow.toml and main.snow
- `snowc deps` in project with no deps succeeds and creates snow.lock
- Second `snowc deps` run prints "Dependencies up to date" (lockfile freshness)

## Decisions Made

| ID | Decision | Rationale |
|----|----------|-----------|
| 10-07-01 | Lockfile freshness via mtime comparison | Simple filesystem-based check; if snow.toml modified after snow.lock, re-resolve |

## Deviations from Plan

### Commit Merge Due to Parallel Execution

Task 2 changes (main.rs, Cargo.toml) were captured in commit `4139550` alongside the Fmt subcommand from plan 10-03, which was being executed by a parallel agent. Both agents modified the same files (snowc main.rs and Cargo.toml). Task 1 (scaffold.rs) was committed atomically in `21bc99e`.

## Task Commits

| Task | Name | Commit | Files |
|------|------|--------|-------|
| 1 | Project scaffolding | 21bc99e | scaffold.rs, lib.rs |
| 2 | snowc init and deps subcommands | 4139550 | main.rs, Cargo.toml (shared with 10-03) |

## Next Phase Readiness

No blockers. The init and deps subcommands are functional. Future plans can build on:
- `snowc init` for project creation workflows
- `snowc deps` for dependency resolution in build pipelines
- `snow_pkg::scaffold_project` for programmatic project creation

## Self-Check: PASSED
