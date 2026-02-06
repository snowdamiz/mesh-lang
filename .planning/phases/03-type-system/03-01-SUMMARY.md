---
phase: 03-type-system
plan: 01
subsystem: compiler
tags: [parser, lexer, type-system, hindley-milner, unification, ena, generics, interface]

# Dependency graph
requires:
  - phase: 02-parser-ast
    provides: "Recursive descent parser with CST, typed AST wrappers, Pratt expression parser"
provides:
  - "Angle-bracket generics (<T>) replacing square-bracket generics ([T])"
  - "Interface, impl, type alias, where clause parser support"
  - "Option sugar (Int?) and Result sugar (T!E) in type positions"
  - "snow-typeck crate with Ty enum, ena-based unification, occurs check"
  - "Level-based generalization/instantiation for let-polymorphism"
  - "TypeEnv with scope stack for variable lookups"
  - "Built-in types (Int, Float, String, Bool) and operator registrations"
  - "TypeError with ConstraintOrigin provenance tracking"
affects:
  - 03-02 (expression inference uses InferCtx, TypeEnv, builtins)
  - 03-03 (pattern matching inference uses Ty, unification)
  - 03-04 (trait/interface checking uses InferCtx, TypeEnv, interface AST)
  - 03-05 (type checker integration tests)

# Tech tracking
tech-stack:
  added: [ena 0.14, rustc-hash 2]
  patterns: [ena union-find for type variables, level-based generalization, scope stack environment]

key-files:
  created:
    - crates/snow-typeck/Cargo.toml
    - crates/snow-typeck/src/lib.rs
    - crates/snow-typeck/src/ty.rs
    - crates/snow-typeck/src/unify.rs
    - crates/snow-typeck/src/env.rs
    - crates/snow-typeck/src/builtins.rs
    - crates/snow-typeck/src/error.rs
  modified:
    - Cargo.toml
    - crates/snow-common/src/token.rs
    - crates/snow-lexer/src/lib.rs
    - crates/snow-parser/src/syntax_kind.rs
    - crates/snow-parser/src/parser/items.rs
    - crates/snow-parser/src/parser/mod.rs
    - crates/snow-parser/src/parser/expressions.rs
    - crates/snow-parser/src/ast/item.rs
    - crates/snow-parser/tests/parser_tests.rs

key-decisions:
  - "Angle brackets <T> for generics (migrated from square brackets [T])"
  - "Option/Result sugar emits raw tokens (QUESTION/BANG) inside TYPE_ANNOTATION, not wrapper nodes for simple types"
  - "self keyword accepted as parameter name in fn signatures (needed for interface methods)"
  - "ena::unify_var_var/unify_var_value API instead of union/union_value for fallible unification"
  - "Type variables use Option<Ty> as UnifyKey::Value with level-based generalization side-table"
  - "Builtin operators hardcoded as monomorphic (Int arithmetic) -- trait dispatch deferred to 03-04"
  - "impl block uses parse_module_path for trait/type names (dot-qualified paths)"

patterns-established:
  - "ena-based InferCtx pattern: fresh_var(), resolve(), unify(), generalize(), instantiate()"
  - "TypeEnv scope stack: push_scope/pop_scope for lexical scoping"
  - "ConstraintOrigin provenance on every TypeError for precise error messages"
  - "GENERIC_PARAM_LIST (definition) vs GENERIC_ARG_LIST (usage) distinction in CST"

# Metrics
duration: 12min
completed: 2026-02-06
---

# Phase 3 Plan 1: Parser Migration and Type System Foundation Summary

**Angle-bracket generics, interface/impl/type-alias parsing, and ena-based snow-typeck crate with HM unification, type environment, and provenance-tracked errors**

## Performance

- **Duration:** 12 min
- **Started:** 2026-02-06T18:43:00Z
- **Completed:** 2026-02-06T18:54:50Z
- **Tasks:** 2
- **Files modified:** 30 (9 new + 21 modified including snapshots)

## Accomplishments

- Migrated parser from square-bracket `[T]` to angle-bracket `<T>` generics with GENERIC_PARAM_LIST/GENERIC_ARG_LIST distinction
- Added interface, impl, type alias, and where clause declaration parsers with full CST support
- Created snow-typeck crate with complete HM type infrastructure: Ty enum, ena-based unification with occurs check, level-based generalization/instantiation
- Built type environment with scope stack and built-in type/operator registration
- All 221 workspace tests pass (128 parser + 19 typeck + 74 lexer/common)

## Task Commits

Each task was committed atomically:

1. **Task 1: Lexer and parser migration for Phase 3 syntax** - `7905a30` (feat)
2. **Task 2: snow-typeck crate with type representation, unification, environment, builtins, and errors** - `f6f8d91` (feat)

## Files Created/Modified

**Created:**
- `crates/snow-typeck/Cargo.toml` - Crate manifest with ena, rustc-hash dependencies
- `crates/snow-typeck/src/lib.rs` - Public API: check() placeholder, TypeckResult
- `crates/snow-typeck/src/ty.rs` - Ty enum (Var/Con/Fun/App/Tuple/Never), TyVar, TyCon, Scheme
- `crates/snow-typeck/src/unify.rs` - InferCtx with unification, occurs check, generalize, instantiate
- `crates/snow-typeck/src/env.rs` - TypeEnv scope stack with push/pop/insert/lookup
- `crates/snow-typeck/src/builtins.rs` - Built-in types and operators registration
- `crates/snow-typeck/src/error.rs` - TypeError enum with ConstraintOrigin provenance

**Modified:**
- `crates/snow-common/src/token.rs` - Added Question and Interface token variants
- `crates/snow-lexer/src/lib.rs` - Added ? lexing in normal and interpolation modes
- `crates/snow-parser/src/syntax_kind.rs` - Added 12 new SyntaxKind variants
- `crates/snow-parser/src/parser/items.rs` - Angle-bracket generics, interface/impl/type-alias/where-clause parsers
- `crates/snow-parser/src/parser/mod.rs` - Dispatch for interface, impl, type alias at item level
- `crates/snow-parser/src/parser/expressions.rs` - Accept self keyword as parameter name
- `crates/snow-parser/src/ast/item.rs` - InterfaceDef, ImplDef, TypeAliasDef AST wrappers

## Decisions Made

- **Angle brackets for generics**: Migrated from `[T]` to `<T>`. No ambiguity in type positions since `<` in expressions remains comparison (parse_type only called in type context).
- **Sugar as raw tokens**: Option sugar (`Int?`) and Result sugar (`T!E`) emit QUESTION/BANG tokens directly inside TYPE_ANNOTATION rather than creating wrapper nodes. This keeps simple types backward-compatible with existing snapshots. Wrapper nodes (OPTION_TYPE/RESULT_TYPE) are defined but only used when the sugar explicitly wraps a type.
- **self as parameter**: Interface methods need `self` as a parameter. Extended parse_param to accept SELF_KW alongside IDENT.
- **ena API**: Used `unify_var_var`/`unify_var_value` instead of `union`/`union_value` since `Option<Ty>` has a fallible merge (two `Some` values conflict).
- **Monomorphic builtins**: Arithmetic operators hardcoded as `(Int, Int) -> Int` for now. Trait-based overloading deferred to plan 03-04.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] self keyword not accepted as parameter name**
- **Found during:** Task 1 (interface parsing)
- **Issue:** Interface methods with `self` parameter failed to parse because parse_param only accepted IDENT tokens, not SELF_KW
- **Fix:** Extended parse_param to accept `SyntaxKind::SELF_KW` alongside `SyntaxKind::IDENT`
- **Files modified:** `crates/snow-parser/src/parser/expressions.rs`
- **Verification:** interface_simple and impl_simple snapshot tests pass correctly
- **Committed in:** `7905a30` (Task 1 commit)

**2. [Rule 3 - Blocking] ena union/union_value API incompatible with Option<Ty>**
- **Found during:** Task 2 (unification engine)
- **Issue:** `ena::UnificationTable::union()` requires `UnifyValue::Error = NoError`, but `Option<Ty>` has fallible merge
- **Fix:** Used `unify_var_var`/`unify_var_value` which handle the fallible case
- **Files modified:** `crates/snow-typeck/src/unify.rs`
- **Verification:** All 19 typeck tests pass
- **Committed in:** `f6f8d91` (Task 2 commit)

---

**Total deviations:** 2 auto-fixed (1 bug, 1 blocking)
**Impact on plan:** Both fixes necessary for correctness. No scope creep.

## Issues Encountered

- TOMBSTONE approach for type sugar wrapping initially caused panics in rowan tree builder (TOMBSTONE Open skips start_node but Close still calls finish_node). Resolved by emitting sugar tokens inline without wrapper nodes for simple types.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness

- Parser fully supports Phase 3 syntax: angle-bracket generics, interface/impl/type-alias/where-clause
- snow-typeck crate has all infrastructure needed for expression inference (plan 03-02)
- InferCtx, TypeEnv, and builtins are ready to use
- No blockers for proceeding to plan 03-02

---
*Phase: 03-type-system*
*Completed: 2026-02-06*

## Self-Check: PASSED
