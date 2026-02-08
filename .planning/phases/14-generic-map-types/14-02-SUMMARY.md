---
phase: 14-generic-map-types
plan: 02
subsystem: parser, typeck, codegen
tags: [map-literal, syntax, type-inference, MIR-desugaring, map]

# Dependency graph
requires:
  - phase: 14-01
    provides: "Polymorphic Map<K,V> with string key support, snow_map_new_typed, snow_map_tag_string"
provides:
  - "Map literal syntax: %{key => value, ...}"
  - "MAP_LITERAL and MAP_ENTRY syntax kinds and AST nodes"
  - "Type inference for map literals: infer Map<K,V> from entry types"
  - "MIR desugaring: map literals to snow_map_new_typed + snow_map_put chains"
affects:
  - "15-polish (any future map-related syntax extensions)"

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Map literal desugaring: %{k=>v} -> snow_map_new_typed(tag) + snow_map_put chain in MIR"
    - "Key type inference reuse: infer_map_key_type resolves Map<K,V> to determine key_type_tag"

key-files:
  created:
    - "tests/e2e/map_literal.snow"
    - "tests/e2e/map_literal_int.snow"
  modified:
    - "crates/snow-parser/src/syntax_kind.rs"
    - "crates/snow-parser/src/parser/expressions.rs"
    - "crates/snow-parser/src/ast/expr.rs"
    - "crates/snow-typeck/src/infer.rs"
    - "crates/snow-codegen/src/mir/lower.rs"
    - "crates/snowc/tests/e2e_stdlib.rs"

key-decisions:
  - "Map literal parsed at PERCENT + L_BRACE in lhs(), not as compound lexer token"
  - "Entries use FAT_ARROW (=>) separator, comma-separated, newlines inside braces are insignificant"
  - "MIR desugaring reuses existing infer_map_key_type for key_type_tag determination"

patterns-established:
  - "Literal syntax desugaring: new literal forms can be added as parser productions + MIR desugaring to runtime calls"

# Metrics
duration: 7min
completed: 2026-02-08
---

# Phase 14 Plan 02: Map Literal Syntax Summary

**Map literal syntax `%{key => value, ...}` with full pipeline: parser, type inference, MIR desugaring, and native execution**

## Performance

- **Duration:** 7 min
- **Started:** 2026-02-08T00:51:41Z
- **Completed:** 2026-02-08T00:58:46Z
- **Tasks:** 2
- **Files modified:** 8 (3 parser, 1 typeck, 1 codegen, 2 test fixtures, 1 test harness)

## Accomplishments
- Map literal syntax `%{"name" => "Alice", "age" => "30"}` parses to MAP_LITERAL with MAP_ENTRY children
- Type inference creates fresh K/V type variables and unifies all entries against them
- MIR desugaring produces snow_map_new_typed(key_type_tag) + snow_map_put chains
- Both string-key and integer-key map literals work end-to-end
- Empty map literals `%{}` parse correctly
- Zero regressions: all 31 e2e tests and full test suite pass

## Task Commits

Each task was committed atomically:

1. **Task 1: Parser and AST support for map literal syntax** - `fe2fc4a` (feat)
2. **Task 2: Type inference, MIR desugaring, and e2e tests** - `a7f8ba9` (feat)

## Files Created/Modified
- `crates/snow-parser/src/syntax_kind.rs` - MAP_LITERAL and MAP_ENTRY syntax kinds added
- `crates/snow-parser/src/parser/expressions.rs` - parse_map_literal function, PERCENT dispatch in lhs()
- `crates/snow-parser/src/ast/expr.rs` - MapLiteral and MapEntry AST nodes with entries(), key(), value()
- `crates/snow-typeck/src/infer.rs` - infer_map_literal: fresh K/V vars, unify entries, return Map<K,V>
- `crates/snow-codegen/src/mir/lower.rs` - lower_map_literal: desugar to snow_map_new_typed + snow_map_put chain
- `tests/e2e/map_literal.snow` - String-key map literal e2e test
- `tests/e2e/map_literal_int.snow` - Integer-key map literal e2e test
- `crates/snowc/tests/e2e_stdlib.rs` - e2e_map_literal and e2e_map_literal_int test functions

## Decisions Made
- **Map literal parsed in lhs() on PERCENT + L_BRACE:** The parser checks `p.nth(1) == SyntaxKind::L_BRACE` when encountering PERCENT. This avoids ambiguity with the modulo operator (which is an infix op handled separately). No lexer changes needed.
- **FAT_ARROW (=>) entry separator:** Entries use `key => value` syntax, consistent with Elixir-style map syntax. Comma-separated with optional trailing comma.
- **MIR desugaring reuses infer_map_key_type:** The existing helper function that resolves `Ty::App(Con("Map"), [K, V])` to determine key_type_tag (0 for Int, 1 for String) is reused for map literal lowering.

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered
None. The implementation went cleanly:
- Parser API uses `p.at(SyntaxKind::EOF)` not `p.at_end()` (minor API discovery, fixed immediately)
- Type inference and MIR desugaring followed established patterns from StructLiteral and existing map functions

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Phase 14 is now fully complete with both plans (14-01 and 14-02) finished
- Map<K,V> is fully generic with string and integer key support
- Map literal syntax provides natural construction: `%{"name" => "Alice"}`
- All 31 e2e tests pass, zero regressions
- Ready to proceed to Phase 15 (Polish)

## Self-Check: PASSED
