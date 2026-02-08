---
phase: 26-polymorphic-list-foundation
plan: 02
subsystem: codegen
tags: [llvm, mir, list-literal, polymorphic, codegen, from-array, list-concat]

# Dependency graph
requires:
  - phase: 26-01
    provides: "List literal parsing, polymorphic type signatures, list literal type inference"
provides:
  - "ListLit MIR variant with snow_list_from_array codegen"
  - "Polymorphic list element storage/retrieval (Bool, Float, String, Ptr)"
  - "List ++ operator via snow_list_concat"
  - "End-to-end list literal compilation for all element types"
affects: []

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "ListLit MIR -> stack-allocate i64 array + snow_list_from_array (single allocation)"
    - "Uniform u64 value storage with type-aware conversion (Bool zext, Float bitcast, Ptr ptrtoint)"
    - "Runtime return type coercion for polymorphic functions (i64 -> Bool/Float/Ptr)"

key-files:
  created:
    - "tests/e2e/list_literal_int.snow"
    - "tests/e2e/list_literal_string.snow"
    - "tests/e2e/list_literal_bool.snow"
    - "tests/e2e/list_concat.snow"
    - "tests/e2e/list_nested.snow"
    - "tests/e2e/list_append_string.snow"
  modified:
    - "crates/snow-codegen/src/mir/mod.rs"
    - "crates/snow-codegen/src/mir/lower.rs"
    - "crates/snow-codegen/src/mir/mono.rs"
    - "crates/snow-codegen/src/codegen/expr.rs"
    - "crates/snow-codegen/src/pattern/compile.rs"
    - "crates/snowc/tests/e2e_stdlib.rs"

key-decisions:
  - "ListLit MIR variant + snow_list_from_array instead of append chain (single allocation, O(n) vs O(n^2))"
  - "Uniform u64 storage with codegen-level type conversion (no runtime type tags)"
  - "snow_list_head/get/reduce return Ptr in known_functions, actual type from typeck resolve_range"

patterns-established:
  - "Polymorphic collection value conversion: store as i64 (zext/bitcast/ptrtoint), retrieve with inverse"
  - "Float coercion for runtime args: f64 -> i64 bitcast when function expects i64"
  - "Runtime return type widening: known_functions returns Ptr, codegen uses MIR ty for LLVM type conversion"

# Metrics
duration: 13min
completed: 2026-02-08
---

# Phase 26 Plan 02: Polymorphic List Codegen Summary

**ListLit MIR variant with snow_list_from_array codegen, polymorphic element storage/retrieval for Bool/Float/String/Ptr, and list ++ concatenation operator**

## Performance

- **Duration:** 13 min
- **Started:** 2026-02-08T22:10:29Z
- **Completed:** 2026-02-08T22:23:41Z
- **Tasks:** 2
- **Files modified:** 11 (5 Rust source, 6 test fixtures)

## Accomplishments
- `[1, 2, 3]` compiles as List<Int> via efficient single-allocation snow_list_from_array
- `["hello", "world"]` compiles as List<String> with correct pointer storage/retrieval
- `[true, false]` compiles as List<Bool> with zero-extend/truncate conversion
- `[[1, 2], [3, 4]]` compiles as List<List<Int>> with nested pointer handling
- `[1, 2] ++ [3, 4]` produces `[1, 2, 3, 4]` via snow_list_concat
- List.get/head return correctly-typed values for all element types
- List.append works with String, Bool, Float elements (runtime arg coercion)
- All 1,212 tests pass (6 new e2e tests, 0 regressions from 1,206 baseline)

## Task Commits

Each task was committed atomically:

1. **Task 1: Add ListLit to MIR and lower list literals** - `30159f5` (feat)
2. **Task 2: Codegen for list literals, polymorphic value conversion, and list ++** - `27375eb` (feat)

