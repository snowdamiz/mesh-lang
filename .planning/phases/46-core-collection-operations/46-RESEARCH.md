# Phase 46: Core Collection Operations - Research

**Researched:** 2026-02-10
**Domain:** Snow stdlib extension -- List and String module functions with polymorphic typing and runtime implementations
**Confidence:** HIGH

## Summary

Phase 46 adds six groups of operations to the Snow standard library: `List.sort`, `List.find`, `List.any`/`List.all`, `List.contains`, `String.split`/`String.join`, and `String.to_int`/`String.to_float`. These follow the established pattern used in Phases 8, 43, and 44 for adding stdlib functions, which requires coordinated changes across four layers:

1. **Typeck** (`infer.rs` + `builtins.rs`) -- register type signatures in both the module map (for `List.sort(...)` syntax) and the flat env (for `list_sort` prefixed lowering).
2. **MIR lowering** (`lower.rs`) -- add `map_builtin_name` entries, `known_functions` entries, and ensure closure arguments are handled correctly for functions that take callbacks.
3. **Codegen** (`intrinsics.rs`) -- declare LLVM external function signatures for the new runtime functions.
4. **Runtime** (`snow-rt`) -- implement the actual C-ABI functions in Rust (`list.rs` and `string.rs`).

The codebase has a well-established, battle-tested pattern for each of these steps. The key complexity lies in (a) functions that take closure parameters (`sort`, `find`, `any`, `all`) which need the `fn_ptr + env_ptr` splitting pattern, (b) functions that return `Option<T>` (`find`, `to_int`, `to_float`) which need the `SnowOption` tagged struct pattern, and (c) `String.split` which returns `List<String>` requiring cross-type interaction.

**Primary recommendation:** Follow the exact 4-layer pattern from existing stdlib functions. Use `TyVar(91000)`/`TyVar(91001)` (already established for List polymorphism) for new List functions. Use the `SnowOption` tagged struct from `env.rs` for functions returning `Option`. Runtime sort should use a simple in-place merge sort on the copied list data.

## Standard Stack

### Core (all changes are within existing crates)

| Crate | Path | Purpose | What to Modify |
|-------|------|---------|----------------|
| snow-typeck | `crates/snow-typeck/src/infer.rs` | Type signatures for module-qualified access | Add to `stdlib_modules()` List + String maps |
| snow-typeck | `crates/snow-typeck/src/builtins.rs` | Flat-prefixed type signatures | Add `list_sort`, `list_find`, etc. entries |
| snow-codegen | `crates/snow-codegen/src/mir/lower.rs` | MIR lowering, name mapping, known_functions | Add to `map_builtin_name`, `known_functions` |
| snow-codegen | `crates/snow-codegen/src/codegen/intrinsics.rs` | LLVM external function declarations | Add `snow_list_sort`, etc. |
| snow-rt | `crates/snow-rt/src/collections/list.rs` | List runtime functions | Implement sort, find, any, all, contains |
| snow-rt | `crates/snow-rt/src/string.rs` | String runtime functions | Implement split, join, to_int, to_float |

### No New Dependencies

All implementation uses existing Rust stdlib (`str::split`, `str::parse`, `slice::sort_by`) and existing Snow runtime primitives (`snow_gc_alloc_actor`, `snow_string_new`, `SnowString`, `SnowOption`). Zero new crate dependencies.

## Architecture Patterns

### Pattern 1: The 4-Layer Stdlib Function Registration

Every stdlib function in Snow requires synchronized registration across 4 layers. Missing any layer causes compilation failures (typeck errors, linker errors, or runtime panics).

**Layer 1 -- Typeck Module Map** (`infer.rs::stdlib_modules()`):
```rust
// In the List module section (around line 323-335):
list_mod.insert("sort".to_string(), Scheme {
    vars: vec![t_var],
    ty: Ty::fun(vec![list_t.clone(), Ty::fun(vec![t.clone(), t.clone()], Ty::int())], list_t.clone())
});
```

