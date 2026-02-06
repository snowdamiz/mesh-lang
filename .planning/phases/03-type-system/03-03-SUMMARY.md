---
phase: 03-type-system
plan: 03
subsystem: type-system
tags: [structs, option, result, type-alias, inference, generics, sugar-syntax, field-access, union-find]

# Dependency graph
requires:
  - phase: 03-type-system/03-02
    provides: "Algorithm J inference engine with let-polymorphism and level-based generalization"
  - phase: 03-type-system/03-01
    provides: "Type representation (Ty, TyCon, TyVar, Scheme), unification, parser with generics syntax"
provides:
  - "Struct type definitions, literals, and field access with generic parameter propagation"
  - "Built-in Option<T> and Result<T, E> with Some/None/Ok/Err constructor inference"
  - "Sugar syntax: Int? -> Option<Int>, T!E -> Result<T, E>"
  - "Transparent type alias resolution with generic parameter substitution"
  - "Struct literal parsing in expression parser"
  - "MissingField, UnknownField, NoSuchField error variants"
affects: [03-05-PLAN, 04-pattern-matching, 05-codegen]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Token-based type annotation resolution from CST (collect_annotation_tokens + parse_type_tokens)"
    - "enter_level/leave_level for proper polymorphic constructor generalization"
    - "Union-find root normalization for correct variable identity during generalization"
    - "Generic type parameter substitution via substitute_type_params()"
    - "Sugar syntax desugaring in type annotation resolution (postfix ? and ! operators)"

key-files:
  created:
    - crates/snow-typeck/tests/structs.rs
  modified:
    - crates/snow-typeck/src/infer.rs
    - crates/snow-typeck/src/ty.rs
    - crates/snow-typeck/src/unify.rs
    - crates/snow-typeck/src/error.rs
    - crates/snow-parser/src/parser/expressions.rs
    - crates/snow-parser/src/ast/expr.rs
    - crates/snow-parser/src/parser/items.rs

key-decisions:
  - "Token-based type annotation parsing rather than AST node traversal for sugar syntax support"
  - "Option/Result constructors registered with enter_level/leave_level for proper generalization"
  - "Union-find resolve() normalizes unbound vars to root key for correct identity"
  - "Struct literals parsed as postfix expressions (NAME_REF followed by L_BRACE)"
  - "Type aliases stored as resolved Ty values parsed from CST tokens after ="

patterns-established:
  - "Token-based recursive descent for type annotations: collect_annotation_tokens() + parse_type_tokens()"
  - "Generic substitution pattern: create fresh vars, zip with param names, substitute in field types"
  - "Sugar desugaring: apply_type_sugar() handles postfix ? and ! on parsed types"

# Metrics
duration: 45min
completed: 2026-02-06
---

# Phase 3 Plan 03: Structs, Option/Result, Type Aliases Summary

**Struct inference with generic field access, built-in Option/Result with Some/None/Ok/Err constructors and Int?/T!E sugar, transparent type alias resolution**

## Performance

- **Duration:** ~45 min
- **Started:** 2026-02-06T10:45:00Z (estimated)
- **Completed:** 2026-02-06T11:27:00Z
- **Tasks:** 2 (TDD RED + GREEN)
- **Files modified:** 8 (1 created, 7 modified)

## Accomplishments

- Struct types with annotated fields can be defined, instantiated as literals, and their fields accessed with full inference including generic parameter propagation
- Option<T> and Result<T, E> are built-in with polymorphic Some/None/Ok/Err constructors that generalize correctly via level-based generalization
- Sugar syntax works in type annotations: `Int?` desugars to `Option<Int>`, `T!E` desugars to `Result<T, E>`
- Type aliases resolve transparently during inference, including generic aliases like `type Pair<A, B> = (A, B)`
- Fixed a critical union-find bug where `resolve()` did not normalize unbound vars to their root representative, causing generalization to create duplicate type variables

## Task Commits

Each task was committed atomically:

1. **Task 1: Write failing tests (RED)** - `ecd711b` (test)
2. **Task 2: Implement inference (GREEN)** - `208e7ee` (feat)

_TDD plan: RED phase wrote 15 failing tests, GREEN phase implemented to pass all 15._

## Files Created/Modified

- `crates/snow-typeck/tests/structs.rs` - 15 integration tests for struct, Option, Result, and type alias inference
- `crates/snow-typeck/src/infer.rs` - Struct literal/field access inference, type annotation resolution with sugar, type alias registration, Option/Result constructor generalization fix
- `crates/snow-typeck/src/unify.rs` - Fixed resolve() to normalize unbound vars to root key in union-find
- `crates/snow-typeck/src/ty.rs` - Added Ty::struct_ty() helper for creating struct types
- `crates/snow-typeck/src/error.rs` - Added MissingField, UnknownField, NoSuchField error variants
- `crates/snow-parser/src/parser/expressions.rs` - Added struct literal postfix parsing (NAME_REF { field: expr })
- `crates/snow-parser/src/ast/expr.rs` - Added StructLiteral and StructLiteralField AST wrappers
- `crates/snow-parser/src/parser/items.rs` - Added tuple type parsing in parse_type()

## Decisions Made

1. **Token-based type annotation parsing**: Rather than traversing AST nodes, we collect significant tokens from the TYPE_ANNOTATION CST node and parse them with a recursive descent parser. This naturally handles nested generics, sugar syntax (? and !), and tuple types without requiring new AST node types.

