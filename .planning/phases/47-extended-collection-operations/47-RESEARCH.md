# Phase 47: Extended Collection Operations - Research

**Researched:** 2026-02-10
**Domain:** Snow stdlib extension -- List zip/flat_map/flatten/enumerate/take/drop + Map merge/to_list/from_list + Set difference/to_list/from_list
**Confidence:** HIGH

## Summary

Phase 47 adds 12 new collection operations across three modules (List, Map, Set), building directly on the 4-layer registration pattern established in Phase 46. The operations divide into three groups:

1. **List operations (COLL-05 through COLL-08):** `zip`, `flat_map`, `flatten`, `enumerate`, `take`, and `drop`. The key complexity is that `zip` and `enumerate` return `List<(A, B)>` / `List<(Int, T)>` -- lists of tuples. The runtime must allocate GC-managed tuples for each pair element. Snow tuples are heap-allocated `{ u64 len, u64[len] elements }` structs created via `snow_gc_alloc_actor`. `flat_map` and `flatten` need nested list unwinding. `take` and `drop` are straightforward subsequence operations.

2. **Map conversions (COLL-11):** `merge`, `to_list`, and `from_list`. `Map.merge(a, b)` combines two maps (b overwrites a). `Map.to_list` returns a `List<(K, V)>` of key-value tuple pairs. `Map.from_list` takes a `List<(K, V)>` and builds a map. These conversion functions bridge Map and List types. The map runtime already has `map_entries`, `map_len`, and `alloc_map` helpers.

3. **Set conversions (COLL-12):** `difference`, `to_list`, and `from_list`. `Set.difference(a, b)` returns elements in `a` not in `b`. `Set.to_list` converts to `List<Int>`. `Set.from_list` takes a `List<Int>` and builds a set. These are straightforward since sets currently store `u64` elements in the same layout as lists.

All 12 functions follow the exact same 4-layer registration pattern (typeck infer.rs + builtins.rs, MIR lower.rs, codegen intrinsics.rs, runtime implementation) that has been proven across 30+ existing stdlib functions.

**Primary recommendation:** Follow the exact 4-layer pattern from Phase 46. The tuple-returning functions (zip, enumerate, Map.to_list) are the only new complexity -- allocate tuples via `snow_gc_alloc_actor` with the `{ u64 len, u64[N] elements }` layout, identical to how `codegen_make_tuple` works.

## Standard Stack

### Core (all changes within existing crates)

| Crate | Path | Purpose | What to Modify |
|-------|------|---------|----------------|
| snow-typeck | `crates/snow-typeck/src/infer.rs` | Type signatures for module-qualified access | Add to List, Map, Set sections in `stdlib_modules()` |
| snow-typeck | `crates/snow-typeck/src/builtins.rs` | Flat-prefixed type signatures | Add `list_zip`, `list_flat_map`, etc. |
| snow-codegen | `crates/snow-codegen/src/mir/lower.rs` | MIR lowering, name mapping, known_functions | Add to `map_builtin_name`, `known_functions` |
| snow-codegen | `crates/snow-codegen/src/codegen/intrinsics.rs` | LLVM external function declarations | Add all 12 new functions |
| snow-rt | `crates/snow-rt/src/collections/list.rs` | List runtime functions | Implement zip, flat_map, flatten, enumerate, take, drop |
| snow-rt | `crates/snow-rt/src/collections/map.rs` | Map runtime functions | Implement merge, to_list, from_list |
| snow-rt | `crates/snow-rt/src/collections/set.rs` | Set runtime functions | Implement difference, to_list, from_list |
| snow-rt | `crates/snow-rt/src/lib.rs` | Re-exports | Add new function exports |
| snowc | `crates/snowc/tests/e2e_stdlib.rs` | E2E tests | Add test functions for each requirement |

### No New Dependencies

All implementations use existing Rust stdlib operations and existing Snow runtime primitives (`snow_gc_alloc_actor`, list builder, tuple allocation pattern). Zero new crate dependencies.

## Architecture Patterns

### Pattern 1: The 4-Layer Stdlib Function Registration (SAME AS Phase 46)

Every new function requires synchronized registration across 4 layers. This is identical to Phase 46.

**Layer 1 -- Typeck Module Map** (`infer.rs::stdlib_modules()`):
```rust
// Example: List.zip in the List module section (after line 357):
list_mod.insert("zip".to_string(), Scheme {
    vars: vec![t_var, u_var],
    ty: Ty::fun(vec![list_t.clone(), list_u.clone()], Ty::list(Ty::Tuple(vec![t.clone(), u.clone()]))),
});
```

**Layer 2 -- Typeck Flat Env** (`builtins.rs::register_builtins()`):
```rust
env.insert("list_zip".into(), Scheme {
    vars: vec![t_var, u_var],
    ty: Ty::fun(vec![list_t.clone(), list_u.clone()], Ty::list(Ty::Tuple(vec![t.clone(), u.clone()]))),
});
```

