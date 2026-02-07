---
phase: 11-multi-clause-functions
plan: 02
subsystem: typechecker
tags: [typeck, multi-clause, pattern-matching, guard-clause, desugaring, exhaustiveness]

# Dependency graph
requires:
  - phase: 11-01
    provides: "Parser support for fn name(pattern) [when guard] = expr syntax, FnDef::guard(), expr_body(), has_eq_body(), Param::pattern()"
  - phase: 04-pattern-matching
    provides: "Pattern inference (infer_pattern), exhaustiveness checking, abstract pattern conversion"
provides:
  - "Multi-clause function grouping (group_multi_clause_fns) collapses consecutive same-name, same-arity FnDef nodes"
  - "Multi-clause function type checking (infer_multi_clause_fn) with clause body type unification"
  - "Exhaustiveness and redundancy checking applied to multi-clause function patterns"
  - "Catch-all-not-last validation as compiler error"
  - "Non-consecutive same-name clause detection as compiler error"
  - "Relaxed guard validation for multi-clause functions (arbitrary Bool expressions)"
  - "New error variants: CatchAllNotLast, NonConsecutiveClauses, ClauseArityMismatch, NonFirstClauseAnnotation, GuardTypeMismatch"
affects:
  - 11-03 (codegen relies on desugared output from type checker)

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "GroupedItem enum separates Single items from MultiClause groups before inference"
    - "Pre-pass grouping in both top-level infer() and infer_block() via group_multi_clause_fns"
    - "Multi-clause desugaring mirrors infer_case: per-clause scope, pattern inference, body type unification, exhaustiveness"
    - "is_catch_all_clause detects wildcard/ident-only params with no guard"
    - "ChildKind/BlockChildKind enums track source ordering during grouped inference"

key-files:
  created: []
  modified:
    - "crates/snow-typeck/src/infer.rs"
    - "crates/snow-typeck/src/error.rs"
    - "crates/snow-typeck/src/diagnostics.rs"
    - "crates/snow-lsp/src/analysis.rs"

key-decisions:
  - "Single FnDef with = expr body treated as 1-clause MultiClause; do/end stays as Single"
  - "Guards in multi-clause functions skip validate_guard_expr, just infer_expr + Bool unify"
  - "Exhaustiveness warnings (not errors) for non-exhaustive multi-clause patterns"
  - "Non-first clause annotations (visibility, generics, return type) produce warnings not errors"

patterns-established:
  - "GroupedItem pre-pass pattern: collect items, group, map indices, process in source order"
  - "Multi-clause inference reuses existing pattern/exhaustiveness infrastructure without AST node creation"

# Metrics
duration: 8min
completed: 2026-02-07
---

# Phase 11 Plan 02: Multi-Clause Function Type Checking Summary

**Multi-clause function grouping, desugaring to case-expression-style inference, exhaustiveness checking, catch-all validation, and relaxed guard expressions**

## Performance

- **Duration:** 8 min
- **Started:** 2026-02-07T19:40:15Z
- **Completed:** 2026-02-07T19:48:33Z
- **Tasks:** 2
- **Files modified:** 4

## Accomplishments
- Consecutive same-name, same-arity FnDef nodes are grouped and type-checked as a single function with pattern matching
- Return types unified across all clauses (Int + String = type error)
- Exhaustiveness checking applied to multi-clause function parameter patterns
- Catch-all clause not last produces compiler error with clear diagnostic
- Guards in multi-clause functions accept arbitrary Bool expressions (function calls, arithmetic, comparisons)
- Non-consecutive same-name function definitions detected and reported as error
- All 218+ existing tests pass with zero regressions; full workspace compiles cleanly

## Task Commits

Each task was committed atomically:

1. **Task 1: Implement clause grouping and desugaring in the type checker** - `ae695d2` (feat)
2. **Task 2: Relax guard validation and add multi-clause error diagnostics** - `a10a2c0` (feat)

## Files Created/Modified
- `crates/snow-typeck/src/infer.rs` - GroupedItem enum, group_multi_clause_fns, infer_multi_clause_fn, is_catch_all_clause, check_non_consecutive_clauses, integrated grouping into infer() and infer_block()
- `crates/snow-typeck/src/error.rs` - 5 new error variants: CatchAllNotLast, NonConsecutiveClauses, ClauseArityMismatch, NonFirstClauseAnnotation, GuardTypeMismatch
- `crates/snow-typeck/src/diagnostics.rs` - Error codes E0022-E0025 and W0002, diagnostic rendering for all new variants
- `crates/snow-lsp/src/analysis.rs` - LSP span extraction for new error variants

## Decisions Made
- Single `= expr` functions are treated as 1-clause MultiClause groups (consistent desugaring path)
- Multi-clause function guards bypass validate_guard_expr entirely, using only infer_expr + Bool unification
- Exhaustiveness non-exhaustive result is a warning (not error) for multi-clause functions, matching case expression behavior
- Non-first clause annotations (pub, generics, return type, where) produce warnings rather than errors for gentle user experience

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Fixed non-exhaustive match in snow-lsp analysis.rs**
- **Found during:** Task 1
- **Issue:** Adding new TypeError variants caused non-exhaustive match in LSP span extraction
- **Fix:** Added match arms for all 5 new variants in type_error_span()
- **Files modified:** crates/snow-lsp/src/analysis.rs
- **Verification:** Full workspace build succeeds
- **Committed in:** ae695d2 (Task 1 commit)

---

**Total deviations:** 1 auto-fixed (1 blocking)
**Impact on plan:** Necessary to keep the workspace compiling. No scope creep.

## Issues Encountered
None

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Type checker fully handles multi-clause function grouping, desugaring, and validation
- Ready for 11-03 (codegen) which needs to handle the desugared function output
- The codegen will need to recognize multi-clause functions (multiple FnDef with same name) and generate appropriate match dispatch code

## Self-Check: PASSED

---
*Phase: 11-multi-clause-functions*
*Completed: 2026-02-07*
