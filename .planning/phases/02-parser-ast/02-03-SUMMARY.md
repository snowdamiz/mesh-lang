---
phase: 02-parser-ast
plan: 03
subsystem: parser
tags: [compound-expressions, if-else, case-match, closures, blocks, let-bindings, return, trailing-closures]

# Dependency graph
requires:
  - phase: 02-02
    provides: "Pratt expression parser with binding power tables, parse_expr() API, snapshot test infrastructure"
  - phase: 02-01
    provides: "Parser struct with event-based architecture, SyntaxKind enum"
provides:
  - "Compound expression parsing: if/else, case/match, closures, trailing closures"
  - "Block parsing with newline/semicolon statement separation"
  - "Statement parsing: let bindings with optional type annotations, return expressions"
  - "parse_block() public API for testing block-level parsing"
affects: [02-04, 02-05]

# Tech tracking
tech-stack:
  added: []
  patterns: ["Block body parsing with statement separation", "Recursive compound expression parsing"]

key-files:
  created:
    - "crates/snow-parser/tests/snapshots/ (21 new snapshot files)"
  modified:
    - "crates/snow-parser/src/parser/expressions.rs"
    - "crates/snow-parser/src/lib.rs"
    - "crates/snow-parser/tests/parser_tests.rs"

# Decisions
decisions:
  - id: "02-03-01"
    decision: "Trailing closures only attach after explicit arg list () -- bare `do` on identifier does not create CALL_EXPR"
    reason: "Bare `do` in `if x do` was incorrectly treated as trailing closure; restricting to post-arglist avoids ambiguity"
  - id: "02-03-02"
    decision: "Closures always use `fn (params) -> body end` with explicit end keyword"
    reason: "Consistent block termination with other compound expressions; single-expression closures still use block body internally"
  - id: "02-03-03"
    decision: "Match arm patterns parsed as expressions for now (Plan 04 adds proper pattern parsing)"
    reason: "Avoids premature pattern syntax commitment; expressions cover identifier and literal patterns"

# Metrics
duration: 4min
completed: 2026-02-06
---

# Phase 2 Plan 3: Compound Expressions Summary

Compound expression parsing with block body, if/else chains, case/match arms, closures, let bindings, return, and trailing closures -- all with 21 snapshot tests proving correct tree structure.

## What Was Done

### Task 1: Implement compound expressions and block parsing
**Commit:** `4a28c39`

Added nine major parsing functions to `expressions.rs`:

1. **`parse_block_body`** -- parses statement sequences separated by newlines/semicolons, stopping at END_KW/ELSE_KW/EOF. Wraps in BLOCK node.
2. **`parse_stmt`** -- dispatches to let binding, return, or expression-statement.
3. **`parse_let_binding`** -- parses `let name [:: Type] = expr` with optional TYPE_ANNOTATION containing type params `[A, B]`.
4. **`parse_return_expr`** -- parses `return [expr]` with optional value (uses `looks_like_expr_start` to decide).
5. **`parse_if_expr`** -- parses `if cond do body [else [if ...] body] end` with recursive else-if chains producing nested IF_EXPR inside ELSE_BRANCH.
6. **`parse_case_expr`** -- parses `case/match expr do arms end` with MATCH_ARM children.
7. **`parse_match_arm`** -- parses `pattern [when guard] -> body`.
8. **`parse_closure`** -- parses `fn (params) -> body end` with PARAM_LIST.
9. **`parse_trailing_closure`** -- parses `do [|params|] body end` after CALL_EXPR arg list.

Also added `parse_param_list` and `parse_param` for parameter parsing with optional type annotations.

Integrated IF_KW, CASE_KW, MATCH_KW, FN_KW into Pratt atom dispatch. Added trailing closure check after call arg list in postfix loop.

Added `parse_block()` public API in `lib.rs` for testing block-level parsing.

### Task 2: Add compound expression and statement snapshot tests
**Commit:** `0d2843a`

Added 21 new snapshot tests covering all compound expression forms:

- **Let bindings (3):** simple, with type annotation, multi-statement
- **Return (3):** with value, with expression, bare return
- **If/else (4):** simple, with else, else-if chain, single-line
- **Case/match (3):** simple case, match boolean, case with when guard
- **Closures (3):** single param, two params, no params
- **Blocks (1):** multi-statement block
- **Trailing closures (1):** basic do/end after call
- **Error cases (2):** missing end, missing identifier after let
- **Newline significance (1):** newlines inside parens ignored

All 58 tests pass (37 existing + 21 new).

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Bare `do` trailing closure incorrectly triggering in if/case conditions**

- **Found during:** Task 2 (snapshot verification)
- **Issue:** The postfix loop had a bare `do` trailing closure check (`if current == DO_KW`) that caused `if x do` to parse `x` as a call with trailing closure instead of as the if-condition followed by `do`-block.
- **Fix:** Removed bare `do` trailing closure from postfix loop. Trailing closures now only attach after explicit `()` arg lists (e.g., `foo() do ... end`).
- **Files modified:** `crates/snow-parser/src/parser/expressions.rs`
- **Commit:** `0d2843a`

## Task Commits

| Task | Name | Commit | Key Files |
|------|------|--------|-----------|
| 1 | Implement compound expressions and block parsing | `4a28c39` | expressions.rs, lib.rs |
| 2 | Add compound expression and statement snapshot tests | `0d2843a` | parser_tests.rs, 21 snapshot files |

## Verification

1. `cargo test --workspace` -- all pass (58 parser tests + all other crate tests)
2. `if x do 1 else 2 end` -- IF_EXPR with two branches via ELSE_BRANCH
3. `case x do 1 -> "a" 2 -> "b" end` -- CASE_EXPR with MATCH_ARM children
4. `fn (x) -> x + 1 end` -- CLOSURE_EXPR with PARAM_LIST and BLOCK body
5. `let x = 5` -- LET_BINDING with NAME and LITERAL
6. Block newline separation: multi-statement blocks separate correctly
7. Parens suppress newlines: `foo(\n1,\n2\n)` parses as single call
8. Missing end error references do span with related span context

## Next Phase Readiness

Plan 04 (definitions and patterns) can proceed. Key interfaces provided:
- `parse_block_body()` for parsing block bodies in def/module/struct
- `parse_param_list()` for function definition parameter parsing
- Match arm patterns currently parsed as expressions; Plan 04 will add proper pattern syntax

## Self-Check: PASSED