**Layer 3a -- MIR Name Mapping** (`lower.rs::map_builtin_name()`):
```rust
"list_zip" => "snow_list_zip".to_string(),
```

**Layer 3b -- MIR Known Functions** (`lower.rs::new()` known_functions):
```rust
self.known_functions.insert(
    "snow_list_zip".to_string(),
    MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr)),
);
```

**Layer 4a -- LLVM Intrinsic Declaration** (`intrinsics.rs`):
```rust
module.add_function(
    "snow_list_zip",
    ptr_type.fn_type(&[ptr_type.into(), ptr_type.into()], false),
    Some(inkwell::module::Linkage::External),
);
```

**Layer 4b -- Runtime Implementation** (`list.rs`):
```rust
#[no_mangle]
pub extern "C" fn snow_list_zip(a: *mut u8, b: *mut u8) -> *mut u8 { ... }
```

### Pattern 2: Allocating Tuples in Runtime Functions

`zip`, `enumerate`, and `Map.to_list` must create tuple values as list elements. Snow tuples are GC-allocated structs with layout `{ u64 len, u64[len] elements }`.

**Tuple allocation in runtime:**
```rust
/// Allocate a 2-element tuple on the GC heap.
unsafe fn alloc_pair(a: u64, b: u64) -> *mut u8 {
    let total = 8 + 2 * 8; // len (u64) + 2 elements
    let p = snow_gc_alloc_actor(total as u64, 8);
    *(p as *mut u64) = 2; // len = 2
    *((p as *mut u64).add(1)) = a;
    *((p as *mut u64).add(2)) = b;
    p
}
```

The tuple pointer is then stored as a `u64` in the list's data array (cast via `as u64`), because Snow lists store all elements as uniform 8-byte values.

### Pattern 3: List Builder for Efficient Construction

For functions that build a new list element-by-element (flat_map, flatten, from_list), use the list builder pattern:

```rust
let list = snow_list_builder_new(estimated_capacity);
snow_list_builder_push(list, element);
// ... push more elements ...
// list is ready to use (len is maintained by push)
```

This is more efficient than repeated `snow_list_append` calls (which copy on each append).

### Pattern 4: Existing Map/Set Internal Helpers

Both map.rs and set.rs have internal helper functions that are reusable:

**Map helpers (already exist):**
- `map_len(m)` -- read length
- `map_entries(m)` -- get entries pointer (`[u64; 2]` pairs)
- `map_key_type(m)` -- get key type tag
- `alloc_map(cap, key_type)` -- allocate new map
- `find_key(m, key)` -- find key index
- `keys_equal(m, a, b)` -- compare keys with dispatch

**Set helpers (already exist):**
- `set_len(s)` -- read length
- `set_data(s)` -- get data pointer
- `alloc_set(cap)` -- allocate new set
- `contains_elem(s, elem)` -- membership test

### Anti-Patterns to Avoid

- **Forgetting the flat env registration in builtins.rs:** Module map in infer.rs handles `List.zip(...)` syntax, but builtins.rs handles the `list_zip` prefixed name. Both are required.
- **Wrong MIR type for tuple-returning functions:** Functions returning `List<(A, B)>` return `MirType::Ptr` (opaque list pointer). The tuple structure is hidden inside the list elements.
- **Using `list_u` type variable where not needed:** `zip` needs both `t_var` and `u_var` (two different list types). `enumerate`, `flat_map`, `flatten`, `take`, `drop` only need `t_var`. Use the correct variables.
- **Forgetting to handle Map key_type tag:** When building a map via `Map.from_list`, the resulting map must inherit the correct key_type tag. Default to `KEY_TYPE_INT` (0) since the map doesn't know the key type without context. The codegen handles key type tagging at call sites for string-key maps.
- **Set elements are Int-only:** The current Set type uses `Ty::int()` for all elements (monomorphic). `Set.from_list` must take `List<Int>` and `Set.to_list` must return `List<Int>`.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Tuple allocation | Custom struct layout | `snow_gc_alloc_actor` with `{ u64 len, u64[N] elems }` | Must match codegen's `__snow_make_tuple` layout exactly |
| List building in flat_map | Repeated snow_list_append | `snow_list_builder_new` + `snow_list_builder_push` | O(1) amortized push vs O(n) copy per append |
| Map merge key comparison | Custom key equality | Existing `keys_equal()` helper in map.rs | Handles both Int and String key types |
| Set membership check | Manual linear scan | Existing `contains_elem()` helper in set.rs | Already handles the element comparison |

**Key insight:** All the building blocks exist in the runtime. Each new function is a thin composition of existing helpers (list builder, tuple allocation, GC allocation, map/set internal helpers).

## Common Pitfalls

### Pitfall 1: Tuple Layout Must Match Codegen

