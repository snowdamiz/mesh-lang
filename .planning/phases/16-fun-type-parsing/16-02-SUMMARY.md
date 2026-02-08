---
phase: 16-fun-type-parsing
plan: 02
subsystem: compiler
tags: [typeck, codegen, function-types, type-annotations, unification]

# Dependency graph
requires:
  - "16-01 (FUN_TYPE CST node kind and parse_type() Fun() parsing)"
provides:
  - "Type checker resolves Fun(params) -> RetType into Ty::Fun"
  - "Codegen handles Fun-typed parameters as MirType::Closure"
  - "End-to-end test proving full Fun() type annotation pipeline"
affects:
  - "Future phases that add function-typed parameters to user-defined functions"

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Token-level Fun() detection in parse_type_tokens (name == \"Fun\" && L_PAREN)"
    - "is_closure flag for resolve_type when param type is Ty::Fun"
    - "User-fn vs runtime-intrinsic distinction for closure arg splitting in codegen"

key-files:
  created:
    - "tests/e2e/fun_type.snow"
  modified:
    - "crates/snow-typeck/src/infer.rs"
    - "crates/snow-codegen/src/mir/lower.rs"
    - "crates/snow-codegen/src/codegen/expr.rs"
    - "crates/snowc/tests/e2e.rs"

key-decisions:
  - "Fun-typed params resolve as MirType::Closure (not FnPtr) so LLVM signature accepts {ptr, ptr} struct"
  - "Closure args not split for user-defined functions (only split for runtime intrinsics)"

# Metrics
duration: 8min
completed: 2026-02-08
---

# Phase 16 Plan 02: Type Checker Fun() Resolution Summary

**Type checker resolves Fun(params) -> RetType annotations into Ty::Fun with codegen closure-param fix and e2e test coverage**

## Performance

- **Duration:** 8 min
- **Started:** 2026-02-08T03:18:22Z
- **Completed:** 2026-02-08T03:25:52Z
- **Tasks:** 2/2
- **Files created:** 1
- **Files modified:** 4

## Accomplishments

- Added `SyntaxKind::ARROW` to both token collection sites in `infer.rs` (main `collect_annotation_tokens` and inline type alias collection)
- Added `Fun(params) -> RetType` parsing in `parse_type_tokens()` producing `Ty::Fun(param_tys, Box::new(ret_ty))`
- Fixed codegen: `Ty::Fun`-typed function parameters now resolve as `MirType::Closure` (not `FnPtr`), so LLVM signatures correctly accept `{ptr, ptr}` closure structs
- Fixed codegen: closure arguments are no longer split into `(fn_ptr, env_ptr)` pairs when calling user-defined functions (splitting only applies to runtime intrinsics)
- Created comprehensive e2e test exercising all three requirements:
  - TYPE-01: Single-param, zero-arity, and multi-param function types
  - TYPE-02: Function type annotations in parameters and type aliases
  - TYPE-03: Closures unify with explicit Fun() type annotations
- All existing tests pass (0 regressions)

## Task Commits

Each task was committed atomically:

1. **Task 1: Add ARROW token collection and Fun() handling to type checker** - `45e5eb6` (feat)
2. **Task 2: Add end-to-end tests for Fun() type annotations** - `c2ccc4e` (feat)

## Files Created/Modified

- `crates/snow-typeck/src/infer.rs` - Added ARROW to two token collection sites + Fun() handling in parse_type_tokens
- `crates/snow-codegen/src/mir/lower.rs` - Fun-typed params resolved with is_closure=true for MirType::Closure
- `crates/snow-codegen/src/codegen/expr.rs` - Closure args passed as struct (not split) to user-defined functions
- `crates/snowc/tests/e2e.rs` - Added e2e_fun_type_annotations test
- `tests/e2e/fun_type.snow` - End-to-end test: apply, run_thunk, apply2 with closure unification

## Decisions Made

1. **Fun-typed params as MirType::Closure** -- When a function parameter has type `Ty::Fun(..)`, it is resolved with `is_closure_context: true`, producing `MirType::Closure` instead of `MirType::FnPtr`. This ensures the LLVM function signature accepts `{ptr, ptr}` closure structs, matching how closures are actually passed at runtime.

2. **No closure splitting for user functions** -- The codegen already split closure `{ptr, ptr}` structs into separate `(fn_ptr, env_ptr)` arguments for runtime intrinsics (map, filter, reduce). This splitting must NOT apply to user-defined functions whose LLVM signatures expect the struct directly. Added `is_user_fn` check to gate splitting.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Codegen Fun-typed parameter resolution**

- **Found during:** Task 2 (e2e test failed with LLVM verification error)
- **Issue:** `resolve_type(param_ty, registry, false)` produced `MirType::FnPtr` (single ptr) for `Ty::Fun`-typed parameters, but closures are `{ptr, ptr}` structs at runtime. LLVM function signatures had wrong parameter count.
- **Fix:** Pass `is_closure_context: true` when param type is `Ty::Fun(..)` in all three parameter resolution sites (regular functions, actor handlers, service init)
- **Files modified:** `crates/snow-codegen/src/mir/lower.rs`
- **Commit:** `c2ccc4e`

**2. [Rule 3 - Blocking] Codegen closure argument splitting for user functions**

- **Found during:** Task 2 (same LLVM verification error)
- **Issue:** `codegen_call` always split `MirType::Closure` arguments into `(fn_ptr, env_ptr)` pairs, even for user-defined functions whose signatures now expect `{ptr, ptr}` structs.
- **Fix:** Added `is_user_fn` check -- only split closures when calling runtime intrinsics, pass struct directly for user functions.
- **Files modified:** `crates/snow-codegen/src/codegen/expr.rs`
- **Commit:** `c2ccc4e`

## Issues Encountered

None beyond the codegen fixes documented above.

## User Setup Required

None -- no external service configuration required.

## Next Phase Readiness

- Fun() type annotations are now fully functional: parse -> typecheck -> codegen -> runtime
- All three requirements met: TYPE-01 (parsing), TYPE-02 (positions), TYPE-03 (unification)
- Phase 16 is complete
- No blockers for subsequent phases

---
*Phase: 16-fun-type-parsing*
*Completed: 2026-02-08*

## Self-Check: PASSED
