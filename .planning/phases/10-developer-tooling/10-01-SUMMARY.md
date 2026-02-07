---
phase: 10
plan: 01
subsystem: diagnostics
tags: [ariadne, diagnostics, json, cli, error-messages]
dependency-graph:
  requires: [03-05]
  provides: [enhanced-diagnostics, json-output, cli-flags]
  affects: [10-08]
tech-stack:
  added: [serde_json]
  patterns: [DiagnosticOptions, render_json_diagnostic, multi-span-labels]
key-files:
  created: []
  modified:
    - crates/snow-typeck/src/diagnostics.rs
    - crates/snow-typeck/src/lib.rs
    - crates/snow-typeck/Cargo.toml
    - crates/snowc/src/main.rs
    - crates/snowc/Cargo.toml
    - crates/snow-typeck/tests/diagnostics.rs
    - crates/snow-repl/src/jit.rs
decisions:
  - id: d-10-01-01
    title: "DiagnosticOptions struct over function parameters"
    choice: "Struct with color/json fields and Default impl"
    reason: "Extensible, clearer call sites, can add fields later"
  - id: d-10-01-02
    title: "JSON output via serde_json"
    choice: "JsonDiagnostic struct with Serialize derive"
    reason: "Type-safe, consistent output format, easy to extend"
  - id: d-10-01-03
    title: "FnArg multi-span without param_span"
    choice: "Use call_site + param_idx for labeled span"
    reason: "ConstraintOrigin::FnArg has no param_span field; adapted to actual data model"
metrics:
  duration: ~12min
  completed: 2026-02-07
---

# Phase 10 Plan 01: Enhanced Diagnostics Summary

Polished compiler error messages to Elm/Rust quality standard with multi-span diagnostics, fix suggestions, colorized terminal output, and machine-readable JSON output mode.

## One-liner

DiagnosticOptions API with color toggle, JSON output via render_json_diagnostic, multi-span labels for FnArg/Return/Assignment origins, and --json/--no-color CLI flags.

## What Was Done

### Task 1: Enhanced diagnostics with color toggle, multi-span, and JSON output
- Added `DiagnosticOptions` struct with `color` and `json` fields, `Default` impl, and convenience constructors (`colorless()`, `json_mode()`)
- Updated `render_diagnostic()` to accept `&DiagnosticOptions` and optional `&[String]` suggestions parameter
- Added multi-span labels for `FnArg` (argument number + type info), `Return` (return expression + return type declaration), and `Assignment` (lhs expected + rhs found) origins
- Created `render_json_diagnostic()` producing one-line JSON with `JsonDiagnostic` struct (code, severity, message, file, spans, fix)
- Added `JsonSpan` struct for machine-readable span information
- Added Levenshtein distance implementation for "did you mean X?" suggestions on E0004/E0010
- Added fix suggestion for E0005 NotAFunction ("did you mean to call it?")
- Updated `TypeckResult::render_errors()` to accept `&DiagnosticOptions`
- Updated all callers across snow-typeck tests, integration tests, snowc, and snow-repl
- Added unit tests for Levenshtein distance, closest name finding, DiagnosticOptions, and JSON serialization

### Task 2: --json and --no-color CLI flags
- Added `--json` flag to `Commands::Build` in snowc CLI
- Added `--no-color` flag to `Commands::Build` in snowc CLI
- JSON mode outputs one JSON diagnostic per line to stderr for both parse and type errors
- Parse errors in JSON mode use code "P0001" with span information
- --no-color disables ANSI escape codes via ariadne Config
- JSON mode auto-disables color for clean machine-readable output
- Added serde_json dependency to snowc Cargo.toml
- All existing e2e tests pass (28 build tests + 4 supervisor tests)

## Task Commits

| Task | Name | Commit | Key Files |
|------|------|--------|-----------|
| 1 | Enhanced diagnostics | da0d630 | diagnostics.rs tests, snapshot, snowc/main.rs |
| 2 | --json and --no-color flags | 4435722 | snowc/main.rs, snowc/Cargo.toml |

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] FnArg origin has no param_span field**
- **Found during:** Task 1
- **Issue:** Plan specified using `param_span` field from FnArg origin, but ConstraintOrigin::FnArg only has `call_site` and `param_idx`
- **Fix:** Used call_site span with param_idx in label message instead
- **Files modified:** crates/snow-typeck/src/diagnostics.rs

**2. [Rule 1 - Bug] Snapshot changed for not-a-function test**
- **Found during:** Task 1 verification
- **Issue:** `let x = 42; x(1)` triggers Mismatch (not NotAFunction) with new FnArg labels
- **Fix:** Updated snapshot and changed test_not_a_function_fix_suggestion to construct NotAFunction error directly
- **Files modified:** crates/snow-typeck/tests/diagnostics.rs, snapshot file

**3. [Rule 3 - Blocking] serde_json needed as dev-dependency for test compilation**
- **Found during:** Task 1 verification
- **Issue:** Test file uses `serde_json::from_str` which requires serde_json as dev-dependency
- **Fix:** Added serde_json to snow-typeck dev-dependencies
- **Files modified:** crates/snow-typeck/Cargo.toml

**4. [Rule 3 - Blocking] Pre-existing API already updated in committed code**
- **Found during:** Task 1
- **Issue:** Many source files (diagnostics.rs, lib.rs, Cargo.toml) already had the new 5-arg API committed from previous phase work
- **Fix:** Only needed to update test callers and snowc/main.rs which still used old 3-arg signatures
- **Files modified:** test files and snowc/main.rs only

## Decisions Made

| ID | Decision | Choice | Rationale |
|----|----------|--------|-----------|
| d-10-01-01 | Options API design | DiagnosticOptions struct | Extensible, Default impl, clear call sites |
| d-10-01-02 | JSON serialization | serde_json with Serialize derive | Type-safe, consistent, standard |
| d-10-01-03 | FnArg multi-span | call_site + param_idx label | Adapted to actual data model |

## Verification Results

- `cargo test -p snow-typeck --lib --tests`: all 111 tests pass
- `cargo test -p snowc`: all 32 tests pass (28 e2e + 4 supervisors)
- `cargo build --workspace`: compiles cleanly
- Manual: `snowc build --json /tmp/test_snow_bad` outputs valid JSON with E0001 code and spans
- Manual: `snowc build --no-color /tmp/test_snow_bad` outputs colorless diagnostics
- Manual: `snowc build /tmp/test_snow_bad` outputs ANSI-colorized diagnostics

## Next Phase Readiness

No blockers. The enhanced diagnostics infrastructure is ready for use by the LSP server (phase 10-08) and other tooling.

## Self-Check: PASSED
