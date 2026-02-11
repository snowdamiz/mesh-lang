---
phase: 49-json-serde-structs
plan: 01
subsystem: runtime
tags: [json, serde, runtime, llvm, codegen, tagged-union]

# Dependency graph
requires:
  - phase: 08-stdlib
    provides: "Base JSON parse/encode runtime, list/map collections, SnowResult type"
provides:
  - "JSON_INT (tag 2) and JSON_FLOAT (tag 6) separate numeric tags"
  - "9 structured JSON functions: object_new/put/get, array_new/push, as_int/as_float/as_string/as_bool"
  - "snow_json_null for Option::None encoding"
  - "4 collection helpers: from_list, from_map, to_list, to_map (callback-based)"
  - "Three-point registration for all 14 functions (runtime, intrinsics, known_functions)"
affects: [49-02, 49-03, json-serde]

# Tech tracking
tech-stack:
  added: []
  patterns: ["callback-based collection encode/decode for generic type support", "SnowResult-returning typed extractors (as_int/as_float/as_string/as_bool)"]

key-files:
  created: []
  modified:
    - "crates/snow-rt/src/json.rs"
    - "crates/snow-codegen/src/codegen/intrinsics.rs"
    - "crates/snow-codegen/src/mir/lower.rs"

key-decisions:
  - "Separate JSON_INT (tag 2) and JSON_FLOAT (tag 6) for round-trip fidelity instead of single JSON_NUMBER"
  - "as_int coerces Float to Int (truncation), as_float promotes Int to Float -- matching numeric widening semantics"
  - "Collection helpers use extern C function pointers (not closures) for callback-based per-element encode/decode"

patterns-established:
  - "Three-point registration: runtime function in json.rs, LLVM declaration in intrinsics.rs, known_functions entry in lower.rs"
  - "Callback-based collection helpers: pass an extern C fn ptr that converts one element, runtime iterates and applies"

# Metrics
duration: 6min
completed: 2026-02-10
---

# Phase 49 Plan 01: JSON Runtime Foundation Summary

**JSON Int/Float tag split and 14 new structured JSON runtime functions (object/array construction, typed extraction, callback-based collection helpers) with full three-point LLVM registration**

## Performance

- **Duration:** 6 min
- **Started:** 2026-02-11T05:18:01Z
- **Completed:** 2026-02-11T05:24:20Z
- **Tasks:** 2
- **Files modified:** 3

## Accomplishments
- Split JSON_NUMBER into JSON_INT (tag 2) and JSON_FLOAT (tag 6) ensuring Int/Float values survive round-trip correctly
- Added 14 new runtime functions: 9 structured (object_new/put/get, array_new/push, as_int/as_float/as_string/as_bool) + snow_json_null + 4 collection helpers (from_list/from_map/to_list/to_map)
- Registered all 14 functions in LLVM intrinsics declarations and MIR known_functions for full three-point registration
- Added 17 new unit tests covering all structured functions, error paths, type coercion, and collection roundtrips

## Task Commits

Each task was committed atomically:

1. **Task 1: Split JSON_NUMBER into JSON_INT and JSON_FLOAT tags** - `1d68df6` (feat)
2. **Task 2: Add 14 new runtime functions with three-point registration** - `6e9e100` (feat)

**Plan metadata:** (pending)

## Files Created/Modified
- `crates/snow-rt/src/json.rs` - Split number tags, added 14 new extern C functions and 17 unit tests
- `crates/snow-codegen/src/codegen/intrinsics.rs` - LLVM declarations for all 14 new functions + test assertions
- `crates/snow-codegen/src/mir/lower.rs` - known_functions entries for all 14 new functions

## Decisions Made
- Used separate JSON_INT (tag 2) and JSON_FLOAT (tag 6) instead of single JSON_NUMBER to ensure Float values survive round-trip as Float and Int values as Int
- as_int coerces Float to Int via truncation, as_float promotes Int to Float -- matching Snow's numeric widening semantics
- Collection helpers (from_list/from_map/to_list/to_map) use extern C function pointer callbacks for per-element encode/decode, enabling generic collection handling without monomorphization

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Removed unnecessary unsafe blocks in from_list and from_map**
- **Found during:** Task 2 (runtime function implementation)
- **Issue:** snow_json_from_list and snow_json_from_map had unnecessary unsafe blocks since all called functions are already safe
- **Fix:** Removed the unsafe blocks, resolving compiler warnings
- **Files modified:** crates/snow-rt/src/json.rs
- **Verification:** cargo test -p snow-rt -- json passes, no warnings from these functions
- **Committed in:** 6e9e100 (Task 2 commit)

---

**Total deviations:** 1 auto-fixed (1 bug)
**Impact on plan:** Minor code quality fix. No scope change.

## Issues Encountered
None

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- All 14 structured JSON runtime functions are available for Plan 02 (MIR lowering for to_json/from_json)
- Three-point registration complete -- compiler can emit calls to all new functions
- Collection helpers ready for List<T> and Map<String, V> field encoding/decoding

## Self-Check: PASSED

All files exist, all commits verified, all content claims confirmed. JSON_NUMBER fully removed from codebase.

---
*Phase: 49-json-serde-structs*
*Completed: 2026-02-10*
