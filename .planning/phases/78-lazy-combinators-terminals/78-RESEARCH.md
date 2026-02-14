# Phase 78: Lazy Combinators & Terminals - Research

**Researched:** 2026-02-13
**Domain:** Runtime iterator adapter handles, lazy pipeline composition via Iter module functions, terminal operations consuming iterators
**Confidence:** HIGH

## Summary

Phase 78 adds lazy iterator combinators (map, filter, take, skip, enumerate, zip) and terminal operations (count, sum, any, all, find, reduce) to the Mesh `Iter` module. The combinators return new iterator handles that evaluate lazily -- no intermediate collections are allocated. Terminal operations consume an iterator by calling `next()` in a loop and producing a scalar result.

The critical architectural constraint is that Mesh structs are LLVM value types (not GC-allocated pointers), and `self` in impl methods is passed by value. This means user-defined Iterator `next(self)` cannot mutate struct fields to advance state. The STACK.md research recommended struct-based state machines (like Rust), but this requires mutable self which Mesh does not support. The pragmatic solution -- already proven by Phase 76 -- is to implement combinators as **runtime C functions** that create GC-allocated adapter iterator handles. Each adapter handle stores a reference to the source iterator plus any additional state (closure for map/filter, count for take/skip, index for enumerate, second iterator for zip). The adapter's `next()` C function delegates to the source iterator's `next()`, applies the transformation, and returns the result. This is the same opaque-Ptr-handle pattern used by ListIterator, MapIterator, SetIterator, and RangeIterator in Phase 76.

The implementation approach for each combinator: (1) add a runtime adapter struct + `_new`/`_next` C functions in `mesh-rt`, (2) register the adapter type name in the type system (MirType::Ptr resolution), (3) register Iterator impl for the adapter in builtins.rs, (4) add the `Iter.method` mapping in the lowerer and type checker, (5) declare intrinsics in codegen. Terminal operations are simpler: pure C functions that take an iterator handle (and optionally a closure) and loop internally, calling the iterator's `next()` until None.

**Primary recommendation:** Implement all combinators and terminals as runtime C functions in mesh-rt, following the existing ListIterator/mesh_list_iter_new/mesh_list_iter_next pattern. Each combinator creates a new adapter handle type. The `Iter` module is already wired as a stdlib module (Phase 76). Add `Iter.map`, `Iter.filter`, `Iter.take`, `Iter.skip`, `Iter.enumerate`, `Iter.zip` as combinator entries and `Iter.count`, `Iter.sum`, `Iter.any`, `Iter.all`, `Iter.find`, `Iter.reduce` as terminal entries.

## Standard Stack

### Core
| Component | Location | Purpose | Why Standard |
|-----------|----------|---------|--------------|
| mesh-rt/src/collections/list.rs (or new iter.rs) | Runtime adapter structs + C functions | Implement lazy adapter handles for map/filter/take/skip/enumerate/zip + terminals | Same pattern as ListIterator/mesh_list_iter_new/next; proven in Phase 76 |
| mesh-typeck/src/builtins.rs | Register Iterator impls for adapter types | MapAdapterIterator, FilterAdapterIterator, etc. need Iterator trait impls | Same pattern as ListIterator Iterator impl registration |
| mesh-typeck/src/infer.rs | Add Iter.map/filter/etc. type signatures to stdlib_modules() | Type checker needs signatures for pipe-chain type inference | Same pattern as Iter.from() (line 832-842) |
| mesh-codegen/src/mir/lower.rs | Add iter_map, iter_filter, etc. to map_builtin_name | Maps Iter.map -> mesh_iter_map for stdlib module resolution | Same pattern as iter_from -> mesh_iter_from (line 9862) |
| mesh-codegen/src/codegen/intrinsics.rs | Declare all new runtime functions | LLVM needs extern declarations for all C functions | Same pattern as mesh_list_iter_new/next declarations |
| mesh-codegen/src/mir/types.rs | Register adapter type names as MirType::Ptr | Adapter handle types resolve to opaque pointers at MIR level | Same pattern as ListIterator -> MirType::Ptr (Phase 76-02) |
| mesh-codegen/src/codegen/expr.rs | Add adapter type mappings in resolve_iterator_fn | Maps Iterator__next__MapAdapter -> mesh_iter_map_next etc. | Same pattern as Iterator__next__ListIterator -> mesh_list_iter_next |

### Supporting
No new external dependencies. All changes are internal to existing crates.

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| Runtime C adapter handles | User-level structs implementing Iterator | Mesh structs are LLVM value types, `self` passed by value -- cannot mutate to advance state. Would require mutable references (not yet in Mesh). Runtime handles work today. |
| Per-adapter-type Iterator impls in builtins.rs | Generic Iterator adapter that dispatches by type tag | Type tag dispatch would require runtime type introspection; per-type impls are simpler and consistent with Phase 76 |
| Separate adapter type per combinator (MapAdapterIterator, FilterAdapterIterator, ...) | Single generic "AdapterIterator" with a type tag | Generic adapter loses type information; per-adapter types allow the trait registry to resolve Iterator impls cleanly |
| Runtime `next()` functions that call closures | Compiler-generated specialized next() per closure | Would require true monomorphization (mono.rs currently only does reachability). Runtime closure calling is proven and simpler. |

## Architecture Patterns

### Overview: The Lazy Adapter Pipeline

When the user writes:
```mesh
Iter.from(list) |> Iter.map(fn x -> x * 2 end) |> Iter.filter(fn x -> x > 5 end) |> Iter.count()
```

