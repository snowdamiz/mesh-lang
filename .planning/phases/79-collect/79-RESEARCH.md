# Phase 79: Collect - Research

**Researched:** 2026-02-13
**Domain:** Runtime iterator materialization into concrete collection types (List, Map, Set, String) via `collect()` module functions
**Confidence:** HIGH

## Summary

Phase 79 adds the ability to materialize iterator pipelines into concrete collection types. This is the "output" end of the lazy iterator API established in Phases 76-78: where `Iter.from()` creates iterators from collections and combinators transform them lazily, `collect()` consumes iterators and builds new collections. The API follows a module-function pattern: `List.collect(iter)`, `Map.collect(iter)`, `Set.collect(iter)`, `String.collect(iter)`, all of which are pipe-friendly (`iter |> List.collect()`).

The implementation is straightforward because all the hard infrastructure is already in place. The type-tag dispatch system (`mesh_iter_generic_next`) allows any collect function to consume any iterator type (base or adapter) uniformly. Each collect function follows the same pattern: call `mesh_iter_generic_next` in a loop until None, accumulate elements into the target collection. `List.collect` appends each element to a list builder. `Map.collect` extracts key-value pairs from tuples and calls `mesh_map_put`. `Set.collect` calls `mesh_set_add` for each element. `String.collect` concatenates single-character strings.

The four new runtime functions (`mesh_list_collect`, `mesh_map_collect`, `mesh_set_collect`, `mesh_string_collect`) are terminal operations -- structurally identical to existing terminals like `mesh_iter_count` and `mesh_iter_sum` -- but instead of accumulating a scalar, they accumulate into a collection. The compiler wiring follows the exact same pattern as all existing stdlib module functions: add type signatures to `stdlib_modules()`, add `map_builtin_name` entries, add intrinsic declarations, write E2E tests.

**Primary recommendation:** Implement four runtime C functions that loop over `mesh_iter_generic_next` and build the target collection. Wire through the existing stdlib module resolution path (List/Map/Set/String modules in type checker, lowerer, and intrinsics). No new architectural concepts needed -- this is pure extension of existing patterns.

## Standard Stack

### Core
| Component | Location | Purpose | Why Standard |
|-----------|----------|---------|--------------|
| `crates/mesh-rt/src/iter.rs` | New runtime collect functions | `mesh_list_collect`, `mesh_map_collect`, `mesh_set_collect`, `mesh_string_collect` | Extends existing terminal operation pattern from same file; uses `mesh_iter_generic_next` for uniform dispatch |
| `crates/mesh-typeck/src/infer.rs` | Type signatures in `stdlib_modules()` | Add `collect` to List, Map, Set, String modules | Same pattern as all existing module function signatures |
| `crates/mesh-codegen/src/mir/lower.rs` | `map_builtin_name` entries | Map `list_collect` -> `mesh_list_collect`, etc. | Same pattern as all existing Iter/List/Map/Set mappings |
| `crates/mesh-codegen/src/codegen/intrinsics.rs` | LLVM extern declarations | Declare `mesh_list_collect`, `mesh_map_collect`, `mesh_set_collect`, `mesh_string_collect` | Same pattern as all existing runtime function declarations |

### Supporting
No new external dependencies. All changes extend existing crates.

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| Module-qualified `List.collect(iter)` | `Iter.collect_list(iter)` on the Iter module | Module-qualified is cleaner (type is in the name) and matches Rust's turbofish pattern spirit; also `Iter.collect_list` clutters the Iter module |
| Separate runtime functions per collection | Single generic `mesh_iter_collect(iter, type_tag)` | Per-collection functions are simpler, avoid needing a collection type tag, and have the correct return types |
| Intermediate list then convert | Direct iterator-to-collection loop | Direct loop is more efficient -- avoids allocating an intermediate List that would be immediately discarded |

## Architecture Patterns

### Pattern 1: Terminal-Style Collect Loop (Runtime)

**What:** Each collect function is a terminal operation that loops calling `mesh_iter_generic_next` until None, accumulating results into the target collection type.

**When to use:** For all four collect functions.

