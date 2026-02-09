---
phase: 36-filter-clause-integration
plan: 01
subsystem: compiler
tags: [parser, ast, typeck, mir, codegen, formatter, for-in, filter, when-clause]

# Dependency graph
requires:
  - phase: 35-for-in-over-collections
    provides: Four-block loop pattern, list builder comprehension, ForIn MIR variants
provides:
  - Optional when filter clause parsing in for-in expressions
  - AST filter() accessor on ForInExpr
  - Bool unification for filter expressions in typeck
  - filter field on all 4 ForIn MIR variants
  - Five-block codegen pattern (header/body/do_body/latch/merge) when filter present
  - Filter traversal in collect_free_vars, collect_function_refs, compile_expr_patterns
  - WHEN_KW formatting in walk_for_in_expr
affects: [36-02-PLAN, integration-tests, e2e-tests]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Five-block loop pattern: header/body/do_body/latch/merge for filtered for-in"
    - "AST filter accessor via WHEN_KW token detection + nth(1) child expression"

key-files:
  created: []
  modified:
    - crates/snow-parser/src/parser/expressions.rs
    - crates/snow-parser/src/ast/expr.rs
    - crates/snow-typeck/src/infer.rs
    - crates/snow-fmt/src/walker.rs
    - crates/snow-codegen/src/mir/mod.rs
    - crates/snow-codegen/src/mir/lower.rs
    - crates/snow-codegen/src/mir/mono.rs
    - crates/snow-codegen/src/pattern/compile.rs
    - crates/snow-codegen/src/codegen/expr.rs
    - crates/snow-codegen/src/codegen/mod.rs

key-decisions:
  - "Filter parsed as direct child of FOR_IN_EXPR (no separate CST node), matching MatchArm guard pattern"
  - "Filter field placed between iterable/collection fields and body in MIR variants"
  - "Filter conditional branch creates forin_do_body block; false skips to latch_bb directly"

patterns-established:
  - "Five-block loop pattern: when filter present, body_bb splits into filter check + do_body_bb"
  - "Filter traversal pattern: all MIR walkers (free vars, function refs, patterns) traverse filter after loop variable binding"

# Metrics
duration: 10min
completed: 2026-02-09
---

# Phase 36 Plan 01: Filter Clause Pipeline Support Summary

**Optional `when` filter clause across all compiler stages: parser, AST, typeck, MIR, lowering, codegen, mono, pattern compilation, and formatter**

## Performance

- **Duration:** 10 min
- **Started:** 2026-02-09T15:56:04Z
- **Completed:** 2026-02-09T16:06:25Z
- **Tasks:** 2
- **Files modified:** 10

## Accomplishments
- Full pipeline support for `when condition` filter clause in for-in loops across all 4 variants (range, list, map, set)
- Parser accepts `for x in list when x > 0 do body end`, AST exposes filter() accessor, typeck validates Bool type
- Codegen emits five-block pattern with conditional branch when filter present, four-block pattern preserved when absent
- All 1,324 existing tests pass with zero regressions, including 11 for-in e2e tests

## Task Commits

Each task was committed atomically:

1. **Task 1: Add filter clause parsing, AST, typeck, and formatter support** - `f3b2792` (feat)
2. **Task 2: Add filter to MIR, lowering, codegen, mono, and pattern compilation** - `7d5f536` (feat)

## Files Created/Modified
- `crates/snow-parser/src/parser/expressions.rs` - Optional `when` clause parsing between iterable and `do`
- `crates/snow-parser/src/ast/expr.rs` - `filter()` accessor on ForInExpr via WHEN_KW token detection
- `crates/snow-typeck/src/infer.rs` - Filter expression inference with Bool unification in loop variable scope
- `crates/snow-fmt/src/walker.rs` - WHEN_KW formatting with surrounding spaces in walk_for_in_expr
- `crates/snow-codegen/src/mir/mod.rs` - `filter: Option<Box<MirExpr>>` on all 4 ForIn variants
- `crates/snow-codegen/src/mir/lower.rs` - Filter lowering in all 4 lower_for_in_* methods + collect_free_vars traversal
- `crates/snow-codegen/src/mir/mono.rs` - Filter traversal in collect_function_refs for all 4 variants
- `crates/snow-codegen/src/pattern/compile.rs` - Filter traversal in compile_expr_patterns for all 4 variants
- `crates/snow-codegen/src/codegen/expr.rs` - Conditional branch (forin_do_body) in all 4 codegen_for_in_* methods
- `crates/snow-codegen/src/codegen/mod.rs` - Updated test constructions with filter: None

## Decisions Made
- Filter parsed as direct child of FOR_IN_EXPR (no separate CST node) -- consistent with MatchArm guard pattern
- Filter field positioned between iterable/collection and body in MIR struct layout for logical grouping
- Conditional branch targets forin_do_body on true and latch_bb on false -- skips both body evaluation and list push

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered
None.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Full filter clause infrastructure is in place across all pipeline stages
- Ready for Plan 02: integration testing (nested loops with filters, closures in filtered loops, pipe chains, e2e tests)
- Break/continue behavior unchanged: break targets merge_bb, continue targets latch_bb regardless of filter presence

## Self-Check: PASSED

All 11 modified files verified present. Both task commits (f3b2792, 7d5f536) verified in git log.

---
*Phase: 36-filter-clause-integration*
*Completed: 2026-02-09*
