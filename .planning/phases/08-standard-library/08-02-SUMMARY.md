---
phase: 08-standard-library
plan: 02
subsystem: collections
tags: [list, map, set, tuple, range, queue, higher-order-functions, closures, pipe-operator, gc-allocation]
requires: ["08-01"]
provides: ["List/Map/Set/Tuple/Range/Queue runtime implementations", "Collection type registrations in type system", "LLVM intrinsic declarations for all collection functions", "Module-qualified access for 6 collection modules", "Prelude names: map, filter, reduce, head, tail"]
affects: ["08-03", "08-04", "08-05"]
tech-stack:
  added: []
  patterns: ["Uniform u64 element representation for type-erased collections", "Opaque pointer (MirType::Ptr) for all collection types at LLVM level", "Closure calling convention: fn_ptr(env_ptr, args) when env_ptr non-null, fn_ptr(args) when null", "Two-list FIFO queue (back list transferred on front depletion)", "Linear-scan key lookup for small maps/sets (Phase 8 simplicity)"]
key-files:
  created:
    - crates/snow-rt/src/collections/mod.rs
    - crates/snow-rt/src/collections/list.rs
    - crates/snow-rt/src/collections/map.rs
    - crates/snow-rt/src/collections/set.rs
    - crates/snow-rt/src/collections/tuple.rs
    - crates/snow-rt/src/collections/range.rs
    - crates/snow-rt/src/collections/queue.rs
    - tests/e2e/stdlib_list_basic.snow
    - tests/e2e/stdlib_map_basic.snow
    - tests/e2e/stdlib_set_basic.snow
    - tests/e2e/stdlib_range_basic.snow
    - tests/e2e/stdlib_queue_basic.snow
  modified:
    - crates/snow-rt/src/lib.rs
    - crates/snow-typeck/src/ty.rs
    - crates/snow-typeck/src/builtins.rs
    - crates/snow-typeck/src/infer.rs
    - crates/snow-codegen/src/codegen/intrinsics.rs
    - crates/snow-codegen/src/mir/lower.rs
    - crates/snow-codegen/src/mir/types.rs
    - crates/snowc/tests/e2e_stdlib.rs
key-decisions:
  - "Uniform u64 representation: all collection elements stored as 8-byte values (i64 for Int, f64 bits for Float, pointer as u64 for String/collections)"
  - "Linear-scan maps/sets: simple Vec-of-pairs backing for small collections, efficient for Phase 8 typical sizes"
  - "Two-list queue: append-based back list transferred directly to front (no reversal needed since append preserves order)"
  - "Prelude names (map, filter, reduce, head, tail) registered as bare names in builtins, mapped to snow_list_* in MIR lowering"
  - "Collection types resolve to MirType::Ptr (opaque pointers) at LLVM level regardless of type parameters"
patterns-established:
  - "Collection module registration pattern: stdlib_modules() + STDLIB_MODULE_NAMES + builtins env + known_functions + map_builtin_name + intrinsic declarations"
  - "Higher-order function calling convention: runtime transmutes fn_ptr to appropriate function pointer type based on env_ptr nullity"
duration: 12min
completed: 2026-02-06
---

# Phase 8 Plan 02: Collections Suite Summary

Full collection runtime suite (List, Map, Set, Tuple, Range, Queue) with 60+ operations, higher-order function support via closure calling convention, and complete compiler pipeline integration through all 5 compiler passes.

## Performance

- Duration: ~12 minutes
- 2 tasks, 2 commits
- 1537 lines of runtime code + 823 lines of compiler pipeline
- 41 runtime unit tests + 5 E2E integration tests (all passing)

## Accomplishments

### Task 1: Collection Runtime Implementations (snow-rt)

Implemented 6 collection types in `crates/snow-rt/src/collections/`:

**List** (list.rs): GC-allocated contiguous buffer with `{len, cap, data[]}` layout. 12 operations: new, length, append, head, tail, get, concat, reverse, map, filter, reduce, from_array. Higher-order functions (map, filter, reduce) use transmute-based closure invocation -- bare functions called as `fn_ptr(element)`, closures called as `fn_ptr(env_ptr, element)`.

**Map** (map.rs): Vec-of-pairs `(key, value)` backing with linear scan. 8 operations: new, put, get, has_key, delete, size, keys, values. Immutable -- all mutations return new maps.

**Set** (set.rs): Vec backing with deduplication on add. 7 operations: new, add, remove, contains, size, union, intersection.

**Tuple** (tuple.rs): Utility functions for runtime tuple access. 4 operations: nth, first, second, size. Expects GC-allocated `{len, elements[]}` layout.

**Range** (range.rs): Half-open interval `[start, end)` stored as two i64 values. 5 operations: new, to_list, map, filter, length. Higher-order functions produce Lists.

**Queue** (queue.rs): Two-list FIFO implementation. 6 operations: new, push, pop, peek, size, is_empty. Pop returns a GC-allocated pair `{element, new_queue_ptr}`.

### Task 2: Compiler Pipeline Integration

**Type system (ty.rs)**: Added `Ty::list()`, `Ty::map()`, `Ty::set()`, `Ty::range()`, `Ty::queue()` constructors plus untyped variants for opaque pointer semantics.

**Type checker (builtins.rs)**: Registered 50+ collection function signatures with proper types. Prelude names (`map`, `filter`, `reduce`, `head`, `tail`) registered as bare names.

**Type checker (infer.rs)**: Extended `stdlib_modules()` with List, Map, Set, Tuple, Range, Queue modules for module-qualified access (`List.append`, `Map.put`, etc.). Updated `STDLIB_MODULE_NAMES`.

**MIR types (types.rs)**: List/Map/Set/Range/Queue/Tuple all resolve to `MirType::Ptr` (opaque pointers at LLVM level).

**Intrinsics (intrinsics.rs)**: 45 new LLVM function declarations matching runtime signatures.

**MIR lowering (lower.rs)**: Extended `map_builtin_name` with all collection function mappings. Registered all collection functions in `known_functions`. Added bare prelude name mappings (map -> snow_list_map, etc.).

**E2E tests**: 5 integration tests verifying full compile-and-run for List, Map, Set, Range, Queue with module-qualified access and string interpolation output.

## Task Commits

| Task | Name | Commit | Key Files |
|------|------|--------|-----------|
| 1 | Collection runtime implementations | `4ded73f` | collections/{list,map,set,tuple,range,queue}.rs, mod.rs, lib.rs |
| 2 | Compiler pipeline integration and E2E tests | `2492897` | ty.rs, builtins.rs, infer.rs, intrinsics.rs, lower.rs, types.rs, e2e_stdlib.rs |

## Files Created/Modified

### Created (12 files)
- `crates/snow-rt/src/collections/mod.rs` -- Collection module root
- `crates/snow-rt/src/collections/list.rs` -- SnowList with HOF support
- `crates/snow-rt/src/collections/map.rs` -- SnowMap with immutable semantics
- `crates/snow-rt/src/collections/set.rs` -- SnowSet with set operations
- `crates/snow-rt/src/collections/tuple.rs` -- Tuple utility functions
- `crates/snow-rt/src/collections/range.rs` -- Range with HOF support
- `crates/snow-rt/src/collections/queue.rs` -- Two-list FIFO queue
- `tests/e2e/stdlib_list_basic.snow` -- List E2E fixture
- `tests/e2e/stdlib_map_basic.snow` -- Map E2E fixture
- `tests/e2e/stdlib_set_basic.snow` -- Set E2E fixture
- `tests/e2e/stdlib_range_basic.snow` -- Range E2E fixture
- `tests/e2e/stdlib_queue_basic.snow` -- Queue E2E fixture

