# Phase 24: Trait System Generics - Research

**Researched:** 2026-02-08
**Domain:** Compiler internals -- trait system, Display codegen, monomorphization, deriving
**Confidence:** HIGH

## Summary

This phase addresses two related limitations in the Snow compiler's trait system:

1. **TGEN-01 (Nested Collection Display):** The `resolve_to_string_callback` function in MIR lowering has an explicit v1.3 fallback where nested collections (e.g., `List<List<Int>>`) resolve to `snow_int_to_string` instead of recursively producing the correct Display callback. The runtime already supports the callback pattern (`snow_list_to_string` takes an `elem_to_str` function pointer), but the compiler does not generate the correct recursive callback for nested types.

2. **TGEN-02 (Generic Type Deriving):** The type checker currently emits `TypeError::GenericDerive` (E0029) when a generic type has a `deriving(...)` clause, and the trait impl registration is guarded by `if generic_params.is_empty()`. Generic structs/sum types need monomorphization-aware trait impl registration so that `Box<Int>` and `Box<String>` each get independent Display/Eq/etc. implementations.

**Primary recommendation:** Fix both issues in two passes: (1) Make `resolve_to_string_callback` recursive so it generates wrapper functions for nested collections and sum types; (2) Remove the `GenericDerive` error, implement per-monomorphization trait function generation, and register trait impls using the structural matching already in `TraitRegistry`.

## Standard Stack

Not applicable -- this is internal compiler work modifying existing Rust crates. No new dependencies needed.

### Core Crates Modified

| Crate | Files | Purpose |
|-------|-------|---------|
| `snow-codegen` | `mir/lower.rs` | MIR lowering: `resolve_to_string_callback`, `wrap_to_string`, `wrap_collection_to_string`, `generate_display_struct`, deriving generation |
| `snow-codegen` | `mir/types.rs` | Type resolution: `resolve_type`, `mangle_type_name`, `mir_type_to_ty` |
| `snow-typeck` | `infer.rs` | `register_struct_def`, `register_sum_type_def` -- remove `GenericDerive` error, add generic deriving support |
| `snow-typeck` | `traits.rs` | `TraitRegistry` -- structural matching already handles generic impls via `freshen_type_params` |
| `snow-typeck` | `error.rs`, `diagnostics.rs` | Remove or repurpose `GenericDerive` error variant |
| `snow-rt` | `collections/list.rs` | Runtime `snow_list_to_string` -- already has the callback pattern, no changes needed |

## Architecture Patterns

### Pattern 1: Display Callback Dispatch (Existing)

**What:** Collection Display uses a callback function pointer pattern. `snow_list_to_string(list_ptr, elem_to_str_fn_ptr)` calls the provided function for each element. The MIR lowerer resolves which function pointer to pass at compile time.

**Current flow:**
```
to_string([1, 2, 3])
  -> wrap_to_string(expr, typeck_ty=List<Int>)
    -> wrap_collection_to_string(expr, List<Int>)
      -> resolve_to_string_callback(Int) = "snow_int_to_string"
      -> emit: snow_list_to_string(list, snow_int_to_string)
```

**Where it breaks (TGEN-01):**
```
to_string([[1, 2], [3, 4]])
  -> wrap_to_string(expr, typeck_ty=List<List<Int>>)
    -> wrap_collection_to_string(expr, List<List<Int>>)
      -> resolve_to_string_callback(List<Int>)
        -> matches Ty::App(Con("List"), _) -- FALLS BACK to "snow_int_to_string"
```

**Source:** `/Users/sn0w/Documents/dev/snow/crates/snow-codegen/src/mir/lower.rs` lines 4484-4521

### Pattern 2: Trait Function Name Mangling (Existing)

**What:** Trait method implementations use the naming convention `Trait__method__TypeName`. For example:
- `Display__to_string__Point` for a struct `Point`
- `Eq__eq__Color` for a sum type `Color`
- `Hash__hash__Point` for a struct `Point`

For primitive types, these are redirected to runtime functions:
- `Display__to_string__Int` -> `snow_int_to_string`
- `Display__to_string__Float` -> `snow_float_to_string`

**Source:** `/Users/sn0w/Documents/dev/snow/crates/snow-codegen/src/mir/lower.rs` lines 3289-3323

### Pattern 3: Monomorphized Type Name Mangling (Existing)

