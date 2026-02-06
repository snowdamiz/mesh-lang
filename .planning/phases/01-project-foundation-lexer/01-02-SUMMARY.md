---
phase: 01-project-foundation-lexer
plan: 02
status: complete
started: 2026-02-06T08:58:44Z
completed: 2026-02-06T09:03:52Z
duration: 5min
subsystem: lexer
tags: [lexer, cursor, tokenizer, iterator, insta, snapshot-tests]

dependency-graph:
  requires: ["01-01"]
  provides: ["core-lexer", "cursor", "keyword-tokenization", "operator-tokenization", "number-tokenization", "string-tokenization", "comment-tokenization"]
  affects: ["01-03"]

tech-stack:
  added: ["cargo-insta"]
  patterns: ["cursor-based-lexing", "iterator-trait", "pending-token-queue", "string-mode-state-machine", "longest-match-first"]

key-files:
  created:
    - crates/snow-lexer/src/cursor.rs
    - crates/snow-lexer/tests/lexer_tests.rs
    - crates/snow-lexer/tests/snapshots/lexer_tests__keywords.snap
    - crates/snow-lexer/tests/snapshots/lexer_tests__operators.snap
    - crates/snow-lexer/tests/snapshots/lexer_tests__numbers.snap
    - crates/snow-lexer/tests/snapshots/lexer_tests__identifiers.snap
    - crates/snow-lexer/tests/snapshots/lexer_tests__simple_string.snap
    - crates/snow-lexer/tests/snapshots/lexer_tests__line_comment.snap
    - crates/snow-lexer/tests/snapshots/lexer_tests__doc_comment.snap
    - crates/snow-lexer/tests/snapshots/lexer_tests__module_doc_comment.snap
    - crates/snow-lexer/tests/snapshots/lexer_tests__mixed_expression.snap
    - crates/snow-lexer/tests/snapshots/lexer_tests__spans_accurate.snap
    - tests/fixtures/keywords.snow
    - tests/fixtures/operators.snow
    - tests/fixtures/numbers.snow
    - tests/fixtures/identifiers.snow
  modified:
    - crates/snow-lexer/src/lib.rs
    - crates/snow-lexer/Cargo.toml

decisions:
  - id: "01-02-string-mode"
    description: "Used StringMode enum state machine (None/Single/Triple) to track lexer position inside strings, with pending_token queue for emitting StringContent then StringEnd in sequence"
  - id: "01-02-comment-space"
    description: "Comments skip optional leading space after delimiter (#, ##, ##!) so content text is clean"

metrics:
  tests-added: 21
  tests-total: 21
  snapshot-files: 10
  fixture-files: 4
---

# Phase 01 Plan 02: Core Lexer Implementation Summary

**One-liner:** Cursor-based lexer implementing Iterator<Item=Token> with full tokenization of 39 keywords, 22 operators (longest-match), 4 number bases with underscores/floats/scientific, simple strings as Start/Content/End sequence, and line/doc/module-doc comments -- verified by 10 insta snapshot tests.

## Task Commits

| Task | Name | Commit | Key Changes |
|------|------|--------|-------------|
| 1 | Implement Cursor and core Lexer | `05953c7` | cursor.rs (Cursor struct with peek/advance/position), lib.rs (Lexer struct, Iterator impl, all token handlers) |
| 2 | Add insta snapshot tests | `f6dc3d0` | lexer_tests.rs (10 tests), 10 snapshot files, 4 fixture files, serde dev-dep |

## What Was Built

### Cursor (`crates/snow-lexer/src/cursor.rs`)
- `Cursor<'src>` struct wrapping source string with byte position tracking
- Methods: `new`, `peek`, `peek_next`, `advance`, `pos`, `is_eof`, `eat_while`, `slice`
- UTF-8 aware: position advances by `char::len_utf8()` bytes
- 8 unit tests covering all methods including multi-byte characters

### Lexer (`crates/snow-lexer/src/lib.rs`)
- `Lexer<'src>` struct implementing `Iterator<Item = Token>`
- `Lexer::new(source)` and `Lexer::tokenize(source) -> Vec<Token>` convenience
- `StringMode` state machine for tracking position inside string literals
- `pending_token` queue for emitting multi-token sequences (StringContent + StringEnd)

**Token handlers implemented:**
- Single-char delimiters: `( ) [ ] { } , ;`
- Multi-char operators with longest-match-first: `= == => ! != < <= <> > >= && || |> + ++ - -> : :: . .. * / %`
- Comments: `#` (line), `##` (doc), `##!` (module doc), `#=` (block placeholder)
- Numbers: decimal, hex (0x), binary (0b), octal (0o), all with underscore separators, floats with `.` and scientific notation (`e`/`E` with optional sign)
- Strings: single-quoted (`"..."`) and triple-quoted (`"""..."""`) with escape sequences, emitting StringStart/StringContent/StringEnd sequence
- Identifiers: alphabetic/underscore start, checked against `keyword_from_str()` for 39 keywords
- Error recovery: unknown characters emit Error token and advance

### Tests (`crates/snow-lexer/tests/lexer_tests.rs`)
- `TokenSnapshot` struct for human-readable YAML output (kind, text, span)
- 10 insta snapshot tests covering all token categories
- 4 test fixture files at `tests/fixtures/`
- 3 inline unit tests in lib.rs (simple expression, string, span accuracy)

## Decisions Made

1. **StringMode state machine** -- Rather than a complex state stack (needed for interpolation in Plan 03), used a simple enum (None/Single/Triple) to track whether the lexer is inside a string. The `pending_token` field queues the StringEnd token that follows StringContent. This keeps Plan 02 simple while establishing the pattern Plan 03 will extend.

2. **Comment leading space skip** -- Comments skip an optional leading space after the delimiter (`#`, `##`, `##!`) so the comment content is cleaner in the token span. The full raw text is still recoverable from the span.

3. **Newlines as whitespace** -- In this plan, newlines are treated as whitespace (skipped). Plan 03 will add newline-as-terminator logic with context-sensitive emission.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Installed cargo-insta CLI tool**
- **Found during:** Task 2
- **Issue:** `cargo insta test` command requires the `cargo-insta` CLI tool which was not installed
- **Fix:** Ran `cargo install cargo-insta` before running snapshot tests
- **Files modified:** None (system tool installed)

**2. [Rule 3 - Blocking] Added serde dev-dependency to snow-lexer**
- **Found during:** Task 2
- **Issue:** Test file needed `serde::Serialize` derive for `TokenSnapshot` struct, but `serde` was only a dependency of `snow-common`, not a dev-dependency of `snow-lexer`
- **Fix:** Added `serde = { workspace = true }` to `[dev-dependencies]` in `crates/snow-lexer/Cargo.toml`
- **Files modified:** `crates/snow-lexer/Cargo.toml`

## Test Results

```
running 11 tests (unit)     -- all passed
running 10 tests (snapshot) -- all passed
Total: 21 tests, 0 failures
```

## Next Phase Readiness

Plan 01-03 can proceed. It will:
- Add string interpolation (extend StringMode to a state stack)
- Add nestable block comments (`#= ... =#`)
- Add newline-as-terminator logic
- Add error recovery improvements

The cursor and main loop are stable foundations for these extensions.

## Self-Check: PASSED
