---
phase: 20
plan: 02
subsystem: codegen-mir
tags: [display, string-interpolation, trait-dispatch, wrap-to-string]
depends_on:
  requires: [20-01]
  provides: [display-string-interpolation-dispatch, primitive-display-runtime-mapping]
  affects: [20-03, 20-04, 20-05]
tech_stack:
  added: []
  patterns: [display-trait-dispatch-in-wrap-to-string, primitive-display-to-runtime-redirect]
key_files:
  created: []
  modified:
    - crates/snow-codegen/src/mir/lower.rs
decisions:
  - id: 20-02-01
    decision: "Primitive Display mangled names redirected to runtime functions at MIR lowering, not codegen"
    rationale: "Avoids needing MIR function bodies for builtin Display impls; snow_int_to_string etc already exist in runtime and are registered as known_functions"
  - id: 20-02-02
    decision: "Display__to_string__String handled as identity (returns arg directly) via short-circuit"
    rationale: "String-to-String conversion is a no-op; avoids needing a runtime identity function"
metrics:
  duration: 10min
  completed: 2026-02-08
---

# Phase 20 Plan 02: Display String Interpolation Dispatch Summary

**One-liner:** Wired wrap_to_string to emit Display__to_string__TypeName for structs/sum types; mapped primitive Display calls to runtime functions in lower_call_expr.

## What Was Done

### Task 1: Update wrap_to_string for Display trait dispatch on user types

Added `MirType::Struct(_) | MirType::SumType(_)` match arms in `wrap_to_string()` (line 2521) before the catch-all `_ =>` case. The new cases:

1. Convert the MirType to a Ty via `mir_type_to_ty()` for trait registry lookup
2. Call `self.trait_registry.find_method_traits("to_string", &ty)` to check for Display impl
3. If Display impl exists, construct a `MirExpr::Call` with mangled name `Display__to_string__TypeName`
4. If no Display impl exists, fall through to the generic `to_string` call (preserving error recovery)

Primitive types (Int, Float, Bool, String) still use direct runtime functions (`snow_int_to_string`, etc.) through the existing match arms, ensuring zero regression for primitive string interpolation.

### Task 2: Map primitive Display dispatch to runtime functions in lower_call_expr

Verified and extended the existing trait call rewriting in `lower_call_expr()` (line 1564) to correctly handle explicit `to_string(x)` calls for all types:

1. **Primitive Display redirect (line 1578-1585):** After the trait dispatch constructs a mangled name like `Display__to_string__Int`, a match redirects it to the corresponding runtime function name (`snow_int_to_string`, `snow_float_to_string`, `snow_bool_to_string`). These runtime names are already registered in `known_functions`, so the call is emitted as a direct `MirExpr::Call`.

2. **String identity short-circuit (line 1609-1615):** After the callee rewriting block, if the callee is `Display__to_string__String`, the first argument is returned directly (the call is elided entirely since String-to-String is identity).

3. **User type dispatch:** For struct/sum types, the mangled name `Display__to_string__TypeName` passes through unchanged and will be resolved at codegen against the user-defined MIR function body.

## Task Commits

| Task | Name | Commit | Key Changes |
|------|------|--------|-------------|
| 1 | Update wrap_to_string for Display trait dispatch | 2bd4168 | lower.rs: +33 lines (Struct/SumType Display dispatch in wrap_to_string) |
| 2 | Map primitive Display dispatch to runtime functions | 11e20d3 | lower.rs: +20 lines (primitive redirect + String identity short-circuit) |

## Verification Results

- `cargo test --workspace`: 1,128 tests pass, 0 failures (zero regressions)
- `cargo build --workspace`: clean compilation (warnings only for existing code)
- String interpolation with primitives: still uses direct runtime functions (no regression)
- `wrap_to_string` for Struct/SumType: emits `Display__to_string__TypeName` when Display impl found
- Explicit `to_string(42)`: dispatches through Display trait -> redirected to `snow_int_to_string`
- Explicit `to_string("hello")`: dispatches through Display -> short-circuited to identity

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 2 - Missing Critical] String identity handling for Display__to_string__String**

- **Found during:** Task 2
- **Issue:** Plan mentioned registering `Display__to_string__String` in `known_functions` or mapping at codegen, but neither approach cleanly handles the identity case
- **Fix:** Added a post-callee-rewriting short-circuit that returns the first argument directly when callee is `Display__to_string__String`, avoiding the need for any runtime function
- **Files modified:** crates/snow-codegen/src/mir/lower.rs
- **Commit:** 11e20d3

## Decisions Made

| ID | Decision | Rationale |
|----|----------|-----------|
| 20-02-01 | Primitive Display mangled names redirected to runtime functions at MIR lowering | Avoids MIR function bodies for builtins; runtime functions already exist |
| 20-02-02 | Display__to_string__String is identity via short-circuit | String-to-String conversion is a no-op |

## Next Phase Readiness

**Unblocked:** String interpolation now dispatches through Display trait for user-defined types. Explicit `to_string()` calls work for all types (primitive and user-defined).

**Ready for:**
- Debug trait registration and dispatch (20-03)
- Eq/Ord for user-defined structs (20-04) -- operator dispatch infrastructure already supports it
- Eq/Ord for sum types (20-05)

**Dependencies satisfied:** The key link from `wrap_to_string` to `find_method_traits` (via `trait_registry`) and from `wrap_to_string` to `mir_type_to_ty` (via types.rs) are both established and working.

## Self-Check: PASSED