The pipe operator desugars this to nested calls:
```
Iter.count(Iter.filter(Iter.map(Iter.from(list), fn x -> x * 2 end), fn x -> x > 5 end))
```

Which the lowerer resolves through the stdlib module path:
```
mesh_iter_count(mesh_iter_filter(mesh_iter_map(mesh_iter_from(list), fn_ptr, env_ptr), fn_ptr, env_ptr))
```

Each combinator returns a new iterator handle (opaque Ptr). The terminal `count` calls `next()` in a loop until None, counting elements. Laziness is achieved because no combinator materializes intermediate results -- each `next()` call chains through adapter handles, calling the source iterator's `next()` and applying transformations on-the-fly.

### Pattern 1: Combinator Adapter Handle (Runtime)

**What:** Each lazy combinator creates a GC-allocated adapter struct that stores a reference to the source iterator plus combinator-specific state (closure, counter, etc.). The adapter implements a `_next` function that delegates to the source iterator and applies the transformation.

**When to use:** For every lazy combinator (map, filter, take, skip, enumerate, zip).

**Example (MapAdapter):**
```rust
// Source: crates/mesh-rt/src/iter.rs (to be created)

/// Adapter state for Iter.map(iter, fn).
#[repr(C)]
struct MapAdapter {
    source: *mut u8,     // Source iterator handle
    fn_ptr: *mut u8,     // Map function pointer
    env_ptr: *mut u8,    // Closure environment (null for bare functions)
    source_next: *mut u8, // Source iterator's next() function pointer
}

#[no_mangle]
pub extern "C" fn mesh_iter_map(
    source: *mut u8,
    fn_ptr: *mut u8,
    env_ptr: *mut u8,
) -> *mut u8 {
    unsafe {
        let adapter = mesh_gc_alloc_actor(
            std::mem::size_of::<MapAdapter>() as u64,
            std::mem::align_of::<MapAdapter>() as u64,
        ) as *mut MapAdapter;
        (*adapter).source = source;
        (*adapter).fn_ptr = fn_ptr;
        (*adapter).env_ptr = env_ptr;
        (*adapter).source_next = std::ptr::null_mut(); // Resolved via type dispatch
        adapter as *mut u8
    }
}

#[no_mangle]
pub extern "C" fn mesh_iter_map_next(adapter_ptr: *mut u8) -> *mut u8 {
    type BareFn = unsafe extern "C" fn(u64) -> u64;
    type ClosureFn = unsafe extern "C" fn(*mut u8, u64) -> u64;

    unsafe {
        let adapter = adapter_ptr as *mut MapAdapter;
        // Call source iterator's next()
        let option = mesh_iter_generic_next((*adapter).source);
        let option_ref = option as *mut MeshOption;
        if (*option_ref).tag == 1 {
            // None -- propagate
            return option;
        }
        // Some -- apply map function
        let elem = (*option_ref).value as u64;
        let mapped = if (*adapter).env_ptr.is_null() {
            let f: BareFn = std::mem::transmute((*adapter).fn_ptr);
            f(elem)
        } else {
            let f: ClosureFn = std::mem::transmute((*adapter).fn_ptr);
            f((*adapter).env_ptr, elem)
        };
        alloc_option(0, mapped as *mut u8) as *mut u8
    }
}
```

### Pattern 2: Type-Tag Iterator Dispatch

**What:** A critical challenge is that when `mesh_iter_map_next` needs to call the source iterator's `next()`, it doesn't know which concrete `_next` function to call (the source could be a ListIterator, another MapAdapter, a FilterAdapter, etc.). This requires a generic dispatch mechanism.

**Solution: Type-tagged iterator handles.** Each iterator handle stores a type tag as its first field. A single `mesh_iter_generic_next(iter)` function dispatches to the correct `_next` function based on the tag.

**Example:**
```rust
/// Type tags for iterator handle dispatch.
const ITER_TAG_LIST: u8 = 0;
const ITER_TAG_MAP_COLLECTION: u8 = 1;
const ITER_TAG_SET: u8 = 2;
const ITER_TAG_RANGE: u8 = 3;
const ITER_TAG_MAP_ADAPTER: u8 = 10;
const ITER_TAG_FILTER_ADAPTER: u8 = 11;
const ITER_TAG_TAKE_ADAPTER: u8 = 12;
const ITER_TAG_SKIP_ADAPTER: u8 = 13;
const ITER_TAG_ENUMERATE_ADAPTER: u8 = 14;
const ITER_TAG_ZIP_ADAPTER: u8 = 15;

/// Generic next() dispatch. Calls the correct _next function based on the
/// iterator handle's type tag (first byte of the struct).
#[no_mangle]
pub extern "C" fn mesh_iter_generic_next(iter: *mut u8) -> *mut u8 {
    unsafe {
        let tag = *iter;  // First byte is the type tag
        match tag {
            ITER_TAG_LIST => mesh_list_iter_next(iter),
            ITER_TAG_MAP_COLLECTION => mesh_map_iter_next(iter),
            ITER_TAG_SET => mesh_set_iter_next(iter),
            ITER_TAG_RANGE => mesh_range_iter_next(iter),
            ITER_TAG_MAP_ADAPTER => mesh_iter_map_next(iter),
            ITER_TAG_FILTER_ADAPTER => mesh_iter_filter_next(iter),
            ITER_TAG_TAKE_ADAPTER => mesh_iter_take_next(iter),
            ITER_TAG_SKIP_ADAPTER => mesh_iter_skip_next(iter),
            ITER_TAG_ENUMERATE_ADAPTER => mesh_iter_enumerate_next(iter),
            ITER_TAG_ZIP_ADAPTER => mesh_iter_zip_next(iter),
            _ => alloc_option(1, std::ptr::null_mut()) as *mut u8, // Unknown -> None
        }
    }
}
```

