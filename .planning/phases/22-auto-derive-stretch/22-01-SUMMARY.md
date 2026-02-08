---
phase: 22-auto-derive-stretch
plan: 01
subsystem: compiler
tags: [parser, typeck, codegen, formatter, deriving, traits, derive-clause]

# Dependency graph
requires:
  - phase: 20-protocol-core
    provides: Debug/Eq/Ord auto-derive infrastructure for structs and sum types
  - phase: 21-extended-protocols
    provides: Hash auto-derive for structs, Display trait registration
provides:
  - DERIVING_CLAUSE SyntaxKind and parser support for `end deriving(Trait1, Trait2, ...)`
  - AST accessors has_deriving_clause() and deriving_traits() on StructDef and SumTypeDef
  - Conditional trait registration in typeck gated by deriving clause
  - Conditional MIR generation gated by deriving clause
  - Display and Hash trait registration for explicit deriving on sum types
  - Formatter preservation of deriving clause
affects: [22-02-PLAN]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Contextual keyword parsing (deriving as IDENT with text check)"
    - "derive_all = !has_deriving backward-compat pattern across typeck and MIR"

key-files:
  created: []
  modified:
    - crates/snow-parser/src/syntax_kind.rs
    - crates/snow-parser/src/parser/items.rs
    - crates/snow-parser/src/ast/item.rs
    - crates/snow-typeck/src/infer.rs
    - crates/snow-codegen/src/mir/lower.rs
    - crates/snow-fmt/src/walker.rs
    - crates/snow-parser/tests/parser_tests.rs

key-decisions:
  - "deriving parsed as contextual keyword (IDENT with text check), not added to TokenKind"
  - "No deriving clause = derive all defaults (backward compatible)"
  - "Explicit deriving() with empty parens = derive nothing"
  - "Display never auto-derived, only via explicit deriving(Display)"
  - "Hash for sum types only via explicit deriving(Hash), not in derive_all"

patterns-established:
  - "derive_all || derive_list.contains() pattern for conditional gating"
  - "DERIVING_CLAUSE as child node of STRUCT_DEF and SUM_TYPE_DEF"

# Metrics
duration: 8min
completed: 2026-02-08
---

# Phase 22 Plan 01: Deriving Clause Summary

**DERIVING_CLAUSE parser syntax with conditional trait gating across typeck, MIR lowering, and formatter -- backward compatible derive-all when no clause present**

## Performance

- **Duration:** 8 min
- **Started:** 2026-02-08T16:37:02Z
- **Completed:** 2026-02-08T16:44:44Z
- **Tasks:** 2
- **Files modified:** 7 (+ 4 snapshot files)

## Accomplishments
- Added DERIVING_CLAUSE SyntaxKind and parser support for `end deriving(Trait1, Trait2, ...)` on both struct and sum type definitions
- Conditional trait impl registration in typeck and MIR generation gated by deriving clause with full backward compatibility
- Formatter preserves deriving clause in both walk_struct_def and walk_block_def
- All 1095+ existing tests pass with zero modifications (backward compatibility confirmed)

## Task Commits

Each task was committed atomically:

1. **Task 1: Parser syntax for deriving clause + AST accessors** - `e7b2692` (feat)
2. **Task 2: Conditional gating in typeck + MIR lowering + formatter** - `b62864e` (feat)

## Files Created/Modified
- `crates/snow-parser/src/syntax_kind.rs` - Added DERIVING_CLAUSE variant to SyntaxKind enum
- `crates/snow-parser/src/parser/items.rs` - Added parse_deriving_clause() and calls in parse_struct_def/parse_sum_type_def
- `crates/snow-parser/src/ast/item.rs` - Added has_deriving_clause() and deriving_traits() to StructDef and SumTypeDef
- `crates/snow-typeck/src/infer.rs` - Conditional Debug/Eq/Ord/Hash/Display registration based on derive list
- `crates/snow-codegen/src/mir/lower.rs` - Conditional generate_* calls based on derive list
- `crates/snow-fmt/src/walker.rs` - DERIVING_CLAUSE handling in walk_struct_def and walk_block_def
- `crates/snow-parser/tests/parser_tests.rs` - 8 new tests for deriving clause parsing and AST accessors

## Decisions Made
- `deriving` parsed as contextual keyword (regular IDENT with text check) -- not added to TokenKind/keyword_from_str
- No deriving clause = derive all default traits (Debug, Eq, Ord, Hash for structs; Debug, Eq, Ord for sum types) -- preserves backward compatibility
- Explicit `deriving()` with empty parens = derive nothing (has_deriving_clause is true, but derive_list is empty)
- Display trait is NEVER auto-derived (only via explicit `deriving(Display)`) -- it was never part of the default set
- Hash for sum types is only available via explicit `deriving(Hash)` -- it was never auto-derived for sum types

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered

None.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness
- Deriving clause infrastructure complete and wired through full pipeline
- Plan 02 can build on this to add Display/Hash-sum generation functions
- typeck already registers Display and Hash impls for explicit deriving -- Plan 02 only needs the MIR generate_* functions

## Self-Check: PASSED

---
*Phase: 22-auto-derive-stretch*
*Completed: 2026-02-08*