### Modified (8 files)
- `crates/snow-rt/src/lib.rs` -- Added collections module and re-exports
- `crates/snow-typeck/src/ty.rs` -- Collection type constructors
- `crates/snow-typeck/src/builtins.rs` -- Collection function type registrations
- `crates/snow-typeck/src/infer.rs` -- stdlib_modules() with collection modules
- `crates/snow-codegen/src/codegen/intrinsics.rs` -- 45 new LLVM declarations
- `crates/snow-codegen/src/mir/lower.rs` -- Builtin name mappings and known_functions
- `crates/snow-codegen/src/mir/types.rs` -- Collection -> MirType::Ptr resolution
- `crates/snowc/tests/e2e_stdlib.rs` -- 5 new E2E tests

## Decisions Made

1. **Uniform u64 representation**: All collection elements stored as 8-byte u64 values. Int as i64 reinterpreted, Float as f64 bits, Bool as 0/1, String/collection pointers as u64. This enables type-erased generic collections without monomorphization.

2. **Linear-scan for Map/Set**: Backed by Vec-of-pairs with O(n) lookup. Phase 8 collections are small; hash maps would add complexity for minimal benefit. Keys compared by u64 equality (pointer identity for strings -- documented limitation).

3. **Two-list queue without reversal**: The back list is built with `append` (chronological order), so when transferred to front it maintains FIFO order without needing reversal. This differs from the classic functional queue but is correct for our append-based API.

4. **Prelude bare names**: `map`, `filter`, `reduce`, `head`, `tail` are auto-imported and resolve to List operations. Module-qualified access (`List.map`, `Range.map`) also works for disambiguation.

5. **MirType::Ptr for all collections**: At the LLVM level, all collection types are opaque pointers. Type safety is enforced by the Snow type checker, not at the LLVM level.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Queue FIFO order incorrect with reverse-on-transfer**
- **Found during:** Task 1 unit tests
- **Issue:** Classic two-list queue reverses back list when transferring to front, but since our `append` already maintains chronological order, reversing produced incorrect LIFO behavior.
- **Fix:** Changed `normalize()` to transfer back list directly without reversal.
- **Files modified:** `crates/snow-rt/src/collections/queue.rs`
- **Commit:** Part of `4ded73f`

**2. [Rule 1 - Bug] E2E test fixtures missing fn main() wrapper**
- **Found during:** Task 2 E2E tests
- **Issue:** Snow requires `fn main() do ... end` wrapper; bare expressions at top level produce linking error (no _main symbol).
- **Fix:** Added `fn main() do ... end` wrapper to all collection E2E fixtures.
- **Files modified:** All `tests/e2e/stdlib_*.snow` collection fixtures
- **Commit:** Part of `2492897`

**3. [Rule 1 - Bug] E2E tests used int_to_string which is not registered in builtins**
- **Found during:** Task 2 E2E tests
- **Issue:** `int_to_string` is a runtime function but not registered as a builtin. Snow uses string interpolation (`"${var}"`) for int-to-string conversion.
- **Fix:** Changed test fixtures to use string interpolation instead of `int_to_string`.
- **Files modified:** All collection E2E fixtures
- **Commit:** Part of `2492897`

## Issues for Future Plans

- **String-keyed maps**: Map keys are compared by u64 equality, so string pointers are compared by identity (not content). String-keyed maps would need content-based comparison (could be added in a future plan).
- **string_split**: Deferred from 08-01, now has List type available. Should be added in a future stdlib plan.
- **Higher-order functions with closures in E2E**: The current E2E tests verify basic operations but not HOF with closures. Full pipe chains (`list |> map(fn) |> filter(fn)`) would require closure codegen integration testing.

## Next Phase Readiness

- All collection types available for subsequent plans (08-03 through 08-05)
- Module-qualified access established for List, Map, Set, Tuple, Range, Queue
- Prelude names (map, filter, reduce, head, tail) available without imports
- Higher-order function runtime support in place for closure integration

## Self-Check: PASSED