**What:** Generic types with type arguments get mangled names via `mangle_type_name`:
- `Option<Int>` -> `Option_Int`
- `Result<Int, String>` -> `Result_Int_String`

Generic type parameters in variant fields resolve to `MirType::Ptr` at the LLVM level.

**Source:** `/Users/sn0w/Documents/dev/snow/crates/snow-codegen/src/mir/types.rs` lines 140-150

### Pattern 4: Structural Type Matching in TraitRegistry (Existing)

**What:** `TraitRegistry.has_impl` and `find_impl` use structural unification via `freshen_type_params`. A registered impl for `List<T>` will match queries for `List<Int>`, `List<String>`, `List<List<Int>>`. This already works correctly and is covered by tests.

**Source:** `/Users/sn0w/Documents/dev/snow/crates/snow-typeck/src/traits.rs` lines 164-180, 436-466

### Pattern 5: Deriving Guard for Generic Types (Current Limitation)

**What:** The type checker currently guards generic types from deriving:
```rust
// Generic types with deriving clause produce an error.
if has_deriving && !generic_params.is_empty() {
    ctx.errors.push(TypeError::GenericDerive { type_name: name.clone() });
}

// Only for non-generic structs (generic structs need monomorphized impls).
if generic_params.is_empty() {
    // ... register trait impls ...
}
```

This is the explicit limitation that Phase 24 removes.

**Source:** `/Users/sn0w/Documents/dev/snow/crates/snow-typeck/src/infer.rs` lines 1510-1518

### Anti-Patterns to Avoid

- **Generating specialized functions at type-definition time:** The problem with generic structs is that at definition time, the concrete type parameters are unknown. Trait functions must be generated either lazily (at instantiation site) or by generating generic versions that work with `Ptr`. Since Snow already uses pointer-sized values for generic params, generating a single set of trait functions that operate on `Ptr` fields and recursively dispatch via function pointers is the right approach.

- **Hardcoding to_string dispatch for each nesting level:** The recursive collection Display should use the same callback pattern uniformly. For `List<List<Int>>`, generate a wrapper function `__list_Int_to_string` that calls `snow_list_to_string` with `snow_int_to_string`, then pass `__list_Int_to_string` as the callback for the outer list's `snow_list_to_string`.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Structural type matching for impl lookup | Custom string-based matching | `TraitRegistry.has_impl` / `freshen_type_params` | Already works, tested with generics |
| Type name mangling for monomorphized types | Ad-hoc string concatenation | `mangle_type_name` in `mir/types.rs` | Consistent convention used everywhere |
| Runtime string building for Display | New runtime functions | Existing `snow_list_to_string`, `snow_map_to_string`, `snow_set_to_string` | Already support the callback pattern |

## Common Pitfalls

### Pitfall 1: Recursive Callback Must Be a Named Function

**What goes wrong:** Attempting to pass an inline expression or closure as the `elem_to_str` callback to `snow_list_to_string`. The runtime expects a bare function pointer (`fn(u64) -> *mut u8`).

**Why it happens:** For nested types like `List<List<Int>>`, the inner element's to_string is itself a two-argument function call (`snow_list_to_string(inner_list, snow_int_to_string)`), but the callback signature expects `fn(u64) -> *mut u8`.

**How to avoid:** Generate a synthetic MIR wrapper function (e.g., `__Display_list_Int_to_string`) that takes a single `u64` (the inner list pointer), casts it, and calls `snow_list_to_string(inner_list_ptr, snow_int_to_string)`. Pass this wrapper as the callback.

**Warning signs:** LLVM errors about function pointer type mismatches; runtime crashes when calling the callback.

### Pitfall 2: Generic Field Types Resolve to MirType::Ptr

**What goes wrong:** When generating Display/Eq/etc. for a generic struct like `Box<T>`, the field `value :: T` resolves to `MirType::Ptr` (or `MirType::Struct("T")`). The generated `wrap_to_string` call needs to know the concrete type to dispatch correctly, but at struct definition time, T is unknown.

**Why it happens:** The type registry stores `Ty::Con(TyCon("T"))` for the field type, which resolves to `MirType::Struct("T")` or `MirType::Ptr` depending on the code path.

**How to avoid:** For generic structs, generate Display/Eq/etc. functions that accept a function pointer table (vtable-like) for the generic operations. Alternatively, generate monomorphized versions at each instantiation site. The simplest approach: register a generic `Display` impl for `Box<T>` using the existing `TraitRegistry` structural matching, and generate the concrete `Display__to_string__Box_Int` function at the point where `Box<Int>` is first used, using the concrete field types.