**Layer 2 -- Typeck Flat Env** (`builtins.rs::register_builtins()`):
```rust
// In the List functions section (around line 293-304):
env.insert("list_sort".into(), Scheme {
    vars: vec![t_var],
    ty: Ty::fun(vec![list_t.clone(), Ty::fun(vec![t.clone(), t.clone()], Ty::int())], list_t.clone())
});
```

**Layer 3a -- MIR Name Mapping** (`lower.rs::map_builtin_name()`):
```rust
"list_sort" => "snow_list_sort".to_string(),
```

**Layer 3b -- MIR Known Functions** (`lower.rs::new()` known_functions):
```rust
self.known_functions.insert(
    "snow_list_sort".to_string(),
    MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr, MirType::Ptr], Box::new(MirType::Ptr)),
);
```

**Layer 4a -- LLVM Intrinsic Declaration** (`intrinsics.rs`):
```rust
module.add_function(
    "snow_list_sort",
    ptr_type.fn_type(&[ptr_type.into(), ptr_type.into(), ptr_type.into()], false),
    Some(inkwell::module::Linkage::External),
);
```

**Layer 4b -- Runtime Implementation** (`list.rs`):
```rust
#[no_mangle]
pub extern "C" fn snow_list_sort(
    list: *mut u8,
    fn_ptr: *mut u8,
    env_ptr: *mut u8,
) -> *mut u8 { ... }
```

### Pattern 2: Closure Parameter Handling (fn_ptr + env_ptr)

Snow closures are `{ ptr, ptr }` structs (function pointer + environment pointer). When calling runtime functions that accept closures, the codegen automatically splits the closure struct into two separate pointer arguments. This is already handled by `codegen_call` in `expr.rs` (lines 561-606).

**Critical:** The MIR type for closure parameters must be `MirType::Closure(params, ret)` in the typeck type signature, BUT in the `known_functions` entry (MIR layer) and LLVM declaration, they appear as separate `(ptr, ptr)` pairs.

The typeck type uses `Ty::fun(vec![t, t], Ty::int())` for the comparator, which becomes `MirType::Closure` during lowering. The runtime function receives `(fn_ptr: *mut u8, env_ptr: *mut u8)` as two separate args.

**Existing verified pattern** (from `snow_list_map`):
- Typeck: `Ty::fun(vec![list_t, t_to_u], list_u)` -- 2 params
- known_functions: `MirType::FnPtr(vec![Ptr, Ptr, Ptr], Ptr)` -- 3 params (closure split into 2)
- LLVM: `ptr_type.fn_type(&[ptr, ptr, ptr], false)` -- 3 params
- Runtime: `fn snow_list_map(list: *mut u8, fn_ptr: *mut u8, env_ptr: *mut u8) -> *mut u8`

### Pattern 3: Returning Option<T> from Runtime Functions

Functions that return `Option<T>` use the `SnowOption` struct pattern from `crates/snow-rt/src/env.rs`:

```rust
#[repr(C)]
pub struct SnowOption {
    pub tag: u8,       // 0 = Some, 1 = None
    pub value: *mut u8, // payload pointer (null for None)
}
```

The option is GC-allocated via `snow_gc_alloc_actor`. At the LLVM level, it returns a `ptr` which the codegen treats as an opaque pointer to a sum type struct. The tag+value layout matches Snow's standard sum type codegen layout (tag byte at offset 0, payload at offset 8 due to alignment).

**Existing verified pattern** (from `snow_env_get`):
- Returns `*mut SnowOption` from runtime
- Typeck type: `Ty::option(Ty::string())`
- MIR type: `MirType::Ptr` (opaque)
- LLVM: `ptr_type.fn_type(...)` returning `ptr`

### Pattern 4: List Functions with Non-Uniform Return Types