**What goes wrong:** If the runtime allocates tuples with a different layout than `codegen_make_tuple`, then `Tuple.first`, `Tuple.second`, `Tuple.nth` will read garbage, or `${}` string interpolation on tuple elements will crash.
**Why it happens:** The codegen creates tuples as `{ u64 len, u64[N] elements }` at offset 0=len, offset 8..=N*8=elements. The runtime must match this exact layout.
**How to avoid:** Use the same layout: `snow_gc_alloc_actor(8 + N*8, 8)`, store len as `u64` at offset 0, elements at offsets 8, 16, etc. Verify with `snow_tuple_first` / `snow_tuple_second` in tests.
**Warning signs:** `Tuple.first(pair)` returns garbage or panics with "index out of bounds".

### Pitfall 2: Storing Tuple Pointers in Lists as u64

**What goes wrong:** Lists store elements as `u64`. Tuple pointers are `*mut u8`. When storing a tuple pointer in a list, it must be cast to `u64`. When reading it back, it must be cast back to `*mut u8`.
**Why it happens:** Confusion between pointer and integer representation.
**How to avoid:** In runtime: `snow_list_builder_push(list, tuple_ptr as u64)`. In the type system, the return type is `List<(A, B)>` which at the runtime level is just `List` (opaque Ptr).
**Warning signs:** Segfaults when accessing tuple elements from a zipped list.

### Pitfall 3: flat_map Capacity Estimation

**What goes wrong:** `flat_map(list, fn)` applies `fn` to each element, where `fn` returns a `List<U>`. The result is the concatenation of all returned lists. If the builder is allocated with too-small capacity, it will write past the buffer.
**Why it happens:** Cannot know the total output size in advance.
**How to avoid:** Two approaches: (1) Collect all sub-lists first, sum their lengths, then build result. (2) Use a Vec<u64> to accumulate, then build a list from it. Approach (1) is cleaner. Alternatively, use repeated `snow_list_concat` on an accumulator (simpler but O(n^2)).
**Warning signs:** Buffer overflows, heap corruption, random crashes.

### Pitfall 4: Map.from_list Expects Tuple Elements

**What goes wrong:** `Map.from_list(list)` takes a `List<(K, V)>` where each element is a tuple pointer. The runtime must read each list element as a `*mut u8` (tuple pointer), then read `tuple[0]` as key and `tuple[1]` as value.
**Why it happens:** Forgetting that list elements are u64-encoded pointers to tuples, not raw key-value pairs.
**How to avoid:** For each element: `let tuple_ptr = *src.add(i) as *mut u8; let key = *((tuple_ptr as *mut u64).add(1)); let val = *((tuple_ptr as *mut u64).add(2));` (offset 0 is len, offset 1 is first element, offset 2 is second element).
**Warning signs:** Map contains garbage keys/values after from_list.

### Pitfall 5: Set.from_list Must Deduplicate

**What goes wrong:** `Set.from_list([1, 2, 2, 3])` should produce a set of `{1, 2, 3}` (size 3), not `{1, 2, 2, 3}` (size 4). Sets must maintain uniqueness invariant.
**Why it happens:** Naively copying list elements into a set without checking for duplicates.
**How to avoid:** Use the existing `contains_elem` helper to check before adding each element. Or use `snow_set_add` which already handles deduplication.
**Warning signs:** `Set.size(Set.from_list([1,1,1]))` returns 3 instead of 1.

### Pitfall 6: Map.merge Key Type Preservation

**What goes wrong:** When merging two maps, the result must preserve the key_type tag (int vs string keys). If map `a` has string keys and map `b` has string keys, the result must also have string keys.
**Why it happens:** Forgetting to copy the key_type tag from the source maps.
**How to avoid:** Use `map_key_type(a)` to get the key type from the first map and pass it to `alloc_map`. If maps have different key types, use the first map's key type (caller's responsibility to ensure consistency).
**Warning signs:** After merging two string-key maps, key lookup with a different string pointer fails.

### Pitfall 7: LLVM alloca Naming Collisions in e2e Tests

**What goes wrong:** Reusing the same variable name across multiple `case` arms in Snow test code triggers LLVM verification error: "Instruction does not dominate all uses!"
**Why it happens:** Pre-existing codegen limitation where alloca names collide.
**How to avoid:** Use unique variable names in all `case` arms in test Snow code (`x1`, `x2`, `x3` instead of `x`, `x`, `x`).
**Warning signs:** LLVM verification error during compilation of test fixtures.

## Detailed Function Specifications

### COLL-05: List.zip(a, b)

- **User API:** `List.zip([1, 2, 3], ["a", "b", "c"])`
- **Returns:** `List<(A, B)>` truncated to shorter list length
- **Typeck:** `fn(List<A>, List<B>) -> List<(A, B)>` with `TyVar(91000)` and `TyVar(91001)`
- **Runtime:** `snow_list_zip(a: *mut u8, b: *mut u8) -> *mut u8`
- **MIR:** `FnPtr(vec![Ptr, Ptr], Ptr)` -- no closures, both args are lists
- **LLVM:** `ptr_type.fn_type(&[ptr_type.into(), ptr_type.into()], false)`
- **Implementation:** `min(len_a, len_b)` iterations, each creating a 2-tuple via `alloc_pair`, stored in result list via builder

### COLL-06: List.flat_map(list, fn) and List.flatten(list)

