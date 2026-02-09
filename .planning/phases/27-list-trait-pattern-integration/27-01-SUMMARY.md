---
phase: 27
plan: 01
subsystem: codegen-runtime
tags: [list, eq, ord, display, debug, callback, mir, runtime]
dependency-graph:
  requires: [26-01, 26-02]
  provides: [list-eq, list-ord, list-display-string, list-debug]
  affects: [27-02]
tech-stack:
  added: []
  patterns: [callback-based-element-comparison, synthetic-mir-wrapper-generation]
key-files:
  created:
    - tests/e2e/list_display_string.snow
    - tests/e2e/list_eq.snow
    - tests/e2e/list_ord.snow
    - tests/e2e/list_debug.snow
  modified:
    - crates/snow-rt/src/collections/list.rs
    - crates/snow-codegen/src/codegen/intrinsics.rs
    - crates/snow-codegen/src/mir/lower.rs
    - crates/snow-typeck/src/builtins.rs
    - crates/snowc/tests/e2e_stdlib.rs
decisions:
  - id: 27-01-D1
    decision: "Callback-based element comparison for snow_list_eq and snow_list_compare"
    rationale: "Matches snow_list_to_string pattern -- runtime receives fn ptr, MIR generates type-specific callback wrappers"
  - id: 27-01-D2
    decision: "Register parametric Eq/Ord impls for List<T> in typeck builtins using single-letter type param"
    rationale: "freshen_type_params treats single uppercase letters as type params, enabling List<Int>, List<String>, etc. to unify"
  - id: 27-01-D3
    decision: "Reuse wrap_collection_to_string for debug/inspect dispatch on collections"
    rationale: "List debug and display use same [elem1, elem2, ...] format"
metrics:
  duration: 17min
  completed: 2026-02-09
---

# Phase 27 Plan 01: List Trait Pattern Integration Summary

Callback-based list equality (snow_list_eq) and lexicographic comparison (snow_list_compare) with synthetic MIR wrapper generation for element callbacks, plus Eq/Ord typeck registration for List<T>

## Task Commits

| Task | Name | Commit | Key Changes |
|------|------|--------|-------------|
| 1 | Runtime functions + LLVM declarations | 1372d77 | snow_list_eq, snow_list_compare in snow-rt; LLVM extern decls in intrinsics.rs; 6 unit tests |
| 2 | MIR dispatch + typeck + e2e tests | 294b345 | Binary op dispatch for Ptr/List in lower_binary_expr; eq/cmp callback generators; List<T> Eq/Ord in builtins; 4 e2e tests |

## What Was Built

### Runtime Layer (snow-rt)
- `snow_list_eq(list_a, list_b, elem_eq_callback) -> i8`: Element-wise equality using callback. Returns 1 if equal, 0 if not. Short-circuits on length mismatch.
- `snow_list_compare(list_a, list_b, elem_cmp_callback) -> i64`: Lexicographic comparison using callback. Returns negative/0/positive. Compares element-by-element up to min length, then by length.

### LLVM Declarations (intrinsics.rs)
- Registered both functions with correct signatures: `(ptr, ptr, ptr) -> i8` and `(ptr, ptr, ptr) -> i64`.

### MIR Lowering (lower.rs)
- **Binary operator dispatch**: When `lhs` is `MirType::Ptr` and typeck type is `List<T>`, dispatches `==`/`!=` to `snow_list_eq` and `<`/`>`/`<=`/`>=` to `snow_list_compare` with appropriate callback.
- **Synthetic callback generators**: `__eq_int_callback`, `__eq_float_callback`, `__eq_bool_callback`, `__eq_string_callback`, `__cmp_int_callback`, `__cmp_string_callback` -- each generated as MIR functions on first use, deduplicated via `known_functions`.
- **Nested list support**: `__eq_list_{inner}_callback` and `__cmp_list_{inner}_callback` for recursive comparison of nested lists.
- **`extract_list_elem_type`**: Free function to extract `T` from `Ty::App(Con("List"), [T])`.
- **Collection Display/Debug dispatch**: Extended to also handle `debug`/`inspect` calls on Ptr-typed collection args.

### Type Checking (builtins.rs)
- Registered parametric `Eq` impl for `List<T>` with `eq` method.
- Registered parametric `Ord` impl for `List<T>` with `lt` and `compare` methods.
- Uses `Ty::Con("T")` as type parameter -- `freshen_type_params` replaces it with fresh inference variables during trait checking.

### E2E Tests
- `list_display_string.snow`: `["hello", "world"]` renders as `[hello, world]` via string interpolation.
- `list_debug.snow`: `[1, 2, 3]` renders as `[1, 2, 3]` via string interpolation.
- `list_eq.snow`: `[1, 2, 3] == [1, 2, 3]` is true; `[1, 2, 3] != [1, 2, 4]` is true.
- `list_ord.snow`: `[1, 2] < [1, 3]` is true; `[1, 3] > [1, 2]` is true.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 2 - Missing Critical] Registered Eq/Ord trait impls for List<T> in typeck**
- **Found during:** Task 2 (e2e test failure)
- **Issue:** Typeck rejected `a == b` on lists because no Eq impl was registered for List<T>
- **Fix:** Added parametric Eq and Ord impls for `List<T>` in `crates/snow-typeck/src/builtins.rs`
- **Files modified:** `crates/snow-typeck/src/builtins.rs`
- **Commit:** 294b345

**2. [Rule 1 - Bug] Fixed MirExpr::If field names**
- **Found during:** Task 2
- **Issue:** Plan used `condition/then_branch/else_branch` but MIR uses `cond/then_body/else_body`
- **Fix:** Used correct field names in generated callback functions
- **Files modified:** `crates/snow-codegen/src/mir/lower.rs`
- **Commit:** 294b345

**3. [Rule 1 - Bug] Adapted e2e test patterns to match Snow language API**
- **Found during:** Task 2
- **Issue:** Plan used `IO.puts()` and `debug()` which don't exist in Snow; Snow uses `println()` and `inspect()`
- **Fix:** Used `println("${xs}")` for display/debug tests; used `println()` for output
- **Files modified:** `tests/e2e/list_debug.snow`, `tests/e2e/list_display_string.snow`
- **Commit:** 294b345

## Decisions Made

| ID | Decision | Rationale |
|----|----------|-----------|
| 27-01-D1 | Callback-based element comparison for snow_list_eq/snow_list_compare | Matches existing snow_list_to_string callback pattern |
| 27-01-D2 | Parametric Eq/Ord impls for List<T> via single-letter type param | freshen_type_params unification enables matching any List<Concrete> |
| 27-01-D3 | Reuse wrap_collection_to_string for debug/inspect on collections | Same [elem1, elem2, ...] format for both Display and Debug on lists |

## Test Results

- `cargo test -p snow-rt -- list`: 26 passed (including 6 new)
- `cargo test -p snow-codegen -- intrinsic`: 3 passed
- `cargo test -p snow-codegen`: 152 passed, 0 failed
- `cargo test -p snowc -- e2e_list_display_string`: passed
- `cargo test -p snowc -- e2e_list_debug`: passed
- `cargo test -p snowc -- e2e_list_eq`: passed
- `cargo test -p snowc -- e2e_list_ord`: passed
- `cargo test`: all 39 test suites passed, 0 failures

## Next Phase Readiness

Phase 27 Plan 02 (Pattern Matching on Lists) can proceed. The Eq/Ord infrastructure established here provides the comparison foundation. No blockers.

## Self-Check: PASSED