Some new functions return values of different types than the list element type:
- `List.find(list, pred)` returns `Option<T>` (a tagged pointer, not a raw element)
- `List.any/all(list, pred)` returns `Bool` (i8, not u64)
- `List.contains(list, elem)` returns `Bool` (i8)
- `List.sort(list, cmp)` returns `List<T>` (a new list pointer)

For `find`, the runtime must allocate a `SnowOption` and set the tag/value correctly. For `any`/`all`/`contains`, they return `i8` (0 or 1) like other bool-returning functions.

### Anti-Patterns to Avoid

- **Forgetting the flat env registration:** The module map in `infer.rs` handles `List.sort(...)` syntax, but the flat env in `builtins.rs` handles the `list_sort` prefixed name that MIR lowering produces. Both must be registered. Forgetting builtins.rs causes type errors during compilation.
- **Wrong closure arity in known_functions:** A function like `sort(list, cmp_fn)` has 2 user-visible params, but the MIR known_functions and LLVM signature need 3 params (list + fn_ptr + env_ptr) because closures are split.
- **Returning Option as raw u64:** Functions returning `Option<T>` must return a GC-allocated `SnowOption` pointer, NOT a raw u64 value. The codegen expects pointer-to-sum-type semantics.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Sorting algorithm | Custom sort implementation | `slice::sort_by` on copied data | Stable, O(n log n), handles edge cases |
| String splitting | Custom char-by-char split | `str::split()` from Rust stdlib | Handles Unicode, empty delimiters, etc. |
| String-to-number parsing | Custom parsing logic | `str::parse::<i64>()` / `str::parse::<f64>()` | Handles edge cases, negative numbers, scientific notation |
| GC allocation for results | Manual malloc | `snow_gc_alloc_actor` | Required for GC integration, actor-aware allocation |
| SnowOption construction | New option struct | `alloc_option` helper from `env.rs` | Already handles tag + value + GC allocation. Move to shared location or duplicate. |

**Key insight:** The Snow runtime already has all the building blocks. Every new function is a thin wrapper around Rust stdlib operations + Snow's existing GC allocation + existing data layout patterns.

## Common Pitfalls

### Pitfall 1: SnowOption Alignment / Layout Mismatch
**What goes wrong:** The `SnowOption` struct has `tag: u8` followed by `value: *mut u8`. Due to Rust struct alignment, there is 7 bytes of padding between tag and value. The codegen sum type layout also uses tag at offset 0 and first field at offset 8 (for 8-byte alignment). If these don't match, pattern matching on the returned Option will read garbage.
**Why it happens:** Mismatch between `#[repr(C)]` layout and codegen assumptions.
**How to avoid:** Use `#[repr(C)]` on SnowOption. Verify the codegen sum type layout for Option has tag at byte 0, payload at byte 8 (which it does -- same as all other sum types).
**Warning signs:** Pattern matching on `Some(x)`/`None` from these functions produces wrong values.

### Pitfall 2: List.sort Comparator Contract
**What goes wrong:** The comparator function signature must return `Int` (i64), not `Bool` or `Ordering`. The requirement says "explicit comparator function" which means the user provides `fn(a, b) -> Int` where negative = a < b, 0 = equal, positive = a > b.
**Why it happens:** Confusion between a predicate (returns bool) and a comparator (returns ordering integer).
**How to avoid:** Type the comparator as `fn(T, T) -> Int` in both typeck layers. The runtime uses the comparator's return value directly with `slice::sort_by` semantics.
**Warning signs:** Type error when user passes `fn(a, b) -> a - b` style comparator.

### Pitfall 3: String.split Returning List<String> -- Element Size
**What goes wrong:** Snow lists store elements as uniform `u64` values. Strings are pointers (`*mut SnowString`), which fit in u64 on 64-bit platforms. But the list builder must store the pointer value cast to u64, not the string bytes.
**Why it happens:** Confusion about Snow's uniform representation where all values are 8 bytes.
**How to avoid:** In `snow_string_split`, allocate each substring as a `SnowString` via `snow_string_new`, cast the pointer to `u64`, and store that in the list.
**Warning signs:** Segfaults when iterating over split results.