**Example (List.collect):**
```rust
// Source: crates/mesh-rt/src/iter.rs (to be added)

/// List.collect(iter) -- materialize iterator into a List.
/// Loops calling generic_next until None, appending each element
/// to a list builder.
#[no_mangle]
pub extern "C" fn mesh_list_collect(iter: *mut u8) -> *mut u8 {
    unsafe {
        // Start with a small capacity list builder
        let list = crate::collections::list::mesh_list_builder_new(8);
        loop {
            let option = mesh_iter_generic_next(iter);
            let opt_ref = option as *mut MeshOption;
            if (*opt_ref).tag == 1 {
                break; // None -- done
            }
            let elem = (*opt_ref).value as u64;
            crate::collections::list::mesh_list_builder_push(list, elem);
        }
        list
    }
}
```

**Key detail:** `mesh_list_builder_new(capacity)` pre-allocates but starts with `len = 0`. `mesh_list_builder_push` does in-place mutation (valid during construction before the list is shared). The initial capacity of 8 is a reasonable default; the builder will grow as needed if more elements are pushed (note: actually the builder does NOT grow -- it writes past the end of its allocation. This is a CRITICAL concern; see Pitfall 1).

### Pattern 2: Map.collect from Key-Value Tuple Iterator

**What:** `Map.collect(iter)` expects an iterator that yields `{key, value}` tuples (same as `Map.from_list` expects). Each yielded element is a pair pointer with layout `{ len: u64 = 2, key: u64, value: u64 }`.

**Example:**
```rust
// Source: crates/mesh-rt/src/iter.rs (to be added)

/// Map.collect(iter) -- materialize iterator of (key, value) tuples into a Map.
#[no_mangle]
pub extern "C" fn mesh_map_collect(iter: *mut u8) -> *mut u8 {
    unsafe {
        let mut map = crate::collections::map::mesh_map_new();
        loop {
            let option = mesh_iter_generic_next(iter);
            let opt_ref = option as *mut MeshOption;
            if (*opt_ref).tag == 1 {
                break; // None
            }
            let tuple_ptr = (*opt_ref).value as *mut u8;
            // Tuple layout: { u64 len=2, u64 key, u64 value }
            let key = *((tuple_ptr as *const u64).add(1));
            let val = *((tuple_ptr as *const u64).add(2));
            map = crate::collections::map::mesh_map_put(map, key, val);
        }
        map
    }
}
```

**Verified source:** The tuple extraction pattern matches `mesh_map_from_list` in `crates/mesh-rt/src/collections/map.rs` line 396-408. Key at offset 1, value at offset 2.

### Pattern 3: Set.collect from Element Iterator

**What:** `Set.collect(iter)` calls `mesh_set_add` for each element.

**Example:**
```rust
// Source: crates/mesh-rt/src/iter.rs (to be added)

/// Set.collect(iter) -- materialize iterator into a Set.
#[no_mangle]
pub extern "C" fn mesh_set_collect(iter: *mut u8) -> *mut u8 {
    unsafe {
        let mut set = crate::collections::set::mesh_set_new();
        loop {
            let option = mesh_iter_generic_next(iter);
            let opt_ref = option as *mut MeshOption;
            if (*opt_ref).tag == 1 {
                break; // None
            }
            let elem = (*opt_ref).value as u64;
            set = crate::collections::set::mesh_set_add(set, elem);
        }
        set
    }
}
```

**Note:** `mesh_set_add` returns a NEW set (immutable semantics). Each call reallocates. This is O(n^2) for n elements but acceptable for the small sets typical in Mesh (same cost model as existing `mesh_set_from_list`).

### Pattern 4: String.collect from Character/String Iterator

**What:** `String.collect(iter)` concatenates string elements yielded by the iterator. Each element is expected to be a MeshString pointer (cast to u64 in the iterator value).

**Example:**
```rust
// Source: crates/mesh-rt/src/iter.rs (to be added)

/// String.collect(iter) -- materialize iterator of strings into a single String.
/// Concatenates all yielded string elements.
#[no_mangle]
pub extern "C" fn mesh_string_collect(iter: *mut u8) -> *mut u8 {
    unsafe {
        // Start with empty string
        let mut result = crate::string::mesh_string_new(std::ptr::null(), 0) as *mut u8;
        loop {
            let option = mesh_iter_generic_next(iter);
            let opt_ref = option as *mut MeshOption;
            if (*opt_ref).tag == 1 {
                break; // None
            }
            let str_ptr = (*opt_ref).value as *const crate::string::MeshString;
            result = crate::string::mesh_string_concat(
                result as *const crate::string::MeshString,
                str_ptr,
            ) as *mut u8;
        }
        result
    }
}
```