**Warning signs:** Generated function calls to `Display__to_string__T` or `Eq__eq__T` which don't exist.

### Pitfall 3: Duplicate Impl Registration

**What goes wrong:** If `Box<Int>` is instantiated in multiple places, the deriving system might try to register `Display` for `Box_Int` multiple times, triggering `DuplicateImpl` errors.

**Why it happens:** The trait impl registration happens during type checking. If monomorphized impls are registered eagerly, duplicates are possible.

**How to avoid:** Use a `HashSet<String>` to track which monomorphized type names have already had their trait impls registered. Check before registering.

**Warning signs:** `TypeError::DuplicateImpl` errors at compile time for types that should work.

### Pitfall 4: Option/Result Already Have Generic Display via Debug

**What goes wrong:** Option and Result are built-in sum types that already have Debug-inspect functions generated. Adding Display for them through deriving could conflict.

**Why it happens:** Option and Result are registered in the type registry with `generic_params: vec!["T"]` and `generic_params: vec!["T", "E"]` respectively. They don't currently have deriving clauses.

**How to avoid:** The success criteria specifically mentions `[Some(1), None]` displaying correctly. This is a List<Option<Int>> case. The key insight: the inner Option<Int> Display needs to be handled in `resolve_to_string_callback` when the element type is a sum type with generic args. Option/Result already have Debug__inspect generated; Display may need to be generated similarly or a Display wrapper for `Option_Int` needs to be synthesized.

**Warning signs:** `to_string([Some(1), None])` producing `[<garbage>, <garbage>]` instead of `[Some(1), None]`.

### Pitfall 5: Sum Type Variant Field Access in Display

**What goes wrong:** For sum types like `Option<T>` with variant `Some(T)`, the Display function needs to match on the variant, extract the payload, and convert it to a string. But the payload type is `Ptr` (generic), so `wrap_to_string` doesn't know what runtime function to call.

**Why it happens:** Generic variant fields are `MirType::Ptr` at the MIR level. The `wrap_to_string` path for `Ptr` checks for collection types via `typeck_ty` but doesn't handle generic struct/sum fields.

**How to avoid:** For monomorphized Display of sum types, use the concrete type arguments to determine the correct to_string dispatch. When generating `Display__to_string__Option_Int`, the field type is known to be `Int`, so dispatch to `snow_int_to_string`.

## Code Examples

### Current resolve_to_string_callback (the v1.3 limitation)

```rust
// Source: /Users/sn0w/Documents/dev/snow/crates/snow-codegen/src/mir/lower.rs:4484-4521
fn resolve_to_string_callback(&self, elem_ty: &Ty) -> String {
    match elem_ty {
        Ty::Con(con) => match con.name.as_str() {
            "Int" => "snow_int_to_string".to_string(),
            "Float" => "snow_float_to_string".to_string(),
            "Bool" => "snow_bool_to_string".to_string(),
            "String" => "snow_string_to_string".to_string(),
            // Collection types nested inside collections -- v1.3 limitation
            "List" | "Map" | "Set" => "snow_int_to_string".to_string(),
            name => {
                // Check if user type has Display impl
                // ...
            }
        },
        Ty::App(con_ty, _) => {
            // Nested generic type (e.g., List<Int> inside a Set)
            // v1.3 limitation: fall back
            "snow_int_to_string".to_string()
        }
        _ => "snow_int_to_string".to_string(),
    }
}
```

### Current GenericDerive guard (the limitation to remove)

```rust
// Source: /Users/sn0w/Documents/dev/snow/crates/snow-typeck/src/infer.rs:1510-1518
// Generic types with deriving clause produce an error.
if has_deriving && !generic_params.is_empty() {
    ctx.errors.push(TypeError::GenericDerive {
        type_name: name.clone(),
    });
}

// Only for non-generic structs (generic structs need monomorphized impls).
if generic_params.is_empty() {
    let impl_ty = Ty::Con(TyCon::new(&name));
    // ... register trait impls ...
}
```

### Runtime callback pattern (already working)

