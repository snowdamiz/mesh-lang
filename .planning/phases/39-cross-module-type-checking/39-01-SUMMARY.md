---
phase: 39-cross-module-type-checking
plan: 01
subsystem: typeck
tags: [cross-module, type-checking, imports, exports, HM-inference]

# Dependency graph
requires:
  - phase: 38-multi-file-build
    provides: "build_project pipeline, ProjectData with parsed modules"
provides:
  - "ImportContext, ModuleExports, ExportedSymbols types for cross-module type info"
  - "check_with_imports() entry point for multi-module type checking"
  - "collect_exports() for extracting exports from type-checked modules"
  - "infer_with_imports() with pre-seeding of traits, structs, sum types"
  - "TraitRegistry accessor methods (trait_defs, all_impls)"
  - "TypeError::ImportModuleNotFound, ImportNameNotFound with diagnostics"
  - "pub TypeRegistry methods and register_variant_constructors"
affects: [39-02, 39-03, phase-40]

# Tech tracking
tech-stack:
  added: []
  patterns: ["accumulator pattern for cross-module type pre-seeding", "empty context delegation for backward compat"]

key-files:
  created: []
  modified:
    - "crates/snow-typeck/src/lib.rs"
    - "crates/snow-typeck/src/infer.rs"
    - "crates/snow-typeck/src/traits.rs"
    - "crates/snow-typeck/src/error.rs"
    - "crates/snow-typeck/src/diagnostics.rs"

key-decisions:
  - "infer() delegates to infer_with_imports(empty) to avoid code duplication"
  - "import_ctx threaded as unused param through infer_item/infer_multi_clause_fn (Plan 02 will use)"
  - "Block-level infer_multi_clause_fn uses ImportContext::empty() since blocks are not top-level"
  - "collect_exports extracts impl trait/type names from AST PATH nodes matching infer_impl_def pattern"
  - "Error codes E0031 (ImportModuleNotFound) and E0034 (ImportNameNotFound) chosen to avoid gaps"

patterns-established:
  - "Pre-seeding pattern: trait defs/impls, struct defs, sum type defs injected before inference"
  - "ImportContext::empty() as backward-compatible default for single-file mode"

# Metrics
duration: 7min
completed: 2026-02-09
---

# Phase 39 Plan 01: Cross-Module Type Checking Foundation Summary

**ImportContext/ModuleExports/ExportedSymbols types with check_with_imports and collect_exports entry points for accumulator-pattern cross-module type checking**

## Performance

- **Duration:** 7 min
- **Started:** 2026-02-09T20:46:06Z
- **Completed:** 2026-02-09T20:53:15Z
- **Tasks:** 2
- **Files modified:** 5

## Accomplishments
- ImportContext, ModuleExports, ExportedSymbols types with all fields for cross-module type info exchange
- check_with_imports() and collect_exports() public API functions for the build pipeline
- infer_with_imports() pre-seeds TraitRegistry, TypeRegistry, and TypeEnv with imported symbols
- TraitRegistry::trait_defs() and all_impls() accessor methods for export collection
- TypeRegistry::register_struct/register_sum_type and register_variant_constructors made pub
- ImportModuleNotFound and ImportNameNotFound error variants with ariadne diagnostic rendering

## Task Commits

Each task was committed atomically:

1. **Task 1: Add ImportContext, ExportedSymbols, check_with_imports, and collect_exports** - `dfc9d16` (feat)
2. **Task 2: Add ImportModuleNotFound and ImportNameNotFound error variants with diagnostics** - `3701be8` (feat)

## Files Created/Modified
- `crates/snow-typeck/src/lib.rs` - ImportContext, ModuleExports, ExportedSymbols types, check_with_imports, collect_exports, register_variant_constructors re-export
- `crates/snow-typeck/src/infer.rs` - infer_with_imports with pre-seeding, pub TypeRegistry methods, pub register_variant_constructors, import_ctx threading
- `crates/snow-typeck/src/traits.rs` - trait_defs() and all_impls() accessor methods on TraitRegistry
- `crates/snow-typeck/src/error.rs` - ImportModuleNotFound and ImportNameNotFound variants with Display
- `crates/snow-typeck/src/diagnostics.rs` - Ariadne and JSON diagnostic rendering for new error variants (E0031, E0034)

## Decisions Made
- infer() delegates to infer_with_imports(ImportContext::empty()) to avoid code duplication while preserving backward compatibility
- import_ctx parameter threaded through infer_item and infer_multi_clause_fn as unused parameter (Plan 02 extends)
- Block-level multi-clause fn inference uses ImportContext::empty() since blocks are always within a single module
- collect_exports extracts impl trait/type names from AST PATH nodes using the same pattern as infer_impl_def
- Chose error codes E0031 and E0034 (next available after E0030 and E0033 respectively)

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Fixed collect_exports impl trait extraction**
- **Found during:** Task 1
- **Issue:** Plan used `impl_def.trait_name()` and `impl_def.target_type()` which don't exist on the parser AST ImplDef node
- **Fix:** Extracted trait and type names from PATH child nodes following the same pattern used in infer_impl_def
- **Files modified:** crates/snow-typeck/src/lib.rs
- **Committed in:** dfc9d16 (Task 1 commit)

---

**Total deviations:** 1 auto-fixed (1 bug fix)
**Impact on plan:** Necessary correction for the collect_exports function to compile. No scope change.

## Issues Encountered
- Disk space exhaustion during test compilation required `cargo clean` before running full test suite. All tests (235 in snow-typeck) passed after cleanup.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- All cross-module type checking interface types are in place
- check_with_imports is ready for Plan 02 to integrate into the build pipeline
- collect_exports is ready for Plan 02 to build the accumulator pattern
- import_ctx parameter is threaded but unused -- Plan 02 will extend infer_item to resolve imports

---
*Phase: 39-cross-module-type-checking*
*Plan: 01*
*Completed: 2026-02-09*