**Performance note:** Repeated `mesh_string_concat` is O(n^2) for n characters since each concat allocates a new string. For large strings this is suboptimal, but matches Mesh's immutable string semantics and is consistent with how string operations work throughout the runtime. A more efficient approach would pre-scan for total length, allocate once, and copy. This optimization can be deferred.

### Pattern 5: Stdlib Module Function Wiring

**What:** Each `collect` function is wired as a module method through the existing stdlib resolution path.

**Type checker (infer.rs, stdlib_modules()):**
```rust
// In the List module section:
list_mod.insert("collect".to_string(), Scheme {
    vars: vec![t_var],
    ty: Ty::fun(vec![Ty::Con(TyCon::new("Ptr"))], Ty::list(t.clone())),
});

// In the Map module section:
map_mod.insert("collect".to_string(), Scheme {
    vars: vec![k_var, v_var],
    ty: Ty::fun(vec![Ty::Con(TyCon::new("Ptr"))], map_kv.clone()),
});

// In the Set module section:
set_mod.insert("collect".to_string(), Scheme::mono(
    Ty::fun(vec![Ty::Con(TyCon::new("Ptr"))], set_t.clone())
));

// In the String module section:
string_mod.insert("collect".to_string(), Scheme::mono(
    Ty::fun(vec![Ty::Con(TyCon::new("Ptr"))], Ty::string())
));
```

**MIR lowerer (lower.rs, map_builtin_name()):**
```rust
"list_collect" => "mesh_list_collect".to_string(),
"map_collect" => "mesh_map_collect".to_string(),
"set_collect" => "mesh_set_collect".to_string(),
"string_collect" => "mesh_string_collect".to_string(),
```

**Intrinsics (intrinsics.rs):**
```rust
// All four collect functions: fn(ptr) -> ptr
module.add_function("mesh_list_collect", ptr_type.fn_type(&[ptr_type.into()], false), Some(Linkage::External));
module.add_function("mesh_map_collect", ptr_type.fn_type(&[ptr_type.into()], false), Some(Linkage::External));
module.add_function("mesh_set_collect", ptr_type.fn_type(&[ptr_type.into()], false), Some(Linkage::External));
module.add_function("mesh_string_collect", ptr_type.fn_type(&[ptr_type.into()], false), Some(Linkage::External));
```

### Anti-Patterns to Avoid

- **Building an intermediate List then converting:** Do NOT implement `Map.collect(iter)` by first doing `mesh_list_collect` then `mesh_map_from_list`. Instead, loop directly from the iterator into the target collection. The intermediate list wastes memory and time.

- **Allocating the list builder with capacity 0:** `mesh_list_builder_new(0)` creates a zero-capacity list. The builder's `push` does NOT grow the buffer -- it writes past the end of the allocation. Always use a reasonable initial capacity or use a two-pass approach (see Pitfall 1).

- **Forgetting that iterators can only be consumed once:** After `List.collect(iter)`, the iterator is exhausted. Calling `Map.collect(iter)` on the same `iter` would produce an empty map. This is expected behavior but should be documented.

- **Treating String.collect element values as characters (integers):** The iterator yields u64 values. For String.collect, these are MeshString pointers (cast to u64), not character codepoints. The collect function must treat them as `*const MeshString`.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| List construction from dynamic elements | Custom memory layout | `mesh_list_builder_new` + `mesh_list_builder_push` | Proven pattern used by all for-in codegen and eager list operations |
| Map construction from key-value pairs | Custom map builder | `mesh_map_new` + `mesh_map_put` loop | Same pattern as `mesh_map_from_list`; handles key deduplication automatically |
| Set construction from elements | Custom set builder | `mesh_set_new` + `mesh_set_add` loop | Same pattern as `mesh_set_from_list`; handles deduplication automatically |
| String construction from parts | Custom byte buffer | `mesh_string_new` + `mesh_string_concat` loop | Consistent with Mesh's immutable string semantics |
| Iterator consumption loop | Custom next-call dispatch | `mesh_iter_generic_next` | Handles all iterator types (collection + adapter) via type-tag dispatch |
| Tuple field extraction | Custom tuple parsing | Offset 1 for first element, offset 2 for second element | Same pattern as `mesh_map_from_list` line 403-404; matches `alloc_pair` layout |
| Stdlib module wiring | Custom dispatch path | STDLIB_MODULES + `map_builtin_name` + intrinsics | Proven for all existing module functions (String, List, Map, Set, Iter, etc.) |

