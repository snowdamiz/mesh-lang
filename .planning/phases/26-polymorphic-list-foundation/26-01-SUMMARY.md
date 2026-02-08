---
phase: 26-polymorphic-list-foundation
plan: 01
subsystem: type-system
tags: [parser, typeck, polymorphism, list-literal, codegen]

dependency-graph:
  requires: []
  provides:
    - LIST_LITERAL SyntaxKind and parser support
    - ListLiteral AST node with elements() iterator
    - Polymorphic List<T> function schemes (TyVar 91000/91001)
    - List literal type inference (infer_list_literal)
    - List literal codegen lowering
  affects:
    - 26-02 (end-to-end tests for list literals)
    - Future phases using List<T> polymorphism

tech-stack:
  added: []
  patterns:
    - Polymorphic type schemes for List functions (TyVar 91000 for T, 91001 for U)
    - List literal desugaring to list_new + list_append chain in codegen

key-files:
  created: []
  modified:
    - crates/snow-parser/src/syntax_kind.rs
    - crates/snow-parser/src/parser/expressions.rs
    - crates/snow-parser/src/ast/expr.rs
    - crates/snow-typeck/src/builtins.rs
    - crates/snow-typeck/src/infer.rs
    - crates/snow-codegen/src/mir/lower.rs

decisions:
  - id: 26-01-D1
    decision: "Use ConstraintOrigin::Annotation for list literal element unification"
    rationale: "Matches the pattern used by infer_map_literal; no custom UnifyOrigin needed"
  - id: 26-01-D2
    decision: "Desugar [e1, e2] to list_new() + list_append chain in codegen"
    rationale: "Simplest lowering, matches map literal pattern; runtime already supports list_new/list_append"
  - id: 26-01-D3
    decision: "Fix Map.keys/values to return List<K>/List<V> instead of untyped List"
    rationale: "Now that List is polymorphic, these should be correctly typed"

metrics:
  duration: 8min
  completed: 2026-02-08
---

# Phase 26 Plan 01: Polymorphic List Foundation Summary

Added list literal syntax `[expr, ...]` to the parser and made all List type signatures polymorphic in the type checker. List literal type inference produces `List<T>` with element type unification. Codegen desugars list literals to `list_new + list_append` chains.

## Task Commits

| # | Task | Commit | Key Changes |
|---|------|--------|-------------|
| 1 | Add LIST_LITERAL to parser | `49c1a15` | SyntaxKind, parser NUD branch, ListLiteral AST node |
| 2 | Make list functions polymorphic + list literal inference | `d9be6d0` | Polymorphic schemes in builtins + infer, infer_list_literal, codegen lowering |

## What Was Built

### Parser: LIST_LITERAL Support
- Added `LIST_LITERAL` SyntaxKind variant after `MAP_ENTRY`
- Added `[expr, expr, ...]` parsing in the NUD (prefix) position of the Pratt parser
- Supports empty lists `[]`, single-element `[x]`, multi-element `[1, 2, 3]`, and trailing commas `[1, 2,]`
- No ambiguity with postfix index access `list[0]` (LED position, different code path)
- Added `ListLiteral` AST node with `elements()` iterator

### Type Checker: Polymorphic List Functions
- Replaced all monomorphic list function registrations with polymorphic schemes using `TyVar(91000)` for T and `TyVar(91001)` for U
- `List.append(List<T>, T) -> List<T>` (was `(List, Int) -> List`)
- `List.get(List<T>, Int) -> T` (was `(List, Int) -> Int`)
- `List.head(List<T>) -> T` (was `(List) -> Int`)
- `List.map(List<T>, (T) -> U) -> List<U>` (was `(List, (Int)->Int) -> List`)
- `List.filter(List<T>, (T) -> Bool) -> List<T>` (was `(List, (Int)->Bool) -> List`)
- `List.reduce(List<T>, U, (U, T) -> U) -> U` (was `(List, Int, (Int,Int)->Int) -> Int`)
- Updated bare prelude names (map, filter, reduce, head, tail) with matching polymorphic schemes
- Fixed `Map.keys` to return `List<K>` and `Map.values` to return `List<V>` (were returning untyped `List`)

### Type Checker: List Literal Inference
- Added `infer_list_literal` function: creates a fresh type variable for element type, unifies all elements against it, returns `List<T>`
- Empty list `[]` produces `List<_fresh_var>` (element type resolved from usage context)
- Heterogeneous elements `[1, "a"]` produce a type error (element types must unify)

### Codegen: List Literal Lowering
- Added `lower_list_literal` in MIR lowering: desugars `[e1, e2, e3]` to `snow_list_new() |> snow_list_append(_, e1) |> snow_list_append(_, e2) |> snow_list_append(_, e3)`
- Matches the pattern used by map literal lowering

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Codegen crate needed ListLiteral handling**
- **Found during:** Task 2 verification (cargo build)
- **Issue:** Adding ListLiteral variant to Expr enum caused non-exhaustive pattern error in snow-codegen's lower_expr
- **Fix:** Added ListLiteral import and lower_list_literal function in codegen
- **Files modified:** crates/snow-codegen/src/mir/lower.rs
- **Commit:** d9be6d0

**2. [Rule 1 - Bug] Map.keys/values returned untyped List**
- **Found during:** Task 2 implementation
- **Issue:** With list_t now scoped inside the polymorphic block, Map.keys/values were using the wrong list_t. This exposed that they should have always been typed.
- **Fix:** Changed Map.keys to return List<K> and Map.values to return List<V> (both in builtins.rs and infer.rs)
- **Files modified:** crates/snow-typeck/src/builtins.rs, crates/snow-typeck/src/infer.rs
- **Commit:** d9be6d0

**3. [Rule 3 - Blocking] Range/JSON functions lost scope of list_t and closure types**
- **Found during:** Task 2 implementation
- **Issue:** Moving list functions into a scoped block removed list_t, int_to_int, int_to_bool from outer scope needed by Range and JSON functions
- **Fix:** Re-declared opaque list_t and closure types after the polymorphic block for Range/JSON functions
- **Files modified:** crates/snow-typeck/src/builtins.rs, crates/snow-typeck/src/infer.rs
- **Commit:** d9be6d0

## Verification Results

- `cargo test -p snow-parser`: 220 tests passed (17 unit + 220 integration)
- `cargo test -p snow-typeck`: 249 tests passed (76 unit + 173 integration)
- `cargo build`: Full workspace builds without errors
- `cargo test`: 1,225+ tests passed, 0 failures, 0 regressions

## Decisions Made

1. **ConstraintOrigin::Annotation for list literal unification** -- Reused existing variant rather than adding a new ListLiteral variant to ConstraintOrigin, matching the pattern used by map literal inference.

2. **List literal desugaring strategy** -- Chose list_new + list_append chain (same as map literal pattern) rather than a dedicated runtime function, keeping codegen simple.

3. **Scoped polymorphic block** -- Used a Rust block `{ }` to scope the List type variables, preventing accidental use of polymorphic types in unrelated sections.

## Next Phase Readiness

Phase 26 Plan 02 can proceed immediately:
- LIST_LITERAL parses correctly
- Type inference works for list literals
- Polymorphic list functions are registered
- Codegen produces valid MIR for list literals
- All 1,225+ tests pass

## Self-Check: PASSED