2. **Option/Result generalization via enter_level/leave_level**: The original code created fresh vars at level 0 and tried to generalize at level 0. Since generalization only captures vars at level > current, this produced monomorphic constructors. Wrapping each registration in enter_level/leave_level creates vars at level 1, allowing proper generalization.

3. **Union-find root normalization**: When `probe_value()` returns `None` for an unbound var, `resolve()` now calls `self.table.find(v)` to get the root representative. Without this, two vars unified via `unify_var_var` (which sets no value) would resolve to different TyVar identities, breaking generalization.

4. **Struct literals as postfix expressions**: The parser recognizes struct literals when a NAME_REF is followed by L_BRACE in postfix position (before function call handling). This avoids ambiguity with block expressions.

5. **Type aliases stored as resolved Ty**: `register_type_alias` parses the CST tokens after `=` to produce the actual aliased `Ty`, not just the alias name. This enables transparent substitution during inference.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Parser doesn't support struct literals**
- **Found during:** Task 2 (struct inference implementation)
- **Issue:** The expression parser had no handling for `Name { field: value }` syntax. STRUCT_LITERAL and STRUCT_LITERAL_FIELD SyntaxKinds existed but nothing produced them.
- **Fix:** Added postfix struct literal parsing in expressions.rs (after NAME_REF, before function call handling) and StructLiteral/StructLiteralField AST wrappers in expr.rs.
- **Files modified:** crates/snow-parser/src/parser/expressions.rs, crates/snow-parser/src/ast/expr.rs
- **Verification:** Struct literal tests parse and infer correctly
- **Committed in:** 208e7ee (Task 2 commit)

**2. [Rule 3 - Blocking] Parser doesn't support tuple types in annotations**
- **Found during:** Task 2 (type alias generic test)
- **Issue:** `parse_type()` only handled IDENT-based types, not `(A, B)` tuple types. This blocked `type Pair<A, B> = (A, B)`.
- **Fix:** Added L_PAREN check at start of parse_type() in items.rs to parse tuple types.
- **Files modified:** crates/snow-parser/src/parser/items.rs
- **Verification:** Generic type alias test passes
- **Committed in:** 208e7ee (Task 2 commit)

**3. [Rule 1 - Bug] Option/Result constructors not properly generalized**
- **Found during:** Task 2 (Option/Result inference)
- **Issue:** `register_option_result_constructors` created fresh vars at level 0, then called `generalize` at level 0. Since generalization only captures vars at level > current level, all constructors were monomorphic.
- **Fix:** Wrapped each constructor registration in `ctx.enter_level()` / `ctx.leave_level()` so vars are created at level 1 and properly generalized at level 0.
- **Files modified:** crates/snow-typeck/src/infer.rs
- **Verification:** test_option_some_inference, test_option_generic_propagation pass
- **Committed in:** 208e7ee (Task 2 commit)

**4. [Rule 1 - Bug] Union-find resolve() doesn't normalize to root key**
- **Found during:** Task 2 (option generic propagation test)
- **Issue:** When two unbound vars are unified via `unify_var_var`, `probe_value` returns `None` for both. `resolve()` returned `Ty::Var(v)` with the original var, not the root representative. This caused generalization to treat unified-but-unbound vars as separate variables, breaking type sharing in polymorphic functions.
- **Fix:** Changed `resolve()` to call `self.table.find(v)` to get root key when `probe_value` returns `None`.
- **Files modified:** crates/snow-typeck/src/unify.rs
- **Verification:** test_option_generic_propagation returns `Option<Int>` instead of `Option<?11>`
- **Committed in:** 208e7ee (Task 2 commit)

**5. [Rule 1 - Bug] Type alias stored alias name instead of aliased type**
- **Found during:** Task 2 (type alias test)
- **Issue:** `register_type_alias` stored `Ty::Con(TyCon::new(&name))` (the alias name itself) instead of parsing the actual aliased type from the CST.
- **Fix:** Added `parse_alias_type()` that collects tokens after `=` sign in the TYPE_ALIAS_DEF CST node and parses them into a proper Ty value.
- **Files modified:** crates/snow-typeck/src/infer.rs
- **Verification:** test_type_alias_simple and test_type_alias_generic pass
- **Committed in:** 208e7ee (Task 2 commit)

---

**Total deviations:** 5 auto-fixed (2 blocking, 3 bugs)
**Impact on plan:** All auto-fixes necessary for correctness. Parser support for struct literals and tuple types was required but not in plan scope. Union-find normalization bug was a subtle correctness issue that would have affected all polymorphic types. No scope creep.

## Issues Encountered

- **Parallel plan 03-04 restructured infer.rs**: Plan 03-04 (traits/interfaces) was running concurrently and significantly restructured `infer.rs` with TraitRegistry, fn_constraints, 7-parameter function signatures, and trait-based operator dispatch. My changes had to be made additive to 03-04's restructured code. The Task 2 commit includes both plans' changes in shared files since they cannot be cleanly separated.
- **3 trait tests from 03-04 fail**: `test_where_clause_satisfied`, `test_where_clause_unsatisfied`, `test_multiple_where_constraints` fail. Verified these are pre-existing failures from 03-04's incomplete GREEN phase (same failures without my changes via `git stash` test).

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness

- Struct, Option, Result, and type alias inference is complete and tested
- 15 new integration tests provide regression coverage
- Ready for plan 03-05 (final type system plan)
- Plan 03-04 (traits) is in progress with 3 remaining test failures in its GREEN phase
- The union-find root normalization fix benefits all future polymorphic type features

## Self-Check: PASSED

---
*Phase: 03-type-system*
*Completed: 2026-02-06*