**CRITICAL: Backward compatibility with existing iterator handles.** The existing ListIterator, MapIterator, SetIterator, RangeIterator structs (Phase 76) do NOT have a type tag field. They start with their data fields directly (e.g., ListIterator starts with `list: *mut u8`). Two approaches:

- **Option A: Add type tag as first field to all existing iterator structs.** This modifies the Phase 76 runtime structs. The existing `mesh_list_iter_new` etc. must be updated to write the tag. The existing `mesh_list_iter_next` etc. must skip past the tag byte. This is invasive but clean.

- **Option B: Wrap existing iterators in a tagged wrapper for adapter usage.** `Iter.from()` creates a tagged wrapper around the existing iterator handle. The adapters only chain through tagged wrappers. The existing untagged handles continue to work for direct `for-in` usage (ForInIterator codegen calls `mesh_list_iter_next` directly). This is less invasive but adds a wrapper layer.

- **Option C: Function pointer table in each adapter.** Instead of type tags, store a function pointer to the source's `_next` function directly in each adapter. This avoids modifying existing iterator structs. Each combinator constructor takes the source iterator AND a function pointer to its `next`. `Iter.from()` returns a struct that includes both the handle AND the next function pointer. This is the most flexible approach.

**Recommendation: Option A (type tag as first field).** This is the cleanest approach. All iterator handles get a uniform layout: `{ tag: u8, ...fields }`. The `mesh_iter_generic_next` dispatch function makes adapter chaining simple. The modification to existing Phase 76 handles is straightforward (add one field, adjust offsets). The `for-in` codegen (`codegen_for_in_iterator`) already calls specific `_next` functions by name, so it's unaffected -- it bypasses the generic dispatch.

**HOWEVER: Struct layout concern.** Existing Phase 76 iterator handles use `#[repr(C)]` with specific field layouts. Adding a `tag: u8` first field changes the layout: the `list: *mut u8` field in ListIterator moves from offset 0 to offset 8 (due to alignment). All existing `_next` functions read fields by direct struct cast `(iter as *mut ListIterator)`, so they would automatically pick up the new layout if the struct definition changes. The concern is `for-in` codegen: `codegen_for_in_iterator` passes the iterator handle to `mesh_list_iter_next` etc. directly -- as long as the C function reads the correct struct layout, this works. BUT: `Iter.from()` currently delegates to `mesh_list_iter_new()` which creates a ListIterator without a tag. If we add a tag, `mesh_list_iter_new` must also write it.

**Alternative Recommendation: Option C (function pointer in each adapter).** This avoids modifying existing Phase 76 handles entirely. Each adapter stores a `next_fn: extern "C" fn(*mut u8) -> *mut u8` function pointer. `Iter.from()` returns a "WrappedIterator" that stores both the source handle and its `_next` function pointer. Combinators chain through wrapped iterators. This is zero-invasive to Phase 76.

**Final recommendation: Use Option C for maximum backward compatibility.** `Iter.from()` wraps the source handle with a vtable-style next pointer. All adapters chain through these wrapped handles. Existing `for-in` codegen is completely unaffected.

### Pattern 3: Terminal Operations

**What:** Terminal operations consume an iterator by calling `next()` in a loop until None, accumulating a result.

**Example (count, sum, reduce):**
```rust
/// Iter.count(iter) -- count elements
#[no_mangle]
pub extern "C" fn mesh_iter_count(iter: *mut u8) -> i64 {
    unsafe {
        let mut count: i64 = 0;
        loop {
            let option = mesh_iter_generic_next(iter);
            let opt_ref = option as *mut MeshOption;
            if (*opt_ref).tag == 1 { break; }  // None
            count += 1;
        }
        count
    }
}

/// Iter.sum(iter) -- sum numeric (Int) elements
#[no_mangle]
pub extern "C" fn mesh_iter_sum(iter: *mut u8) -> i64 {
    unsafe {
        let mut sum: i64 = 0;
        loop {
            let option = mesh_iter_generic_next(iter);
            let opt_ref = option as *mut MeshOption;
            if (*opt_ref).tag == 1 { break; }  // None
            sum += (*opt_ref).value as i64;
        }
        sum
    }
}

/// Iter.reduce(iter, init, fn) -- fold with accumulator
#[no_mangle]
pub extern "C" fn mesh_iter_reduce(
    iter: *mut u8,
    init: u64,
    fn_ptr: *mut u8,
    env_ptr: *mut u8,
) -> u64 {
    type BareFn = unsafe extern "C" fn(u64, u64) -> u64;
    type ClosureFn = unsafe extern "C" fn(*mut u8, u64, u64) -> u64;
    unsafe {
        let mut acc = init;
        loop {
            let option = mesh_iter_generic_next(iter);
            let opt_ref = option as *mut MeshOption;
            if (*opt_ref).tag == 1 { break; }
            let elem = (*opt_ref).value as u64;
            acc = if env_ptr.is_null() {
                let f: BareFn = std::mem::transmute(fn_ptr);
                f(acc, elem)
            } else {
                let f: ClosureFn = std::mem::transmute(fn_ptr);
                f(env_ptr, acc, elem)
            };
        }
        acc
    }
}
```

### Pattern 4: Iter.from() Enhanced with Generic Next Dispatch

**What:** `Iter.from()` currently delegates to `mesh_list_iter_new()`. For Phase 78, it must return a "wrapped iterator" that includes a function pointer to the source's `next()` function, enabling generic dispatch in adapter chains.