## Files Created/Modified
- `crates/snow-codegen/src/mir/mod.rs` - Added MirExpr::ListLit { elements, ty } variant
- `crates/snow-codegen/src/mir/lower.rs` - Changed lower_list_literal to produce ListLit; updated known_functions return types for list_head/get/reduce to Ptr
- `crates/snow-codegen/src/mir/mono.rs` - Added ListLit match arm in collect_function_refs
- `crates/snow-codegen/src/codegen/expr.rs` - ListLit codegen (stack array + from_array), list concat dispatch, polymorphic return conversion (Bool i64->i1, Float i64->f64, Ptr/Struct i64->ptr), Float arg coercion (f64->i64 bitcast)
- `crates/snow-codegen/src/pattern/compile.rs` - Added ListLit match arm in compile_expr_patterns
- `crates/snowc/tests/e2e_stdlib.rs` - 6 new e2e test functions
- `tests/e2e/list_literal_int.snow` - Int list literal test
- `tests/e2e/list_literal_string.snow` - String list literal test
- `tests/e2e/list_literal_bool.snow` - Bool list literal test
- `tests/e2e/list_concat.snow` - List ++ concatenation test
- `tests/e2e/list_nested.snow` - Nested List<List<Int>> test
- `tests/e2e/list_append_string.snow` - List.append with String values test

## Decisions Made

1. **ListLit MIR variant replaces append chain** -- Plan 01 desugared `[e1, e2, e3]` to `list_new() |> list_append(e1) |> list_append(e2) |> list_append(e3)` which is O(n^2) due to repeated copying. Plan 02 introduces a dedicated ListLit MIR variant that codegen translates to a single snow_list_from_array call -- O(n) with one allocation.

2. **known_functions return types widened to Ptr** -- Changed snow_list_head, snow_list_get, and snow_list_reduce return types from MirType::Int to MirType::Ptr. The actual type at codegen time comes from resolve_range (typeck), ensuring correct LLVM type conversion. This is safe because Int and Ptr are both 8 bytes at LLVM level.

3. **Polymorphic return conversion extended** -- Added i64->f64 bitcast for Float returns and expanded i64->i1 truncation to handle any bit width > 1 (not just i8). Also added i64->ptr for Struct/SumType/Pid types. This generalizes the existing map_get return coercion to work for all polymorphic collection operations.

4. **Float arg coercion added** -- Added f64->i64 bitcast in the runtime intrinsic argument coercion logic. This was missing for Float values passed to functions like snow_list_append that expect uniform u64 elements.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 2 - Missing Critical] Added Float argument coercion for runtime functions**
- **Found during:** Task 2 (analyzing codegen_call coercion logic)
- **Issue:** Float values passed to snow_list_append would cause LLVM type mismatch (f64 vs i64)
- **Fix:** Added BasicMetadataValueEnum::FloatValue arm in runtime arg coercion: f64 -> i64 bitcast
- **Files modified:** crates/snow-codegen/src/codegen/expr.rs
- **Verification:** Float list operations compile correctly
- **Committed in:** 27375eb (Task 2 commit)

**2. [Rule 2 - Missing Critical] Extended return type conversion for Struct/SumType/Pid**
- **Found during:** Task 2 (reviewing polymorphic return paths)
- **Issue:** Only String | Ptr had i64->ptr conversion; Struct, SumType, Pid would get raw i64
- **Fix:** Added Struct(_) | SumType(_) | Pid(_) to the inttoptr conversion check
- **Files modified:** crates/snow-codegen/src/codegen/expr.rs
- **Verification:** All existing tests pass
- **Committed in:** 27375eb (Task 2 commit)

---

**Total deviations:** 2 auto-fixed (2 missing critical)
**Impact on plan:** Both fixes essential for correct polymorphic value handling. No scope creep.

## Issues Encountered
None.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- All LIST requirements (LIST-01 through LIST-05) are complete
- List literals, polymorphic operations, and concatenation all working end-to-end
- Ready for next phase (27 or beyond)

## Self-Check: PASSED

---
*Phase: 26-polymorphic-list-foundation*
*Completed: 2026-02-08*
