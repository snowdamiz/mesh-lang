---
phase: 42-diagnostics-integration
plan: 02
subsystem: diagnostics
tags: [module-qualified-types, display-prefix, tycon, multi-module, e2e-tests, type-errors]

# Dependency graph
requires:
  - phase: 42-diagnostics-integration
    plan: 01
    provides: "Named-source ariadne diagnostics showing file paths"
  - phase: 39-cross-module-typeck
    provides: "ImportContext, ModuleExports, cross-module type checking"
provides:
  - "Module-qualified type names in error messages (Geometry.Point instead of Point)"
  - "TyCon::display_prefix field for display-only module qualification"
  - "current_module threading from build pipeline through type checker"
  - "Comprehensive 4-module E2E integration test validating complete module system"
affects: [compiler-diagnostics, editor-tooling, error-messages]

# Tech tracking
tech-stack:
  added: []
  patterns: ["TyCon::with_module() for display-only module qualification", "Manual PartialEq/Hash on TyCon excluding display_prefix"]

key-files:
  created: []
  modified:
    - crates/snow-typeck/src/ty.rs
    - crates/snow-typeck/src/lib.rs
    - crates/snow-typeck/src/infer.rs
    - crates/snow-typeck/src/unify.rs
    - crates/snowc/src/main.rs
    - crates/snow-codegen/src/mir/types.rs
    - crates/snowc/tests/e2e.rs

key-decisions:
  - "display_prefix on TyCon excluded from PartialEq/Hash to preserve type identity -- unqualified names for type checking, qualified names for display only"
  - "current_module threaded via ImportContext and stored on InferCtx for local type qualification"
  - "Struct literal return type lookups env TyCon to preserve display_prefix from import resolution"

patterns-established:
  - "TyCon::with_module(name, module) for module-qualified display in error messages"
  - "Local types get display_prefix from ctx.current_module in multi-module mode"
  - "Imported types get display_prefix from the source module namespace name"

# Metrics
duration: 8min
completed: 2026-02-09
---

# Phase 42 Plan 02: Module-Qualified Type Display Summary

**TyCon display_prefix for module-qualified type names in errors (Geometry.Point) with comprehensive 4-module E2E integration test**

## Performance

- **Duration:** 8 min
- **Started:** 2026-02-09T23:35:29Z
- **Completed:** 2026-02-09T23:43:51Z
- **Tasks:** 2
- **Files modified:** 7

## Accomplishments
- Added display_prefix field to TyCon with manual PartialEq/Hash excluding it (preserves type identity)
- Imported types display module-qualified names in error messages (e.g., "Geometry.Point")
- Local types show current module prefix in multi-module mode
- Builtins never show module prefix (backward compatible)
- Single-file programs unchanged (display_prefix is None)
- Comprehensive 4-module E2E test validates structs, cross-module calls, nested paths, qualified access

## Task Commits

Each task was committed atomically:

1. **Task 1: Add display_prefix to TyCon and thread current_module** - `b4d19ba` (feat)
2. **Task 2: Comprehensive multi-module E2E tests and module-qualified error tests** - `04e000a` (test)

## Files Created/Modified
- `crates/snow-typeck/src/ty.rs` - TyCon with display_prefix field, manual PartialEq/Hash, with_module() constructor
- `crates/snow-typeck/src/lib.rs` - ImportContext.current_module field
- `crates/snow-typeck/src/infer.rs` - Display prefix set on imported/local struct types, struct literal env lookup
- `crates/snow-typeck/src/unify.rs` - InferCtx.current_module field
- `crates/snowc/src/main.rs` - current_module threaded from project.graph module name
- `crates/snow-codegen/src/mir/types.rs` - Fixed TyCon struct literal to use TyCon::new()
- `crates/snowc/tests/e2e.rs` - 3 new E2E tests (comprehensive integration, module-qualified error, file path error)

## Decisions Made
- Used display_prefix (display-only) approach instead of qualifying TyCon.name -- avoids breaking codegen, MIR lowering, type registry lookups
- Struct literal infer_struct_literal looks up TyCon from env to preserve display_prefix from import resolution
- current_module set on both ImportContext (input from driver) and InferCtx (accessible during inference)

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Fixed codegen TyCon struct literal**
- **Found during:** Task 1
- **Issue:** snow-codegen/src/mir/types.rs constructed TyCon with struct literal syntax `TyCon { name: ... }`, missing the new display_prefix field
- **Fix:** Changed to use `TyCon::new(base_name)` constructor
- **Files modified:** crates/snow-codegen/src/mir/types.rs
- **Verification:** `cargo build -p snowc` succeeds
- **Committed in:** b4d19ba (Task 1 commit)

**2. [Rule 1 - Bug] Fixed struct literal display_prefix propagation**
- **Found during:** Task 2
- **Issue:** Struct literal return type in infer_struct_literal always used TyCon::new() (no display_prefix), so type errors from struct literal expressions showed bare type names
- **Fix:** Added env lookup to extract TyCon with display_prefix from the registered struct type scheme
- **Files modified:** crates/snow-typeck/src/infer.rs
- **Verification:** e2e_module_qualified_type_in_error test passes with "Geometry.Point" in error
- **Committed in:** 04e000a (Task 2 commit)

---

**Total deviations:** 2 auto-fixed (1 blocking, 1 bug)
**Impact on plan:** Both auto-fixes necessary for correctness. No scope creep.

## Issues Encountered
None.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- DIAG-01 complete: file paths in diagnostics (42-01)
- DIAG-02 complete: module-qualified type names in errors (42-02)
- DIAG-03 complete: single-file backward compatibility (existing tests)
- Success Criterion 3 complete: comprehensive multi-module project compiles and runs correctly
- Phase 42 (Diagnostics & Integration) fully complete
- v1.8 Module System milestone fully complete

## Self-Check: PASSED

All files exist. All commits verified. All 3 new E2E tests pass. All 111 E2E tests pass. All 235 snow-typeck tests pass. Zero regressions.

---
*Phase: 42-diagnostics-integration*
*Completed: 2026-02-09*