**Design:** Create a `WrappedIterator` struct:
```rust
#[repr(C)]
struct WrappedIterator {
    source: *mut u8,                               // Source iterator handle
    next_fn: unsafe extern "C" fn(*mut u8) -> *mut u8,  // Source's next function
}
```

`Iter.from(list)` creates a WrappedIterator with `source = mesh_list_iter_new(list)` and `next_fn = mesh_list_iter_next`. All combinators take and return WrappedIterators (or their own adapter structs that also store a source + next_fn). Terminals call `(adapter.next_fn)(adapter.source)` to advance.

**Alternative simpler approach:** Instead of a WrappedIterator abstraction, use type-tag dispatch. `Iter.from()` creates a tagged wrapper: `{ tag: ITER_TAG_LIST, inner: mesh_list_iter_new(list) }`. Then `mesh_iter_generic_next` dispatches by tag. This is simpler but requires modifying `Iter.from()` and adding dispatch to existing handles.

**Simplest viable approach:** Give ALL iterator handles (both Phase 76 collection iterators and Phase 78 adapters) a type tag as the FIRST field. Modify `mesh_list_iter_new`, `mesh_map_iter_new`, `mesh_set_iter_new`, `mesh_range_iter_new` to write a tag. Modify their structs to have `tag: u8` first. Add `mesh_iter_generic_next` that dispatches by tag. This unifies everything cleanly.

### Pattern 5: Stdlib Module Function Wiring

**What:** Each Iter.method() call is wired through the existing stdlib module resolution path in both the type checker and the MIR lowerer.

**Type checker (infer.rs, stdlib_modules()):**
```rust
// Iter module (Phase 78 additions)
// Iter.map: fn(Iterator, fn(T) -> U) -> Iterator
iter_mod.insert("map".to_string(), Scheme { ... });
// Iter.filter: fn(Iterator, fn(T) -> Bool) -> Iterator
iter_mod.insert("filter".to_string(), Scheme { ... });
// Iter.take: fn(Iterator, Int) -> Iterator
iter_mod.insert("take".to_string(), Scheme { ... });
// etc.
```

**MIR lowerer (lower.rs, map_builtin_name()):**
```rust
"iter_map" => "mesh_iter_map".to_string(),
"iter_filter" => "mesh_iter_filter".to_string(),
"iter_take" => "mesh_iter_take".to_string(),
"iter_skip" => "mesh_iter_skip".to_string(),
"iter_enumerate" => "mesh_iter_enumerate".to_string(),
"iter_zip" => "mesh_iter_zip".to_string(),
"iter_count" => "mesh_iter_count".to_string(),
"iter_sum" => "mesh_iter_sum".to_string(),
"iter_any" => "mesh_iter_any".to_string(),
"iter_all" => "mesh_iter_all".to_string(),
"iter_find" => "mesh_iter_find".to_string(),
"iter_reduce" => "mesh_iter_reduce".to_string(),
```

**Intrinsics (intrinsics.rs):**
Each runtime function gets an LLVM extern declaration with the correct signature:
- Combinators with closures: `fn(ptr, ptr, ptr) -> ptr` (iter, fn_ptr, env_ptr)
- Combinators without closures: `fn(ptr, i64) -> ptr` (take/skip: iter, n)
- Enumerate: `fn(ptr) -> ptr` (iter only)
- Zip: `fn(ptr, ptr) -> ptr` (iter1, iter2)
- Count: `fn(ptr) -> i64`
- Sum: `fn(ptr) -> i64`
- Any/All: `fn(ptr, ptr, ptr) -> i8` (iter, fn_ptr, env_ptr)
- Find: `fn(ptr, ptr, ptr) -> ptr` (iter, fn_ptr, env_ptr) returns Option
- Reduce: `fn(ptr, i64, ptr, ptr) -> i64` (iter, init, fn_ptr, env_ptr)

### Anti-Patterns to Avoid

- **Allocating intermediate collections in combinators:** `Iter.map(iter, fn)` must NOT call `next()` in a loop and build a new list. It must return an adapter handle that computes lazily. The whole point is zero intermediate allocations (COMB-06).

- **Monomorphized struct adapters without mutable self:** The STACK.md recommended struct-based state machines, but Mesh structs are LLVM value types. `next(self)` cannot advance state. Do NOT attempt this approach without first adding mutable references to Mesh.

- **Breaking existing for-in ForInIterator codegen:** The Phase 76 `codegen_for_in_iterator` calls specific `_next` functions by mangled name (e.g., `Iterator__next__ListIterator` -> `mesh_list_iter_next`). Phase 78 must not break this path. The generic dispatch (`mesh_iter_generic_next`) is used ONLY by combinator/terminal runtime code, not by for-in codegen.

- **Forgetting closure splitting in intrinsic declarations:** Runtime C functions expect closures as separate `(fn_ptr, env_ptr)` arguments. The codegen automatically splits closure struct args for runtime intrinsics (see `codegen_call`, line 607-641). The intrinsic declarations must match: `fn(ptr, ptr, ptr)` for `(iter, fn_ptr, env_ptr)`.