```rust
// Source: /Users/sn0w/Documents/dev/snow/crates/snow-rt/src/collections/list.rs:298-330
pub extern "C" fn snow_list_to_string(
    list: *mut u8,
    elem_to_str: *mut u8,
) -> *mut u8 {
    type ElemToStr = unsafe extern "C" fn(u64) -> *mut u8;
    unsafe {
        let len = list_len(list) as usize;
        let data = list_data(list);
        let f: ElemToStr = std::mem::transmute(elem_to_str);
        let mut result = snow_string_new(b"[".as_ptr(), 1) as *mut u8;
        for i in 0..len {
            if i > 0 { /* append ", " */ }
            let elem_str = f(*data.add(i));
            /* append elem_str */
        }
        /* append "]" */
        result
    }
}
```

### Trait function name mangling pattern

```
Trait__method__TypeName

Examples:
  Display__to_string__Point
  Display__to_string__Option_Int   (monomorphized)
  Eq__eq__Box_String               (monomorphized generic)
  Hash__hash__Color
```

### Monomorphized type name mangling

```
Base_Arg1_Arg2

Examples:
  Option_Int
  Result_Int_String
  Box_Int
  Box_String
```

## State of the Art

| Old Approach (v1.3) | New Approach (Phase 24) | What Changes |
|---------------------|------------------------|--------------|
| Nested collection Display falls back to `snow_int_to_string` | Generate recursive wrapper functions for nested collection callbacks | `resolve_to_string_callback` handles `Ty::App` recursively |
| `GenericDerive` error blocks deriving on generic types | Remove error, generate monomorphized trait functions | `register_struct_def` / `register_sum_type_def` + MIR lowering |
| Trait impls only registered for non-generic types | Register generic trait impls (structural matching already supports this) | `TraitRegistry` pattern already handles `List<T>` matching `List<Int>` |
| No to_string callback for sum types in collections | Generate Display wrappers for monomorphized sum types | `resolve_to_string_callback` handles `Ty::App(Con("Option"), [Int])` |

## Detailed Implementation Strategy

### TGEN-01: Nested Collection Display

**Core change:** Make `resolve_to_string_callback` recursive. When the element type is `Ty::App(Con("List"), [inner_ty])`, generate a synthetic MIR wrapper function:

```
fn __display_list_Int_to_str(elem: u64) -> ptr {
    return snow_list_to_string(elem as ptr, snow_int_to_string)
}
```

Then return `"__display_list_Int_to_str"` as the callback name.

For sum types like `Option<Int>`, the callback should be the monomorphized Display function:
- If `Display__to_string__Option_Int` exists (from deriving or a manual impl), use it
- Otherwise, if `Debug__inspect__Option_Int` exists, use it as fallback
- Otherwise, fall back to a reasonable default

**Files to modify:**
1. `crates/snow-codegen/src/mir/lower.rs` -- `resolve_to_string_callback` (make recursive), add synthetic wrapper function generation
2. Possibly `crates/snow-codegen/src/mir/lower.rs` -- `wrap_collection_to_string` (handle Option/Result element types)

### TGEN-02: Generic Type Deriving

**Core changes:**

1. **Remove the GenericDerive error** in `crates/snow-typeck/src/infer.rs` (lines 1510-1515 for structs, 1794-1799 for sum types)

2. **Register generic trait impls** in `TraitRegistry` for generic types. Use the type-parameter form: register `Display` impl for `Ty::App(Con("Box"), [Con("T")])`. The structural matching in `freshen_type_params` will then match queries for `Box<Int>`, `Box<String>`, etc.

3. **Generate monomorphized MIR trait functions** at lowering time. When `lower_struct_def` encounters a generic struct with deriving, it needs to:
   - Generate a "template" approach: for each concrete instantiation seen during type checking, generate the trait functions with the concrete field types
   - OR generate the trait functions once using `Ptr` for generic fields, with recursive dispatch through the element callback pattern

4. **Track monomorphized instantiations** -- the type checker already resolves concrete types at call sites. These concrete types need to flow to the MIR lowerer so it can generate `Display__to_string__Box_Int`, `Eq__eq__Box_Int`, etc.

**Key insight:** The simplest approach may be to register a parametric impl (e.g., `Display for Box<T>`) in the trait registry and then, at each struct literal or instantiation site in the MIR lowerer, check if the monomorphized version of the trait function needs to be generated. The MIR lowerer already sees concrete types for all expressions.

