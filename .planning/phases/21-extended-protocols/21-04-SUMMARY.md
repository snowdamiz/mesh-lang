---
phase: 21
plan: 04
subsystem: protocols
tags: [display, collections, list, map, set, runtime, mir, string-interpolation]
requires: ["20-01", "20-02", "21-03"]
provides: ["Collection Display via callback-based runtime helpers"]
affects: ["22-xx (any future collection extensions)"]
tech-stack:
  added: []
  patterns: ["callback function pointers for element-to-string conversion"]
key-files:
  created: []
  modified:
    - crates/snow-rt/src/collections/list.rs
    - crates/snow-rt/src/collections/map.rs
    - crates/snow-rt/src/collections/set.rs
    - crates/snow-rt/src/string.rs
    - crates/snow-codegen/src/codegen/intrinsics.rs
    - crates/snow-codegen/src/mir/lower.rs
key-decisions:
  - "Callback fn(u64)->*mut u8 signature for uniform element-to-string conversion"
  - "snow_string_to_string identity function for string elements in collections"
  - "wrap_to_string accepts Option<&Ty> for collection type resolution from typeck"
  - "Nested collections (List<List<Int>>) fall back to snow_int_to_string (v1.3 limitation)"
duration: "11min"
completed: "2026-02-08"
---

# Phase 21 Plan 04: Collection Display Summary

Collection Display via callback-based runtime helpers that accept element-to-string function pointers, with MIR lowering that resolves element types from typeck Ty::App.

## Accomplishments

### Task 1: Runtime collection-to-string helpers (9f2ed86)
- Added `snow_list_to_string(list, elem_to_str)` producing `[elem1, elem2, ...]`
- Added `snow_map_to_string(map, key_to_str, val_to_str)` producing `%{k => v, ...}`
- Added `snow_set_to_string(set, elem_to_str)` producing `#{elem1, elem2, ...}`
- Added `snow_string_to_string` identity function for string element callbacks
- All helpers accept bare `fn(u64) -> *mut u8` callback for element conversion
- 6 runtime tests: list/map/set to_string + empty collection variants

### Task 2: MIR lowering for collection Display dispatch (dff4b49)
- Declared `snow_list_to_string`, `snow_map_to_string`, `snow_set_to_string`, `snow_string_to_string` in LLVM intrinsics
- Added `wrap_collection_to_string` helper that matches `Ty::App(Con("List"|"Map"|"Set"), args)` and emits runtime calls
- Added `resolve_to_string_callback` that maps element types to runtime to_string function names (Int -> `snow_int_to_string`, String -> `snow_string_to_string`, user types -> `Display__to_string__TypeName`)
- Extended `wrap_to_string` with `Option<&Ty>` parameter for collection type resolution
- Added collection Display interception in `lower_call_expr` for explicit `to_string(collection)` calls
- Registered collection Display functions in `known_functions`
- 3 MIR tests: list/map/set Display emit correct runtime calls with correct callbacks

## Task Commits

| Task | Commit | Description |
|------|--------|-------------|
| 1 | 9f2ed86 | Runtime collection-to-string helpers with callback function pointers |
| 2 | dff4b49 | MIR lowering for collection Display dispatch + intrinsics declarations |

## Files Modified

| File | Changes |
|------|---------|
| crates/snow-rt/src/collections/list.rs | +snow_list_to_string + 2 tests |
| crates/snow-rt/src/collections/map.rs | +snow_map_to_string + 2 tests |
| crates/snow-rt/src/collections/set.rs | +snow_set_to_string + 2 tests |
| crates/snow-rt/src/string.rs | +snow_string_to_string identity |
| crates/snow-codegen/src/codegen/intrinsics.rs | +4 collection Display declarations + test assertions |
| crates/snow-codegen/src/mir/lower.rs | +wrap_collection_to_string, resolve_to_string_callback, collection call-site dispatch, 3 tests |

## Decisions Made

1. **Callback signature**: `fn(u64) -> *mut u8` -- uniform signature matches how all Snow values are stored as u64 in collections. The MIR lowerer passes the address of the appropriate runtime function (snow_int_to_string, snow_float_to_string, etc.).

2. **snow_string_to_string identity**: For `List<String>`, the element callback is an identity function (`snow_string_to_string`) that casts the u64 pointer back to `*mut SnowString`. This avoids special-casing string elements in the runtime helpers.

3. **typeck_ty parameter**: `wrap_to_string` now accepts `Option<&Ty>` to resolve collection types. String interpolation passes the typeck Ty from the expression's text range; Debug inspect passes None (struct fields with collection types get generic fallback).

4. **Nested collection fallback**: `List<List<Int>>` falls back to `snow_int_to_string` as the callback -- a known v1.3 limitation that produces incorrect but safe output.

## Deviations from Plan

### Auto-added Missing Functionality

**1. [Rule 2 - Missing Critical] snow_string_to_string identity function**
- **Found during:** Task 1
- **Issue:** No existing function to use as a callback for string elements in collections. `Display__to_string__String` is short-circuited to identity in MIR but has no actual function symbol.
- **Fix:** Added `snow_string_to_string(val: u64) -> *mut SnowString` in string.rs as a simple identity cast.
- **Files modified:** crates/snow-rt/src/string.rs
- **Commit:** 9f2ed86

## Next Phase Readiness

Phase 21 (Extended Protocols) is now complete:
- Plan 01: Hash protocol (FNV-1a runtime + auto-derive + Map key hashing)
- Plan 02: Default protocol (static method + primitive short-circuits)
- Plan 03: Default method implementations (parser + typeck + MIR lowering)
- Plan 04: Collection Display (callback-based runtime helpers + MIR dispatch)

All v1.3 trait protocol work is done. Ready for Phase 22 (final milestone phase).

## Self-Check: PASSED