- **Ignoring short-circuit semantics for filter + take:** `Iter.from(1..1000000) |> Iter.filter(...) |> Iter.take(10)` must stop after finding 10 matches (Success Criterion 4). The TakeAdapter's `_next` function counts yielded elements and returns None after reaching the limit. The FilterAdapter passes through, and TakeAdapter short-circuits. This works naturally with the adapter chain.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Closure calling convention | Custom closure invocation | Existing BareFn/ClosureFn pattern from mesh_list_map/filter/reduce | Proven pattern handles both bare functions and closures with env_ptr; used by all existing list operations |
| Option allocation for iterator results | Custom tagged union | `alloc_option(tag, value)` from option.rs | Already used by all Phase 76 iterator _next functions |
| GC allocation for adapter handles | Custom allocator | `mesh_gc_alloc_actor(size, align)` | Standard pattern for all runtime allocations |
| Pair/tuple allocation | Custom pair struct | `alloc_pair(a, b)` from list.rs | Used by enumerate and zip for (index, elem) / (a, b) tuples |
| Stdlib module resolution | Custom module dispatch | STDLIB_MODULES + map_builtin_name pattern | Phase 76 already wired Iter as stdlib module; just add more entries |
| Closure argument splitting | Custom fn_ptr/env_ptr extraction | codegen_call's automatic closure splitting (line 607-641) | Automatically splits {fn_ptr, env_ptr} struct for runtime intrinsics |

**Key insight:** Phase 78 is primarily a runtime implementation exercise. The compiler infrastructure (Iter stdlib module, pipe operator, closure splitting, Iterator trait, intrinsic declarations) is all in place from Phase 76. The new work is: (1) runtime adapter structs + C functions, (2) a generic next dispatch mechanism, (3) type checker signatures for new Iter methods, (4) map_builtin_name entries, (5) intrinsic declarations.

## Common Pitfalls

### Pitfall 1: Generic Next Dispatch Missing for Adapter Chains
**What goes wrong:** `Iter.from(list) |> Iter.map(fn)` works, but `Iter.from(list) |> Iter.map(fn) |> Iter.filter(fn)` crashes because FilterAdapter calls the source (MapAdapter) `next()` but doesn't know which function to call.
**Why it happens:** Without generic dispatch, each adapter hardcodes calls to specific `_next` functions. When adapters are chained, the inner adapter type is unknown at compile time.
**How to avoid:** Implement a generic dispatch mechanism (type tag or function pointer) so any adapter can call any source iterator's `next()` without knowing its concrete type.
**Warning signs:** Single-combinator pipelines work but two-combinator chains crash or return wrong results.

### Pitfall 2: Iterator Consumed Multiple Times
**What goes wrong:** Using the same iterator variable in two different pipelines yields empty results on the second use because the iterator was already exhausted.
**Why it happens:** Iterators are stateful -- calling `next()` advances the cursor. If a user does `let iter = Iter.from(list)` then uses `iter` twice, the second use sees an exhausted iterator.
**How to avoid:** This is expected behavior (matches Rust, Python, Java). Document that iterators are single-use. Users who need multiple passes should call `Iter.from()` again.
**Warning signs:** Users complaining about empty results from second pipeline.

### Pitfall 3: Closure Lifetime in Adapter Handles
**What goes wrong:** A closure's environment is garbage-collected before the adapter's `next()` function accesses it. The adapter holds a raw `env_ptr` that becomes dangling.
**Why it happens:** The GC does not trace through opaque adapter handles to find closure environment pointers. If the closure's environment is the only reference to some data, the GC may collect it.
**How to avoid:** Ensure adapter handles are GC-allocated via `mesh_gc_alloc_actor`. The GC's mark phase must trace through adapter handles. Currently, the GC uses conservative stack scanning -- any pointer value on the stack is treated as a potential root. As long as the adapter handle chain is reachable from the stack, the closure environments stored within are also reachable (they're pointer fields in GC-allocated memory that the conservative scanner will find). Verify this works with a test that uses closures capturing local variables.
**Warning signs:** Intermittent crashes in multi-combinator pipelines with closures that capture variables.

### Pitfall 4: Modifying Phase 76 Iterator Struct Layout
**What goes wrong:** Adding a type tag field to existing Phase 76 iterator structs (ListIterator, etc.) breaks the `for-in` codegen path because `codegen_for_in_iterator` passes handles directly to `mesh_list_iter_next` etc., which expect the old layout.
**Why it happens:** The structs are `#[repr(C)]` with specific field offsets. Adding a `tag: u8` first field shifts all other fields by 8 bytes (due to alignment).
**How to avoid:** If using type-tag dispatch, update ALL functions that read from iterator handles: `mesh_list_iter_next`, `mesh_map_iter_next`, etc. must account for the tag field. Since they cast `iter_ptr as *mut ListIterator`, and the struct definition includes the tag, this should work automatically -- just add the tag field to the struct and write it in `_new`.
**Warning signs:** For-in over lists/maps/sets produces garbage values or crashes after Phase 78 changes.

### Pitfall 5: Short-Circuit Not Working for Take
**What goes wrong:** `Iter.from(1..1000000) |> Iter.filter(fn x -> x > 500000 end) |> Iter.take(10) |> Iter.count()` processes all 1M elements instead of stopping early.
**Why it happens:** The TakeAdapter's `_next` returns None after `n` yields, but the terminal (count) keeps calling `mesh_iter_generic_next` which then calls TakeAdapter's `_next` which returns None -- this IS correct. The issue would be if the terminal doesn't stop on None, or if TakeAdapter doesn't track its count correctly.
**How to avoid:** TakeAdapter must have a `remaining: i64` counter that decrements on each Some yield and returns None when reaching 0. FilterAdapter must propagate None from its source (when source is exhausted). Verify with the exact test case from Success Criterion 4.
**Warning signs:** Pipeline processes more elements than expected; performance test shows no improvement over eager approach.

