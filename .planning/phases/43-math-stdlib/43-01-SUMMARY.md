---
phase: 43-math-stdlib
plan: 01
subsystem: compiler
tags: [llvm-intrinsics, math, type-conversion, stdlib, inkwell]

# Dependency graph
requires:
  - phase: 08-stdlib
    provides: "Four-layer stdlib module pattern (typeck, MIR, codegen, runtime)"
  - phase: 39-cross-module
    provides: "User-defined module resolution in lower_field_access"
provides:
  - "Math module with abs, min, max (polymorphic Int/Float) and pi constant"
  - "Int module with to_float conversion"
  - "Float module with to_int conversion (truncation toward zero)"
  - "LLVM intrinsic codegen pattern for math operations (no runtime functions needed)"
  - "User module shadowing precedence over stdlib modules in lower_field_access"
affects: [43-02-PLAN, stdlib, codegen]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "LLVM intrinsic dispatch in codegen_call for type-overloaded math operations"
    - "Compile-time constant emission in codegen_var for Math.pi"
    - "User modules shadow stdlib modules in lower_field_access resolution order"

key-files:
  created: []
  modified:
    - "crates/snow-typeck/src/infer.rs"
    - "crates/snow-codegen/src/mir/lower.rs"
    - "crates/snow-codegen/src/codegen/expr.rs"
    - "crates/snowc/tests/e2e_stdlib.rs"

key-decisions:
  - "Used TyVar(92000) for Math polymorphic type variable to avoid collision with existing ranges"
  - "User-defined modules take precedence over stdlib modules in lower_field_access to prevent name collisions"
  - "Math.pi emitted as compile-time f64 constant in codegen_var, not codegen_call"

patterns-established:
  - "LLVM intrinsic pattern: Intrinsic::find -> get_declaration -> build_call in codegen_call match block"
  - "Module shadowing: user modules checked before stdlib modules in lower_field_access"

# Metrics
duration: 8min
completed: 2026-02-10
---

# Phase 43 Plan 01: Core Math Operations Summary

**Math.abs/min/max (polymorphic Int/Float), Math.pi constant, Int.to_float, Float.to_int via LLVM intrinsics -- zero runtime functions**

## Performance

- **Duration:** 8 min
- **Started:** 2026-02-10T01:41:56Z
- **Completed:** 2026-02-10T01:50:24Z
- **Tasks:** 3
- **Files modified:** 4

## Accomplishments
- Math module with polymorphic abs/min/max using LLVM intrinsics (llvm.abs, llvm.fabs, llvm.smin, llvm.smax, llvm.minnum, llvm.maxnum)
- Math.pi as a compile-time constant accessed without parentheses
- Int.to_float (sitofp) and Float.to_int (fptosi, truncation toward zero) type conversions
- User module shadowing fix: user-defined modules now take precedence over stdlib modules in lower_field_access
- 9 new e2e tests, all 53 stdlib tests pass, all 111 e2e tests pass, zero regressions

## Task Commits

Each task was committed atomically:

1. **Task 1: Register Math/Int/Float modules in typeck and MIR lowering** - `965dac4` (feat)
2. **Task 2: Implement LLVM intrinsic codegen for abs/min/max/pi and type conversions** - `e7fa4b5` (feat)
3. **Task 3: Add e2e tests for core math operations and type conversions** - `60daeb1` (test)

## Files Created/Modified
- `crates/snow-typeck/src/infer.rs` - Added Math/Int/Float module type signatures in stdlib_modules(), added to STDLIB_MODULE_NAMES
- `crates/snow-codegen/src/mir/lower.rs` - Added to STDLIB_MODULES, added builtin name mappings, fixed user module precedence in lower_field_access
- `crates/snow-codegen/src/codegen/expr.rs` - Added Math.pi constant in codegen_var, LLVM intrinsic dispatch in codegen_call for all 6 operations
- `crates/snowc/tests/e2e_stdlib.rs` - Added 9 e2e tests covering all math operations and type conversions

## Decisions Made
- Used TyVar(92000) for Math module polymorphic type variable (avoids collision with List at 91000, Map at 90000, Default at 99000)
- User modules shadow stdlib modules to prevent breaking existing tests with user-defined "math.snow" modules
- Math.pi handled in codegen_var (not codegen_call) since it's a constant accessed without parentheses

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Fixed inkwell API: .left() -> .basic() for try_as_basic_value()**
- **Found during:** Task 2
- **Issue:** Plan used `.left()` method from Either type, but inkwell 0.8.0 uses `.basic()` on ValueKind enum
- **Fix:** Changed all 6 `.left()` calls to `.basic()` in intrinsic result extraction
- **Files modified:** crates/snow-codegen/src/codegen/expr.rs
- **Verification:** cargo check passes
- **Committed in:** e7fa4b5 (Task 2 commit)

**2. [Rule 1 - Bug] Fixed user module shadowing regression from new stdlib modules**
- **Found during:** Task 3 (e2e test verification)
- **Issue:** Adding "Math" to STDLIB_MODULES caused 3 existing e2e tests to fail: user-defined math.snow modules were intercepted by stdlib resolution first, producing invalid function names like "math_add" instead of the user-defined "add"
- **Fix:** Reordered lower_field_access to check user_modules before STDLIB_MODULES, giving user-defined modules shadowing precedence
- **Files modified:** crates/snow-codegen/src/mir/lower.rs
- **Verification:** All 111 e2e tests pass, all 53 stdlib tests pass
- **Committed in:** 60daeb1 (Task 3 commit)

**3. [Rule 1 - Bug] Fixed test using wrong type annotation syntax**
- **Found during:** Task 3
- **Issue:** Plan specified `let x: Int = 42` but Snow uses `::` for type annotations (and only in function params). The test test `int_float_type_annotations_still_work` was invalid.
- **Fix:** Rewrote test as `int_float_module_no_conflict_with_types` to verify Int/Float work as both modules and type-compatible values (Pitfall 7)
- **Files modified:** crates/snowc/tests/e2e_stdlib.rs
- **Verification:** Test passes, confirming no name collision
- **Committed in:** 60daeb1 (Task 3 commit)

---

**Total deviations:** 3 auto-fixed (3 bugs)
**Impact on plan:** All auto-fixes necessary for correctness. No scope creep. The module shadowing fix is essential for backwards compatibility.

## Issues Encountered
None beyond the auto-fixed deviations above.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Math/Int/Float module infrastructure established for Plan 02 (sqrt, pow, floor, ceil, round)
- LLVM intrinsic dispatch pattern in codegen_call ready to extend
- User module shadowing fix ensures future stdlib modules won't break existing user code
- Requirements covered: MATH-01 (abs), MATH-02 (min/max), MATH-06 (pi), MATH-07 (type conversion)

## Self-Check: PASSED

All files exist, all commits verified.

---
*Phase: 43-math-stdlib*
*Completed: 2026-02-10*