### Pitfall 4: String.to_int/to_float Returning Option
**What goes wrong:** These must return `Option<Int>` and `Option<Float>` respectively, not panic on invalid input. The SnowOption value field stores the parsed value as a u64 bit pattern (for Int, it's the i64 value reinterpreted; for Float, it's the f64 bits transmuted to u64).
**Why it happens:** Forgetting that Float values must be stored as their bit pattern, not as an integer conversion.
**How to avoid:** For `to_int`: `value = parsed_i64 as u64`. For `to_float`: `value = f64::to_bits(parsed_f64)`.
**Warning signs:** `String.to_float("3.14")` returns a garbage value when unwrapped.

### Pitfall 5: List.contains Needs Equality Comparison
**What goes wrong:** `List.contains(list, elem)` needs to compare elements for equality. Since list elements are uniform u64 values, simple `==` works for Int and Bool, but NOT for String (which are pointers -- two different String pointers with the same content are not equal by pointer comparison).
**Why it happens:** Snow's uniform representation means String equality requires content comparison, not pointer comparison.
**How to avoid:** The requirement says `List.contains(list, elem) returning Bool`. For the MVP, use raw u64 equality (works for Int, Bool, and pointer identity). This is consistent with how the language currently handles equality in other contexts. If String content equality is needed, it can be handled later via the Eq trait infrastructure. Alternatively, the user can use `List.any(list, fn(x) -> x == elem end)` for String content comparison.
**Warning signs:** `List.contains(["hello"], "hello")` returns false when strings are different allocations.

## Code Examples

### List.sort Runtime Implementation
```rust
// In crates/snow-rt/src/collections/list.rs
#[no_mangle]
pub extern "C" fn snow_list_sort(
    list: *mut u8,
    fn_ptr: *mut u8,
    env_ptr: *mut u8,
) -> *mut u8 {
    type BareFn = unsafe extern "C" fn(u64, u64) -> i64;
    type ClosureFn = unsafe extern "C" fn(*mut u8, u64, u64) -> i64;

    unsafe {
        let len = list_len(list);
        if len <= 1 {
            return list; // Already sorted
        }
        // Copy elements into a mutable Vec for sorting
        let src = list_data(list);
        let mut elements: Vec<u64> = Vec::with_capacity(len as usize);
        for i in 0..len as usize {
            elements.push(*src.add(i));
        }
        // Sort using the comparator
        if env_ptr.is_null() {
            let f: BareFn = std::mem::transmute(fn_ptr);
            elements.sort_by(|a, b| {
                let cmp = f(*a, *b);
                if cmp < 0 { std::cmp::Ordering::Less }
                else if cmp > 0 { std::cmp::Ordering::Greater }
                else { std::cmp::Ordering::Equal }
            });
        } else {
            let f: ClosureFn = std::mem::transmute(fn_ptr);
            elements.sort_by(|a, b| {
                let cmp = f(env_ptr, *a, *b);
                if cmp < 0 { std::cmp::Ordering::Less }
                else if cmp > 0 { std::cmp::Ordering::Greater }
                else { std::cmp::Ordering::Equal }
            });
        }
        // Allocate new list with sorted elements
        let new_list = alloc_list(len);
        *(new_list as *mut u64) = len;
        let dst = list_data_mut(new_list);
        for (i, elem) in elements.iter().enumerate() {
            *dst.add(i) = *elem;
        }
        new_list
    }
}
```