**Key insight:** Phase 79 requires zero new architectural concepts. Every collect function is a minor variation of the terminal operation pattern (loop over `generic_next`, accumulate, return). The compiler wiring is identical to adding any other stdlib module function. The only novel aspect is choosing the correct list builder capacity strategy for `mesh_list_collect`.

## Common Pitfalls

### Pitfall 1: List Builder Capacity Overflow
**What goes wrong:** `mesh_list_builder_new(n)` allocates space for `n` elements. `mesh_list_builder_push` writes elements sequentially with NO bounds checking. If more than `n` elements are pushed, the write goes past the allocated memory, corrupting the GC heap.
**Why it happens:** The list builder pattern was designed for for-in comprehensions where the collection size is known ahead of time. Iterators from adapter chains have an unknown number of elements.
**How to avoid:** Two approaches: (A) Pre-scan by collecting into a temporary Vec<u64> in the runtime function, then build the list from the known count. (B) Use a growable approach: allocate an initial capacity, track usage, and reallocate + copy when full. Approach (A) is simpler and safer. Approach (B) avoids the temporary buffer.
**Recommended approach:** (A) Use a Rust `Vec<u64>` inside `mesh_list_collect` to collect all elements, then call `mesh_list_from_array(vec.as_ptr(), vec.len())` to create the final list in one shot. This avoids any capacity guessing and is safe.
**Warning signs:** Mysterious crashes, corrupted data, or GC heap corruption when collecting large iterators into lists.

### Pitfall 2: Map.collect Tuple Layout Mismatch
**What goes wrong:** `Map.collect(iter)` extracts key/value from tuple pointers at wrong offsets, producing garbage keys or values.
**Why it happens:** The tuple layout from `alloc_pair` is `{ len: u64, first: u64, second: u64 }` (len=2 at offset 0, first at offset 1 in u64 units, second at offset 2). If the developer uses byte offsets or misses the len field, extraction fails.
**How to avoid:** Use the exact same offset pattern as `mesh_map_from_list` (map.rs line 403-404): `key = *((tuple_ptr as *const u64).add(1))` and `val = *((tuple_ptr as *const u64).add(2))`.
**Warning signs:** Map has correct number of entries but keys/values are wrong (shifted by one field).

### Pitfall 3: String.collect Treating Values as Integers Instead of String Pointers
**What goes wrong:** `String.collect(iter)` treats each yielded value as an integer character code instead of as a MeshString pointer. The result is garbage or crashes.
**Why it happens:** Iterator elements are stored as u64 values. For Int elements, the u64 IS the integer. For String elements, the u64 is a pointer to a MeshString. The collect function must interpret the value correctly.
**How to avoid:** `String.collect` must cast `(*opt_ref).value` to `*const MeshString` and pass it to `mesh_string_concat`. Do NOT cast to integer.
**Warning signs:** `String.collect` produces empty strings, crashes, or garbage output.

### Pitfall 4: Missing Module Function in One Layer
**What goes wrong:** Adding `List.collect` to the type checker but forgetting the lowerer `map_builtin_name` entry, or declaring the intrinsic with wrong signature.
**Why it happens:** Module function wiring requires changes in 3 files (infer.rs, lower.rs, intrinsics.rs). Missing any one layer causes compilation or link failure.
**How to avoid:** Use the checklist: for each new module function, add (1) type signature in `stdlib_modules()`, (2) `map_builtin_name` entry, (3) intrinsic declaration, (4) runtime `extern "C"` function. Verify all four are consistent.
**Warning signs:** "Unknown function" from type checker, mangled name not found by lowerer, linker error for undefined symbol.

### Pitfall 5: Set.collect Not Handling Duplicates
**What goes wrong:** Developer assumes `mesh_set_add` always increases set size. If the iterator yields duplicate elements, the set size is less than the number of elements yielded. This is correct behavior but may confuse test expectations.
**Why it happens:** `mesh_set_add` returns a copy with no change when the element already exists.
**How to avoid:** Write test expectations that account for deduplication: `[1, 2, 2, 3, 3] |> Iter.from() |> Set.collect()` should have size 3, not 5.
**Warning signs:** Tests failing because expected set size equals iterator element count.

