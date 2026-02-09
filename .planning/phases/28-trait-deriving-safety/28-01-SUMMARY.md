---
phase: 28-trait-deriving-safety
plan: 01
subsystem: compiler
tags: [typeck, traits, deriving, diagnostics]
requires:
  - phase: 22
    provides: Auto-derive system
provides:
  - Compile-time Ord-requires-Eq enforcement
  - MissingDerivePrerequisite error variant (E0029)
affects: [phase-29]
tech-stack:
  added: []
  patterns: [trait-dependency-validation]
key-files:
  created: []
  modified:
    - crates/snow-typeck/src/error.rs
    - crates/snow-typeck/src/diagnostics.rs
    - crates/snow-typeck/src/infer.rs
    - crates/snow-lsp/src/analysis.rs
    - crates/snowc/tests/e2e.rs
key-decisions:
  - id: 28-01-D1
    decision: "Emit error and early-return instead of silently adding Eq"
    rationale: "User opted into selective deriving; respect that with a clear error and suggestion"
patterns-established:
  - "Trait dependency validation pattern in type checker (extensible to future trait prerequisites)"
duration: 6min
completed: 2026-02-09
---

# Phase 28 Plan 01: Trait Deriving Safety Summary

Compile-time Ord-requires-Eq enforcement via MissingDerivePrerequisite error (E0029) with help text suggesting `deriving(Eq, Ord)`

## What Was Done

### Task 1: Add MissingDerivePrerequisite error variant, diagnostics, and validation checks

Added `MissingDerivePrerequisite` variant to the `TypeError` enum with three fields (`trait_name`, `requires`, `type_name`) and a `Display` impl. Assigned error code E0029 in the diagnostics module with full ariadne report rendering including a `.with_help()` suggesting the fix. Added validation checks in both `register_struct_def` and `register_sum_type_def` that detect `deriving(Ord)` without `Eq` and early-return to prevent broken MIR generation. Also updated `snow-lsp` match exhaustiveness for the new variant.

**Commit:** `f0bc507` feat(28-01): add MissingDerivePrerequisite error variant, diagnostics, and validation

### Task 2: Add e2e tests for trait deriving safety

Added three e2e tests covering all success criteria:
- `e2e_deriving_ord_without_eq_struct`: struct with `deriving(Ord)` produces compile error mentioning Eq
- `e2e_deriving_ord_without_eq_sum`: sum type with `deriving(Ord)` produces compile error mentioning Eq
- `e2e_deriving_eq_ord_together`: `deriving(Eq, Ord)` compiles and produces correct `false\ntrue\n` output

**Commit:** `c875e9b` test(28-01): add e2e tests for trait deriving safety

## Task Commits

| Task | Commit | Type | Description |
|------|--------|------|-------------|
| 1 | f0bc507 | feat | Error variant, diagnostics, validation checks |
| 2 | c875e9b | test | E2E tests for struct, sum type, and Eq+Ord together |

## Decisions Made

| ID | Decision | Rationale |
|----|----------|-----------|
| 28-01-D1 | Emit error and early-return instead of silently adding Eq | User opted into selective deriving; respect that with a clear error and suggestion |

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Updated snow-lsp match exhaustiveness**

- **Found during:** Task 1
- **Issue:** Adding a new `TypeError` variant caused a non-exhaustive match in `crates/snow-lsp/src/analysis.rs` (`type_error_span` function)
- **Fix:** Added `TypeError::MissingDerivePrerequisite { .. } => None` to the match
- **Files modified:** `crates/snow-lsp/src/analysis.rs`
- **Commit:** f0bc507

## Verification Results

- `cargo build`: PASS (full workspace compiles)
- `cargo test -p snow-typeck`: PASS (233 tests, 0 failures)
- `cargo test -p snowc --test e2e`: PASS (40 tests, 0 failures)
- All 9 deriving tests pass (6 existing + 3 new)
- Existing backward compat test (`e2e_deriving_backward_compat`) passes: no deriving clause still derives all defaults
- Existing selective deriving test (`e2e_deriving_selective`) passes: `deriving(Eq)` alone works fine

## Success Criteria Verification

- [x] DERIVE-01: `deriving(Ord)` without `Eq` on both structs and sum types emits error code E0029 at compile time
- [x] DERIVE-02: `deriving(Eq, Ord)` compiles and runs correctly, producing correct equality and ordering results
- [x] DERIVE-03: Error diagnostic includes help text suggesting `add Eq to the deriving list: deriving(Eq, Ord)`
- [x] No regression in any existing tests

## Self-Check: PASSED
