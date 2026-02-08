---
phase: 23-pattern-matching-codegen
plan: 01
subsystem: codegen
tags: [pattern-matching, llvm, sum-types, maranget, decision-tree]

# Dependency graph
requires:
  - phase: 18-sum-types
    provides: "MirSumTypeDef, MirVariantDef, ConstructVariant codegen"
  - phase: 20-pattern-matching
    provides: "Maranget pattern compiler, decision tree codegen"
provides:
  - "Correct constructor tag assignment from sum type definitions"
  - "Correct variant field type resolution from sum type definitions"
  - "sum_type_defs parameter threaded through compile_match pipeline"
affects: [23-02, 24-ordering-compare, pattern-matching-codegen]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "sum_type_defs passed as &FxHashMap<String, MirSumTypeDef> through pattern compilation"
    - "Fallback tag resolution for types not in sum_type_defs map (backwards compatibility)"

key-files:
  created: []
  modified:
    - "crates/snow-codegen/src/pattern/compile.rs"
    - "crates/snow-codegen/src/codegen/expr.rs"

key-decisions:
  - "Thread sum_type_defs as separate parameter rather than embedding in PatMatrix struct"
  - "Fallback to appearance-order tags when type not found in sum_type_defs (defensive)"
  - "Monomorphized type name lookup: try exact name then split on underscore for base name"

patterns-established:
  - "Pattern compiler receives sum_type_defs for correct tag and type resolution"

# Metrics
duration: 5min
completed: 2026-02-08
---

# Phase 23 Plan 01: Pattern Compiler Tag/Type Fix Summary

**Fixed constructor tag mismatch and Unit placeholder field types in pattern compiler by threading sum_type_defs through compile_match pipeline**

## Performance

- **Duration:** 5 min
- **Started:** 2026-02-08T17:53:58Z
- **Completed:** 2026-02-08T17:59:06Z
- **Tasks:** 1
- **Files modified:** 2

## Accomplishments
- Constructor tags in decision trees now match the sum type definition order, not the pattern appearance order
- Variant field bindings get correct types from MirSumTypeDef instead of MirType::Unit placeholders
- Added 2 new tests validating tag correctness and field type correctness
- All 141 tests pass (16 pattern compiler tests including 2 new ones)

## Task Commits

Each task was committed atomically:

1. **Task 1: Fix constructor tag assignment and field type resolution** - `ca67463` (feat)

**Plan metadata:** (pending)

## Files Created/Modified
- `crates/snow-codegen/src/pattern/compile.rs` - Added sum_type_defs parameter to compile_match and all internal functions; fixed tag lookup in collect_head_constructors; fixed field type resolution in specialize_for_constructor; added 2 new tests
- `crates/snow-codegen/src/codegen/expr.rs` - Updated compile_match call site to pass self.sum_type_defs

## Decisions Made
- Threaded `sum_type_defs: &FxHashMap<String, MirSumTypeDef>` as a separate parameter through all functions rather than embedding it in the PatMatrix struct. PatMatrix is cloned frequently; a reference parameter avoids unnecessary data duplication.
- Used a fallback strategy for tag lookup: if the type name is not found in sum_type_defs (e.g., in tests with ad-hoc types), falls back to the old appearance-order counting. This preserves backward compatibility for the 14 existing tests.
- For monomorphized generic types like `Option_Int`, the lookup tries the exact name first, then splits on `_` to try the base name `Option`. This matches the existing pattern used in the codegen layer.

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered

None.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness
- Pattern compiler now correctly assigns tags and field types from sum type definitions
- Ready for Plan 02 (Ordering type registration and compare method)
- The `compile_match` signature change is the key API boundary; any future callers must pass sum_type_defs

## Self-Check: PASSED

---
*Phase: 23-pattern-matching-codegen*
*Completed: 2026-02-08*
