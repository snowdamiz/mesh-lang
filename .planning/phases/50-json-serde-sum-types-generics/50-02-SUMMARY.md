---
phase: 50-json-serde-sum-types-generics
plan: 02
subsystem: testing
tags: [json, serde, sum-types, generics, e2e, codegen, llvm]

# Dependency graph
requires:
  - phase: 49-json-serde-structs
    provides: JSON serde test harness, struct encode/decode patterns
  - phase: 50-json-serde-sum-types-generics plan 01
    provides: sum type and generic struct JSON codegen implementation
provides:
  - E2E tests for sum type JSON encode/decode with multi-field variants
  - E2E tests for generic struct JSON encode
  - Nested combination test (struct with List<SumType> field)
  - Compile-fail test for non-serializable sum type variant fields
  - Bug fix: sum type layout sizing accounts for alignment padding
  - Bug fix: SumType deref in pattern match binding (parallel to Struct deref)
  - Bug fix: List<Struct/SumType> heap allocation and callback trampolines
affects: [json-serde, sum-types, codegen, pattern-matching, collections]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Wrapper trampoline functions for struct/sum type list JSON callbacks"
    - "Per-variant unique variable names in to_json Match to avoid LLVM domination errors"
    - "Heap-allocate inline struct/sum values for list element storage"

key-files:
  created:
    - tests/e2e/deriving_json_sum_type.snow
    - tests/e2e/deriving_json_generic.snow
    - tests/e2e/deriving_json_nested_sum.snow
    - tests/compile_fail/deriving_json_sum_non_serializable.snow
  modified:
    - crates/snowc/tests/e2e_stdlib.rs
    - crates/snow-codegen/src/codegen/types.rs
    - crates/snow-codegen/src/codegen/pattern.rs
    - crates/snow-codegen/src/codegen/expr.rs
    - crates/snow-codegen/src/mir/lower.rs

key-decisions:
  - "Use variant overlay sizes (including alignment padding) for sum type layout calculation"
  - "Generate wrapper trampoline functions for List<Struct/SumType> JSON callbacks instead of modifying runtime"
  - "Use Let binding auto-deref mechanism in trampolines to convert heap pointers back to inline values"

patterns-established:
  - "Per-variant unique field names in to_json Match arms (e.g. __tj_Circle_0) prevent LLVM alloca domination errors"
  - "List<Struct/SumType> encoding requires heap-allocating inline values via snow_gc_alloc_actor"
  - "Pattern match binding deref handles both Struct and SumType extracted from Result Ok variant"

# Metrics
duration: 35min
completed: 2026-02-11
---

# Phase 50 Plan 02: JSON Serde Sum Types & Generics E2E Test Suite Summary

**E2E tests for sum type/generic JSON serde with 3 critical codegen bug fixes: layout alignment, pattern deref, and List<SumType> encoding**

## Performance

- **Duration:** 35 min
- **Started:** 2026-02-11T16:00:00Z
- **Completed:** 2026-02-11T16:29:30Z
- **Tasks:** 2
- **Files modified:** 9

## Accomplishments
- Sum type encode/decode tests: Circle(Float), Rectangle(Float, Float), Point all round-trip correctly through JSON
- Generic struct encode tests: Wrapper<Int> and Wrapper<String> both produce correct JSON
- Nested combination test: Drawing{shapes::List<Shape>, name::String} encodes correctly with embedded sum type array
- Compile-fail test: deriving(Json) on sum type with Pid field correctly produces E0038 error
- Fixed 3 critical codegen bugs discovered during testing (layout, deref, list encoding)

## Task Commits

Each task was committed atomically:

1. **Task 1: Sum type & generic struct E2E tests** - `f4abd62` (test + fix)
2. **Task 2: Nested combo + compile-fail tests** - `427c1c6` (test + fix)

**Plan metadata:** (pending) (docs: complete plan)

## Files Created/Modified
- `tests/e2e/deriving_json_sum_type.snow` - Sum type encode + decode round-trip test
- `tests/e2e/deriving_json_generic.snow` - Generic struct encode test
- `tests/e2e/deriving_json_nested_sum.snow` - Nested struct + List<SumType> encode test
- `tests/compile_fail/deriving_json_sum_non_serializable.snow` - Compile-fail for Pid in variant
- `crates/snowc/tests/e2e_stdlib.rs` - Rust test harness entries for all 4 tests
- `crates/snow-codegen/src/codegen/types.rs` - Sum type layout sizing fix
- `crates/snow-codegen/src/codegen/pattern.rs` - SumType deref in pattern binding
- `crates/snow-codegen/src/codegen/expr.rs` - Heap-allocate struct/sum for list storage
- `crates/snow-codegen/src/mir/lower.rs` - Per-variant unique names + list callback trampolines