- **User API:** `List.flat_map([1, 2, 3], fn(x) -> [x, x * 2] end)` / `List.flatten([[1, 2], [3, 4]])`
- **Returns:** `List<U>` / `List<T>` (flattened)

**flat_map:**
- **Typeck:** `fn(List<T>, fn(T) -> List<U>) -> List<U>` with `TyVar(91000)` and `TyVar(91001)`
- **Runtime:** `snow_list_flat_map(list: *mut u8, fn_ptr: *mut u8, env_ptr: *mut u8) -> *mut u8`
- **MIR:** `FnPtr(vec![Ptr, Ptr, Ptr], Ptr)` -- closure split into fn_ptr + env_ptr
- **LLVM:** `ptr_type.fn_type(&[ptr_type.into(), ptr_type.into(), ptr_type.into()], false)`
- **Implementation:** For each element, call the closure to get a sub-list, then collect all sub-list elements into a Vec<u64>, then build result list from the Vec.

**flatten:**
- **Typeck:** `fn(List<List<T>>) -> List<T>` with `TyVar(91000)`
- **Runtime:** `snow_list_flatten(list: *mut u8) -> *mut u8`
- **MIR:** `FnPtr(vec![Ptr], Ptr)` -- single list argument
- **LLVM:** `ptr_type.fn_type(&[ptr_type.into()], false)`
- **Implementation:** Iterate outer list, for each element (a list pointer), iterate inner list and collect all elements into result.

### COLL-07: List.enumerate(list)

- **User API:** `List.enumerate(["a", "b", "c"])` -> `[(0, "a"), (1, "b"), (2, "c")]`
- **Returns:** `List<(Int, T)>`
- **Typeck:** `fn(List<T>) -> List<(Int, T)>` with `TyVar(91000)`
- **Runtime:** `snow_list_enumerate(list: *mut u8) -> *mut u8`
- **MIR:** `FnPtr(vec![Ptr], Ptr)`
- **LLVM:** `ptr_type.fn_type(&[ptr_type.into()], false)`
- **Implementation:** Iterate, create `(i, elem)` tuple pairs via `alloc_pair`, store in result list.

**Type signature note:** The return type `List<(Int, T)>` should be expressed as `Ty::list(Ty::Tuple(vec![Ty::int(), t.clone()]))` in both infer.rs and builtins.rs.

### COLL-08: List.take(list, n) and List.drop(list, n)

- **User API:** `List.take([1, 2, 3, 4], 2)` -> `[1, 2]` / `List.drop([1, 2, 3, 4], 2)` -> `[3, 4]`
- **Returns:** `List<T>`
- **Typeck:** `fn(List<T>, Int) -> List<T>` with `TyVar(91000)`
- **Runtime:** `snow_list_take(list: *mut u8, n: i64) -> *mut u8` / `snow_list_drop(list: *mut u8, n: i64) -> *mut u8`
- **MIR:** `FnPtr(vec![Ptr, Int], Ptr)` -- list + integer count
- **LLVM:** `ptr_type.fn_type(&[ptr_type.into(), i64_type.into()], false)`
- **Implementation (take):** `let actual_n = min(n, len); alloc_list_from(list_data(list), actual_n, actual_n)`
- **Implementation (drop):** `let actual_n = min(n, len); alloc_list_from(list_data(list).add(actual_n), len - actual_n, len - actual_n)`

### COLL-11: Map.merge(a, b), Map.to_list(map), Map.from_list(list)

**merge:**
- **User API:** `Map.merge(map_a, map_b)` -- b's entries overwrite a's for duplicate keys
- **Typeck:** `fn(Map<K, V>, Map<K, V>) -> Map<K, V>` with `TyVar(90000)`, `TyVar(90001)`
- **Runtime:** `snow_map_merge(a: *mut u8, b: *mut u8) -> *mut u8`
- **MIR:** `FnPtr(vec![Ptr, Ptr], Ptr)`
- **LLVM:** `ptr_type.fn_type(&[ptr_type.into(), ptr_type.into()], false)`
- **Implementation:** Start with a copy of `a`, then iterate `b`'s entries and `put` each into the copy.

**to_list:**
- **User API:** `Map.to_list(map)` -> list of (key, value) tuples
- **Typeck:** `fn(Map<K, V>) -> List<(K, V)>` with `TyVar(90000)`, `TyVar(90001)`
- **Runtime:** `snow_map_to_list(map: *mut u8) -> *mut u8`
- **MIR:** `FnPtr(vec![Ptr], Ptr)`
- **LLVM:** `ptr_type.fn_type(&[ptr_type.into()], false)`
- **Implementation:** Iterate map entries, create `(key, value)` tuple pairs via `alloc_pair`, store in result list via builder.