**Files to modify:**
1. `crates/snow-typeck/src/infer.rs` -- Remove `GenericDerive` error, register parametric trait impls
2. `crates/snow-typeck/src/error.rs` -- Remove or repurpose `GenericDerive` variant
3. `crates/snow-typeck/src/diagnostics.rs` -- Remove `GenericDerive` diagnostic
4. `crates/snow-codegen/src/mir/lower.rs` -- Generate monomorphized trait functions for generic structs/sum types
5. `crates/snow-codegen/src/mir/types.rs` -- Possibly update `mir_type_to_ty` to handle monomorphized type names

## Open Questions

1. **How are generic struct instantiations tracked?**
   - What we know: The type checker resolves concrete types at each usage site (struct literals, pattern matches). The `types` HashMap maps `TextRange -> Ty`, so concrete types are available during MIR lowering.
   - What's unclear: Is there an existing mechanism to enumerate all concrete instantiations of a generic type, or does this need to be built?
   - Recommendation: During MIR lowering, when a struct literal for a generic struct is encountered, check if the monomorphized trait functions have been generated yet. Use a `HashSet<String>` to track which `Trait__method__TypeName_Arg` combinations have been generated.

2. **Should Option and Result get Display impls?**
   - What we know: Success criterion 2 requires `to_string([Some(1), None])` to produce `[Some(1), None]`. This means `Option<Int>` needs a Display-compatible function. Option is a built-in sum type, not user-defined, so it doesn't have a `deriving` clause.
   - What's unclear: Should Option/Result automatically get Display impls registered, or should the Display for collection elements fall back to Debug__inspect when no Display is available?
   - Recommendation: Generate Display functions for builtin sum types (Option, Result) when they are monomorphized with concrete types. Alternatively, have `resolve_to_string_callback` fall back to `Debug__inspect__TypeName` when `Display__to_string__TypeName` is not available.

3. **Interaction with where-clauses**
   - What we know: The trait registry supports `check_where_constraints`, and `FnConstraints` tracks where-clause constraints.
   - What's unclear: If `Box<T> deriving(Eq)` is used with `T = SomeType` that doesn't implement `Eq`, should this be caught by the type checker?
   - Recommendation: For Phase 24, don't add where-clause checking for derived traits on generic types. This can be a follow-up. The generated Eq functions will still work (they'll compare Ptr-level equality) even without the constraint check.

## Sources

### Primary (HIGH confidence)
- `/Users/sn0w/Documents/dev/snow/crates/snow-codegen/src/mir/lower.rs` -- MIR lowering, Display generation, to_string dispatch (4484-4521, 2436-2505, 3269-3287)
- `/Users/sn0w/Documents/dev/snow/crates/snow-typeck/src/infer.rs` -- Struct registration, GenericDerive guard (1441-1618)
- `/Users/sn0w/Documents/dev/snow/crates/snow-typeck/src/traits.rs` -- TraitRegistry, structural matching, freshen_type_params (164-180, 299-347)
- `/Users/sn0w/Documents/dev/snow/crates/snow-codegen/src/mir/types.rs` -- Type resolution, name mangling (23-178)
- `/Users/sn0w/Documents/dev/snow/crates/snow-typeck/src/builtins.rs` -- Display/Eq/Hash trait registration (746-893)
- `/Users/sn0w/Documents/dev/snow/crates/snow-rt/src/collections/list.rs` -- Runtime snow_list_to_string (298-330)
- `/Users/sn0w/Documents/dev/snow/crates/snow-codegen/src/codegen/intrinsics.rs` -- Runtime function declarations (375-388)

### Secondary (MEDIUM confidence)
- `/Users/sn0w/Documents/dev/snow/crates/snow-typeck/src/error.rs` -- GenericDerive error definition (241-243)
- `/Users/sn0w/Documents/dev/snow/crates/snow-typeck/src/diagnostics.rs` -- GenericDerive diagnostic (1336-1341)
- `/Users/sn0w/Documents/dev/snow/crates/snow-codegen/src/mir/mono.rs` -- Monomorphization pass (reachability only, lines 1-31)

## Metadata

**Confidence breakdown:**
- Architecture: HIGH -- Read all relevant source files directly, traced the exact code paths
- Implementation strategy: HIGH -- The existing patterns (callback dispatch, structural matching, name mangling) are well-understood and provide clear extension points
- Pitfalls: HIGH -- Identified from actual code analysis, not speculation

**Research date:** 2026-02-08
**Valid until:** 2026-03-08 (stable internal codebase, no external dependencies)
