---
phase: 04-pattern-matching-adts
plan: 01
subsystem: parser
tags: [lexer, parser, ast, sum-types, adt, patterns, or-pattern, as-pattern, constructor-pattern, rowan]

# Dependency graph
requires:
  - phase: 02-parser-foundation
    provides: "Parser infrastructure, SyntaxKind, pattern parsing (WILDCARD_PAT, IDENT_PAT, LITERAL_PAT, TUPLE_PAT)"
  - phase: 03-type-system
    provides: "Type alias parsing, generic params, angle bracket syntax"
provides:
  - "BAR token kind for bare `|` (was Error, now first-class)"
  - "SUM_TYPE_DEF / VARIANT_DEF / VARIANT_FIELD CST node kinds"
  - "CONSTRUCTOR_PAT / OR_PAT / AS_PAT pattern node kinds"
  - "parse_sum_type_def parser function for `type Name do ... end`"
  - "Layered pattern parser: parse_as_pattern -> parse_or_pattern -> parse_primary_pattern"
  - "Typed AST wrappers: SumTypeDef, VariantDef, VariantField, ConstructorPat, OrPat, AsPat"
affects:
  - 04-02 (type checker registration of sum types)
  - 04-03 (exhaustiveness checking needs pattern AST)
  - 04-04 (match compiler needs pattern AST)

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Layered pattern precedence: as > or > primary (composable postfix pattern layers)"
    - "Contextual keyword: `as` parsed as IDENT with text check, not a reserved keyword"
    - "Heuristic-based constructor detection: uppercase IDENT + L_PAREN in patterns"
    - "Lookahead-based item dispatch: scan past generics to find `do` vs `=` for sum type vs alias"

key-files:
  created: []
  modified:
    - "crates/snow-common/src/token.rs"
    - "crates/snow-lexer/src/lib.rs"
    - "crates/snow-parser/src/syntax_kind.rs"
    - "crates/snow-parser/src/parser/mod.rs"
    - "crates/snow-parser/src/parser/items.rs"
    - "crates/snow-parser/src/parser/patterns.rs"
    - "crates/snow-parser/src/parser/expressions.rs"
    - "crates/snow-parser/src/ast/pat.rs"
    - "crates/snow-parser/src/ast/item.rs"
    - "crates/snow-parser/tests/parser_tests.rs"
    - "crates/snow-typeck/src/infer.rs"

key-decisions:
  - "Bare `|` now lexes as TokenKind::Bar (was Error) -- enables or-patterns and future pipeline syntax"
  - "Constructor patterns use heuristic: uppercase IDENT + L_PAREN = constructor, else IDENT_PAT; nullary constructors like None parse as IDENT_PAT and will be resolved during type checking"
  - "Qualified constructors (Shape.Circle) detected by IDENT.DOT.IDENT lookahead; unqualified (Some) by uppercase + parens"
  - "`as` is a contextual keyword (IDENT with text 'as'), not a reserved keyword -- avoids breaking existing code"
  - "Sum type dispatch: scan past name and optional generic params to find DO_KW (sum type) vs EQ (type alias)"
  - "Variant fields use :: for type annotation (matching Snow convention), not : (which is for struct literal field values)"

patterns-established:
  - "Layered pattern parsing: parse_pattern -> parse_as_pattern -> parse_or_pattern -> parse_primary_pattern"
  - "Parser nth_text() method for lookahead text inspection without consuming tokens"
  - "Sum type definition structure: SUM_TYPE_DEF > VARIANT_DEF* > (VARIANT_FIELD | TYPE_ANNOTATION)*"

# Metrics
duration: 8min
completed: 2026-02-06
---

# Phase 4 Plan 01: Sum Type + Pattern Syntax Summary

**BAR token, sum type definitions (type Shape do ... end), and constructor/or/as pattern parsing with typed AST wrappers**

## Performance

- **Duration:** 8 min
- **Started:** 2026-02-06T21:01:26Z
- **Completed:** 2026-02-06T21:09:04Z
- **Tasks:** 3/3
- **Files modified:** 11

## Accomplishments
- Bare `|` now lexes as a first-class BAR token instead of Error, enabling or-patterns
- Sum type definitions (`type Shape do Circle(Float) Rectangle(width :: Float, height :: Float) Point end`) parse into SUM_TYPE_DEF/VARIANT_DEF CST nodes with support for positional fields, named fields, and generics
- Pattern parser restructured into layered precedence (as > or > primary) supporting constructor patterns (qualified: `Shape.Circle(r)` and unqualified: `Some(x)`), or-patterns (`A | B`), and as-patterns (`p as x`)
- Full typed AST wrappers (SumTypeDef, VariantDef, VariantField, ConstructorPat, OrPat, AsPat) with accessor methods for downstream type checker use
- 17 new tests (11 snapshot + 6 AST accessor), all 145 parser tests passing, full workspace green

## Task Commits

Each task was committed atomically:

1. **Task 1: Lexer BAR token and SyntaxKind extensions** - `399d0b9` (feat)
2. **Task 2: Parser support for sum type definitions and extended patterns** - `21d627d` (feat)
3. **Task 3: Typed AST wrappers for new node types** - `65b5090` (feat)