**from_list:**
- **User API:** `Map.from_list(list_of_tuples)` -> map
- **Typeck:** `fn(List<(K, V)>) -> Map<K, V>` with `TyVar(90000)`, `TyVar(90001)`
- **Runtime:** `snow_map_from_list(list: *mut u8) -> *mut u8`
- **MIR:** `FnPtr(vec![Ptr], Ptr)`
- **LLVM:** `ptr_type.fn_type(&[ptr_type.into()], false)`
- **Implementation:** Iterate list, read each element as tuple pointer, extract key (offset 1) and value (offset 2), `snow_map_put` each.

### COLL-12: Set.difference(a, b), Set.to_list(set), Set.from_list(list)

**difference:**
- **User API:** `Set.difference(set_a, set_b)` -> elements in a but not in b
- **Typeck:** `fn(Set, Set) -> Set` (monomorphic, Set is currently Int-only)
- **Runtime:** `snow_set_difference(a: *mut u8, b: *mut u8) -> *mut u8`
- **MIR:** `FnPtr(vec![Ptr, Ptr], Ptr)`
- **LLVM:** `ptr_type.fn_type(&[ptr_type.into(), ptr_type.into()], false)`
- **Implementation:** Like `intersection` but inverted: keep elements from `a` that are NOT in `b`.

**to_list:**
- **User API:** `Set.to_list(set)` -> `List<Int>`
- **Typeck:** `fn(Set) -> List<Int>` (monomorphic)
- **Runtime:** `snow_set_to_list(set: *mut u8) -> *mut u8`
- **MIR:** `FnPtr(vec![Ptr], Ptr)`
- **LLVM:** `ptr_type.fn_type(&[ptr_type.into()], false)`
- **Implementation:** Allocate list with set's length, copy elements.

**from_list:**
- **User API:** `Set.from_list(list)` -> Set
- **Typeck:** `fn(List<Int>) -> Set` (monomorphic)
- **Runtime:** `snow_set_from_list(list: *mut u8) -> *mut u8`
- **MIR:** `FnPtr(vec![Ptr], Ptr)`
- **LLVM:** `ptr_type.fn_type(&[ptr_type.into()], false)`
- **Implementation:** Iterate list, `snow_set_add` each element (handles dedup).

## Code Examples

### alloc_pair Helper (for zip, enumerate, Map.to_list)

```rust
// Add to crates/snow-rt/src/collections/list.rs (shared by map.rs via crate::collections::list::alloc_pair)
// Or add as a local helper in each file that needs it.
unsafe fn alloc_pair(a: u64, b: u64) -> *mut u8 {
    let total = 8 + 2 * 8; // u64 len + 2 u64 elements
    let p = snow_gc_alloc_actor(total as u64, 8);
    *(p as *mut u64) = 2;           // len = 2
    *((p as *mut u64).add(1)) = a;  // first element
    *((p as *mut u64).add(2)) = b;  // second element
    p
}
```

### List.zip Runtime Implementation

```rust
#[no_mangle]
pub extern "C" fn snow_list_zip(a: *mut u8, b: *mut u8) -> *mut u8 {
    unsafe {
        let len_a = list_len(a);
        let len_b = list_len(b);
        let len = len_a.min(len_b);

        let result = alloc_list(len);
        *(result as *mut u64) = len;
        let src_a = list_data(a);
        let src_b = list_data(b);
        let dst = list_data_mut(result);

        for i in 0..len as usize {
            let pair = alloc_pair(*src_a.add(i), *src_b.add(i));
            *dst.add(i) = pair as u64;
        }
        result
    }
}
```

### List.flat_map Runtime Implementation

```rust
#[no_mangle]
pub extern "C" fn snow_list_flat_map(
    list: *mut u8,
    fn_ptr: *mut u8,
    env_ptr: *mut u8,
) -> *mut u8 {
    type BareFn = unsafe extern "C" fn(u64) -> *mut u8;
    type ClosureFn = unsafe extern "C" fn(*mut u8, u64) -> *mut u8;

    unsafe {
        let len = list_len(list);
        let src = list_data(list);
        let mut all_elems: Vec<u64> = Vec::new();

        if env_ptr.is_null() {
            let f: BareFn = std::mem::transmute(fn_ptr);
            for i in 0..len as usize {
                let sub_list = f(*src.add(i));
                let sub_len = list_len(sub_list) as usize;
                let sub_data = list_data(sub_list);
                for j in 0..sub_len {
                    all_elems.push(*sub_data.add(j));
                }
            }
        } else {
            let f: ClosureFn = std::mem::transmute(fn_ptr);
            for i in 0..len as usize {
                let sub_list = f(env_ptr, *src.add(i));
                let sub_len = list_len(sub_list) as usize;
                let sub_data = list_data(sub_list);
                for j in 0..sub_len {
                    all_elems.push(*sub_data.add(j));
                }
            }
        }

        let result_len = all_elems.len() as u64;
        let result = alloc_list(result_len);
        *(result as *mut u64) = result_len;
        let dst = list_data_mut(result);
        for (i, elem) in all_elems.iter().enumerate() {
            *dst.add(i) = *elem;
        }
        result
    }
}
```

### List.flatten Runtime Implementation

