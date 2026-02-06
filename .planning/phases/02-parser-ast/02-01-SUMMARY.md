---
phase: 02-parser-ast
plan: 01
subsystem: parser
tags: [rowan, cst, syntax-kind, parser, event-based, newline-significance]

requires:
  - phase: 01-project-foundation-lexer
    provides: TokenKind enum (85 variants), Token struct with Span, Lexer iterator
provides:
  - SyntaxKind enum mapping all TokenKind variants plus composite node kinds
  - Rowan-based CST types (SnowLanguage, SyntaxNode, SyntaxToken)
  - Event-based Parser struct with open/close/advance API
  - ParseError with message, span, and related span
  - Newline significance handling (insignificant inside delimiters)
affects: [02-02 (expression parser), 02-03 (compound expressions), 02-04 (declarations), 02-05 (AST wrappers)]

tech-stack:
  added: [rowan 0.16.1]
  patterns: [event-based parsing, forward-parent technique, delimiter-depth newline significance]

key-files:
  created:
    - crates/snow-parser/Cargo.toml
    - crates/snow-parser/src/lib.rs
    - crates/snow-parser/src/syntax_kind.rs
    - crates/snow-parser/src/cst.rs
    - crates/snow-parser/src/error.rs
    - crates/snow-parser/src/parser/mod.rs
  modified:
    - Cargo.toml
    - Cargo.lock

key-decisions:
  - "SyntaxKind uses SCREAMING_SNAKE_CASE with #[allow(non_camel_case_types)] for consistency with rowan conventions"
  - "Comments (line, doc, module doc) are always trivia -- skipped by parser lookahead"
  - "WHITESPACE SyntaxKind exists for future use but lexer does not emit whitespace tokens (they are skipped during lexing)"
  - "Parser uses pub(crate) visibility -- internal to snow-parser until public API in 02-05"
  - "Forward parent technique for open_before() wrapping (matklad/rust-analyzer pattern)"

duration: 7min
completed: 2026-02-06
---

# Phase 2 Plan 1: Parser Crate Scaffolding Summary

**Event-based parser with rowan CST, 131-variant SyntaxKind enum, delimiter-depth newline significance, and forward-parent tree building**

## Performance
- **Duration:** 7min
- **Started:** 2026-02-06T17:09:53Z
- **Completed:** 2026-02-06T17:16:49Z
- **Tasks:** 2/2
- **Files modified:** 8

## Accomplishments
- Created snow-parser crate with rowan 0.16 dependency in workspace
- Defined SyntaxKind enum: 2 sentinels + 85 token-mapped kinds + 1 WHITESPACE + 43 composite node kinds = 131 total variants
- Implemented From<TokenKind> for SyntaxKind covering all 85 lexer token variants
- Implemented rowan::Language for SnowLanguage with SyntaxNode/SyntaxToken/SyntaxElement type aliases
- Defined ParseError struct with message, span, and optional related span (for "opened here" diagnostics)
- Built event-based Parser struct with:
  - Lookahead: current(), nth(), current_text(), current_span(), at(), at_any()
  - Node management: open(), open_before(), close() with forward-parent technique
  - Token consumption: advance(), expect(), eat(), eat_newlines()
  - Error reporting: error(), error_with_related(), has_error()
  - Newline significance: delimiter depth tracking (paren/bracket/brace), transparent skipping in lookahead
  - Tree building: build_tree() converting events to rowan GreenNode with forward-parent chain resolution
- Added 17 tests: 4 for SyntaxKind, 3 for ParseError, 10 for Parser

## Task Commits
1. **Task 1: Create snow-parser crate with SyntaxKind enum and rowan types** - `edd8e93` (feat)
2. **Task 2: Implement event-based Parser struct with newline significance** - `d261637` (feat)

## Files Created/Modified
- `Cargo.toml` - Added snow-parser to workspace members, rowan to workspace deps
- `Cargo.lock` - Updated with rowan 0.16.1 and transitive deps
- `crates/snow-parser/Cargo.toml` - New crate with snow-common, snow-lexer, rowan deps
- `crates/snow-parser/src/lib.rs` - Public API with Parse struct, module declarations, re-exports
- `crates/snow-parser/src/syntax_kind.rs` - SyntaxKind enum (131 variants), From<TokenKind>, is_trivia()
- `crates/snow-parser/src/cst.rs` - SnowLanguage marker type, rowan::Language impl, type aliases
- `crates/snow-parser/src/error.rs` - ParseError with constructors, Display impl
- `crates/snow-parser/src/parser/mod.rs` - Event enum, Parser struct, all core parsing methods, build_tree, 10 tests

## Decisions Made
1. **SyntaxKind naming**: Used SCREAMING_SNAKE_CASE with `#[allow(non_camel_case_types)]` to match rowan ecosystem conventions (rust-analyzer uses the same pattern).
2. **Comments as trivia**: All comment types (line `#`, doc `##`, module doc `##!`) are always skipped by parser lookahead. They appear in the CST via advance() but don't affect parsing decisions.
3. **No whitespace tokens**: The lexer strips whitespace entirely. WHITESPACE SyntaxKind exists for potential future lossless CST reconstruction but is not currently populated.
4. **pub(crate) visibility**: Parser internals are crate-private. The public API (parse() function) will be wired up in Plan 02-05.
5. **Forward parent technique**: Used matklad's approach for open_before() -- sets a forward_parent link on the completed node's Open event, processed during build_tree by following chains and opening in reverse order.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Missing Ord/PartialOrd derives on SyntaxKind**
- **Found during:** Task 1
- **Issue:** rowan::Language requires `Kind: Ord` but SyntaxKind only derived PartialEq/Eq/Hash
- **Fix:** Added `PartialOrd, Ord` to the derive macro
- **Files modified:** crates/snow-parser/src/syntax_kind.rs
- **Commit:** edd8e93

**2. [Rule 3 - Blocking] Missing #[allow(non_camel_case_types)] on SyntaxKind**
- **Found during:** Task 1
- **Issue:** Rust warns about non-CamelCase enum variants for SCREAMING_SNAKE_CASE names (101 warnings)
- **Fix:** Added `#[allow(non_camel_case_types)]` attribute
- **Files modified:** crates/snow-parser/src/syntax_kind.rs
- **Commit:** edd8e93

## Issues Encountered
None beyond the auto-fixed deviations above.

## Next Phase Readiness
Parser crate is fully scaffolded and ready for Plan 02-02 (Pratt expression parser). The Parser struct provides all the infrastructure needed:
- `open()`/`close()`/`open_before()` for building the CST node tree
- `current()`/`nth()` for lookahead with transparent newline/comment skipping
- `advance()`/`expect()`/`eat()` for token consumption
- `build_tree()` for converting events to rowan GreenNode
- Delimiter depth tracking for newline significance

The `parse()` public function in lib.rs is still a `todo!()` placeholder -- it will be wired up in Plan 02-05 after all grammar rules are implemented.

## Self-Check: PASSED

---
*Phase: 02-parser-ast*
*Completed: 2026-02-06*
