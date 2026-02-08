---
phase: 20
plan: 01
subsystem: typeck
tags: [unification, display, trait-registry, type-identity]
depends_on:
  requires: [18, 19]
  provides: [con-app-unification, display-trait-primitive-impls]
  affects: [20-02, 20-03, 20-04, 20-05]
tech_stack:
  added: []
  patterns: [con-app-type-identity-unification]
key_files:
  created: []
  modified:
    - crates/snow-typeck/src/unify.rs
    - crates/snow-typeck/src/builtins.rs
decisions:
  - id: 20-01-01
    decision: "Con(c) unifies with App(Con(c), []) bidirectionally for non-generic struct types"
    rationale: "infer_struct_literal returns App(Con, []) while name_to_type returns Con for the same type; both representations are semantically identical"
  - id: 20-01-02
    decision: "Display trait registered as compiler-known with to_string(self) -> String signature"
    rationale: "Follows exact pattern of Eq trait registration; enables trait dispatch for string conversion"
metrics:
  duration: 6min
  completed: 2026-02-08
---

# Phase 20 Plan 01: Typeck Identity Fix + Display Trait Registration Summary

**One-liner:** Fixed Con vs App(Con, []) unification gap blocking all user-defined trait impls; registered Display trait with primitive impls for to_string dispatch.

## What Was Done

### Task 1: Fix typeck Ty::Con vs Ty::App(Con, []) unification gap

Added a new match arm in `InferCtx::unify()` in `unify.rs` that handles the case where `Ty::Con("Point")` needs to unify with `Ty::App(Con("Point"), [])`. This arises because `infer_struct_literal()` returns `App(Con(name), [])` for non-generic structs while `name_to_type()` returns `Con(name)` -- both represent the same type but were previously incompatible in the unifier.

The new case is placed after the existing Pid escape hatch and before the `(App, App)` case. It matches bidirectionally (Con on either side) and requires the args list to be empty (prevents false unification of `Con("List")` with `App(Con("List"), [Int])`).

Two unit tests added:
- `con_unifies_with_app_con_empty_args`: verifies bidirectional unification
- `con_does_not_unify_with_app_con_nonempty_args`: verifies non-empty args are rejected

### Task 2: Register Display trait and primitive impls

Added Display trait definition and primitive implementations in `register_compiler_known_traits()` in `builtins.rs`, following the exact pattern established by the Eq trait:

1. Registered `Display` trait with single method `to_string(self) -> String` (has_self=true, param_count=0)
2. Registered impls for Int, Float, String, Bool
3. Added test `display_trait_registered_for_primitives` verifying all four primitives have Display impls and `find_method_traits("to_string", &Ty::int())` returns `["Display"]`

## Task Commits

| Task | Name | Commit | Key Changes |
|------|------|--------|-------------|
| 1 | Fix typeck Con vs App unification gap | 47071a4 | unify.rs: +39 lines (new match arm + 2 tests) |
| 2 | Register Display trait and primitive impls | 431e703 | builtins.rs: +52 lines (trait def + 4 impls + test) |

## Verification Results

- `cargo test --workspace`: 1,127 tests pass, 0 failures (zero regressions)
- `cargo test -p snow-typeck -- con_unifies_with_app`: PASS
- `cargo test -p snow-typeck -- display_trait_registered`: PASS
- New unification case correctly handles bidirectional Con/App identity
- Display trait findable via `find_method_traits("to_string", &ty)` for all primitives

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] ConstraintOrigin::Assignment is a struct variant**

- **Found during:** Task 1 (writing tests)
- **Issue:** Plan specified `ConstraintOrigin::Assignment` as a unit variant, but it's actually a struct variant requiring fields
- **Fix:** Used `builtin_origin()` helper (already defined in test module) instead
- **Files modified:** crates/snow-typeck/src/unify.rs (test code only)
- **Commit:** 47071a4

## Decisions Made

| ID | Decision | Rationale |
|----|----------|-----------|
| 20-01-01 | Con(c) unifies with App(Con(c), []) bidirectionally | Both representations are semantically identical; fix is strictly more permissive |
| 20-01-02 | Display registered as compiler-known trait | Follows Eq pattern; enables trait dispatch for to_string calls |

## Next Phase Readiness

**Unblocked:** The typeck identity gap that blocked all user-defined trait impls at call sites is now resolved. Phases 20-02 through 20-05 can proceed without the "expected X, found X" error.

**Ready for:** Display string interpolation dispatch (20-02), Eq/Ord for user-defined types (20-04, 20-05).

## Self-Check: PASSED