```rust
#[no_mangle]
pub extern "C" fn snow_list_flatten(list: *mut u8) -> *mut u8 {
    unsafe {
        let outer_len = list_len(list) as usize;
        let outer_data = list_data(list);
        let mut all_elems: Vec<u64> = Vec::new();

        for i in 0..outer_len {
            let sub_list = *outer_data.add(i) as *mut u8;
            let sub_len = list_len(sub_list) as usize;
            let sub_data = list_data(sub_list);
            for j in 0..sub_len {
                all_elems.push(*sub_data.add(j));
            }
        }

        let result_len = all_elems.len() as u64;
        let result = alloc_list(result_len);
        *(result as *mut u64) = result_len;
        let dst = list_data_mut(result);
        for (i, elem) in all_elems.iter().enumerate() {
            *dst.add(i) = *elem;
        }
        result
    }
}
```

### List.enumerate Runtime Implementation

```rust
#[no_mangle]
pub extern "C" fn snow_list_enumerate(list: *mut u8) -> *mut u8 {
    unsafe {
        let len = list_len(list);
        let src = list_data(list);
        let result = alloc_list(len);
        *(result as *mut u64) = len;
        let dst = list_data_mut(result);

        for i in 0..len as usize {
            let pair = alloc_pair(i as u64, *src.add(i));
            *dst.add(i) = pair as u64;
        }
        result
    }
}
```

### List.take / List.drop Runtime Implementation

```rust
#[no_mangle]
pub extern "C" fn snow_list_take(list: *mut u8, n: i64) -> *mut u8 {
    unsafe {
        let len = list_len(list);
        let actual_n = (n.max(0) as u64).min(len);
        alloc_list_from(list_data(list), actual_n, actual_n)
    }
}

#[no_mangle]
pub extern "C" fn snow_list_drop(list: *mut u8, n: i64) -> *mut u8 {
    unsafe {
        let len = list_len(list);
        let actual_n = (n.max(0) as u64).min(len);
        let remaining = len - actual_n;
        alloc_list_from(list_data(list).add(actual_n as usize), remaining, remaining)
    }
}
```

### Map.merge Runtime Implementation

```rust
#[no_mangle]
pub extern "C" fn snow_map_merge(a: *mut u8, b: *mut u8) -> *mut u8 {
    unsafe {
        let a_len = map_len(a) as usize;
        let b_len = map_len(b) as usize;
        let kt = map_key_type(a);
        // Start with a copy of a
        let mut result = alloc_map(a_len as u64 + b_len as u64, kt);
        *(result as *mut u64) = a_len as u64;
        if a_len > 0 {
            ptr::copy_nonoverlapping(
                map_entries(a) as *const u8,
                map_entries_mut(result) as *mut u8,
                a_len * ENTRY_SIZE,
            );
        }
        // Put each entry from b (overwrites duplicates)
        let b_entries = map_entries(b);
        for i in 0..b_len {
            let key = (*b_entries.add(i))[0];
            let val = (*b_entries.add(i))[1];
            result = snow_map_put(result, key, val);
        }
        result
    }
}
```

### Map.to_list / Map.from_list Runtime Implementation

```rust
#[no_mangle]
pub extern "C" fn snow_map_to_list(map: *mut u8) -> *mut u8 {
    unsafe {
        let len = map_len(map) as usize;
        let entries = map_entries(map);
        let list = super::list::snow_list_builder_new(len as i64);
        for i in 0..len {
            let key = (*entries.add(i))[0];
            let val = (*entries.add(i))[1];
            let pair = alloc_pair(key, val);
            super::list::snow_list_builder_push(list, pair as u64);
        }
        list
    }
}

#[no_mangle]
pub extern "C" fn snow_map_from_list(list: *mut u8) -> *mut u8 {
    unsafe {
        let len = super::list::snow_list_length(list as *mut u8);
        let mut map = snow_map_new();
        let data = (list as *const u64).add(2); // list data pointer
        for i in 0..len as usize {
            let tuple_ptr = *data.add(i) as *mut u8;
            let key = *((tuple_ptr as *const u64).add(1));   // tuple[0]
            let val = *((tuple_ptr as *const u64).add(2));   // tuple[1]
            map = snow_map_put(map, key, val);
        }
        map
    }
}
```

### Set.difference / Set.to_list / Set.from_list Runtime Implementation

```rust
#[no_mangle]
pub extern "C" fn snow_set_difference(a: *mut u8, b: *mut u8) -> *mut u8 {
    unsafe {
        let a_len = set_len(a) as usize;
        let result = alloc_set(a_len as u64);
        let a_data = set_data(a);
        let dst = set_data_mut(result);
        let mut count = 0;

        for i in 0..a_len {
            let elem = *a_data.add(i);
            if !contains_elem(b, elem) {
                *dst.add(count) = elem;
                count += 1;
            }
        }

        *(result as *mut u64) = count as u64;
        result
    }
}

#[no_mangle]
pub extern "C" fn snow_set_to_list(set: *mut u8) -> *mut u8 {
    unsafe {
        let len = set_len(set);
        let src = set_data(set);
        let list = super::list::snow_list_builder_new(len as i64);
        for i in 0..len as usize {
            super::list::snow_list_builder_push(list, *src.add(i));
        }
        list
    }
}

#[no_mangle]
pub extern "C" fn snow_set_from_list(list: *mut u8) -> *mut u8 {
    unsafe {
        let len = super::list::snow_list_length(list as *mut u8);
        let data = (list as *const u64).add(2); // list data pointer
        let mut set = snow_set_new();
        for i in 0..len as usize {
            set = snow_set_add(set, *data.add(i));
        }
        set
    }
}
```

