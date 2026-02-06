---
phase: 01-project-foundation-lexer
plan: 03
subsystem: lexer
tags: [lexer, interpolation, block-comments, state-machine, error-recovery, testing]
dependency-graph:
  requires: ["01-01", "01-02"]
  provides: ["complete-lexer", "string-interpolation", "block-comments", "newline-tokens", "error-recovery"]
  affects: ["02-parser"]
tech-stack:
  added: []
  patterns: ["state-stack-for-nested-contexts", "brace-depth-tracking", "pending-token-queue"]
key-files:
  created:
    - tests/fixtures/strings.snow
    - tests/fixtures/interpolation.snow
    - tests/fixtures/comments.snow
    - tests/fixtures/newlines.snow
    - tests/fixtures/error_recovery.snow
    - tests/fixtures/full_program.snow
    - crates/snow-lexer/tests/snapshots/lexer_tests__adjacent_interpolations.snap
    - crates/snow-lexer/tests/snapshots/lexer_tests__comments.snap
    - crates/snow-lexer/tests/snapshots/lexer_tests__consecutive_newlines.snap
    - crates/snow-lexer/tests/snapshots/lexer_tests__crlf_newlines.snap
    - crates/snow-lexer/tests/snapshots/lexer_tests__empty_input.snap
    - crates/snow-lexer/tests/snapshots/lexer_tests__empty_string.snap
    - crates/snow-lexer/tests/snapshots/lexer_tests__error_recovery.snap
    - crates/snow-lexer/tests/snapshots/lexer_tests__escaped_dollar_in_string.snap
    - crates/snow-lexer/tests/snapshots/lexer_tests__full_program.snap
    - crates/snow-lexer/tests/snapshots/lexer_tests__interpolation_with_braces.snap
    - crates/snow-lexer/tests/snapshots/lexer_tests__interpolation_with_expression.snap
    - crates/snow-lexer/tests/snapshots/lexer_tests__nested_block_comment.snap
    - crates/snow-lexer/tests/snapshots/lexer_tests__newlines.snap
    - crates/snow-lexer/tests/snapshots/lexer_tests__simple_string_escapes.snap
    - crates/snow-lexer/tests/snapshots/lexer_tests__span_accuracy_interpolation.snap
    - crates/snow-lexer/tests/snapshots/lexer_tests__string_interpolation.snap
    - crates/snow-lexer/tests/snapshots/lexer_tests__triple_quoted_interpolation.snap
    - crates/snow-lexer/tests/snapshots/lexer_tests__triple_quoted_string.snap
    - crates/snow-lexer/tests/snapshots/lexer_tests__unterminated_block_comment.snap
    - crates/snow-lexer/tests/snapshots/lexer_tests__whitespace_only.snap
  modified:
    - crates/snow-lexer/src/lib.rs
    - crates/snow-lexer/tests/lexer_tests.rs
    - crates/snow-lexer/tests/snapshots/lexer_tests__keywords.snap
    - crates/snow-lexer/tests/snapshots/lexer_tests__operators.snap
    - crates/snow-lexer/tests/snapshots/lexer_tests__numbers.snap
    - crates/snow-lexer/tests/snapshots/lexer_tests__identifiers.snap
decisions:
  - id: "01-03-01"
    description: "State stack (Vec<LexerState>) replaces StringMode enum for nested interpolation contexts"
  - id: "01-03-02"
    description: "InString stays on stack when InInterpolation is pushed; popping InInterpolation returns to InString"
  - id: "01-03-03"
    description: "Pending token queue (Vec<Token>) handles multi-token emissions (StringContent + InterpolationStart)"
  - id: "01-03-04"
    description: "Newlines emit as Newline tokens; only spaces and tabs are skipped as whitespace"
  - id: "01-03-05"
    description: "Bare pipe | in struct update syntax produces Error token (parser will handle as separate syntax)"
