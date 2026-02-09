---
phase: 39-cross-module-type-checking
plan: 03
subsystem: compiler
tags: [cross-module, type-checking, mir-merge, codegen, multi-file, imports]

# Dependency graph
requires:
  - phase: 39-01
    provides: ImportContext, ExportedSymbols, check_with_imports, collect_exports
  - phase: 39-02
    provides: ModuleGraph, compilation_order, topological sort, module discovery
provides:
  - Multi-module build pipeline with accumulator pattern type checking
  - MIR merge for multi-module codegen (merge_mir_modules)
  - User module qualified access in MIR lowerer
  - 11 cross-module E2E tests covering all import scenarios
affects: [phase-40, phase-41]

# Tech tracking
tech-stack:
  added: []
  patterns: [accumulator-pattern-typeck, mir-merge-codegen, user-module-qualified-access]

key-files:
  created: []
  modified:
    - crates/snowc/src/main.rs
    - crates/snow-codegen/src/lib.rs
    - crates/snow-codegen/src/mir/lower.rs
    - crates/snow-typeck/src/lib.rs
    - crates/snow-typeck/src/infer.rs
    - crates/snow-typeck/src/unify.rs
    - crates/snowc/tests/e2e.rs

key-decisions:
  - "Lower all modules to raw MIR (no per-module monomorphization), merge into single MirModule, then monomorphize once"
  - "Track qualified_modules and imported_functions in TypeckResult for MIR lowerer awareness"
  - "Skip trait dispatch for imported function names to prevent mangling (e.g., add -> Add__add__Int)"
  - "Deduplicate MIR functions/structs/sum_types by name during merge"

patterns-established:
  - "Accumulator pattern: type-check all modules in topological order, accumulating exports"
  - "MIR merge: lower_to_mir_raw per module, merge_mir_modules with dedup, monomorphize merged"
  - "User module dispatch: user_modules HashMap checked in is_module_or_special and lower_field_access"

# Metrics
duration: 28min
completed: 2026-02-09
---

# Phase 39 Plan 03: Build Pipeline Integration Summary

**Multi-module type checking with accumulator pattern, MIR merge for cross-module codegen, and 11 E2E tests**

## Performance

- **Duration:** ~28 min
- **Started:** 2026-02-09T21:05:00Z
- **Completed:** 2026-02-09T21:33:00Z
- **Tasks:** 2
- **Files modified:** 7

## Accomplishments
- Build pipeline type-checks ALL modules in topological order using accumulator pattern
- MIR merge enables cross-module function calls to work through codegen and runtime
- 11 comprehensive E2E tests cover qualified access, selective import, structs, sum types, nested modules, error cases, and regression
- Zero test regressions across entire workspace (94 E2E tests, 1000+ unit tests)

## Task Commits

Each task was committed atomically:

1. **Task 1: Accumulator pattern build pipeline** - `dd1ae18` (feat)
2. **Task 1b: MIR merge and codegen integration** - `0903160` (fix, Rule 3 deviation)
3. **Task 2: Cross-module E2E tests** - `0b999a8` (test)

## Files Created/Modified
- `crates/snowc/src/main.rs` - Build pipeline with accumulator pattern type checking, build_import_context(), MIR merge integration
- `crates/snow-codegen/src/lib.rs` - lower_to_mir_raw(), merge_mir_modules(), compile_mir_to_binary(), compile_mir_to_llvm_ir()
- `crates/snow-codegen/src/mir/lower.rs` - user_modules, imported_functions in Lowerer; qualified access for user modules; trait dispatch skip
- `crates/snow-typeck/src/lib.rs` - TypeckResult.qualified_modules, TypeckResult.imported_functions
- `crates/snow-typeck/src/infer.rs` - Populate qualified_modules and imported_functions from InferCtx
- `crates/snow-typeck/src/unify.rs` - InferCtx.imported_functions tracking
- `crates/snowc/tests/e2e.rs` - compile_multifile_and_run/expect_error helpers, 11 new tests

## Decisions Made
- Used `lower_to_mir_raw()` (no monomorphization) per module, then `merge_mir_modules()` runs monomorphization once on the merged MIR. This prevents unreachable builtin functions (like `Ord__compare__String`) from causing codegen failures in library modules that have no `main()`.
- Imported function names tracked separately from qualified module functions. Selective imports (`from Module import fn`) register in `imported_functions` to prevent the MIR lowerer from routing them through trait dispatch.
- User modules checked in `is_module_or_special` alongside stdlib modules and service modules to ensure `Math.add(2, 3)` is routed through `lower_field_access` instead of trait method dispatch.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Multi-module codegen required MIR merge pipeline**
- **Found during:** Task 2 (E2E test execution)
- **Issue:** The plan's codegen path compiled only the entry module, so cross-module function calls produced "Undefined variable" errors at link time
- **Fix:** Added `lower_to_mir_raw()`, `merge_mir_modules()`, `compile_mir_to_binary()` to snow-codegen. Updated build() to lower all modules to raw MIR, merge into single module, then compile.
- **Files modified:** crates/snow-codegen/src/lib.rs, crates/snowc/src/main.rs
- **Verification:** Cross-module function call (Math.add(2, 3)) produces correct output "5"
- **Committed in:** 0903160

**2. [Rule 3 - Blocking] MIR lowerer unaware of user-defined modules**
- **Found during:** Task 2 (E2E test execution)
- **Issue:** `Math.add(2, 3)` was treated as method call (trait dispatch) instead of module-qualified function call. Also, `from Math import add` functions went through trait dispatch, producing `Add__add__Int` instead of `add`.
- **Fix:** Added `user_modules` and `imported_functions` to MIR Lowerer. Extended `is_module_or_special` check. Added `qualified_modules` and `imported_functions` to TypeckResult.
- **Files modified:** crates/snow-codegen/src/mir/lower.rs, crates/snow-typeck/src/lib.rs, crates/snow-typeck/src/infer.rs, crates/snow-typeck/src/unify.rs
- **Verification:** Both qualified (`Math.add(2,3)`) and unqualified (`add(10,20)`) cross-module calls work correctly
- **Committed in:** 0903160

**3. [Rule 1 - Bug] Snow syntax corrections in E2E tests**
- **Found during:** Task 2 (E2E test writing)
- **Issue:** Plan used `:: RetType` (Haskell-style) for return types instead of `-> RetType`, `{curly braces}` for import lists instead of bare identifiers, and `= expr` body form with typed params
- **Fix:** Corrected all tests to use proper Snow syntax: `-> RetType`, `from Module import name1, name2`, `do...end` blocks
- **Files modified:** crates/snowc/tests/e2e.rs
- **Verification:** All tests parse and compile correctly
- **Committed in:** 0b999a8

---

**Total deviations:** 3 auto-fixed (1 bug, 2 blocking)
**Impact on plan:** All deviations were necessary for correctness. MIR merge is essential infrastructure for multi-module codegen. No scope creep.

## Issues Encountered
- Per-module monomorphization eliminated unreachable builtin functions in single-file builds but kept them in library modules (no entry function -> all functions reachable). Resolved by deferring monomorphization to the merged module.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Full cross-module type checking and codegen pipeline is operational
- All XMOD and IMPORT requirements verified through E2E tests
- Ready for Phase 40 (if planned) or Phase 41 (MIR optimizations)

---
*Phase: 39-cross-module-type-checking*
*Completed: 2026-02-09*
