---
phase: 16-fun-type-parsing
plan: 01
subsystem: compiler
tags: [parser, cst, function-types, rowan, syntax-kind]

# Dependency graph
requires: []
provides:
  - "FUN_TYPE CST node kind in SyntaxKind enum"
  - "parse_type() Fun(params) -> RetType parsing"
affects:
  - "16-02 (type checker needs FUN_TYPE nodes to resolve function type annotations)"
  - "snow-fmt (formatter may need FUN_TYPE case -- currently falls through to default token walk)"
  - "snow-lsp (LSP may need FUN_TYPE handling for hover/goto)"

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Special-case IDENT text comparison in type parser (Fun detected by p.current_text() == \"Fun\")"

key-files:
  created: []
  modified:
    - "crates/snow-parser/src/syntax_kind.rs"
    - "crates/snow-parser/src/parser/items.rs"

key-decisions:
  - "Fun remains an IDENT, not a keyword -- type-position disambiguation only"
  - "FUN_TYPE placed after RESULT_TYPE in SyntaxKind enum (type annotation section)"

patterns-established:
  - "Fun type parsing via text comparison + lookahead: p.current_text() == \"Fun\" && p.nth(1) == L_PAREN"

# Metrics
duration: 1min
completed: 2026-02-08
---

# Phase 16 Plan 01: Parser Infrastructure Summary

**FUN_TYPE CST node kind added and parse_type() extended to parse Fun(params) -> RetType as function type annotations**

## Performance

- **Duration:** 1 min
- **Started:** 2026-02-08T03:13:34Z
- **Completed:** 2026-02-08T03:14:58Z
- **Tasks:** 2/2
- **Files modified:** 2

## Accomplishments
- Added `FUN_TYPE` composite node kind to `SyntaxKind` enum for function type annotations
- Extended `parse_type()` to detect `Fun(` and parse full function type syntax: `Fun(ParamTypes) -> ReturnType`
- Handles zero-arity `Fun() -> T`, multi-param `Fun(Int, String) -> Bool`, and nested `Fun(Fun(Int) -> String) -> Bool`
- `Fun` without `(` correctly falls through to normal IDENT type parsing

## Task Commits

Each task was committed atomically:

1. **Task 1: Add FUN_TYPE to SyntaxKind and update tests** - `47f59c0` (feat)
2. **Task 2: Add Fun() function type parsing to parse_type()** - `aa9155d` (feat)

## Files Created/Modified
- `crates/snow-parser/src/syntax_kind.rs` - Added FUN_TYPE variant, updated test count to >= 73
- `crates/snow-parser/src/parser/items.rs` - Added Fun type branch in parse_type() before IDENT fallthrough

## Decisions Made
- Fun stays as IDENT (not a keyword) -- special-cased by text comparison only in type position
- FUN_TYPE node placed in the type-annotation section of SyntaxKind, after RESULT_TYPE

## Deviations from Plan

None -- plan executed exactly as written.

## Issues Encountered

None.

## User Setup Required

None -- no external service configuration required.

## Next Phase Readiness
- FUN_TYPE CST nodes are now emitted by the parser for Fun type annotations
- Plan 16-02 can proceed: type checker needs to collect ARROW tokens and handle FUN_TYPE/Fun in parse_type_tokens()
- No blockers

---
*Phase: 16-fun-type-parsing*
*Completed: 2026-02-08*

## Self-Check: PASSED