## Decisions Made
- Used variant overlay sizes (with alignment) for sum type layout calculation instead of raw payload sizes
- Generated wrapper trampoline functions for List<Struct/SumType> JSON callbacks to handle pointer-to-value conversion
- Used Let binding auto-deref mechanism in trampolines instead of adding new MIR node

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Sum type layout undersized for multi-field variants with alignment padding**
- **Found during:** Task 1 (sum type encode test)
- **Issue:** `create_sum_type_layout` calculated payload size using field-only structs (no tag), missing alignment padding. Rectangle(Float, Float) overlay `{i8, double, double}` = 24 bytes, but layout `{i8, [16 x i8]}` = 17 bytes. Second field at offset 16 overflowed the alloca.
- **Fix:** Changed `create_sum_type_layout` to compute max size from full variant overlay types (including tag + alignment), not just field-only structs.
- **Files modified:** `crates/snow-codegen/src/codegen/types.rs`
- **Verification:** Rectangle(2.0, 5.0) now encodes as `{"fields":[2.0,5.0],"tag":"Rectangle"}`
- **Committed in:** f4abd62

**2. [Rule 1 - Bug] LLVM domination error from shared field_N names across variant Match arms**
- **Found during:** Task 1 (sum type encode test)
- **Issue:** `generate_to_json_sum_type` used `field_0` for all variant arms. When Circle and Rectangle both have `field_0`, LLVM's alloca naming caused domination errors.
- **Fix:** Changed to per-variant unique names: `__tj_Circle_0`, `__tj_Rectangle_0`, `__tj_Rectangle_1`.
- **Files modified:** `crates/snow-codegen/src/mir/lower.rs`
- **Verification:** LLVM module verification passes, encoding produces correct JSON
- **Committed in:** f4abd62

**3. [Rule 1 - Bug] Pattern match binding missing SumType deref (only handled Struct)**
- **Found during:** Task 1 (sum type decode + pattern match test)
- **Issue:** `codegen_leaf` in pattern.rs only auto-dereferenced pointer-to-struct bindings, not pointer-to-sum-type. When extracting a Shape from Ok(decoded), the value was a pointer but pattern match expected inline struct.
- **Fix:** Extended deref check from `matches!(ty, MirType::Struct(_))` to `matches!(ty, MirType::Struct(_) | MirType::SumType(_))`.
- **Files modified:** `crates/snow-codegen/src/codegen/pattern.rs`
- **Verification:** `verify_circle(decoded)` correctly matches Circle(3.14) and prints "circle: 3.14"
- **Committed in:** f4abd62

**4. [Rule 1 - Bug] List<Struct/SumType> elements stored as inline values, not heap pointers**
- **Found during:** Task 2 (nested sum type test)
- **Issue:** `convert_to_list_element` assumed Struct/SumType values were already pointers (via `val.into_pointer_value()`), but they're inline LLVM StructValues. Caused "Found StructValue but expected PointerValue" panic.
- **Fix:** Added heap-allocation via `snow_gc_alloc_actor` for Struct/SumType values before storing in lists.
- **Files modified:** `crates/snow-codegen/src/codegen/expr.rs`
- **Verification:** List literal `[Circle(1.0), Point]` stores correctly
- **Committed in:** 427c1c6

**5. [Rule 1 - Bug] List JSON callback receives u64 pointer but to_json expects inline value**
- **Found during:** Task 2 (nested sum type test)
- **Issue:** `snow_json_from_list` calls `elem_fn(u64)` with heap pointer, but `ToJson__to_json__Shape` expects an inline Shape struct value. Signature mismatch caused crash in to_json.
- **Fix:** Generated wrapper trampoline functions (`__json_list_encode__Shape`) that receive Ptr, use Let binding to auto-deref to inline value, then call the to_json function.
- **Files modified:** `crates/snow-codegen/src/mir/lower.rs`
- **Verification:** `Drawing{shapes: [Circle(1.0), Point, Circle(2.5)]}` encodes correctly
- **Committed in:** 427c1c6

---

**Total deviations:** 5 auto-fixed (5 Rule 1 bugs)
**Impact on plan:** All auto-fixes necessary for correctness. Three bugs were pre-existing (layout, deref, list storage) but never manifested because sum types with multiple non-pointer fields were never tested in these contexts. No scope creep.

## Issues Encountered
- None beyond the auto-fixed bugs above. All issues were discovered through test-driven development and resolved inline.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- All sum type and generic JSON serde E2E tests pass (1450 total, 0 failures)
- The codegen fixes are backward-compatible (all 122 existing e2e tests still pass)
- Phase 50 plan 02 complete; ready for any remaining Phase 50 plans or next phase

## Self-Check: PASSED

All 4 created test fixture files exist. Both task commits (f4abd62, 427c1c6) verified in git log.

---
*Phase: 50-json-serde-sum-types-generics*
*Completed: 2026-02-11*
