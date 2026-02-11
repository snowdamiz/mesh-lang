---
phase: 50-json-serde-sum-types-generics
plan: 01
subsystem: compiler
tags: [json, serde, sum-types, generics, codegen, mir, typeck, deriving]

# Dependency graph
requires:
  - phase: 49-json-serde-structs
    provides: "Struct JSON codegen (ToJson/FromJson), emit_to/from_json_for_type, runtime helpers, If-based Result propagation pattern"
provides:
  - "snow_json_array_get runtime function for indexed JSON array access"
  - "generate_to_json_sum_type: Match-based MIR for sum type -> tagged JSON object"
  - "generate_from_json_sum_type: If-chain MIR for tagged JSON object -> sum type"
  - "ToJson/FromJson trait impl registration for sum types in typeck"
  - "is_json_serializable accepts generic type params (T, U, V)"
  - "Json.encode dispatches for both Struct and SumType arguments"
  - "SumTypeName.from_json resolution in typeck and MIR"
  - "emit_to/from_json_for_type handle non-Option SumType"
affects: [50-02, e2e-json-tests, json-round-trip]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Tagged union JSON encoding: {\"tag\":\"Variant\",\"fields\":[...]}"
    - "If-chain tag matching for from_json (consistent with Phase 49 If-based Result propagation)"
    - "Unique per-variant variable names to avoid LLVM domination errors"
    - "Single-letter uppercase heuristic for generic type param serializability"

key-files:
  created: []
  modified:
    - "crates/snow-rt/src/json.rs"
    - "crates/snow-codegen/src/codegen/intrinsics.rs"
    - "crates/snow-codegen/src/mir/lower.rs"
    - "crates/snow-typeck/src/infer.rs"

key-decisions:
  - "Use array encoding for all variant fields (positional and named) -- simpler, unambiguous, matches success criteria"
  - "Single-letter uppercase heuristic for is_json_serializable generic params -- invalid instantiations fail at link time"
  - "If-chain for from_json tag matching (not Match on SnowResult) -- consistent with Phase 49 lesson"
  - "Unique variable names per variant in from_json (e.g., __fval_Circle_0, __fields_arr_Circle) to avoid LLVM SSA domination errors"

patterns-established:
  - "Sum type JSON: generate_to_json_sum_type follows generate_hash_sum_type pattern (Match on self, per-variant arms)"
  - "Sum type from_json: build_variant_from_json_body extracts fields array then decodes each element by index"
  - "Three-point registration for new runtime functions: runtime Rust + LLVM declaration + known_functions"

# Metrics
duration: 9min
completed: 2026-02-11
---

# Phase 50 Plan 01: Sum Type JSON Codegen Summary

**Tagged union JSON encoding for sum types (Match-based to_json, If-chain from_json), Json.encode/from_json dispatch, snow_json_array_get runtime, and is_json_serializable fix for generic type params**

## Performance

- **Duration:** 9 min
- **Started:** 2026-02-11T15:58:17Z
- **Completed:** 2026-02-11T16:07:26Z
- **Tasks:** 2
- **Files modified:** 4

## Accomplishments
- Three-point registration of snow_json_array_get (runtime + LLVM + known_functions) with 3 unit tests
- Full sum type JSON codegen: generate_to_json_sum_type (Match-based, tagged JSON objects), generate_from_json_sum_type (If-chain tag matching), build_variant_from_json_body (per-field array decoding)
- Typeck registration: ToJson/FromJson impls for sum types, SumTypeName.from_json resolution, is_json_serializable accepts generic type params
- Dispatch wiring: Json.encode handles both Struct and SumType, SumTypeName.from_json checks struct_defs and sum_type_defs
- emit_to/from_json_for_type handle non-Option SumType branches
- All 1446 existing tests pass with zero regressions

## Task Commits

Each task was committed atomically:

1. **Task 1: Runtime + LLVM registration for snow_json_array_get** - `2c28f91` (feat)
2. **Task 2: Typeck registration + MIR generation + dispatch wiring** - `cc07786` (feat)

## Files Created/Modified
- `crates/snow-rt/src/json.rs` - Added snow_json_array_get runtime function + 3 unit tests
- `crates/snow-codegen/src/codegen/intrinsics.rs` - LLVM declaration for snow_json_array_get
- `crates/snow-codegen/src/mir/lower.rs` - generate_to_json_sum_type, generate_from_json_sum_type, build_variant_from_json_body, sum type dispatch in Json.encode/from_json, emit_to/from_json SumType branches, known_functions registration
- `crates/snow-typeck/src/infer.rs` - ToJson/FromJson impl registration for sum types, SumTypeName.from_json typeck resolution, is_json_serializable generic type param fix

## Decisions Made
- Used array encoding for all variant fields (positional and named) -- simpler and matches the success criteria format `{"tag":"Variant","fields":[...]}`
- Single-letter uppercase heuristic for is_json_serializable generic params (T, U, V, K, A, B) -- invalid instantiations fail at link time with missing function errors
- If-chain for from_json tag matching (not Match) -- consistent with Phase 49's established pattern for runtime SnowResult handling
- Unique per-variant variable names in from_json (e.g., `__fval_Circle_0`, `__fields_arr_Circle`) to avoid LLVM SSA domination errors across different branches

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Fixed snow_list_get parameter type in snow_json_array_get**
- **Found during:** Task 1 (snow_json_array_get implementation)
- **Issue:** Plan's code example used `index as u64` but snow_list_get takes i64
- **Fix:** Changed to pass `index` directly (already i64)
- **Files modified:** crates/snow-rt/src/json.rs
- **Verification:** cargo test -p snow-rt passes
- **Committed in:** 2c28f91 (Task 1 commit)

---

**Total deviations:** 1 auto-fixed (1 bug)
**Impact on plan:** Trivial type fix. No scope creep.

## Issues Encountered
None

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- All production code for sum type JSON is in place
- Ready for Plan 02: E2E integration tests to verify end-to-end behavior
- Compiler supports deriving(Json) on sum types and generic structs

---
*Phase: 50-json-serde-sum-types-generics*
*Completed: 2026-02-11*

## Self-Check: PASSED