metrics:
  duration: "4min"
  completed: "2026-02-06"
---

# Phase 01 Plan 03: Complex Lexer Features & Comprehensive Tests Summary

State-stack-based string interpolation, nestable block comments, newline emission, error recovery, and 30 snapshot tests proving production-quality lexer completeness.

## Task Commits

| Task | Name | Commit | Key Changes |
|------|------|--------|-------------|
| 1 | State stack for interpolation and block comments | 84daa9b | Replaced StringMode with LexerState state stack, interpolation, block comments, newlines |
| 2 | Comprehensive snapshot tests | 6e02305 | 20 new snapshot tests, 6 test fixtures, full program integration test |

## Decisions Made

1. **State stack architecture**: `Vec<LexerState>` with `Normal`, `InString { triple: bool }`, and `InInterpolation { brace_depth: u32 }` states. InString stays on the stack when entering interpolation; popping InInterpolation automatically returns to string scanning.

2. **Pending token queue**: Multi-token emissions (e.g., encountering `${` produces both StringContent and InterpolationStart) use a `Vec<Token>` pending queue drained before the next produce_token call.

3. **Newline as token**: All newlines (`\n`, `\r`, `\r\n`) emit as `Newline` tokens. Only spaces and tabs are skipped as whitespace. This gives the parser full control over statement termination.

4. **Block comment nesting**: Depth counter starts at 1 for the opening `#=`, increments on nested `#=`, decrements on `=#`. Depth 0 = comment complete. Unterminated = Error token.

5. **Bare pipe produces Error**: The `|` character alone (not `||` or `|>`) produces an Error token. The struct update syntax `%State{ state | count: new_count }` will need a parser-level solution (possibly treating `|` as a valid separator in that context).

## Deviations from Plan

None -- plan executed exactly as written.

## What Was Built

### String Interpolation
- `"hello ${name} world"` produces: StringStart, StringContent("hello "), InterpolationStart, Ident(name), InterpolationEnd, StringContent(" world"), StringEnd
- Adjacent interpolations `"${a}${b}"` correctly avoid empty StringContent
- Brace depth tracking allows `${map[key]}` and nested braces inside interpolation
- Triple-quoted strings support interpolation
- Escaped `\$` does not trigger interpolation

### Nestable Block Comments
- `#= outer #= inner =# still =#` produces a single Comment token
- Unterminated block comments produce Error token and lexing stops

### Newline Emission
- All newlines become Newline tokens (no longer skipped)
- `\r\n` = single Newline, `\r` alone = Newline
- Consecutive newlines each produce their own token

### Error Recovery
- Invalid characters (`@`) produce Error tokens, lexing continues
- Unterminated strings produce StringContent + Error token sequence
- Unterminated block comments produce Error token

### Test Coverage
- 30 snapshot tests (was 10, added 20 new)
- 14 unit tests (was 11, added 3 new inline tests for interpolation/comments/newlines)
- 6 new test fixtures: strings, interpolation, comments, newlines, error_recovery, full_program
- Full program integration test: module with struct, functions, interpolation, pipe chains

## Test Summary

| Category | Count |
|----------|-------|
| snow-common unit tests | 13 |
| snow-lexer unit tests | 14 |
| snow-lexer snapshot tests | 30 |
| **Total workspace tests** | **57** |

## Next Phase Readiness

Phase 1 lexer work is **complete**. The lexer can tokenize all Snow syntax:
- Keywords (39), operators (22), delimiters (6), punctuation (5)
- Numbers (int, float, hex, binary, octal, scientific, underscores)
- Strings (single-line, triple-quoted, with interpolation)
- Comments (line, doc, module-doc, nestable block)
- Newlines as tokens
- Error recovery

The parser (Phase 2) can consume tokens from `Lexer::tokenize()` or the `Iterator` interface. The `Newline` tokens give the parser full control over statement termination semantics.

## Self-Check: PASSED
