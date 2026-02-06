---
phase: "04"
plan: "04"
subsystem: "type-checker"
tags: ["exhaustiveness", "pattern-matching", "guards", "redundancy", "wiring"]
depends_on:
  requires: ["04-02", "04-03"]
  provides: ["exhaustiveness-wiring", "guard-validation", "redundancy-warnings"]
  affects: ["04-05"]
tech-stack:
  added: []
  patterns: ["AST-to-abstract pattern conversion", "guard expression validation", "exhaustiveness TypeRegistry bridge"]
key-files:
  created:
    - "crates/snow-typeck/tests/exhaustiveness_integration.rs"
  modified:
    - "crates/snow-typeck/src/infer.rs"
    - "crates/snow-typeck/src/error.rs"
    - "crates/snow-typeck/src/diagnostics.rs"
    - "crates/snow-typeck/src/lib.rs"
    - "crates/snow-typeck/src/unify.rs"
decisions:
  - id: "04-04-guard-validation"
    description: "Guards restricted to comparisons, boolean ops, literals, name refs, and named function calls"
    rationale: "Guards must be simple boolean expressions for predictable match semantics"
  - id: "04-04-guarded-exclusion"
    description: "Guarded arms excluded from exhaustiveness matrix (treated as potentially non-matching)"
    rationale: "A guard may not match, so guarded arms cannot guarantee coverage"
  - id: "04-04-multiclause-deferred"
    description: "Multi-clause function definitions deferred to a future plan"
    rationale: "Requires parser changes (pattern parameters), function grouping in type checker, and cross-clause exhaustiveness -- too complex for this plan scope"
  - id: "04-04-redundancy-warnings"
    description: "Redundant arms stored in ctx.warnings, not ctx.errors"
    rationale: "Redundant arms are not fatal -- the match still works, but the arm is unreachable"
metrics:
  duration: "7min"
  completed: "2026-02-06"
---

# Phase 4 Plan 04: Exhaustiveness Wiring & Guard Validation Summary

Wired Maranget's exhaustiveness algorithm (Plan 03) into case/match inference (Plan 02), with guard expression validation and comprehensive integration tests.

## Task Commits

| Task | Type | Commit | Description |
|------|------|--------|-------------|
| 1 | feat | `4b22a0a` | Exhaustiveness/redundancy wiring, guard validation, 16 integration tests |
| 2 | -- | deferred | Multi-clause function definitions (documented as future work) |

## What Was Built

### AST-to-Abstract Pattern Conversion

`ast_pattern_to_abstract(pat, env, type_registry) -> AbsPat`

Converts Snow AST patterns to the abstract `Pat` representation used by the exhaustiveness algorithm:

- `Pattern::Wildcard` -> `Pat::Wildcard`
- `Pattern::Ident` -> `Pat::Wildcard` (variables match anything) OR `Pat::Constructor` (if ident resolves to a nullary variant)
- `Pattern::Literal` -> `Pat::Literal` with appropriate `LitKind`
- `Pattern::Tuple` -> `Pat::Constructor { name: "Tuple", args }`
- `Pattern::Constructor` -> `Pat::Constructor { name, type_name, args }`
- `Pattern::Or` -> `Pat::Or { alternatives }`
- `Pattern::As` -> recurse into inner pattern

### Type-to-TypeInfo Conversion

`type_to_type_info(ty, type_registry) -> AbsTypeInfo`

Converts resolved scrutinee types to abstract type info:
- `Bool` -> `TypeInfo::Bool`
- Sum types -> `TypeInfo::SumType { variants }`
- Everything else -> `TypeInfo::Infinite`

### Exhaustiveness Registry Bridge

`build_abs_type_registry(type_registry) -> AbsTypeRegistry`

Bridges the infer-level TypeRegistry (with SumTypeDefInfo) to the exhaustiveness-level TypeRegistry (with ConstructorSig). Also registers builtin types (Bool, Option, Result).