### Pitfall 6: Type Checker Return Type Mismatch for Terminals
**What goes wrong:** `Iter.count(iter)` is declared in stdlib_modules() as returning `Int`, but the MIR lowerer resolves it to a function returning `Ptr` (because Iter functions are assumed to return iterator handles). The type mismatch causes codegen to treat the i64 result as a pointer.
**Why it happens:** The Iter module was set up for combinators (which return Ptr/iterator handles). Terminals return scalar types (Int, Bool, Option).
**How to avoid:** Carefully declare each terminal's return type in stdlib_modules(): `count` returns `Int`, `sum` returns `Int`, `any`/`all` return `Bool`, `find` returns `Option<T>`, `reduce` returns the accumulator type. The type checker must propagate these correctly through pipe chains.
**Warning signs:** Terminal operation results printed as garbage pointer values instead of integers.

### Pitfall 7: Value Encoding Mismatch Between Iterator Elements and Closure Arguments
**What goes wrong:** List elements are stored as `u64` values. For Int, the value IS the integer. For String/Ptr types, the value is a pointer cast to u64. The adapter's map closure receives a `u64` but the user expects a typed value. If the closure operates on a String, it receives a pointer as u64 -- but codegen might have emitted the closure expecting a `ptr` type argument.
**Why it happens:** There's a type erasure boundary between the typed MIR world and the untyped runtime world. Closures are compiled with concrete LLVM types, but runtime adapter handles pass raw u64 values.
**How to avoid:** Runtime closures in Mesh are compiled as `fn(u64) -> u64` (or `fn(*mut u8, u64) -> u64` with env). The codegen automatically converts between typed values and u64 at call boundaries. For integers, the u64 IS the value. For pointers (String, List, etc.), the u64 is the pointer value. This works correctly because all Mesh values fit in 64 bits. Verify with tests using both Int and String element types.
**Warning signs:** Map/filter closures produce wrong results when operating on String or Ptr-typed elements.

## Code Examples

### User-Facing Syntax: Complete Lazy Pipeline

```mesh
fn main() do
  let list = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]

  # Lazy pipeline: no intermediate lists allocated
  let result = Iter.from(list)
    |> Iter.map(fn x -> x * 2 end)
    |> Iter.filter(fn x -> x > 10 end)
    |> Iter.take(3)
    |> Iter.count()

  println(result.to_string())  # prints: 3
end
```

### User-Facing Syntax: Terminal Operations

```mesh
fn main() do
  let list = [1, 2, 3, 4, 5]

  # Count
  let c = Iter.from(list) |> Iter.count()
  println(c.to_string())  # 5

  # Sum
  let s = Iter.from(list) |> Iter.sum()
  println(s.to_string())  # 15

  # Any
  let has_even = Iter.from(list) |> Iter.any(fn x -> x % 2 == 0 end)
  println(has_even.to_string())  # true

  # All
  let all_pos = Iter.from(list) |> Iter.all(fn x -> x > 0 end)
  println(all_pos.to_string())  # true

  # Find
  let first_even = Iter.from(list) |> Iter.find(fn x -> x % 2 == 0 end)
  println(first_even.to_string())  # Some(2)

  # Reduce (fold)
  let product = Iter.from(list) |> Iter.reduce(1, fn acc, x -> acc * x end)
  println(product.to_string())  # 120
end
```

### User-Facing Syntax: Enumerate and Zip

```mesh
fn main() do
  let list = ["a", "b", "c"]

  # Enumerate: produces (index, element) tuples
  let indexed = Iter.from(list) |> Iter.enumerate()
  # Each next() yields a tuple: (0, "a"), (1, "b"), (2, "c")

  # Zip: combines two iterators element-wise
  let names = ["alice", "bob"]
  let ages = [30, 25]
  let pairs = Iter.from(names) |> Iter.zip(Iter.from(ages))
  # Each next() yields a tuple: ("alice", 30), ("bob", 25)
end
```

### User-Facing Syntax: Short-Circuit with Take

```mesh
fn main() do
  # Only processes elements until 10 matches are found
  let count = Iter.from(1..1000000)
    |> Iter.filter(fn x -> x % 100 == 0 end)
    |> Iter.take(10)
    |> Iter.count()

  println(count.to_string())  # 10 (stops early!)
end
```

## Requirement Mapping

| Requirement | What It Needs | Implementation Approach |
|-------------|---------------|------------------------|
| COMB-01: Iter.map(iter, fn) | Lazy transform adapter | Runtime MapAdapter struct + mesh_iter_map/mesh_iter_map_next. Stores source iter + closure. next() calls source next, applies fn, returns mapped value. |
| COMB-02: Iter.filter(iter, fn) | Lazy filter adapter | Runtime FilterAdapter struct + mesh_iter_filter/mesh_iter_filter_next. Stores source iter + predicate. next() calls source next in loop until predicate passes or None. |
| COMB-03: Iter.take(iter, n) / Iter.skip(iter, n) | Lazy limit adapters | TakeAdapter: counter counts yields, returns None after n. SkipAdapter: skips first n elements on first calls, then passes through. |
| COMB-04: Iter.enumerate(iter) | Index-tracking adapter | EnumerateAdapter: counter starts at 0. next() calls source next, returns alloc_pair(index, elem). |
| COMB-05: Iter.zip(iter1, iter2) | Two-source adapter | ZipAdapter: stores two source iterators. next() calls both nexts, if either is None returns None, else returns alloc_pair(a, b). |
| COMB-06: No intermediate collections | All combinators lazy | Enforced by design -- no combinator calls next() in a loop or allocates a list. Only the Option allocation for each yielded element. |
| TERM-01: Iter.count(iter) | Terminal counting elements | mesh_iter_count: loop calling next() until None, increment counter. Returns i64. |
| TERM-02: Iter.sum(iter) | Terminal summing elements | mesh_iter_sum: loop calling next() until None, add element as i64. Returns i64. |
| TERM-03: Iter.any(iter, fn) / Iter.all(iter, fn) | Terminal predicate tests | mesh_iter_any: loop, call fn on each elem, return true on first match. mesh_iter_all: return false on first non-match. Short-circuit. |
| TERM-04: Iter.find(iter, fn) | Terminal finding element | mesh_iter_find: loop, call fn on each elem, return Some(elem) on first match, None if exhausted. |
| TERM-05: Iter.reduce(iter, init, fn) | Terminal fold | mesh_iter_reduce: loop with accumulator, call fn(acc, elem) on each. Returns final acc. |

