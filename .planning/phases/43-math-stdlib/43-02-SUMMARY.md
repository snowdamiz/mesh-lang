---
phase: 43-math-stdlib
plan: 02
subsystem: compiler
tags: [llvm-intrinsics, math, pow, sqrt, floor, ceil, round, fptosi, stdlib]

# Dependency graph
requires:
  - phase: 43-math-stdlib
    plan: 01
    provides: "Math/Int/Float module infrastructure, LLVM intrinsic dispatch pattern, user module shadowing"
provides:
  - "Math.pow(Float, Float) -> Float via llvm.pow intrinsic"
  - "Math.sqrt(Float) -> Float via llvm.sqrt intrinsic"
  - "Math.floor(Float) -> Int via llvm.floor + fptosi"
  - "Math.ceil(Float) -> Int via llvm.ceil + fptosi"
  - "Math.round(Float) -> Int via llvm.round + fptosi (half away from zero)"
  - "Complete Math stdlib: all 7 MATH requirements satisfied"
affects: [stdlib, codegen]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Float->Int intrinsic pattern: LLVM float intrinsic + fptosi conversion in single codegen arm"

key-files:
  created: []
  modified:
    - "crates/snow-typeck/src/infer.rs"
    - "crates/snow-codegen/src/mir/lower.rs"
    - "crates/snow-codegen/src/codegen/expr.rs"
    - "crates/snowc/tests/e2e_stdlib.rs"

key-decisions:
  - "pow and sqrt are Float-only (not polymorphic) -- users convert with Int.to_float() if needed"
  - "floor/ceil/round return Int (not Float) via fptosi after LLVM intrinsic -- this is the whole purpose of these functions"

patterns-established:
  - "Float->Int intrinsic pattern: call llvm.floor/ceil/round (f64->f64), then build_float_to_signed_int (f64->i64)"

# Metrics
duration: 4min
completed: 2026-02-10
---

# Phase 43 Plan 02: Remaining Math Operations Summary

**Math.pow/sqrt via LLVM intrinsics, Math.floor/ceil/round with Float->Int fptosi conversion -- completing all 7 MATH stdlib requirements**

## Performance

- **Duration:** 4 min
- **Started:** 2026-02-10T01:52:47Z
- **Completed:** 2026-02-10T01:57:04Z
- **Tasks:** 3
- **Files modified:** 4

## Accomplishments
- Math.pow(Float, Float) -> Float and Math.sqrt(Float) -> Float using llvm.pow and llvm.sqrt intrinsics
- Math.floor/ceil/round(Float) -> Int using LLVM intrinsics + fptosi conversion chain
- 7 new e2e tests covering all operations plus combined usage and conversion ergonomics
- All 60 stdlib tests pass, all 111 e2e tests pass, zero regressions
- All 7 MATH requirements now satisfied: MATH-01 (abs), MATH-02 (min/max), MATH-03 (pow/sqrt), MATH-04 (floor/ceil), MATH-05 (round), MATH-06 (pi), MATH-07 (type conversion)

## Task Commits

Each task was committed atomically:

1. **Task 1: Register pow/sqrt/floor/ceil/round type signatures and name mappings** - `058398c` (feat)
2. **Task 2: Implement LLVM intrinsic codegen for pow/sqrt/floor/ceil/round** - `1381fe4` (feat)
3. **Task 3: Add e2e tests for pow, sqrt, floor, ceil, round and integration tests** - `1a42e5f` (test)

## Files Created/Modified
- `crates/snow-typeck/src/infer.rs` - Added pow/sqrt (Float->Float) and floor/ceil/round (Float->Int) type signatures in Math module
- `crates/snow-codegen/src/mir/lower.rs` - Added 5 builtin name mappings (math_pow->snow_math_pow, etc.)
- `crates/snow-codegen/src/codegen/expr.rs` - Added 5 LLVM intrinsic dispatch arms; floor/ceil/round chain intrinsic + fptosi
- `crates/snowc/tests/e2e_stdlib.rs` - Added 7 e2e tests: pow, sqrt, floor, ceil, round, combined usage, pow with conversion

## Decisions Made
- pow and sqrt are Float-only (not polymorphic like abs/min/max) per research recommendation -- keeps API simple, users convert with Int.to_float()
- floor/ceil/round return Int (not Float) via fptosi after LLVM intrinsic -- this matches the requirement "convert Float to Int" and is the purpose of these functions

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered
None.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Math stdlib is complete: all 7 MATH requirements satisfied
- Phase 43 (Math Stdlib) is fully done -- both Plan 01 and Plan 02 complete
- Ready for next v1.9 phase

## Self-Check: PASSED

All files exist, all commits verified.

---
*Phase: 43-math-stdlib*
*Completed: 2026-02-10*