### List.find Runtime Implementation (returns SnowOption)
```rust
// In crates/snow-rt/src/collections/list.rs
// Note: SnowOption alloc_option helper must be accessible here
#[no_mangle]
pub extern "C" fn snow_list_find(
    list: *mut u8,
    fn_ptr: *mut u8,
    env_ptr: *mut u8,
) -> *mut u8 {
    type BareFn = unsafe extern "C" fn(u64) -> u64;
    type ClosureFn = unsafe extern "C" fn(*mut u8, u64) -> u64;

    unsafe {
        let len = list_len(list);
        let src = list_data(list);
        if env_ptr.is_null() {
            let f: BareFn = std::mem::transmute(fn_ptr);
            for i in 0..len as usize {
                let elem = *src.add(i);
                if f(elem) != 0 {
                    return alloc_option(0, elem as *mut u8) as *mut u8; // Some(elem)
                }
            }
        } else {
            let f: ClosureFn = std::mem::transmute(fn_ptr);
            for i in 0..len as usize {
                let elem = *src.add(i);
                if f(env_ptr, elem) != 0 {
                    return alloc_option(0, elem as *mut u8) as *mut u8; // Some(elem)
                }
            }
        }
        alloc_option(1, std::ptr::null_mut()) as *mut u8 // None
    }
}
```

### String.split Runtime Implementation
```rust
// In crates/snow-rt/src/string.rs
#[no_mangle]
pub extern "C" fn snow_string_split(
    s: *const SnowString,
    delim: *const SnowString,
) -> *mut u8 {
    unsafe {
        let text = (*s).as_str();
        let delimiter = (*delim).as_str();
        let parts: Vec<&str> = text.split(delimiter).collect();
        let count = parts.len();
        // Use snow_list_builder_new + snow_list_builder_push pattern
        let list = crate::collections::list::snow_list_builder_new(count as i64);
        for part in parts {
            let snow_str = snow_string_new(part.as_ptr(), part.len() as u64);
            crate::collections::list::snow_list_builder_push(list, snow_str as u64);
        }
        list
    }
}
```

### String.to_int Runtime Implementation
```rust
// In crates/snow-rt/src/string.rs
#[no_mangle]
pub extern "C" fn snow_string_to_int(s: *const SnowString) -> *mut u8 {
    unsafe {
        let text = (*s).as_str().trim();
        match text.parse::<i64>() {
            Ok(val) => alloc_option(0, val as u64 as *mut u8) as *mut u8,
            Err(_) => alloc_option(1, std::ptr::null_mut()) as *mut u8,
        }
    }
}
```

### Typeck Registration Example (List module in infer.rs)
```rust
// Add to the List module section around line 334:
let t_t_to_int = Ty::fun(vec![t.clone(), t.clone()], Ty::int());

list_mod.insert("sort".to_string(), Scheme {
    vars: vec![t_var],
    ty: Ty::fun(vec![list_t.clone(), t_t_to_int], list_t.clone()),
});
list_mod.insert("find".to_string(), Scheme {
    vars: vec![t_var],
    ty: Ty::fun(vec![list_t.clone(), t_to_bool.clone()], Ty::option(t.clone())),
});
list_mod.insert("any".to_string(), Scheme {
    vars: vec![t_var],
    ty: Ty::fun(vec![list_t.clone(), t_to_bool.clone()], Ty::bool()),
});
list_mod.insert("all".to_string(), Scheme {
    vars: vec![t_var],
    ty: Ty::fun(vec![list_t.clone(), t_to_bool.clone()], Ty::bool()),
});
list_mod.insert("contains".to_string(), Scheme {
    vars: vec![t_var],
    ty: Ty::fun(vec![list_t.clone(), t.clone()], Ty::bool()),
});
```

### Typeck Registration Example (String module in infer.rs)
```rust
// Add to the String module section (around line 260):
string_mod.insert("split".to_string(), Scheme::mono(
    Ty::fun(vec![Ty::string(), Ty::string()], Ty::list(Ty::string())),
));
string_mod.insert("join".to_string(), Scheme::mono(
    Ty::fun(vec![Ty::list(Ty::string()), Ty::string()], Ty::string()),
));
string_mod.insert("to_int".to_string(), Scheme::mono(
    Ty::fun(vec![Ty::string()], Ty::option(Ty::int())),
));
string_mod.insert("to_float".to_string(), Scheme::mono(
    Ty::fun(vec![Ty::string()], Ty::option(Ty::float())),
));
```