## File Touch Points

### New File
1. **`crates/mesh-rt/src/iter.rs`** -- All iterator adapter structs (MapAdapter, FilterAdapter, TakeAdapter, SkipAdapter, EnumerateAdapter, ZipAdapter), their `_new` and `_next` functions, all terminal operations (count, sum, any, all, find, reduce), generic next dispatch function, and `Iter.from()` enhancement with type-tag dispatch.

### Modified Files
2. **`crates/mesh-rt/src/lib.rs`** -- Add `mod iter;` to expose new module.
3. **`crates/mesh-rt/src/collections/list.rs`** -- Modify ListIterator struct to include type tag (if using type-tag dispatch approach). Update `mesh_list_iter_new` to write tag. Update `mesh_iter_from` to use new wrapper or tagged approach.
4. **`crates/mesh-rt/src/collections/map.rs`** -- Modify MapIterator struct for type tag (if applicable).
5. **`crates/mesh-rt/src/collections/set.rs`** -- Modify SetIterator struct for type tag (if applicable).
6. **`crates/mesh-rt/src/collections/range.rs`** -- Modify RangeIterator struct for type tag (if applicable).
7. **`crates/mesh-typeck/src/infer.rs`** -- Add Iter.map/filter/take/skip/enumerate/zip/count/sum/any/all/find/reduce to `stdlib_modules()` Iter section with correct type signatures.
8. **`crates/mesh-codegen/src/mir/lower.rs`** -- Add iter_map, iter_filter, etc. entries in `map_builtin_name()`.
9. **`crates/mesh-codegen/src/codegen/intrinsics.rs`** -- Declare all new runtime functions as LLVM externs.
10. **`crates/mesh-codegen/src/codegen/expr.rs`** -- Add adapter type mappings in `resolve_iterator_fn` for new adapter types (if they need Iterator__next__AdapterType resolution).
11. **`crates/mesh-codegen/src/mir/types.rs`** -- Register adapter type names (MapAdapterIterator, etc.) as MirType::Ptr (if they appear in trait resolution).

### Test Files
12. **`tests/e2e/iter_map_filter.mpl`** -- E2E: map + filter pipeline
13. **`tests/e2e/iter_take_skip.mpl`** -- E2E: take/skip combinators
14. **`tests/e2e/iter_enumerate_zip.mpl`** -- E2E: enumerate/zip combinators
15. **`tests/e2e/iter_terminals.mpl`** -- E2E: count/sum/any/all/find/reduce
16. **`tests/e2e/iter_chain_short_circuit.mpl`** -- E2E: multi-combinator chain with take proving short-circuit
17. **`crates/meshc/tests/e2e.rs`** -- Test harness entries

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Eager List.map/filter/reduce only | Adding lazy Iter.map/filter/etc. (new paradigm) | Phase 78 (now) | Users get both eager (List.method) and lazy (Iter.method) approaches |
| Iter.from() only entry point | Adding map/filter/take/skip/enumerate/zip combinators | Phase 78 (now) | Enables pipe-style lazy iterator composition |
| No terminal iterator operations | Adding count/sum/any/all/find/reduce | Phase 78 (now) | Completes the iterator API with consuming operations |
| Direct _next() function calls only | Adding generic next dispatch | Phase 78 (now) | Enables chaining arbitrary combinator adapters |

**Existing eager operations remain unchanged:**
- `List.map(list, fn)` -- eagerly produces a new List (unchanged)
- `List.filter(list, fn)` -- eagerly produces a new List (unchanged)
- `List.reduce(list, init, fn)` -- eagerly folds (unchanged)
- All other List/Map/Set operations -- unchanged

## Open Questions

1. **Type-tag vs function-pointer dispatch for adapter chaining**
   - What we know: Adapters need to call their source iterator's `next()` without knowing the concrete type. Type-tag dispatch (match on first byte) and function-pointer dispatch (store next_fn as field) are both viable.
   - What's unclear: Which approach has better runtime performance and simpler implementation. Type-tag requires modifying Phase 76 iterator handles. Function-pointer requires an extra pointer field in each adapter.
   - Recommendation: The planner should choose one. Function-pointer avoids Phase 76 struct changes but adds a pointer per adapter. Type-tag is cleaner long-term but invasive to Phase 76. **Lean toward type-tag** for uniformity, since the Phase 76 struct changes are mechanical.

2. **Where to put the new runtime code: new file or existing files?**
   - What we know: Phase 76 put iterator handles in each collection's file (list.rs, map.rs, set.rs, range.rs). Phase 78 adapters are not specific to any collection type.
   - What's unclear: Whether a new `iter.rs` module is cleaner, or whether adapters should go in `list.rs` alongside the existing iterator code.
   - Recommendation: Create `crates/mesh-rt/src/iter.rs` for all adapter structs, generic dispatch, and terminal operations. This keeps collection-specific code separate from generic iterator infrastructure.

