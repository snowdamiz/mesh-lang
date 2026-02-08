---
phase: 14-generic-map-types
plan: 01
subsystem: runtime, typeck, codegen
tags: [map, generics, polymorphism, string-keys, key-comparison, HM-inference]

# Dependency graph
requires:
  - phase: 08-stdlib-collections
    provides: "Basic Map<Int,Int> runtime, MIR lowering, and codegen"
provides:
  - "Polymorphic Map<K,V> type signatures in typeck (Scheme with type vars)"
  - "String-key map comparison via snow_string_eq content equality"
  - "Runtime key_type tag in map header for dispatch"
  - "Codegen ptr-to-int and int-to-ptr coercion for uniform value representation"
  - "snow_map_tag_string runtime function for lazy key_type tagging"
affects:
  - "15-polish (any future map-related features)"
  - "future generic collection work (List<T>, Set<T>)"

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Polymorphic Scheme with high-numbered placeholder TyVars (90000, 90001) for stdlib builtins"
    - "Runtime key_type tag packed in upper 8 bits of capacity field"
    - "Lazy key-type tagging at first put site via snow_map_tag_string wrapper"
    - "LLVM-level argument coercion: ptr-to-i64 and i64-to-ptr for uniform value representation"

key-files:
  created:
    - "tests/e2e/stdlib_map_string_keys.snow"
  modified:
    - "crates/snow-rt/src/collections/map.rs"
    - "crates/snow-typeck/src/infer.rs"
    - "crates/snow-typeck/src/builtins.rs"
    - "crates/snow-codegen/src/mir/lower.rs"
    - "crates/snow-codegen/src/codegen/intrinsics.rs"
    - "crates/snow-codegen/src/codegen/expr.rs"
    - "crates/snowc/tests/e2e_stdlib.rs"

key-decisions:
  - "Use lazy key_type tagging at Map.put site instead of Map.new (HM generalization prevents type resolution at new)"
  - "Pack key_type tag in upper 8 bits of capacity field (56-bit capacity is more than sufficient)"
  - "Use snow_map_tag_string wrapper call in MIR instead of modifying snow_map_put signature"
  - "Add bidirectional LLVM coercion (ptr-to-i64 for args, i64-to-ptr for returns) in codegen_call"

patterns-established:
  - "Polymorphic stdlib modules: use Scheme with TyVar(90000+) placeholders, instantiate() creates fresh vars"
  - "Lazy runtime tagging: when compile-time type info is unavailable due to HM generalization, detect at usage site"
  - "Uniform value representation: all map keys/values are u64 at runtime, with ptr<->int casts in codegen"

# Metrics
duration: 18min
completed: 2026-02-08
---

# Phase 14 Plan 01: Generic Map Types Summary

**Polymorphic Map<K,V> with string-key support via key_type tag dispatch and lazy tagging at Map.put**

## Performance

- **Duration:** 18 min
- **Started:** 2026-02-08T00:27:26Z
- **Completed:** 2026-02-08T00:45:26Z
- **Tasks:** 3
- **Files modified:** 7

## Accomplishments
- Map type signatures are now fully polymorphic: Map<K,V> with type variable inference
- String-key maps work end-to-end: Map.put/get/has_key/delete/size all use content comparison
- Runtime key_type tag enables dispatch between integer equality and snow_string_eq
- Bidirectional LLVM coercion handles ptr<->i64 conversion for uniform value representation
- Zero regressions: all 29 e2e tests and full test suite pass

## Task Commits

Each task was committed atomically:

1. **Task 1: Runtime key_type tag and string-aware key comparison** - `0ea3f04` (feat)
2. **Task 2: Polymorphic Map type signatures in typeck and generic MIR/codegen dispatch** - `de6e0b3` (feat)
3. **Task 2 fix: Lazy key_type tagging at Map.put** - `e81eb17` (fix)
4. **Task 3: String-key Map e2e test** - `a6051fb` (test)