### Typeck Registration: Full List Module Additions (infer.rs)

```rust
// After existing Phase 46 entries (line ~357), add Phase 47:
// Type variables already defined above: t_var=TyVar(91000), u_var=TyVar(91001), t, u, list_t, list_u

// COLL-05: zip
list_mod.insert("zip".to_string(), Scheme {
    vars: vec![t_var, u_var],
    ty: Ty::fun(vec![list_t.clone(), list_u.clone()], Ty::list(Ty::Tuple(vec![t.clone(), u.clone()]))),
});

// COLL-06: flat_map and flatten
let t_to_list_u = Ty::fun(vec![t.clone()], list_u.clone());
list_mod.insert("flat_map".to_string(), Scheme {
    vars: vec![t_var, u_var],
    ty: Ty::fun(vec![list_t.clone(), t_to_list_u], list_u.clone()),
});
list_mod.insert("flatten".to_string(), Scheme {
    vars: vec![t_var],
    ty: Ty::fun(vec![Ty::list(list_t.clone())], list_t.clone()),
});

// COLL-07: enumerate
list_mod.insert("enumerate".to_string(), Scheme {
    vars: vec![t_var],
    ty: Ty::fun(vec![list_t.clone()], Ty::list(Ty::Tuple(vec![Ty::int(), t.clone()]))),
});

// COLL-08: take and drop
list_mod.insert("take".to_string(), Scheme {
    vars: vec![t_var],
    ty: Ty::fun(vec![list_t.clone(), Ty::int()], list_t.clone()),
});
list_mod.insert("drop".to_string(), Scheme {
    vars: vec![t_var],
    ty: Ty::fun(vec![list_t.clone(), Ty::int()], list_t.clone()),
});
```

### Typeck Registration: Map Module Additions (infer.rs)

```rust
// After existing Map module entries (line ~375):
// Type variables already defined: k_var=TyVar(90000), v_var=TyVar(90001), k, v, map_kv

// COLL-11: merge, to_list, from_list
map_mod.insert("merge".to_string(), Scheme {
    vars: vec![k_var, v_var],
    ty: Ty::fun(vec![map_kv.clone(), map_kv.clone()], map_kv.clone()),
});
map_mod.insert("to_list".to_string(), Scheme {
    vars: vec![k_var, v_var],
    ty: Ty::fun(vec![map_kv.clone()], Ty::list(Ty::Tuple(vec![k.clone(), v.clone()]))),
});
map_mod.insert("from_list".to_string(), Scheme {
    vars: vec![k_var, v_var],
    ty: Ty::fun(vec![Ty::list(Ty::Tuple(vec![k.clone(), v.clone()]))], map_kv.clone()),
});
```

### Typeck Registration: Set Module Additions (infer.rs)

```rust
// After existing Set module entries (line ~387):
// set_t = Ty::set_untyped() (monomorphic, Int-only)

// COLL-12: difference, to_list, from_list
set_mod.insert("difference".to_string(), Scheme::mono(Ty::fun(vec![set_t.clone(), set_t.clone()], set_t.clone())));
set_mod.insert("to_list".to_string(), Scheme::mono(Ty::fun(vec![set_t.clone()], Ty::list(Ty::int()))));
set_mod.insert("from_list".to_string(), Scheme::mono(Ty::fun(vec![Ty::list(Ty::int())], set_t.clone())));
```

## Tuple-Returning Function Type System Considerations

The return type `List<(A, B)>` requires `Ty::list(Ty::Tuple(vec![...]))`. This is a legitimate Snow type -- `Ty::Tuple` is a first-class member of the `Ty` enum. However, there are considerations:

1. **Type erasure at MIR level:** At the MIR/codegen level, `List<(A, B)>` is just `Ptr`. The tuple structure is invisible -- elements are u64 slots containing tuple pointers.

2. **String interpolation:** `println("${pair}")` where `pair` is a `(Int, String)` tuple currently goes through `to_string` dispatch. If to_string for tuples is not wired, the user will get a type error or a raw pointer printed. This is NOT a Phase 47 concern -- the user can still access tuple elements via `Tuple.first(pair)`, `Tuple.second(pair)`.

3. **No Tuple e2e tests exist:** The codebase has no e2e tests for tuple creation or access. Phase 47 tests will be the first to exercise tuple-in-list patterns. The `__snow_make_tuple` codegen path and `snow_tuple_nth` runtime functions are already implemented, but we should verify they work with list storage in our tests.

## Open Questions

