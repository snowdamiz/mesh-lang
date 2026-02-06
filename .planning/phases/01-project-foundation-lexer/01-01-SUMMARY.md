---
phase: 01-project-foundation-lexer
plan: 01
subsystem: compiler-infra
tags: [rust, cargo, serde, insta, tokenization, spans]

requires:
  - phase: none
    provides: first phase
provides:
  - "Cargo workspace with snow-common, snow-lexer, snowc crates"
  - "Complete TokenKind enum (85 variants)"
  - "Span type with byte-offset tracking and LineIndex"
  - "LexError type for error recovery"
affects: [01-02, 01-03, phase-2]

tech-stack:
  added: [serde 1.0, insta 1.46]
  patterns: [multi-crate-workspace, byte-offset-spans, keyword-match-dispatch]

key-files:
  created:
    - Cargo.toml
    - Cargo.lock
    - .cargo/config.toml
    - .gitignore
    - crates/snow-common/Cargo.toml
    - crates/snow-common/src/lib.rs
    - crates/snow-common/src/span.rs
    - crates/snow-common/src/token.rs
    - crates/snow-common/src/error.rs
    - crates/snow-lexer/Cargo.toml
    - crates/snow-lexer/src/lib.rs
    - crates/snowc/Cargo.toml
    - crates/snowc/src/main.rs
  modified: []

key-decisions:
  - "39 keywords (not 37 as plan header stated) -- the plan's actual keyword list enumerates 39 including when, where, with"
  - "SelfKw variant name for the self keyword to avoid Rust keyword conflict"
  - "Workspace-level dependency declarations for serde and insta"
  - "serde Serialize derived on all shared types for insta snapshot compatibility"
  - "keyword_from_str uses match statement (compiler-optimized) not HashMap"

patterns-established:
  - "Multi-crate workspace: snow-common (types) -> snow-lexer (tokenization) -> snowc (binary)"
  - "Byte-offset spans: Span{start, end} with u32, LineIndex for on-demand line/col"
  - "Error recovery types: LexError with typed LexErrorKind variants"
  - "Serialize derives on all types for insta YAML snapshot testing"

duration: 3min
completed: 2026-02-06
---

# Phase 1 Plan 01: Cargo Workspace and Core Types Summary

**Rust workspace with 3 crates, 85-variant TokenKind enum, byte-offset Span with LineIndex, and LexError type -- all Serialize-ready for insta snapshots.**

## Performance

- Start: 2026-02-06T08:51:24Z
- Duration: ~3 minutes
- Tasks: 2/2 completed
- Tests: 13 passing

## Accomplishments

1. **Cargo workspace scaffolding**: Root workspace with `snow-common`, `snow-lexer`, `snowc` crates. Workspace-level dependencies for `serde` and `insta`. LLVM 18 env var pre-configured in `.cargo/config.toml` for future Phase 5 (no LLVM dependency yet).

2. **Complete TokenKind enum**: 85 variants covering all Snow syntax:
   - 39 keywords (after, alias, and, case, cond, def, do, else, end, false, fn, for, if, impl, import, in, let, link, match, module, monitor, nil, not, or, pub, receive, return, self, send, spawn, struct, supervisor, trait, trap, true, type, when, where, with)
   - 22 operators (+, -, *, /, %, ==, !=, <, >, <=, >=, &&, ||, !, |>, .., <>, ++, =, ->, =>, ::)
   - 6 delimiters (parens, brackets, braces)
   - 5 punctuation (comma, dot, colon, semicolon, newline)
   - 7 literal/string tokens (int, float, string start/end/content, interpolation start/end)
   - 4 identifier/comment tokens (ident, comment, doc comment, module doc comment)
   - 2 special tokens (eof, error)

3. **Span and LineIndex**: Byte-offset `Span` (u32 start/end) with `new()`, `len()`, `is_empty()`, `merge()`. `LineIndex` pre-computes newline positions and provides `line_col()` via binary search for O(log n) lookups.

4. **LexError type**: `LexError` with `LexErrorKind` enum covering 6 error categories. `Display` implementations for human-readable error messages. `std::error::Error` implemented.

5. **keyword_from_str helper**: Match-based keyword lookup function for lexer to distinguish keywords from identifiers. All 39 keywords tested.

## Task Commits

| Task | Name | Commit | Key Files |
|------|------|--------|-----------|
| 1 | Create Cargo workspace and crate scaffolding | 8964cb5 | Cargo.toml, .cargo/config.toml, .gitignore, crate Cargo.toml/src files |
| 2 | Define TokenKind enum, Span type, LexError type | 375f04f | token.rs, span.rs, error.rs, snow-common/Cargo.toml |

## Files Created/Modified

**Created (13 files):**
- `Cargo.toml` -- Workspace root
- `Cargo.lock` -- Dependency lockfile
- `.cargo/config.toml` -- LLVM 18 env var config
- `.gitignore` -- Rust/insta ignores
- `crates/snow-common/Cargo.toml` -- Common types crate
- `crates/snow-common/src/lib.rs` -- Module re-exports
- `crates/snow-common/src/span.rs` -- Span, LineIndex
- `crates/snow-common/src/token.rs` -- Token, TokenKind, keyword_from_str
- `crates/snow-common/src/error.rs` -- LexError, LexErrorKind
- `crates/snow-lexer/Cargo.toml` -- Lexer crate
- `crates/snow-lexer/src/lib.rs` -- Lexer stub
- `crates/snowc/Cargo.toml` -- Binary crate
- `crates/snowc/src/main.rs` -- Binary entry point stub

## Decisions Made

1. **39 keywords, not 37**: The plan header said "37 keywords" but the actual keyword list in the plan enumerates 39 entries (including `when`, `where`, `with`). Implemented all 39 as listed.

2. **SelfKw variant naming**: Used `SelfKw` for the `self` keyword to avoid conflict with Rust's reserved `Self` identifier.

3. **Workspace-level deps**: `serde` and `insta` declared at workspace level in root `Cargo.toml` for consistency across crates.

4. **Match-based keyword dispatch**: `keyword_from_str` uses a `match` statement rather than `HashMap` -- the Rust compiler optimizes string matching well and avoids allocation.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Fixed keyword count mismatch (37 vs 39)**

- **Found during:** Task 2 verification
- **Issue:** Plan header stated "37 keywords" but the plan's actual keyword list enumerates 39 entries (including `when`, `where`, `with`). The test assertion `keywords.len() == 37` failed because 39 keywords were correctly implemented from the plan's list.
- **Fix:** Updated comments and assertions from 37 to 39 keywords, and total variant count from 83 to 85.
- **Files modified:** `crates/snow-common/src/token.rs`
- **Commit:** 375f04f

## Issues Encountered

None beyond the keyword count mismatch noted in Deviations.

## Next Phase Readiness

Plan 01-02 (Lexer Implementation) is fully unblocked:
- All token types defined in `TokenKind`
- `Span` and `LineIndex` ready for position tracking
- `LexError` types ready for error recovery
- `insta` configured as dev-dependency in `snow-lexer`
- `pub struct Lexer;` stub in `snow-lexer/src/lib.rs` ready to implement

## Self-Check: PASSED