## Detailed Function Specifications

### COLL-01: List.sort(list, cmp_fn)
- **User API:** `List.sort(list, fn(a, b) -> a - b end)`
- **Typeck:** `fn(List<T>, fn(T, T) -> Int) -> List<T>` with TyVar(91000)
- **Runtime:** `snow_list_sort(list: ptr, fn_ptr: ptr, env_ptr: ptr) -> ptr`
- **Implementation:** Copy elements to Vec, `sort_by` with user comparator, allocate new list
- **Returns:** New sorted list (immutable semantics preserved)

### COLL-02: List.find(list, pred)
- **User API:** `List.find(list, fn(x) -> x > 5 end)`
- **Typeck:** `fn(List<T>, fn(T) -> Bool) -> Option<T>` with TyVar(91000)
- **Runtime:** `snow_list_find(list: ptr, fn_ptr: ptr, env_ptr: ptr) -> ptr` (SnowOption)
- **Implementation:** Linear scan, return first match as Some, or None
- **Returns:** GC-allocated SnowOption (tag 0 = Some with element, tag 1 = None)

### COLL-03: List.any(list, pred) / List.all(list, pred)
- **User API:** `List.any(list, fn(x) -> x > 0 end)` / `List.all(list, fn(x) -> x > 0 end)`
- **Typeck:** `fn(List<T>, fn(T) -> Bool) -> Bool` with TyVar(91000)
- **Runtime:** `snow_list_any(list: ptr, fn_ptr: ptr, env_ptr: ptr) -> i8`
- **Runtime:** `snow_list_all(list: ptr, fn_ptr: ptr, env_ptr: ptr) -> i8`
- **Implementation:** Short-circuit scan, return 1/0
- **Returns:** i8 (Bool representation)

### COLL-04: List.contains(list, elem)
- **User API:** `List.contains(list, 42)`
- **Typeck:** `fn(List<T>, T) -> Bool` with TyVar(91000)
- **Runtime:** `snow_list_contains(list: ptr, elem: i64) -> i8`
- **Implementation:** Linear scan with raw u64 equality
- **Note:** NO closure parameter -- this is a simple value comparison. Works correctly for Int, Bool. For String, pointer identity only (consistent with raw u64 semantics).
- **Returns:** i8 (Bool representation)

### COLL-09: String.split(s, delim) / String.join(list, sep)
- **User API:** `String.split("a,b,c", ",")` / `String.join(["a", "b"], ",")`
- **Typeck:** `fn(String, String) -> List<String>` / `fn(List<String>, String) -> String`
- **Runtime:** `snow_string_split(s: ptr, delim: ptr) -> ptr` (List)
- **Runtime:** `snow_string_join(list: ptr, sep: ptr) -> ptr` (String)
- **Implementation:** `str::split` / iterate list building string with separator
- **Note:** `String.join` takes `List<String>` as first arg (not a method on list). This matches the requirement signature.

### COLL-10: String.to_int(s) / String.to_float(s)
- **User API:** `String.to_int("42")` / `String.to_float("3.14")`
- **Typeck:** `fn(String) -> Option<Int>` / `fn(String) -> Option<Float>`
- **Runtime:** `snow_string_to_int(s: ptr) -> ptr` (SnowOption)
- **Runtime:** `snow_string_to_float(s: ptr) -> ptr` (SnowOption)
- **Implementation:** `str::parse`, return Some on success, None on failure
- **Critical:** For `to_float`, the SnowOption value stores `f64::to_bits()` as the u64 payload, because Snow's uniform value representation stores everything as 8 bytes.

## SnowOption Sharing Strategy