### Guard Expression Validation

`validate_guard_expr(expr) -> Result<(), String>`

Validates that guard expressions use only allowed constructs:
- Comparisons: `==`, `!=`, `<`, `>`, `<=`, `>=`
- Boolean ops: `and`, `or`, `&&`, `||`
- Unary: `not`, `!`
- Literals, name references, parenthesized grouping
- Named function calls (builtins)

Guard type is unified with Bool.

### Wiring in infer_case

The `infer_case` function now:
1. Collects abstract patterns and guard status for each arm
2. After type-checking all arms, calls `check_exhaustiveness` with unguarded patterns only
3. Calls `check_redundancy` with all patterns
4. Emits `NonExhaustiveMatch` as a hard error
5. Emits `RedundantArm` as a warning

### New Error Types

- `NonExhaustiveMatch { scrutinee_type, missing_patterns, span }` -- E0012
- `RedundantArm { arm_index, span }` -- W0001 (warning)
- `InvalidGuardExpression { reason, span }` -- E0013

### Warnings Infrastructure

- Added `warnings: Vec<TypeError>` to `InferCtx` and `TypeckResult`
- Warnings rendered with `ReportKind::Warning` in ariadne diagnostics
- Redundant arms use warning severity, not error

## Test Coverage (16 integration tests)

- **Non-exhaustive**: Sum type missing variant, Bool missing false, Int without wildcard
- **Exhaustive**: Sum type all variants, Bool both values, wildcard covers all, Int with wildcard
- **Redundancy**: Wildcard-first (arm 1 redundant), duplicate arm (arm 1 redundant), no redundancy
- **Guards**: Excluded from exhaustiveness, guarded + unguarded fallback, comparison guard, boolean op guard, pattern binding in guard
- **Or-patterns**: All variants covered via or-pattern

## Decisions Made

1. **Guard validation scope**: Guards allow comparisons, boolean ops, literals, name refs, and named function calls. Complex expressions (lambdas, case expressions, assignments) are rejected. This keeps guards predictable.

2. **Guarded arm exclusion**: Guarded arms are excluded from the exhaustiveness matrix because the guard might not match. An arm `Circle(_) when r > 0 -> ...` does not guarantee that Circle is fully covered.

3. **Multi-clause functions deferred**: Multi-clause function definitions (`fn fact(0) -> 1; fn fact(n) -> n * fact(n-1)`) require parser changes to support pattern parameters, function grouping logic, and cross-clause exhaustiveness. This is a significant feature that deserves its own plan.

4. **Redundancy is warning, not error**: Redundant arms are unreachable but don't affect correctness. They use the warning channel (`ctx.warnings`) and `ReportKind::Warning` in diagnostics, with W0001 code.

## Deviations from Plan

None -- plan executed as written. Multi-clause functions explicitly allowed to be deferred per plan note.

## Deferred Work

### Multi-clause Function Definitions

**What**: Adjacent `fn` definitions with the same name grouped as pattern-matching clauses:
```
fn fact(0) -> 1
fn fact(n) -> n * fact(n - 1)
```

**Why deferred**: Requires:
1. Parser: `parse_param` extended to accept pattern parameters (literal, wildcard, tuple, constructor)
2. Type checker: Adjacent same-name fn grouping, arity consistency check, return type unification
3. Exhaustiveness: Cross-clause parameter pattern exhaustiveness checking

**Recommended plan**: Dedicate a future plan (e.g., 05-XX) to multi-clause functions with proper parser groundwork.

## Next Phase Readiness

All Plan 04 success criteria are met:
- Exhaustiveness integrated into `infer_case`
- NonExhaustiveMatch is a hard error
- RedundantArm is a warning
- Guards work with restricted expressions
- Multi-clause documented as deferred

Phase 4 can proceed to Plan 05 (if any remaining) or phase completion.

## Self-Check: PASSED