1. **flat_map Return Type: u64 vs Ptr**
   - What we know: `flat_map(list, fn)` calls `fn` which returns a list pointer. The closure's return type in the type system is `fn(T) -> List<U>`. At runtime, the closure returns `*mut u8` (a list pointer). The BareFn type should be `fn(u64) -> *mut u8` (element as u64, returns list pointer).
   - What's unclear: Whether the closure return value is consistently a pointer (`*mut u8`) or could be `u64`. Based on how `map` works (returns `u64`), flat_map's closure must be typed to return a `*mut u8` which is also a valid u64 on 64-bit.
   - Recommendation: Use `u64` as the return type in the BareFn/ClosureFn type aliases (matching map's pattern), then cast the u64 to `*mut u8` to read the sub-list. This is safe because pointers fit in u64 on 64-bit platforms, and this is how all other list operations handle opaque pointers.

2. **Map.from_list Key Type Detection**
   - What we know: `Map.from_list` takes a list of tuples and builds a map. The map needs a key_type tag (0=Int, 1=String) for correct key comparison.
   - What's unclear: How to determine the key type from the input list. The runtime cannot inspect the Snow type system.
   - Recommendation: Default to `KEY_TYPE_INT` (0) since the runtime cannot determine key type. For string-key maps, the codegen already handles key type tagging at `snow_map_tag_string` call sites for string-key map literals. The user would need to use `Map.put` on an existing string-key map for string keys. Alternatively, accept this limitation: `Map.from_list` always creates integer-key maps. If string support is needed, add an overload later.

3. **alloc_pair Location**
   - What we know: `alloc_pair` is needed by list.rs (zip, enumerate), map.rs (to_list), and potentially set.rs.
   - What's unclear: Best location for the shared helper.
   - Recommendation: Define `alloc_pair` as `pub(crate)` in `list.rs` (where it's most used) and import it from `map.rs` via `super::list::alloc_pair`. Alternatively, put it in a shared `collections/mod.rs` or `tuple.rs`. The simplest approach is to define it in `list.rs` and reference from other modules.

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Phase 8: Basic collection ops only | Phase 46: Added sort, find, any, all, contains | 2026-02-10 | Collection ops are well-established |
| SnowOption in env.rs only | Phase 46: Extracted to shared option.rs | 2026-02-10 | Sharing pattern proven |
| Set Int-only (monomorphic) | Still Int-only in current codebase | Phase 8 | Set.to_list/from_list use Int |

## Sources

### Primary (HIGH confidence)

All findings are based on direct code inspection of the current Snow codebase:

- `crates/snow-rt/src/collections/list.rs` -- Full list runtime with all existing functions including Phase 46 additions (zip, flat_map etc. do NOT exist yet)
- `crates/snow-rt/src/collections/map.rs` -- Map runtime with internal helpers (alloc_map, map_entries, keys_equal)
- `crates/snow-rt/src/collections/set.rs` -- Set runtime with internal helpers (alloc_set, contains_elem)
- `crates/snow-rt/src/collections/tuple.rs` -- Tuple runtime (snow_tuple_nth, etc.) and tuple layout documentation
- `crates/snow-rt/src/option.rs` -- SnowOption shared module (Phase 46 extraction)
- `crates/snow-typeck/src/infer.rs` -- Module map registration with type variables (TyVar 91000/91001 for List, 90000/90001 for Map)
- `crates/snow-typeck/src/builtins.rs` -- Flat env registration (monomorphic Set, polymorphic List/Map)
- `crates/snow-typeck/src/ty.rs` -- Type constructors (Ty::list, Ty::map, Ty::set, Ty::Tuple, Ty::set_untyped)
- `crates/snow-codegen/src/mir/lower.rs` -- map_builtin_name (lines 7498-7600), known_functions (lines 540-590), STDLIB_MODULES, lower_tuple_expr (line 6224)
- `crates/snow-codegen/src/codegen/intrinsics.rs` -- LLVM external declarations for all map/set/list functions
- `crates/snow-codegen/src/codegen/expr.rs` -- codegen_make_tuple (line 2789), __snow_make_tuple handler (line 627)
- `.planning/phases/46-core-collection-operations/` -- Phase 46 research, plans, summaries, and verification (patterns verified working)

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- all modifications are in well-understood, existing files with clear patterns established in Phase 46
- Architecture: HIGH -- 4-layer registration pattern used 30+ times; tuple allocation pattern verified in codegen_make_tuple and tuple.rs
- Pitfalls: HIGH -- identified through direct code reading of existing patterns, runtime layouts, and type system
- Runtime implementation: HIGH -- all helpers exist, new functions are thin compositions of existing primitives
- Type system (tuple returns): MEDIUM -- `Ty::list(Ty::Tuple(...))` is valid Ty construction, but no e2e tests exercise this pattern yet. The type checker handles `Ty::Tuple` as a first-class type, so this should work, but tuple-in-list is untested territory.

**Research date:** 2026-02-10
**Valid until:** 2026-03-10 (stable internal codebase, unlikely to change)