### Pitfall 6: Map Key Type Tag Not Set for String Keys
**What goes wrong:** `Map.collect(iter)` creates a map with `mesh_map_new()` which defaults to integer key type (key_type=0). If the iterator yields string-keyed tuples, key comparison uses integer equality instead of string content equality, causing lookup failures.
**Why it happens:** `mesh_map_new()` creates a map with KEY_TYPE_INT=0. String key maps require `mesh_map_new_typed(1)`.
**How to avoid:** For the initial implementation, use `mesh_map_new()` (integer keys). String key detection would require runtime type inspection of the first tuple's key, which adds complexity. Document that `Map.collect` produces integer-keyed maps. String-keyed map collection can be added later (e.g., `Map.collect_str_keys(iter)`).
**Warning signs:** String keys in collected maps don't match on lookup because integer comparison is used.

## Code Examples

### User-Facing Syntax: List.collect

```mesh
fn main() do
  let list = [1, 2, 3, 4, 5]

  # Pipeline -> List
  let doubled = Iter.from(list)
    |> Iter.map(fn x -> x * 2 end)
    |> List.collect()
  println(doubled.to_string())  # [2, 4, 6, 8, 10]

  # Filter -> List
  let evens = Iter.from(list)
    |> Iter.filter(fn x -> x % 2 == 0 end)
    |> List.collect()
  println(evens.to_string())  # [2, 4]
end
```

### User-Facing Syntax: Map.collect from Tuples

```mesh
fn main() do
  let list = [1, 2, 3]

  # Enumerate produces (index, value) tuples -> collect into Map
  let index_map = Iter.from(list)
    |> Iter.enumerate()
    |> Map.collect()
  println(index_map.to_string())  # %{0 => 1, 1 => 2, 2 => 3}

  # Zip two iterators -> collect into Map
  let keys = [10, 20, 30]
  let vals = [100, 200, 300]
  let kv_map = Iter.from(keys)
    |> Iter.zip(Iter.from(vals))
    |> Map.collect()
  println(kv_map.to_string())  # %{10 => 100, 20 => 200, 30 => 300}
end
```

### User-Facing Syntax: Set.collect

```mesh
fn main() do
  let list = [1, 2, 2, 3, 3, 3]

  # Collect into Set (deduplicates)
  let unique = Iter.from(list) |> Set.collect()
  println(Set.size(unique).to_string())  # 3
end
```

### User-Facing Syntax: String.collect

```mesh
fn main() do
  let words = ["hello", " ", "world"]

  # Collect string iterator into single String
  let joined = Iter.from(words) |> String.collect()
  println(joined)  # "hello world"
end
```

## Requirement Mapping

| Requirement | What It Needs | Implementation Approach |
|-------------|---------------|------------------------|
| COLL-01: Materialize iterator into List via `List.collect(iter)` | Runtime loop + list builder; type checker + lowerer + intrinsic wiring | `mesh_list_collect`: loop `mesh_iter_generic_next`, collect into `Vec<u64>`, build list with `mesh_list_from_array`. Add `collect` to List module in stdlib_modules, map_builtin_name, intrinsics. |
| COLL-02: Materialize iterator of tuples into Map via `Map.collect(iter)` | Runtime loop + map put; tuple field extraction | `mesh_map_collect`: loop `mesh_iter_generic_next`, extract key/val from tuple (offsets 1,2), `mesh_map_put`. Add `collect` to Map module. |
| COLL-03: Materialize iterator into Set via `Set.collect(iter)` | Runtime loop + set add | `mesh_set_collect`: loop `mesh_iter_generic_next`, `mesh_set_add` each element. Add `collect` to Set module. |
| COLL-04: Materialize string iterator into String via `String.collect(iter)` | Runtime loop + string concat | `mesh_string_collect`: loop `mesh_iter_generic_next`, `mesh_string_concat` each string. Add `collect` to String module. |

## File Touch Points

### Modified Files

1. **`crates/mesh-rt/src/iter.rs`** -- Add 4 new `extern "C"` functions: `mesh_list_collect`, `mesh_map_collect`, `mesh_set_collect`, `mesh_string_collect`. Add necessary imports from collections/list.rs, collections/map.rs, collections/set.rs, string.rs.

