---
phase: 11-multi-clause-functions
plan: 01
subsystem: parser
tags: [parser, multi-clause, pattern-matching, guard-clause, ast]

# Dependency graph
requires:
  - phase: 04-pattern-matching
    provides: "Pattern parser (parse_pattern) and pattern AST nodes"
provides:
  - "Parser support for fn name(pattern) [when guard] = expr syntax"
  - "FN_EXPR_BODY syntax kind for expression body nodes"
  - "parse_fn_clause_param and parse_fn_clause_param_list functions"
  - "GuardClause and FnExprBody AST nodes with accessors"
  - "FnDef::guard(), expr_body(), has_eq_body() methods"
  - "Param::pattern() accessor for pattern children"
affects:
  - 11-02 (type checker desugaring uses guard/expr_body/has_eq_body)
  - 11-03 (codegen relies on desugared output from type checker)

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "parse_fn_clause_param dispatches to parse_pattern for pattern params, falls through to regular param parsing for idents"
    - "FN_EXPR_BODY wrapper node distinguishes = expr body from do/end block body"
    - "Guard clause parsed as GUARD_CLAUSE node containing arbitrary Bool expression"

key-files:
  created: []
  modified:
    - "crates/snow-parser/src/syntax_kind.rs"
    - "crates/snow-parser/src/parser/items.rs"
    - "crates/snow-parser/src/parser/expressions.rs"
    - "crates/snow-parser/src/ast/item.rs"
    - "crates/snow-parser/tests/parser_tests.rs"

key-decisions:
  - "Always use parse_fn_clause_param_list for all fn def param lists (transparent backward compat)"
  - "Guard clause parsed before body detection (when before = in grammar)"
  - "FN_EXPR_BODY wraps the expression so AST can distinguish body forms"

patterns-established:
  - "Pattern param detection: literal/wildcard/constructor dispatch to parse_pattern, lowercase ident falls through to regular param"
  - "Dual body form: check EQ for = expr, check DO_KW for do/end, error otherwise"

# Metrics
duration: 5min
completed: 2026-02-07
---

# Phase 11 Plan 01: Multi-Clause Function Parser Summary

**Parser extended with `fn name(pattern) [when guard] = expr` syntax alongside existing `do/end` form, with pattern params and guard clauses**

## Performance

- **Duration:** 5 min
- **Started:** 2026-02-07T19:30:40Z
- **Completed:** 2026-02-07T19:35:47Z
- **Tasks:** 2
- **Files modified:** 5 (+ 8 snapshot files)

## Accomplishments
- Parser accepts both `fn name(pattern) = expr` and `fn name(param) do ... end` body forms
- Pattern parameters work: literal (0, 1, true), wildcard (_), constructor (Some(x)), tuple ((a, b)), negative literal (-1)
- Guard clauses parse: `fn abs(n) when n < 0 = -n` produces FN_DEF with GUARD_CLAUSE child
- AST accessors (guard, expr_body, has_eq_body, Param::pattern) ready for downstream type checker
- All 166 existing tests pass with zero regressions; 26 new tests added (192 total)

## Task Commits

Each task was committed atomically:

1. **Task 1: Add syntax kind and parser support for = expr body form and pattern params** - `f646bfd` (feat)
2. **Task 2: Add AST accessors for guard clauses and expression body** - `4a9a2c6` (feat)

## Files Created/Modified
- `crates/snow-parser/src/syntax_kind.rs` - Added FN_EXPR_BODY syntax kind variant
- `crates/snow-parser/src/parser/items.rs` - Modified parse_fn_def for dual body forms and guard clauses
- `crates/snow-parser/src/parser/expressions.rs` - Added parse_fn_clause_param and parse_fn_clause_param_list
- `crates/snow-parser/src/ast/item.rs` - Added GuardClause, FnExprBody AST nodes; added guard/expr_body/has_eq_body/pattern accessors
- `crates/snow-parser/tests/parser_tests.rs` - 26 new integration tests for multi-clause function syntax
- 8 new insta snapshot files for tree structure verification

## Decisions Made
- Always use parse_fn_clause_param_list for all fn def param lists -- this is transparent for backward compat since the function handles both pattern and regular params
- Guard clause is parsed before body detection -- grammar reads as `fn name(params) [when guard] [= expr | do ... end]`
- FN_EXPR_BODY node wraps the body expression to allow the AST to cleanly distinguish between body forms via child node kind

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered

None.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness
- Parser foundation for multi-clause functions is complete
- Plan 02 (type checker desugaring) can use guard(), expr_body(), has_eq_body(), and Param::pattern() to detect and desugar multi-clause function definitions into match expressions
- Plan 03 (codegen) requires no additional parser changes

---
*Phase: 11-multi-clause-functions*
*Plan: 01*
*Completed: 2026-02-07*

## Self-Check: PASSED