3. **Type checker signatures for polymorphic iterator functions**
   - What we know: `Iter.from` is declared as `fn(List<T>) -> ListIterator`. Phase 78 combinators like `Iter.map` need to accept any iterator type and return a new iterator type. The type checker uses HM inference.
   - What's unclear: How to express "any iterator" in the type signature. Should `Iter.map` be typed as `fn(Ptr, Closure) -> Ptr`? Or should it use a type variable?
   - Recommendation: Use `Ptr` for all iterator handle types in the type checker signatures. Iterators are opaque handles at the type level. The type checker just needs to know that `Iter.map(iter, fn)` returns an iterator (Ptr), and that `Iter.count(iter)` returns Int. Element types are erased at the untyped runtime boundary.

4. **Should Iter.reduce take an initial value or use the first element?**
   - What we know: Existing `mesh_list_reduce` takes `(list, init, fn_ptr, env_ptr)` with an explicit initial value. Rust's `Iterator::reduce` uses the first element as the initial value (no init param), while `fold` takes an explicit init.
   - What's unclear: Which API to expose.
   - Recommendation: Follow the existing convention: `Iter.reduce(iter, init, fn)` with an explicit initial value, matching `List.reduce`. This is simpler and avoids the "what if the iterator is empty" error case.

5. **Iter.from() handling for Map/Set/Range**
   - What we know: Currently `Iter.from()` only handles List (delegates to `mesh_list_iter_new`). Users might want `Iter.from(map)` or `Iter.from(1..10)`.
   - What's unclear: Whether to add type-tag dispatch in `Iter.from()` for non-List collections.
   - Recommendation: Defer multi-type `Iter.from()` dispatch. Phase 78 focuses on List-backed lazy pipelines. Map/Set/Range can be added incrementally. The for-in loop already handles these collections via specialized ForInMap/Set/Range MIR nodes.

## Sources

### Primary (HIGH confidence)
- `crates/mesh-rt/src/collections/list.rs` lines 186-285 -- mesh_list_map/filter/reduce signatures and closure calling convention (verified: BareFn/ClosureFn pattern)
- `crates/mesh-rt/src/collections/list.rs` lines 790-835 -- ListIterator struct, mesh_list_iter_new/next, mesh_iter_from (verified: Phase 76 implementation)
- `crates/mesh-rt/src/collections/list.rs` lines 503-603 -- mesh_list_find/any/all signatures (verified: predicate closure pattern)
- `crates/mesh-rt/src/collections/list.rs` lines 626-765 -- mesh_list_zip/enumerate/take/drop (verified: alloc_pair for tuples, clamped n for take/drop)
- `crates/mesh-rt/src/option.rs` full file -- MeshOption { tag: u8, value: *mut u8 }, alloc_option (verified: tag 0=Some, tag 1=None)
- `crates/mesh-typeck/src/infer.rs` lines 832-842 -- Iter module in stdlib_modules() (verified: Iter.from signature pattern)
- `crates/mesh-codegen/src/mir/lower.rs` lines 9586-9592 -- STDLIB_MODULES includes "Iter" (verified)
- `crates/mesh-codegen/src/mir/lower.rs` lines 9861-9862 -- map_builtin_name "iter_from" mapping (verified)
- `crates/mesh-codegen/src/mir/lower.rs` lines 5842-5885 -- lower_pipe_expr desugaring `x |> f(a)` to `f(x, a)` (verified: pipe chain semantics)
- `crates/mesh-codegen/src/codegen/expr.rs` lines 595-641 -- codegen_call closure splitting: auto-splits {fn_ptr, env_ptr} for runtime intrinsics (verified)
- `crates/mesh-codegen/src/codegen/expr.rs` lines 3774-3791 -- resolve_iterator_fn: maps mangled names to runtime function names (verified: Phase 76 mappings)
- `crates/mesh-codegen/src/codegen/intrinsics.rs` lines 804-821 -- iterator runtime function declarations (verified: Phase 76)
- `crates/mesh-codegen/src/mir/types.rs` -- ListIterator/MapIterator/SetIterator/RangeIterator -> MirType::Ptr (verified: Phase 76-02)
- `.planning/phases/76-iterator-protocol/76-02-SUMMARY.md` -- Phase 76 Plan 02 summary documenting ForInIterator pipeline, resolve_iterator_fn, two-phase function resolution (verified)

### Secondary (MEDIUM confidence)
- `.planning/research/STACK.md` lines 110-127 -- Lazy iterator combinators design: struct-based state machines recommendation (note: constrained by Mesh's value-type structs)
- `.planning/research/FEATURES.md` lines 55-85 -- Iterator combinator feature analysis (lazy vs eager, dual approach recommendation)
- `.planning/research/PITFALLS.md` lines 227-236 -- Pitfall 12: monomorphization explosion for iterator adapters (mitigated by runtime approach)
- `.planning/research/FEATURES.md` lines 183 -- Existing eager List.map/filter/reduce unchanged requirement

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- all changes to existing crates verified against source code with exact line numbers; integration points fully mapped from Phase 76 patterns
- Architecture: HIGH -- runtime adapter handle pattern is a proven extension of Phase 76's ListIterator pattern; generic dispatch is the only new architectural concept; all compiler wiring follows existing Iter.from() pattern
- Pitfalls: HIGH -- 7 pitfalls identified from codebase analysis; closure lifetime, struct layout, and dispatch concerns derived from actual runtime code inspection; short-circuit verification mapped to success criteria
- Code examples: HIGH -- patterns derived from existing runtime conventions (BareFn/ClosureFn, alloc_option, alloc_pair) and Phase 76 iterator handle patterns

**Research date:** 2026-02-13
**Valid until:** 2026-03-13 (stable -- compiler internals don't change externally)