The `SnowOption` struct and `alloc_option` helper currently live in `crates/snow-rt/src/env.rs`. They need to be accessible from both `string.rs` and `collections/list.rs`. Options:

1. **Move to a shared location** like `crates/snow-rt/src/lib.rs` or a new `crates/snow-rt/src/option.rs`
2. **Duplicate** the struct+helper in each file that needs it
3. **Re-export** from `env.rs` with `pub use`

**Recommendation:** Move `SnowOption` and `alloc_option` to a new `crates/snow-rt/src/option.rs` module, re-export from `lib.rs`. This keeps the code DRY and makes it clear where Option runtime support lives. The `env.rs` module would import from the new location.

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Bare function names (map, filter) | Module-qualified (List.map) + bare prelude | Phase 8 | Both still work; new functions should follow module-qualified pattern |
| Non-polymorphic collections | Polymorphic List<T> with TyVar(91000) | Phase 8 | New List functions MUST use same TyVars |
| No Option returns from runtime | SnowOption struct in env.rs | Phase 8 | Established pattern for find/to_int/to_float |

## Open Questions

1. **List.contains String Equality**
   - What we know: Raw u64 comparison works for Int and Bool but not String content equality.
   - What's unclear: Should `List.contains` do content equality for strings (using `snow_string_eq`)?
   - Recommendation: Start with raw u64 equality (pointer identity) for simplicity. Document that users should use `List.any(list, fn(x) -> x == elem end)` for String content comparison. This matches how the rest of the collection system works (callback-based operations for complex equality).

2. **String.join First Argument**
   - What we know: The requirement says `String.join(list, sep)` which means the list is the first argument.
   - What's unclear: Should this be in the String module or the List module?
   - Recommendation: Keep it in the String module as specified. The signature `String.join(list_of_strings, separator)` is natural. Users write `String.join(parts, ",")`.

3. **to_float Bit Pattern Storage**
   - What we know: Snow stores all values uniformly as u64. Int is stored directly, but Float must be stored as f64 bit pattern.
   - What's unclear: Does the existing codegen correctly handle extracting a Float from an Option<Float> value?
   - Recommendation: Verify by looking at how the codegen handles Float values in Option. The sum type payload extraction should interpret the 8-byte value based on the declared type. Test thoroughly.

## Sources

### Primary (HIGH confidence)
- `crates/snow-typeck/src/builtins.rs` -- Full builtin registration pattern, TyVar allocations
- `crates/snow-typeck/src/infer.rs` -- Module map registration, stdlib_modules() function
- `crates/snow-codegen/src/mir/lower.rs` -- map_builtin_name, known_functions, STDLIB_MODULES
- `crates/snow-codegen/src/codegen/intrinsics.rs` -- LLVM external function declarations
- `crates/snow-codegen/src/codegen/expr.rs` -- Closure splitting in codegen_call (lines 561-606)
- `crates/snow-rt/src/collections/list.rs` -- Existing list runtime (map, filter, reduce patterns)
- `crates/snow-rt/src/string.rs` -- Existing string runtime functions
- `crates/snow-rt/src/env.rs` -- SnowOption struct and alloc_option helper

### Secondary (MEDIUM confidence)
- `crates/snowc/tests/e2e_stdlib.rs` -- E2E test patterns for stdlib functions
- `tests/e2e/stdlib_list_basic.snow` -- Example Snow code using List module

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- all modifications are in well-understood, existing files with clear patterns
- Architecture: HIGH -- 4-layer registration pattern is repeated 30+ times in existing code; thoroughly verified
- Pitfalls: HIGH -- identified through direct code reading of existing patterns and runtime implementations
- Runtime implementation: HIGH -- existing list functions (map, filter, reduce) demonstrate exact same closure callback pattern
- Option return type: MEDIUM -- SnowOption pattern verified in env.rs, but cross-module sharing is new territory

**Research date:** 2026-02-10
**Valid until:** 2026-03-10 (stable internal codebase, unlikely to change)
