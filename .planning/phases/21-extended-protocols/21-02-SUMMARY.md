---
phase: 21
plan: 02
subsystem: type-system
tags: [default, traits, static-method, mir-lowering, zero-initialization]
depends_on:
  requires: ["21-01"]
  provides: ["Default trait with static method pattern, primitive default short-circuits"]
  affects: ["21-03", "21-04"]
tech-stack:
  added: []
  patterns: ["Static trait method (has_self=false)", "Call-site type resolution for return type", "Polymorphic builtin function registration"]
key-files:
  created: []
  modified:
    - crates/snow-typeck/src/builtins.rs
    - crates/snow-codegen/src/mir/lower.rs
key-decisions:
  - decision: "Register default() as polymorphic function () -> T in typeck env"
    reason: "Typeck needs to resolve the callee type for default() calls; without env registration, infer_name_ref returns UnboundVariable error and the call expression type is never recorded in the types map"
  - decision: "Use :: for type annotations in test Snow source (not :)"
    reason: "Snow uses :: for type annotations in let bindings (let x :: Int = expr), consistent with function parameter syntax (a :: Int)"
  - decision: "Default dispatch before trait method rewriting in lower_call_expr"
    reason: "Existing trait dispatch requires !args.is_empty() to check first arg type; default() has no args, so it needs separate detection path using call-site type from resolve_range"
  - decision: "TyVar(99000) for default's type parameter"
    reason: "High-numbered TyVar to avoid collision with typeck's fresh variable allocation"
duration: "12min"
completed: "2026-02-08"
---

# Phase 21 Plan 02: Default Protocol Summary

**Default trait with static method (no self), primitive short-circuits to MIR literals, call-site type resolution via polymorphic function registration**

## Performance

- Duration: ~12 min
- Tests: 5 new (1 typeck + 4 MIR lowering), 0 regressions
- Lines added: ~199 (57 typeck + 142 codegen)

## Accomplishments

1. **Default trait registered as first static trait in Snow** -- `has_self: false`, `return_type: None` (Self), establishing the pattern for future static trait methods.

2. **Primitive Default impls** -- Int, Float, String, Bool registered with concrete return types in TraitRegistry.

3. **Polymorphic `default()` in typeck env** -- Registered as `Scheme { vars: [T], ty: () -> T }` so typeck can resolve the return type from context (e.g., `let x :: Int = default()` unifies T with Int).

4. **Primitive Default short-circuits in MIR** -- `default()` for Int/Float/Bool/String returns MIR literals (IntLit(0), FloatLit(0.0), BoolLit(false), StringLit("")) without emitting any function call.

5. **User-defined Default support** -- Non-primitive types emit `MirExpr::Call { func: Default__default__TypeName, args: [] }`, relying on the standard impl lowering pipeline for the method body.

6. **Error recovery** -- When type can't be resolved (missing annotation, no inference context), emits warning (not panic), following the 19-03 error recovery pattern.

## Task Commits

| Task | Name | Commit | Key Changes |
|------|------|--------|-------------|
| 1 | Register Default trait + primitive impls | `37089a8` | Default trait def (has_self=false), 4 primitive impls, typeck test |
| 2 | Default__default primitive short-circuits + call-site type resolution | `b37ba18` | Polymorphic default() in env, MIR short-circuits, 4 lowering tests |

## Files Modified

| File | Changes |
|------|---------|
| `crates/snow-typeck/src/builtins.rs` | Default trait + impls registration, `default()` polymorphic env entry, test |
| `crates/snow-codegen/src/mir/lower.rs` | Bare `default()` detection in lower_call_expr, primitive short-circuits, user type call emission, 4 tests |

## Decisions Made

1. **Polymorphic env registration for default()**: Typeck requires `default` to be in the env for infer_name_ref to succeed. Registered as `() -> T` with fresh type variable T, so call-site type annotation drives resolution.

2. **Snow type annotation syntax**: Uses `::` (not `:`) for let-binding type annotations, matching function parameter syntax.

3. **Default dispatch ordering**: Placed before the existing trait method rewriting block (which requires `!args.is_empty()`), since `default()` has zero arguments.

4. **TyVar(99000)**: High-numbered type variable for the polymorphic default function to avoid collision with typeck's incremental fresh variable allocation.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] default() unresolvable without typeck env registration**
- **Found during:** Task 2
- **Issue:** The plan assumed `default()` call type could be resolved purely from call-site context in MIR lowering. However, typeck must first successfully infer the call expression type, which requires `default` to be in the TypeEnv. Without registration, `infer_name_ref` returns `UnboundVariable` error and the call expression type is never stored.
- **Fix:** Added `default` as a polymorphic function `() -> T` in `register_builtins`, enabling typeck to resolve the return type from type annotation context.
- **Files modified:** `crates/snow-typeck/src/builtins.rs`
- **Commit:** `b37ba18`

**2. [Rule 1 - Bug] Snow syntax uses :: for type annotations, not :**
- **Found during:** Task 2 (test writing)
- **Issue:** Initial tests used `let x: Int = default()` but Snow's parser expects `let x :: Int = default()` for type annotations.
- **Fix:** Updated all four test sources to use `::` syntax.
- **Files modified:** `crates/snow-codegen/src/mir/lower.rs`
- **Commit:** `b37ba18`

## Issues Found

None.

## Next Phase Readiness

- Default protocol fully operational for primitives
- Static trait method pattern (has_self=false) established for reuse in future protocols
- User-defined Default for structs ready (impl lowering pipeline handles the generated call)
- Ready for 21-03 (next protocol in phase 21)

## Self-Check: PASSED