## Files Created/Modified
- `crates/snow-rt/src/collections/map.rs` - Key_type tag constants, keys_equal dispatch, snow_map_new_typed, snow_map_tag_string
- `crates/snow-typeck/src/infer.rs` - Polymorphic Map module with Scheme{vars:[K,V]} in stdlib_modules()
- `crates/snow-typeck/src/builtins.rs` - Polymorphic Map builtins (map_new, map_put, etc.) with type variables
- `crates/snow-codegen/src/mir/lower.rs` - snow_map_tag_string in known_functions, string-key detection in lower_call_expr
- `crates/snow-codegen/src/codegen/intrinsics.rs` - LLVM declarations for snow_map_new_typed and snow_map_tag_string
- `crates/snow-codegen/src/codegen/expr.rs` - ptr-to-i64 and i64-to-ptr coercion in codegen_call
- `tests/e2e/stdlib_map_string_keys.snow` - E2e test: string-key map operations
- `crates/snowc/tests/e2e_stdlib.rs` - e2e_map_string_keys test function

## Decisions Made
- **Lazy key_type tagging instead of Map.new() dispatch:** HM let-generalization creates a polymorphic scheme for `let m = Map.new()`, meaning the type variables from Map.new() are never unified with the concrete types from Map.put(). The fix: detect string keys at Map.put/get/has_key/delete call sites in MIR lowering, and wrap the map argument with `snow_map_tag_string()` to ensure string comparison is used.
- **Bidirectional ptr/int coercion in codegen:** Runtime map functions use uniform u64 values. When string pointers are passed as map keys/values, codegen emits ptrtoint. When map_get returns a string value as u64, codegen emits inttoptr. This is a general-purpose coercion in the runtime intrinsic call path, benefiting all functions.
- **Key_type packed in capacity upper bits:** Using 8 bits of the 64-bit capacity field for the key_type tag. This avoids adding a new field to the map header, maintaining the same memory layout.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] HM let-generalization prevents type resolution at Map.new()**
- **Found during:** Task 3 (integration testing)
- **Issue:** The plan specified determining key_type at Map.new() via resolved Ty::App(Map, [K, V]). However, HM let-generalization creates a polymorphic scheme for `let m = Map.new()`, so the type variables TyVar(7), TyVar(8) from instantiation are generalized away and never unified with String from Map.put().
- **Fix:** Changed approach to lazy tagging: detect string keys at Map.put/get/has_key/delete call sites in MIR lowering, wrap map argument with snow_map_tag_string() call. Added snow_map_tag_string runtime function that upgrades empty maps to string key_type.
- **Files modified:** crates/snow-codegen/src/mir/lower.rs, crates/snow-rt/src/collections/map.rs, crates/snow-codegen/src/codegen/intrinsics.rs
- **Verification:** Both e2e_map_basic and e2e_map_string_keys pass
- **Committed in:** e81eb17

**2. [Rule 2 - Missing Critical] ptr-to-int and int-to-ptr coercion in codegen_call**
- **Found during:** Task 2 (codegen implementation)
- **Issue:** Runtime map functions expect i64 arguments for keys/values, but string values at LLVM level are ptr type. Similarly, map_get returns i64 but the caller may expect ptr for string values.
- **Fix:** Extended the argument coercion logic in codegen_call to handle PointerValue->IntValue (ptrtoint) when runtime expects i64, and IntValue->PointerValue (inttoptr) when caller expects ptr/string.
- **Files modified:** crates/snow-codegen/src/codegen/expr.rs
- **Verification:** String keys/values correctly converted at LLVM level
- **Committed in:** de6e0b3

---

**Total deviations:** 2 auto-fixed (1 bug, 1 missing critical)
**Impact on plan:** Both fixes necessary for correctness. The approach change from compile-time to runtime-level key_type tagging is a fundamental design improvement that avoids fighting the HM type system. No scope creep.

## Issues Encountered
- HM let-generalization was the main challenge: understanding why TyVar(7) from Map.new() was not being resolved to String required deep analysis of the type inference flow (enter_level/leave_level/generalize). The solution (lazy tagging) is cleaner than fighting the type system.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Map<String, V> and Map<Int, V> both work through the full compiler pipeline
- The polymorphic type signatures enable future Map<K, V> with any key/value types
- The ptr-to-int coercion in codegen is general-purpose and benefits other uniform-value functions
- No blockers for subsequent phases

## Self-Check: PASSED

---
*Phase: 14-generic-map-types*
*Completed: 2026-02-08*