2. **`crates/mesh-typeck/src/infer.rs`** -- Add `collect` to 4 module sections in `stdlib_modules()`:
   - List module: `fn(Ptr) -> List<T>`
   - Map module: `fn(Ptr) -> Map<K, V>`
   - Set module: `fn(Ptr) -> Set`
   - String module: `fn(Ptr) -> String`

3. **`crates/mesh-codegen/src/mir/lower.rs`** -- Add 4 entries in `map_builtin_name()`:
   - `"list_collect"` -> `"mesh_list_collect"`
   - `"map_collect"` -> `"mesh_map_collect"`
   - `"set_collect"` -> `"mesh_set_collect"`
   - `"string_collect"` -> `"mesh_string_collect"`

4. **`crates/mesh-codegen/src/codegen/intrinsics.rs`** -- Add 4 LLVM extern declarations:
   - `mesh_list_collect(ptr) -> ptr`
   - `mesh_map_collect(ptr) -> ptr`
   - `mesh_set_collect(ptr) -> ptr`
   - `mesh_string_collect(ptr) -> ptr`

### New Test Files

5. **`tests/e2e/collect_list.mpl`** -- E2E: `List.collect` with map, filter, take pipelines
6. **`tests/e2e/collect_map.mpl`** -- E2E: `Map.collect` from enumerate and zip tuple iterators
7. **`tests/e2e/collect_set_string.mpl`** -- E2E: `Set.collect` with deduplication, `String.collect` for string joining
8. **`crates/meshc/tests/e2e.rs`** -- Test harness entries for all new E2E tests

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Eager `List.map`/`List.filter` produce new Lists directly | Lazy pipeline -> `List.collect()` materializes at end | Phase 79 (now) | Users can choose lazy-then-collect for complex pipelines or eager for simple operations |
| `Map.from_list(list_of_tuples)` only from Lists | `Map.collect(iter)` directly from any iterator of tuples | Phase 79 (now) | No intermediate List needed for iterator-to-Map conversion |
| `Set.from_list(list_of_elems)` only from Lists | `Set.collect(iter)` directly from any iterator | Phase 79 (now) | No intermediate List needed for iterator-to-Set conversion |
| `String.join(list, sep)` requires a pre-built List | `String.collect(iter)` from any string iterator | Phase 79 (now) | Enables lazy string construction from iterator pipelines |

## Open Questions

1. **List.collect capacity strategy**
   - What we know: `mesh_list_builder_push` does NOT bounds-check or grow the buffer. The builder was designed for known-size scenarios (for-in over collections with known length).
   - What's unclear: Whether to use a Rust `Vec<u64>` intermediary (safe, simple) or implement a growable list builder in the runtime (more efficient but more work).
   - Recommendation: Use `Vec<u64>` approach for simplicity and safety. Collect all elements into a Rust Vec, then call `mesh_list_from_array(vec.as_ptr(), vec.len())`. This is O(n) amortized and avoids any risk of buffer overflow. The Vec lives on the Rust stack/heap (not GC heap) during collection, then the final list is GC-allocated.

2. **Map.collect key type (integer vs string keys)**
   - What we know: `mesh_map_new()` defaults to integer key type. `mesh_map_new_typed(1)` creates a string-key map. The key type affects comparison behavior.
   - What's unclear: How to detect at runtime whether the iterator yields integer or string keys without inspecting the first tuple.
   - Recommendation: Default to integer keys via `mesh_map_new()` for the initial implementation. This covers the common case of integer-keyed maps built from `Iter.enumerate()`. String-keyed map collection is a valid future extension. A potential approach: peek at the first element to detect key type, or provide `Map.collect_str(iter)` as a separate function.

