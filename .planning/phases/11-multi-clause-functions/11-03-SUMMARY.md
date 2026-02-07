---
phase: 11-multi-clause-functions
plan: 03
subsystem: codegen
tags: [mir, llvm, codegen, formatter, multi-clause, guards, pattern-matching, e2e]

# Dependency graph
requires:
  - phase: 11-01
    provides: Parser support for = expr body form, guard clauses, pattern parameters
  - phase: 11-02
    provides: Type checker grouping, desugaring, exhaustiveness checking for multi-clause functions
provides:
  - MIR lowering for multi-clause functions (grouping and Match body generation)
  - MIR lowering for = expr body form single-clause functions
  - Guard variable binding fix in LLVM codegen pattern matching
  - Formatter support for = expr body, guard clauses, pattern parameters
  - Comprehensive e2e tests proving full compiler pipeline
affects: [12-pipeline-operators, 13-module-system]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Multi-clause function grouping at MIR level (consecutive same-name FnDef detection)"
    - "If-else chain for multi-param multi-clause functions"
    - "Entry-block alloca placement for guard variable bindings"

key-files:
  created:
    - tests/e2e/multi_clause.snow
    - tests/e2e/multi_clause_guards.snow
  modified:
    - crates/snow-codegen/src/mir/lower.rs
    - crates/snow-codegen/src/codegen/pattern.rs
    - crates/snow-fmt/src/walker.rs
    - crates/snowc/tests/e2e.rs

key-decisions:
  - "Single-param multi-clause uses MirExpr::Match for efficient pattern dispatch; multi-param uses if-else chain"
  - "Guard variable binding done via entry-block allocas before guard expression evaluation"
  - "Leaf node skips already-bound variables to avoid duplicate allocas from guard pre-binding"

patterns-established:
  - "Multi-clause grouping: consecutive same-name = expr FnDefs form a group in MIR lowering"
  - "Guard codegen: bind pattern variables before guard evaluation using entry-block allocas"

# Metrics
duration: 12min
completed: 2026-02-07
---

# Phase 11 Plan 03: Multi-Clause Function Codegen Summary

**MIR lowering for multi-clause functions with guard variable binding fix, formatter support, and e2e tests proving fib(10)=55 and guard clauses work end-to-end**

## Performance

- **Duration:** 12 min
- **Started:** 2026-02-07T19:53:15Z
- **Completed:** 2026-02-07T20:05:00Z
- **Tasks:** 2
- **Files modified:** 5

## Accomplishments
- Multi-clause functions compile and run correctly: fib(10)=55 via `fn fib(0)=0; fn fib(1)=1; fn fib(n)=fib(n-1)+fib(n-2)`
- Guard clauses work: `fn abs(n) when n < 0 = -n; fn abs(n) = n` produces abs(-5)=5, abs(3)=3
- Formatter handles `= expr` body form, `when` guards, and pattern parameters without crashing
- Zero regressions across entire test suite (1000+ tests)

## Task Commits

Each task was committed atomically:

1. **Task 1: MIR lowering and formatter support** - `0063d52` (feat)
2. **Task 2: E2E tests and guard variable binding fix** - `2b85dbe` (feat)

## Files Created/Modified
- `crates/snow-codegen/src/mir/lower.rs` - Multi-clause function grouping, Match body generation, = expr body lowering
- `crates/snow-codegen/src/codegen/pattern.rs` - Guard variable binding fix (entry-block allocas)
- `crates/snow-fmt/src/walker.rs` - FN_EXPR_BODY, GUARD_CLAUSE, WHEN_KW handling in formatter
- `tests/e2e/multi_clause.snow` - E2E: fib, to_string, double, square
- `tests/e2e/multi_clause_guards.snow` - E2E: abs, classify with when guards
- `crates/snowc/tests/e2e.rs` - 4 new e2e test functions

## Decisions Made
- **Single-param vs multi-param strategy:** Single-param multi-clause functions use `MirExpr::Match` directly (efficient pattern dispatch through decision tree). Multi-param functions use if-else chain (avoids needing MirExpr::Tuple which doesn't exist).
- **Guard variable binding:** Pattern variables are bound in the entry block (allocas) and values stored before guard evaluation. The Leaf node skips already-bound variables to avoid duplicates.
- **Square function type annotation:** The e2e test uses `fn square(x :: Int) -> Int do ... end` because untyped do/end functions have a pre-existing type inference limitation (not related to multi-clause).

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Guard expressions could not access pattern-bound variables**
- **Found during:** Task 2 (E2E testing of guard clauses)
- **Issue:** `fn abs(n) when n < 0 = -n` failed with "Undefined variable 'n'" because the guard expression was evaluated before pattern variables were bound in codegen
- **Fix:** Modified `codegen_guard` in pattern.rs to extract bindings from the success Leaf node and bind them (with entry-block allocas) before evaluating the guard expression. Modified `codegen_leaf` to skip variables already bound by the guard node.
- **Files modified:** crates/snow-codegen/src/codegen/pattern.rs
- **Verification:** abs(-5)=5, abs(3)=3, classify(10)="positive" all produce correct output
- **Committed in:** 2b85dbe (Task 2 commit)

---

**Total deviations:** 1 auto-fixed (1 bug)
**Impact on plan:** Bug fix was essential for guard clause functionality. This was a pre-existing gap in the pattern match codegen that wasn't triggered before because no existing code path used guards on pattern variables.

## Issues Encountered
- Untyped `do/end` functions (e.g., `fn square(x) do x * x end` without type annotations) fail with "Unsupported binop type: Unit" -- pre-existing type inference limitation, not caused by this plan. Worked around in e2e test by adding type annotations.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Phase 11 (Multi-Clause Functions) is complete across all 3 plans
- Parser (11-01), type checker (11-02), and codegen/formatter (11-03) all working
- All four roadmap success criteria satisfied:
  1. fn fib(0)/fib(1)/fib(n) compiles and runs correctly (verified: fib(10)=55)
  2. Non-exhaustive multi-clause functions produce warning
  3. Type inference unifies return types across clauses (verified: type mismatch produces error)
  4. Works with literal, boolean, variable, and wildcard patterns
- Ready for Phase 12 (Pipeline Operators) or Phase 13 (Module System)

## Self-Check: PASSED

---
*Phase: 11-multi-clause-functions*
*Completed: 2026-02-07*
