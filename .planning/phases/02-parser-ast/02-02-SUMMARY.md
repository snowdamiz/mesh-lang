---
phase: 02-parser-ast
plan: 02
subsystem: parser
tags: [pratt-parser, expression-parsing, binding-power, operator-precedence, rowan-cst]

# Dependency graph
requires:
  - phase: 02-01
    provides: "Parser struct with event-based architecture, SyntaxKind enum, CST types"
  - phase: 01-03
    provides: "Lexer with string interpolation and all token types"
provides:
  - "Pratt expression parser with binding power tables for all Snow operators"
  - "Expression parsing for literals, identifiers, binary/unary, calls, field access, indexing, pipe, grouping, string interpolation"
  - "parse_expr() public API for testing expression parsing in isolation"
  - "debug_tree() utility for CST debug output"
affects: [02-03, 02-04, 02-05]

# Tech tracking
tech-stack:
  added: []
  patterns: ["Pratt parsing with binding power tables", "insta snapshot testing for CST structure"]

key-files:
  created:
    - "crates/snow-parser/src/parser/expressions.rs"
    - "crates/snow-parser/tests/parser_tests.rs"
    - "crates/snow-parser/tests/snapshots/ (37 snapshot files)"
  modified:
    - "crates/snow-parser/src/parser/mod.rs"
    - "crates/snow-parser/src/lib.rs"

key-decisions:
  - "Grouped expressions and single-element tuples both use TUPLE_EXPR node (parser does not distinguish)"
  - "PIPE_EXPR separate from BINARY_EXPR for pipe operator"
  - "parse_expr() and debug_tree() added as public API for testing"

patterns-established:
  - "Pratt parsing: expr_bp(p, min_bp) with infix_binding_power/prefix_binding_power tables"
  - "Snapshot testing: parse_and_debug() helper with insta::assert_snapshot! for CST verification"
  - "Postfix operations at implicit bp 25, tighter than all prefix/infix"

# Metrics
duration: 3min
completed: 2026-02-06
---

# Phase 2 Plan 2: Pratt Expression Parser Summary

**Pratt expression parser with binding power tables for all Snow operators, 37 snapshot tests proving correct precedence and associativity**

## Performance

- **Duration:** 3 min
- **Started:** 2026-02-06T17:20:11Z
- **Completed:** 2026-02-06T17:23:32Z
- **Tasks:** 2
- **Files modified:** 41

## Accomplishments
- Full Pratt expression parser handling all Snow binary operators (pipe, logical, comparison, arithmetic, range, concat) with correct precedence
- Prefix operators (-, !, not) and postfix operations (call, field access, indexing) with appropriate binding powers
- String interpolation parsing with INTERPOLATION child nodes in STRING_EXPR
- 37 insta snapshot tests covering literals, binary/unary ops, calls, field access, indexing, pipe chains, grouping, tuples, interpolation, and error recovery

## Task Commits

Each task was committed atomically:

1. **Task 1: Implement Pratt expression parser with binding power tables** - `0dae3dc` (feat)
2. **Task 2: Add expression parser tests proving correct precedence** - `bebd2a2` (test)

## Files Created/Modified
- `crates/snow-parser/src/parser/expressions.rs` - Pratt expression parser with binding power tables, atom parsing, postfix/infix loop
- `crates/snow-parser/src/parser/mod.rs` - Added `pub(crate) mod expressions;`
- `crates/snow-parser/src/lib.rs` - Added `parse_expr()` and `debug_tree()` public APIs for testing
- `crates/snow-parser/tests/parser_tests.rs` - 37 snapshot tests for expression parsing
- `crates/snow-parser/tests/snapshots/` - 37 insta snapshot files

## Decisions Made
- Grouped expressions `(a + b)` and single-element tuples use the same `TUPLE_EXPR` node kind. The parser does not distinguish them; semantic analysis can differentiate based on comma presence.
- `PIPE_EXPR` is a separate node kind from `BINARY_EXPR` for the pipe operator `|>`, making it easy for later phases to identify pipe chains.
- Added `parse_expr()` and `debug_tree()` as public functions in `lib.rs` to enable integration testing. These will be useful throughout parser development.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 2 - Missing Critical] Added parse_expr() and debug_tree() public API**
- **Found during:** Task 2 (test creation)
- **Issue:** Parser internals are `pub(crate)`, so integration tests cannot call `expr()` directly. Need a public entry point for testing.
- **Fix:** Added `parse_expr()` (wraps expression in SOURCE_FILE root) and `debug_tree()` (formats CST as indented text) to `lib.rs`.
- **Files modified:** `crates/snow-parser/src/lib.rs`
- **Verification:** All 37 tests pass using these helpers.
- **Committed in:** `bebd2a2` (Task 2 commit)

---

**Total deviations:** 1 auto-fixed (1 missing critical)
**Impact on plan:** Essential for testing. No scope creep.

## Issues Encountered
None.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Expression parser complete for basic forms
- Ready for Plan 03: compound expressions (if/else, case/match, closures, blocks)
- `parse_expr()` and `debug_tree()` utilities available for all future parser testing
- All 111 workspace tests pass with zero regressions

## Self-Check: PASSED

---
*Phase: 02-parser-ast*
*Completed: 2026-02-06*