3. **String.collect performance for large strings**
   - What we know: Repeated `mesh_string_concat` is O(n^2) for n elements since each concat copies the entire accumulated string.
   - What's unclear: Whether this matters in practice for Mesh use cases.
   - Recommendation: Accept O(n^2) for now (consistent with Mesh's immutable string model). A future optimization could pre-collect into a Vec<u8> and build the string in one shot. This can be added later without API changes.

4. **Should pipe-friendly syntax `iter |> List.collect()` work with zero arguments?**
   - What we know: The pipe operator desugars `x |> f()` to `f(x)`. So `iter |> List.collect()` becomes `List.collect(iter)`, which is `mesh_list_collect(iter)`. This should work with the standard pipe desugaring -- `f()` means `f` takes one argument (the piped value).
   - What's unclear: Whether the lowerer handles the zero-explicit-args case correctly for module functions.
   - Recommendation: This should work automatically since the pipe desugaring in `lower_pipe_expr` (lower.rs line 5842-5885) already handles this pattern. Verify with E2E tests using both `List.collect(iter)` and `iter |> List.collect()` syntax.

## Sources

### Primary (HIGH confidence)
- `crates/mesh-rt/src/iter.rs` -- Full file reviewed: type tag dispatch, generic_next, all adapter structs and terminal operations (512 lines). Terminal operation pattern (count, sum, reduce) is the template for collect functions.
- `crates/mesh-rt/src/collections/list.rs` lines 297-311 -- `mesh_list_builder_new` and `mesh_list_builder_push` verified: push does in-place write with NO bounds check. Capacity must be known or buffer overflow occurs.
- `crates/mesh-rt/src/collections/list.rs` lines 63-70 -- `alloc_pair` verified: tuple layout is `{ len: u64, first: u64, second: u64 }`.
- `crates/mesh-rt/src/collections/list.rs` lines 316-320 -- `mesh_list_from_array(data, count)` verified: allocates list from raw data pointer. Safe alternative to builder for known-size arrays.
- `crates/mesh-rt/src/collections/map.rs` lines 96-98 -- `mesh_map_new()` verified: creates integer-key map (KEY_TYPE_INT=0).
- `crates/mesh-rt/src/collections/map.rs` lines 123-153 -- `mesh_map_put` verified: returns NEW map with immutable semantics.
- `crates/mesh-rt/src/collections/map.rs` lines 396-408 -- `mesh_map_from_list` verified: tuple extraction at offset 1 (key) and offset 2 (value). This is the proven pattern for Map.collect.
- `crates/mesh-rt/src/collections/set.rs` lines 59-82 -- `mesh_set_add` verified: returns NEW set, handles duplicates.
- `crates/mesh-rt/src/collections/set.rs` lines 282-292 -- `mesh_set_from_list` verified: loop over list elements calling `mesh_set_add`. This is the proven pattern for Set.collect.
- `crates/mesh-rt/src/string.rs` lines 82-93 -- `mesh_string_new(data, len)` verified: allocates GC string from raw bytes.
- `crates/mesh-rt/src/string.rs` lines 97-111 -- `mesh_string_concat(a, b)` verified: allocates new string with combined length, copies both.
- `crates/mesh-rt/src/option.rs` -- MeshOption `{ tag: u8, value: *mut u8 }`, tag 0 = Some, tag 1 = None.
- `crates/mesh-typeck/src/infer.rs` lines 212-942 -- `stdlib_modules()` verified: all existing module function patterns (List, Map, Set, String, Iter) with type signature conventions.
- `crates/mesh-codegen/src/mir/lower.rs` lines 9586-9875 -- `STDLIB_MODULES` and `map_builtin_name` verified: pattern for all stdlib function name mappings.
- `crates/mesh-codegen/src/codegen/intrinsics.rs` lines 804-866 -- All existing iterator intrinsic declarations verified: pattern for extern function declarations.
- `crates/mesh-typeck/src/unify.rs` lines 202-217 -- `iterator_ptr_compatible` verified: ListIterator and adapter types unify with Ptr. Collect functions take Ptr (iterator handle) as input.

### Secondary (MEDIUM confidence)
- `.planning/phases/78-lazy-combinators-terminals/78-RESEARCH.md` -- Phase 78 research confirming type-tag dispatch, generic_next pattern, terminal operation structure.
- `.planning/phases/78-lazy-combinators-terminals/78-VERIFICATION.md` -- Phase 78 verification confirming all infrastructure is working and tested.

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- All changes extend existing, verified patterns. No new crates, no new dependencies, no new architectural concepts.
- Architecture: HIGH -- Collect functions are structurally identical to existing terminal operations (count, sum, reduce). Compiler wiring follows the exact same 4-layer path used by all 50+ existing stdlib module functions.
- Pitfalls: HIGH -- 6 pitfalls identified from codebase analysis. The list builder capacity issue (Pitfall 1) is the only non-trivial concern, with a clear mitigation (Vec<u64> intermediate). All other pitfalls are standard "don't forget this layer" integration concerns.

**Research date:** 2026-02-13
**Valid until:** 2026-03-13 (stable -- compiler internals don't change externally)