## Files Created/Modified
- `crates/snow-common/src/token.rs` - Added TokenKind::Bar variant (operators 23->24, total 87->88)
- `crates/snow-lexer/src/lib.rs` - Changed bare `|` from Error to Bar in lex_pipe
- `crates/snow-parser/src/syntax_kind.rs` - Added BAR, SUM_TYPE_DEF, VARIANT_DEF, VARIANT_FIELD, CONSTRUCTOR_PAT, OR_PAT, AS_PAT, GUARD_CLAUSE
- `crates/snow-parser/src/parser/mod.rs` - Added nth_text() method, updated item dispatch for sum type vs type alias
- `crates/snow-parser/src/parser/items.rs` - Added parse_sum_type_def, parse_variant_def, parse_variant_field_or_type
- `crates/snow-parser/src/parser/patterns.rs` - Restructured into layered parse_as_pattern/parse_or_pattern/parse_primary_pattern with constructor support
- `crates/snow-parser/src/parser/expressions.rs` - Updated trailing closure to use BAR instead of ERROR for pipe delimiters
- `crates/snow-parser/src/ast/pat.rs` - Added ConstructorPat, OrPat, AsPat with accessors; extended Pattern enum
- `crates/snow-parser/src/ast/item.rs` - Added SumTypeDef, VariantDef, VariantField with accessors; extended Item enum
- `crates/snow-parser/tests/parser_tests.rs` - Added 11 snapshot tests + 6 AST accessor tests
- `crates/snow-typeck/src/infer.rs` - Added placeholder arms for new Pattern/Item variants

## Decisions Made
- **Bar token replaces Error for bare `|`**: This is a semantic change -- the lexer now treats `|` as meaningful rather than erroneous. Trailing closure pipe params (`do |x| ... end`) updated to use BAR instead of ERROR.
- **Constructor pattern heuristic**: Uppercase IDENT + L_PAREN = constructor pattern. Nullary constructors (like `None`) parse as IDENT_PAT and will be resolved to constructors during type checking. This avoids needing name resolution in the parser.
- **`as` as contextual keyword**: Rather than reserving "as" as a keyword (which would break any code using it as a variable name), it's detected by text comparison on IDENT tokens only in pattern context.
- **Sum type dispatch via lookahead**: The parser scans past the type name and optional generic params to determine if `do` (sum type) or `=` (type alias) follows, avoiding ambiguity.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Trailing closure pipe params broken by Bar token change**
- **Found during:** Task 1
- **Issue:** Trailing closure code (`do |x, y| ... end`) checked for `SyntaxKind::ERROR` with text `"|"`, but bare `|` now emits BAR instead of ERROR
- **Fix:** Updated all 4 occurrences in `parse_trailing_closure` to check for `SyntaxKind::BAR` instead of `SyntaxKind::ERROR`
- **Files modified:** `crates/snow-parser/src/parser/expressions.rs`
- **Verification:** `trailing_closure_basic` snapshot test passes unchanged
- **Committed in:** 399d0b9 (Task 1 commit)

**2. [Rule 1 - Bug] Lexer snapshot test expected Error for bare `|`**
- **Found during:** Task 1
- **Issue:** `lexer_tests__full_program.snap` expected `kind: Error` for the `|` token
- **Fix:** Accepted updated snapshot with `kind: Bar`
- **Files modified:** `crates/snow-lexer/tests/snapshots/lexer_tests__full_program.snap`
- **Verification:** All 30 lexer tests pass
- **Committed in:** 399d0b9 (Task 1 commit)

**3. [Rule 3 - Blocking] Type checker exhaustiveness error on new Pattern variants**
- **Found during:** Task 3
- **Issue:** `infer_pattern` match in `snow-typeck/src/infer.rs` didn't cover `Pattern::Constructor`, `Pattern::Or`, `Pattern::As`, causing compile error
- **Fix:** Added placeholder arms that infer fresh type variables (full implementation deferred to Phase 04 type checker plans)
- **Files modified:** `crates/snow-typeck/src/infer.rs`
- **Verification:** Full workspace `cargo test --workspace` passes
- **Committed in:** 65b5090 (Task 3 commit)

**4. [Rule 3 - Blocking] Item enum match missing SumTypeDef variant in type checker**
- **Found during:** Task 3
- **Issue:** `infer_item` match in `snow-typeck/src/infer.rs` didn't cover `Item::SumTypeDef`
- **Fix:** Added `Item::SumTypeDef(_) => None` arm (registration deferred to Phase 04 type checker plans)
- **Files modified:** `crates/snow-typeck/src/infer.rs`
- **Verification:** Full workspace `cargo test --workspace` passes
- **Committed in:** 65b5090 (Task 3 commit)

**5. [Rule 1 - Bug] Duplicate #[test] attribute on struct_angle_bracket_generics**
- **Found during:** Task 2
- **Issue:** Stray `#[test]` attribute before a comment block caused a duplicate attribute warning
- **Fix:** Removed the extra `#[test]` attribute
- **Files modified:** `crates/snow-parser/tests/parser_tests.rs`
- **Verification:** No more duplicate attribute warning
- **Committed in:** 21d627d (Task 2 commit)

---

**Total deviations:** 5 auto-fixed (2 bugs, 2 blocking, 1 bug)
**Impact on plan:** All auto-fixes necessary for correctness. Trailing closure fix and type checker placeholders were required consequences of the planned changes. No scope creep.

## Issues Encountered
None -- all issues were handled via deviation rules.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Sum types parse correctly and are available via typed AST wrappers
- Pattern parser supports all new forms needed for exhaustiveness checking
- Type checker has placeholder arms ready for Plan 04-02 to implement sum type registration
- GUARD_CLAUSE SyntaxKind added for future guard parsing improvements

## Self-Check: PASSED

---
*Phase: 04-pattern-matching-adts, Plan: 01*
*Completed: 2026-02-06*
