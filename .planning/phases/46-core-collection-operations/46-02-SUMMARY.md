---
phase: "46"
plan: "02"
subsystem: "string"
tags: [string, runtime, split, join, to_int, to_float, option, parsing, e2e]
dependency-graph:
  requires: ["Phase 46 Plan 01 (shared SnowOption module, list builder)"]
  provides: ["String.split", "String.join", "String.to_int", "String.to_float"]
  affects: ["snow-rt", "snow-typeck", "snow-codegen", "snowc e2e tests"]
tech-stack:
  added: []
  patterns: ["list builder for string-to-list conversion", "f64::to_bits for float Option storage"]
key-files:
  created:
    - tests/e2e/stdlib_string_split_join.snow
    - tests/e2e/stdlib_string_parse.snow
  modified:
    - crates/snow-rt/src/string.rs
    - crates/snow-rt/src/lib.rs
    - crates/snow-typeck/src/infer.rs
    - crates/snow-typeck/src/builtins.rs
    - crates/snow-codegen/src/mir/lower.rs
    - crates/snow-codegen/src/codegen/intrinsics.rs
    - crates/snowc/tests/e2e_stdlib.rs
key-decisions:
  - "Used unique variable names in case arms to avoid pre-existing LLVM alloca naming collision"
  - "Used Ptr MIR types (not String) for all 4 new functions since they interact with List/Option opaque pointers"
  - "Added bare name mappings for split/join but not to_int/to_float (to avoid conflict with Int.to_float/Float.to_int)"
metrics:
  duration: "~6 min"
  completed: "2026-02-10"
---

# Phase 46 Plan 02: String Split/Join/Parse Operations Summary

String split, join, to_int, and to_float implemented across all 4 compiler layers with safe Option returns and full e2e test coverage.

## Performance

- **Duration:** ~6 min
- **Started:** 2026-02-10T09:00:32Z
- **Completed:** 2026-02-10T09:06:06Z
- **Tasks:** 3
- **Files modified:** 9

## Accomplishments
- String.split and String.join for splitting/joining strings with List<String>
- String.to_int and String.to_float with safe Option<Int>/Option<Float> returns
- Pattern matching on parse results works correctly (Some/None)
- All 66 e2e tests pass (0 regressions)

## Task Commits

Each task was committed atomically:

1. **Task 1: Implement String runtime functions** - `f2ee319` (feat)
2. **Task 2: Register across typeck, MIR, codegen** - `8d90998` (feat)
3. **Task 3: Add e2e tests** - `e998b78` (test)

## Files Created/Modified
- `crates/snow-rt/src/string.rs` - 4 new extern "C" functions: snow_string_split, snow_string_join, snow_string_to_int, snow_string_to_float
- `crates/snow-rt/src/lib.rs` - Re-exports for new string functions
- `crates/snow-typeck/src/infer.rs` - String module map entries (split, join, to_int, to_float)
- `crates/snow-typeck/src/builtins.rs` - Flat env entries (string_split, string_join, string_to_int, string_to_float)
- `crates/snow-codegen/src/mir/lower.rs` - map_builtin_name + known_functions entries with Ptr types
- `crates/snow-codegen/src/codegen/intrinsics.rs` - LLVM external declarations for all 4 functions
- `crates/snowc/tests/e2e_stdlib.rs` - 2 new e2e test functions
- `tests/e2e/stdlib_string_split_join.snow` - Split/join roundtrip test
- `tests/e2e/stdlib_string_parse.snow` - to_int/to_float with pattern matching

## Decisions Made
- Used `MirType::Ptr` (not `MirType::String`) for all 4 functions in known_functions since they interact with List and Option opaque pointers
- Added bare name mappings for `split` and `join` but not for `to_int`/`to_float` to avoid collision with `Int.to_float`/`Float.to_int` from other modules
- Used unique variable names (`n1`, `n2`, `f1`, `f2`, `n3`) in case arms of parse test to work around pre-existing LLVM alloca naming collision when same binding name is reused across multiple case blocks

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Fixed Snow test syntax: case arms use newlines not pipe separators**
- **Found during:** Task 3
- **Issue:** Plan specified `| None -> ...` syntax with pipe separators in case arms, but Snow uses newline-separated arms without pipes
- **Fix:** Used correct Snow syntax with newline-separated case arms
- **Files modified:** tests/e2e/stdlib_string_parse.snow
- **Commit:** e998b78

**2. [Rule 1 - Bug] Used unique variable names in case arms to avoid LLVM alloca collision**
- **Found during:** Task 3
- **Issue:** Reusing the same variable name (`n`) across multiple case blocks triggers "Instruction does not dominate all uses" LLVM verification error (pre-existing codegen limitation)
- **Fix:** Used unique names (`n1`, `n2`, `f1`, `f2`, `n3`) across case arms
- **Files modified:** tests/e2e/stdlib_string_parse.snow
- **Commit:** e998b78

**3. [Rule 1 - Bug] Added fn main wrapper to test fixtures**
- **Found during:** Task 3
- **Issue:** Plan showed test code without `fn main() do ... end` wrapper; Snow requires main function
- **Fix:** Wrapped all test fixture code in `fn main() do ... end`
- **Files modified:** tests/e2e/stdlib_string_split_join.snow, tests/e2e/stdlib_string_parse.snow
- **Commit:** e998b78

---

**Total deviations:** 3 auto-fixed (3 bugs in plan's test code)
**Impact on plan:** All fixes necessary for correct test execution. Core implementation unchanged.

## Issues Encountered
None beyond the test syntax issues noted in deviations.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Phase 46 complete: all core collection operations (List sort/find/any/all/contains + String split/join/to_int/to_float) implemented
- Ready for next v1.9 phase

---
*Phase: 46-core-collection-operations*
*Completed: 2026-02-10*

## Self-Check: PASSED

All 9 modified/created files found. All 3 task commits verified (f2ee319, 8d90998, e998b78).
