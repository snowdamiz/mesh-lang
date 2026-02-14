# Phase 76: Iterator Protocol - Research

**Researched:** 2026-02-13
**Domain:** Compiler trait ecosystem extension -- Iterator + Iterable protocols, for-in desugaring via trait dispatch, runtime iterator handles, Iter.from() entry point
**Confidence:** HIGH

## Summary

Phase 76 introduces the Iterator protocol to Mesh: an `Iterator` trait with `type Item` and `fn next(self) -> Option<Self.Item>`, an `Iterable` trait with `type Iter` and `fn iter(self) -> Self.Iter`, for-in desugaring through these traits, built-in Iterable impls for List/Map/Set/Range, and an `Iter.from()` pipe-friendly entry point. The core infrastructure is ready: associated types (Phase 74) provide `type Item`/`type Iter` resolution, the `TraitRegistry` already handles `resolve_associated_type`, and the Option sum type exists at both the type level (`Ty::option(inner)`) and runtime level (`MeshOption { tag, value }`).

The existing for-in system uses four specialized MIR nodes (`ForInRange`, `ForInList`, `ForInMap`, `ForInSet`) with hardcoded indexed iteration in codegen. Phase 76 adds a fifth path: `ForInIterator`, a new MIR node for trait-based iteration using `next()` + Option tag-check loops. The critical constraint is **zero regressions** (ITER-05): existing for-in loops over known collection types MUST continue using the existing optimized paths. The Iterator-based path is a **fallback** activated only when the iterable type is not a recognized built-in collection but implements `Iterable` (or `Iterator` directly).

The implementation spans all four compiler passes plus the runtime: register traits in `builtins.rs`, extend `infer_for_in` in `infer.rs` to handle Iterable/Iterator types, add `ForInIterator` MIR node in `mod.rs`, add `lower_for_in_iterator` in `lower.rs`, add `codegen_for_in_iterator` in `expr.rs`, handle reachability in `mono.rs`, and add `mesh_list_iter_new`/`mesh_list_iter_next` (and similar) runtime C functions in `mesh-rt`.

**Primary recommendation:** Implement as a dual-path system: preserve all four existing ForIn* MIR nodes and their codegen unchanged, add new Iterator/Iterable traits in builtins, add a new ForInIterator MIR node as the fallback path in `lower_for_in_expr`, and add runtime iterator handle functions for each built-in collection type.

## Standard Stack

### Core
| Component | Location | Purpose | Why Standard |
|-----------|----------|---------|--------------|
| mesh-typeck/builtins.rs | `register_compiler_known_traits` (line 821) | Register Iterator and Iterable trait definitions with associated types | Existing pattern for all compiler-known traits; Phase 75 added Output assoc type the same way |
| mesh-typeck/infer.rs | `infer_for_in` (line 4104) | Type inference for for-in loops; currently handles Range/List/Map/Set | Must add Iterable/Iterator fallback to resolve Item type for unknown collection types |
| mesh-typeck/traits.rs | `TraitRegistry`, `resolve_associated_type` (line 356) | Trait impl lookup and associated type resolution | Phase 74 infrastructure; used by Phase 75 for Output; will resolve Item and Iter types |
| mesh-codegen/mir/mod.rs | MIR expression definitions (line 146) | Define ForInIterator MIR node alongside ForInRange/List/Map/Set | Follows exact same structural pattern as existing ForIn* nodes |
| mesh-codegen/mir/lower.rs | `lower_for_in_expr` (line 5999) | Dispatch for-in by collection type; needs Iterator fallback | Single integration point: add new branch after all existing checks |
| mesh-codegen/mir/mono.rs | `collect_function_refs` (line 227) | Reachability pass for monomorphization | Must handle ForInIterator the same way it handles ForInList/Map/Set |
| mesh-codegen/codegen/expr.rs | codegen_expr dispatch (line 146) | Generate LLVM IR for for-in loops | Must add codegen_for_in_iterator: next() call + tag-check loop |
| mesh-rt/src/collections/ | list.rs, map.rs, set.rs, range.rs | Runtime C functions for collection operations | Need new iterator handle functions: *_iter_new, *_iter_next |
| mesh-rt/src/option.rs | `MeshOption { tag, value }` | Option sum type runtime representation | Iterator.next() returns Option; codegen must extract tag + payload |

### Supporting
No new external dependencies. All changes are internal to existing crates.

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| Two-trait (Iterator + Iterable) | Single Iterator trait for everything | Cannot distinguish stateful iterators from iterable collections; collections would need internal mutable state, breaking value semantics |
| ForInIterator MIR node | Desugar to While+Match at MIR level | Creates complex MIR trees that codegen cannot optimize; breaks comprehension semantics; harder to debug. ARCHITECTURE.md explicitly warns against this anti-pattern |
| Runtime iterator handles (Ptr) | Stack-allocated iterator structs | Inconsistent with how all other collections work in Mesh (all are Ptr handles); would require new MirType variant |
| Iter.from() module function | Iterable.iter() only | Breaks pipe-friendly ergonomics (`list |> Iter.from() |> Iter.map(fn)`); both are needed |

## Architecture Patterns

### Current For-In Pipeline (Before Phase 76)

```
Source: for x in expr do body end
  -> Parser: ForInExpr(binding, iterable, filter?, body)
    -> Type Checker (infer_for_in):
       1. Infer iterable type
       2. Match: DotDot -> bind x as Int
                 List<T> -> bind x as T
                 Map<K,V> -> bind {k,v} as K,V
                 Set<T> -> bind x as T
                 Unknown -> bind x as Int (fallback)
       3. Return List<body_ty> (comprehension semantics)
    -> MIR Lowerer (lower_for_in_expr):
       DotDot -> ForInRange { var, start, end, filter, body, ty: Ptr }
       Map type -> ForInMap { key_var, val_var, collection, filter, body, ... }
       Set type -> ForInSet { var, collection, filter, body, ... }
       Otherwise -> ForInList { var, collection, filter, body, elem_ty, ... }
    -> Codegen:
       ForInRange  -> counter loop: i=start; while i<end; i++
       ForInList   -> counter loop: i=0; while i<len; list_get(i)
       ForInMap    -> counter loop: i=0; while i<size; entry_key(i), entry_value(i)
       ForInSet    -> counter loop: i=0; while i<size; element_at(i)
       ALL: list_builder_new -> push body results -> return built list
```

### Target For-In Pipeline (After Phase 76)

```
Source: for x in expr do body end
  -> Parser: ForInExpr(binding, iterable, filter?, body) [NO CHANGE]
    -> Type Checker (infer_for_in):
       1. Infer iterable type
       2. Match: DotDot -> bind x as Int  [EXISTING]
                 List<T> -> bind x as T   [EXISTING]
                 Map<K,V> -> bind {k,v}   [EXISTING]
                 Set<T> -> bind x as T    [EXISTING]
                 *** NEW: has_impl("Iterable", &iter_ty)? ***
                   -> resolve Item from Iterable impl
                   -> bind x as resolved Item type
                 Unknown -> bind x as Int [EXISTING FALLBACK]
       3. Return List<body_ty> (comprehension semantics) [UNCHANGED]
    -> MIR Lowerer (lower_for_in_expr):
       DotDot -> ForInRange  [EXISTING, UNCHANGED]
       Map type -> ForInMap  [EXISTING, UNCHANGED]
       Set type -> ForInSet  [EXISTING, UNCHANGED]
       List type -> ForInList [EXISTING, UNCHANGED]
       *** NEW: has_impl("Iterable", &iterable_ty)? ***
         -> ForInIterator { var, iterator, filter, body, elem_ty, body_ty,
                            next_fn, ty }
       Fallback -> ForInList [EXISTING]
    -> Codegen:
       ForInRange/List/Map/Set -> [EXISTING, UNCHANGED]
       *** NEW: ForInIterator -> ***
         1. Call Iterable__iter__Type(iterable) -> iterator_ptr
         2. Loop: call Iterator__next__Type(iterator_ptr)
         3. Extract tag from Option result
         4. If tag == 0 (Some): extract payload, bind to var, run body, push result
         5. If tag == 1 (None): exit loop
         6. Return built list (comprehension semantics preserved)
```

### Pattern 1: Iterator Trait Registration in Builtins

**What:** Register Iterator and Iterable traits as compiler-known traits with associated types, following the exact pattern used by Add/Sub/Mul/Div/Neg.

**When to use:** During trait registration in `register_compiler_known_traits`.

**Example:**
```rust
// Source: crates/mesh-typeck/src/builtins.rs (to be added)

// ── Iterator trait ──────────────────────────────────────
registry.register_trait(TraitDef {
    name: "Iterator".to_string(),
    methods: vec![TraitMethodSig {
        name: "next".to_string(),
        has_self: true,
        param_count: 0,
        return_type: None,  // Option<Self.Item> -- resolved per impl
        has_default_body: false,
    }],
    associated_types: vec![AssocTypeDef { name: "Item".to_string() }],
});

// ── Iterable trait ──────────────────────────────────────
registry.register_trait(TraitDef {
    name: "Iterable".to_string(),
    methods: vec![TraitMethodSig {
        name: "iter".to_string(),
        has_self: true,
        param_count: 0,
        return_type: None,  // Self.Iter -- resolved per impl
        has_default_body: false,
    }],
    associated_types: vec![
        AssocTypeDef { name: "Item".to_string() },
        AssocTypeDef { name: "Iter".to_string() },
    ],
});
```

### Pattern 2: Built-In Iterable Impls for Collections

**What:** Register Iterable impls for List, Map, Set, and Range. Each impl specifies the Item type and the Iter type (an opaque iterator handle).

**Example:**
```rust
// Source: crates/mesh-typeck/src/builtins.rs (to be added)

// impl Iterable for List<T>
{
    let list_t = Ty::App(Box::new(Ty::Con(TyCon::new("List"))), vec![Ty::Con(TyCon::new("T"))]);
    let mut methods = FxHashMap::default();
    methods.insert("iter".to_string(), ImplMethodSig {
        has_self: true,
        param_count: 0,
        return_type: None,  // returns the iterator handle (Ptr)
    });
    let mut assoc_types = FxHashMap::default();
    assoc_types.insert("Item".to_string(), Ty::Con(TyCon::new("T")));
    assoc_types.insert("Iter".to_string(), Ty::Con(TyCon::new("ListIterator")));
    let _ = registry.register_impl(ImplDef {
        trait_name: "Iterable".to_string(),
        impl_type: list_t,
        impl_type_name: "List".to_string(),
        methods,
        associated_types: assoc_types,
    });
}

// impl Iterator for ListIterator<T>
{
    let list_iter_t = Ty::Con(TyCon::new("ListIterator"));
    let mut methods = FxHashMap::default();
    methods.insert("next".to_string(), ImplMethodSig {
        has_self: true,
        param_count: 0,
        return_type: None,  // Option<T> -- resolved per concrete T
    });
    let mut assoc_types = FxHashMap::default();
    assoc_types.insert("Item".to_string(), Ty::Con(TyCon::new("T")));
    let _ = registry.register_impl(ImplDef {
        trait_name: "Iterator".to_string(),
        impl_type: list_iter_t,
        impl_type_name: "ListIterator".to_string(),
        methods,
        associated_types: assoc_types,
    });
}
```

**Note:** The exact type representations for iterator handle types (ListIterator, MapIterator, etc.) need careful design. These are compiler-internal types that map to `MirType::Ptr` at the MIR level (opaque runtime handles, like all collections). The type checker needs to resolve associated types through these, but codegen treats them as plain pointers.

### Pattern 3: ForInIterator MIR Node

**What:** A new MIR expression variant for iterator-based for-in loops, parallel to ForInList/ForInMap/ForInSet/ForInRange.

**Example:**
```rust
// Source: crates/mesh-codegen/src/mir/mod.rs (to be added)

/// For-in loop over any type implementing Iterable/Iterator.
/// Desugared to repeated next() calls with Option tag checking.
ForInIterator {
    /// Loop variable name.
    var: String,
    /// The iterator expression (result of calling Iterable.iter(collection)).
    iterator: Box<MirExpr>,
    /// Optional filter expression (`when condition`).
    filter: Option<Box<MirExpr>>,
    /// Loop body.
    body: Box<MirExpr>,
    /// Resolved element type (Iterator::Item for the concrete type).
    elem_ty: MirType,
    /// Type of body expression.
    body_ty: MirType,
    /// Mangled name for the next() function: "Iterator__next__TypeName".
    next_fn: String,
    /// The Option<Item> sum type name for the next() return value.
    option_ty: MirType,
    /// Result type (Ptr for comprehension semantics -- list of body results).
    ty: MirType,
},
```

**Also needs:** Match arm in `MirExpr::ty()` and `collect_function_refs` in mono.rs.

### Pattern 4: Lowering For-In to ForInIterator

**What:** In `lower_for_in_expr`, after all existing collection type checks, add a new branch that checks for Iterable trait implementation and lowers to ForInIterator.

**Example:**
```rust
// Source: crates/mesh-codegen/src/mir/lower.rs (to be modified)

fn lower_for_in_expr(&mut self, for_in: &ForInExpr) -> MirExpr {
    // EXISTING: check for DotDot range
    if let Some(Expr::BinaryExpr(ref bin)) = for_in.iterable() {
        if bin.op().map(|t| t.kind()) == Some(SyntaxKind::DOT_DOT) {
            return self.lower_for_in_range(for_in, bin);
        }
    }

    // EXISTING: detect collection type from typeck
    let iterable_ty = for_in
        .iterable()
        .and_then(|e| self.get_ty(e.syntax().text_range()))
        .cloned();

    if let Some(ref ty) = iterable_ty {
        if let Some((key_ty, val_ty)) = extract_map_types(ty) {
            return self.lower_for_in_map(for_in, &key_ty, &val_ty);
        }
        if let Some(elem_ty) = extract_set_elem_type(ty) {
            return self.lower_for_in_set(for_in, &elem_ty);
        }
        if let Some(elem_ty) = extract_list_elem_type(ty) {
            return self.lower_for_in_list(for_in, &elem_ty);
        }

        // *** NEW: check if type implements Iterable or Iterator ***
        let ty_for_lookup = /* convert Ty to lookup form */;
        if self.trait_registry.has_impl("Iterable", &ty_for_lookup) {
            return self.lower_for_in_iterator(for_in, &ty_for_lookup);
        }
        if self.trait_registry.has_impl("Iterator", &ty_for_lookup) {
            return self.lower_for_in_direct_iterator(for_in, &ty_for_lookup);
        }
    }

    // EXISTING: fallback to list iteration
    self.lower_for_in_list(for_in, &Ty::int())
}
```

### Pattern 5: Codegen for ForInIterator

**What:** Generate LLVM IR for the iterator-based loop: call next(), extract tag from Option result, branch on Some/None, bind element, run body, collect results.

**Example (pseudocode):**
```
codegen_for_in_iterator(var, iterator_expr, filter, body, elem_ty, next_fn, option_ty, ty):
  // Setup
  iter_val = codegen(iterator_expr)          // Call Iterable__iter__Type(collection)
  iter_alloca = alloca(Ptr)                  // Store iterator handle
  store iter_val -> iter_alloca
  result_list = call mesh_list_builder_new(0) // Pre-allocate result list
  result_alloca = alloca(Ptr)
  store result_list -> result_alloca

  // Loop header: call next()
  loop_header:
    iter = load iter_alloca
    next_result = call Iterator__next__Type(iter) // Returns Option (MeshOption*)
    // Option is a sum type: { tag: i8, value: ptr }
    // tag 0 = Some, tag 1 = None
    tag_ptr = struct_gep(next_result, 0)
    tag = load tag_ptr
    is_some = icmp eq tag, 0                 // Some has tag 0 in Mesh's Option
    br is_some, loop_body, loop_exit

  // Loop body: extract element, bind, run body
  loop_body:
    value_ptr = struct_gep(next_result, 1)   // Payload field
    raw_value = load value_ptr
    typed_elem = convert_from_list_element(raw_value, elem_ty) // Reuse existing helper
    var_alloca = alloca(elem_llvm_ty)
    store typed_elem -> var_alloca
    // Bind loop variable
    locals[var] = var_alloca
    // Optional filter
    if filter:
      filter_val = codegen(filter)
      br filter_val, do_body, latch
    // Body
    body_val = codegen(body)
    body_as_i64 = convert_to_list_element(body_val, body_ty)
    call mesh_list_builder_push(result_list, body_as_i64)
    br loop_header

  // Loop exit
  loop_exit:
    final_list = load result_alloca
    return final_list
```

**Key detail:** The Option tag encoding. In Mesh, `Some` is variant 0 (tag=0) and `None` is variant 1 (tag=1). The codegen must check `tag == 0` for Some. This matches the `MeshOption` struct in `mesh-rt/src/option.rs` where `tag: 0 = Some, tag: 1 = None`.

### Pattern 6: Runtime Iterator Handle Functions

**What:** New C-callable functions in mesh-rt that create and advance iterator handles for each collection type.

**Example:**
```rust
// Source: crates/mesh-rt/src/collections/list.rs (to be added)

/// Internal iterator state for List iteration.
struct ListIterator {
    list: *mut u8,    // The underlying MeshList
    index: i64,       // Current position
    length: i64,      // Cached length
}

#[no_mangle]
pub extern "C" fn mesh_list_iter_new(list: *mut u8) -> *mut u8 {
    unsafe {
        let len = mesh_list_length(list);
        let iter = mesh_gc_alloc_actor(
            std::mem::size_of::<ListIterator>() as u64,
            std::mem::align_of::<ListIterator>() as u64,
        ) as *mut ListIterator;
        (*iter).list = list;
        (*iter).index = 0;
        (*iter).length = len;
        iter as *mut u8
    }
}

#[no_mangle]
pub extern "C" fn mesh_list_iter_next(iter: *mut u8) -> *mut u8 {
    unsafe {
        let iter = iter as *mut ListIterator;
        if (*iter).index >= (*iter).length {
            // Return None: tag=1, value=null
            alloc_option(1, std::ptr::null_mut())
        } else {
            let elem = mesh_list_get((*iter).list, (*iter).index);
            (*iter).index += 1;
            // Return Some: tag=0, value=elem as pointer
            alloc_option(0, elem as *mut u8)
        }
    }
}
```

**Same pattern for:** `mesh_map_iter_new`/`mesh_map_iter_next`, `mesh_set_iter_new`/`mesh_set_iter_next`, `mesh_range_iter_new`/`mesh_range_iter_next`.

### Anti-Patterns to Avoid

- **Desugaring Iterator loops to While+Match at MIR level:** ARCHITECTURE.md (line 543-549) explicitly warns against this. The MIR tree becomes complex and hard to optimize. Use a dedicated `ForInIterator` MIR node that codegen compiles directly to a tag-check loop.

- **Making Iterator stateful via mutation of Self:** Mesh has no `&mut self`. Iterator state is managed by runtime handles (opaque Ptr). The `next()` function advances internal state through the runtime handle, consistent with how all collection operations work. (ARCHITECTURE.md line 561-565)

- **Replacing existing ForIn* paths with Iterator:** The existing `ForInList`, `ForInMap`, `ForInSet`, `ForInRange` paths are optimized (counter-based, no Option allocation per element). Replacing them with Iterator-based loops would add overhead (heap-allocated Option per element). Keep both paths: existing for known types, Iterator for user types.

- **Creating separate dispatch paths for built-in vs user trait methods:** `resolve_trait_callee` (lower.rs line 5300) already handles both built-in and user types through the same `find_method_traits` API. Do not add special cases for Iterator/Iterable.

- **Confusing associated types with generic parameters:** Do NOT represent `Iterator<Item = Int>` as `Iterator<Int>` (a type application). Associated types are determined by the impl, not the caller. (ARCHITECTURE.md line 537-541)

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Associated type resolution | Custom Item/Iter lookup | `TraitRegistry::resolve_associated_type` | Implemented in Phase 74; used successfully by Phase 75 for Output |
| Trait impl registration | New registration mechanism | `register_compiler_known_traits` pattern in builtins.rs | Handles method validation, duplicate checks, associated type validation |
| Trait method dispatch | Custom iterator dispatch | `resolve_trait_callee` in lower.rs (line 5300) | Already mangles to `Trait__Method__Type` pattern; handles both built-in and user types |
| Option type construction | Custom tagged union codegen | Existing `ConstructVariant` + `MeshOption` runtime | Option sum type already works end-to-end through codegen |
| List builder for comprehension results | Custom result collection | `mesh_list_builder_new` + `mesh_list_builder_push` | Used by all existing ForIn* codegen paths |
| Type-to-MIR-type resolution | Custom type mapping | `resolve_range` / `resolve_type` | Already used everywhere in the lowerer |

**Key insight:** Phase 76 is primarily about wiring existing infrastructure together in a new way. The trait system (Phase 74 associated types, Phase 75 Output resolution), the Option sum type, the for-in codegen patterns (list builder, counter loop, break/continue), and the runtime handle pattern are all proven and working. The new work is: (1) registering two new traits, (2) adding one new MIR node, (3) adding one new codegen path, (4) adding runtime iterator handle functions.

## Common Pitfalls

### Pitfall 1: Breaking Comprehension Semantics for Iterator-Based For-In
**What goes wrong:** The iterator-based for-in returns Unit or the last element instead of a List of collected body results. Existing code like `let squares = for x in custom_iterable do x * x end` breaks because it no longer returns a list.
**Why it happens:** Rust's for loops return Unit. A developer familiar with Rust might implement the iterator loop as `while let Some(x) = next() { body }` which returns Unit.
**How to avoid:** ForInIterator codegen MUST use the same list builder pattern as all existing ForIn* nodes: `mesh_list_builder_new()` before the loop, `mesh_list_builder_push(body_result)` after each body execution, return the built list. The MIR node's `ty` field must be `MirType::Ptr` (list pointer), not `MirType::Unit`.
**Warning signs:** `for x in expr do body end` returns Unit when the iterable is a user-defined Iterable type.

### Pitfall 2: Option Tag Encoding Mismatch
**What goes wrong:** Codegen checks `tag == 1` for Some but the runtime returns `tag == 0` for Some (or vice versa). The loop never executes, or runs infinitely.
**Why it happens:** Mesh's Option type has `Some` as variant 0 (tag=0) and `None` as variant 1 (tag=1). This matches the `MeshOption` struct in `option.rs`. But a developer might assume `Some=1` (truthy) and `None=0` (falsy).
**How to avoid:** Verify against `mesh-rt/src/option.rs`: `tag: 0 = Some` (first variant), `tag: 1 = None` (second variant). The codegen must check `icmp eq tag, 0` for the Some case. Cross-reference with existing match codegen for Option values.
**Warning signs:** Iterator loop never executes (treating all results as None) or never terminates (treating None as Some).

### Pitfall 3: Iterator Handle Lifetime and GC
**What goes wrong:** The iterator handle is stack-allocated or not visible to the GC, causing the underlying collection to be collected mid-iteration.
**Why it happens:** Iterator handles hold a pointer to the collection they iterate over. If the handle is not GC-visible, the GC might collect the collection.
**How to avoid:** Allocate iterator handles with `mesh_gc_alloc_actor` (like all other runtime objects). The handle's internal pointer to the collection acts as a GC root, keeping the collection alive. This is the same pattern used by all collection runtime functions.
**Warning signs:** Intermittent crashes or corrupted data during iteration, especially under GC pressure.

### Pitfall 4: Iterable vs Iterator Type Confusion in lower_for_in_expr
**What goes wrong:** The lowerer checks for `Iterator` impl on the collection type instead of `Iterable`. Collections implement `Iterable` (they produce iterators), not `Iterator` (they ARE NOT iterators). Checking `has_impl("Iterator", &list_ty)` returns false because List does not implement Iterator.
**Why it happens:** Confusing the two traits. Iterable has `iter(self) -> Self.Iter`; Iterator has `next(self) -> Option<Self.Item>`.
**How to avoid:** The lowering should check for `Iterable` first (the common case for collections), call `Iterable__iter__Type(collection)` to get the iterator, then use `Iterator__next__IterType(iter)` for the loop. Also support direct `Iterator` types (if the user passes an iterator directly to for-in, skip the `iter()` call).
**Warning signs:** For-in over collections falls through to the List fallback instead of using the Iterator path.

### Pitfall 5: Monomorphization Missing ForInIterator
**What goes wrong:** The `collect_function_refs` function in `mono.rs` does not traverse `ForInIterator` nodes. The mangled `next_fn` name is not collected as reachable. The function is pruned by the mono pass and missing at link time.
**Why it happens:** `collect_function_refs` has explicit match arms for `ForInRange`, `ForInList`, `ForInMap`, `ForInSet`. Adding a new MIR node without adding a corresponding match arm means the node's sub-expressions and referenced function names are never traversed.
**How to avoid:** Add a `MirExpr::ForInIterator { iterator, filter, body, next_fn, .. }` arm in `collect_function_refs` that: (a) recursively collects refs from `iterator`, `filter`, `body`, and (b) adds `next_fn` and the `iter_fn` names to the reachable set.
**Warning signs:** Linker error: "undefined reference to `Iterator__next__ListIterator`" or similar.

### Pitfall 6: Type Checker Fallback Hides Iterator Errors
**What goes wrong:** The `CollectionType::Unknown` fallback in `infer_for_in` (line 4196-4203) binds the loop variable as `Int` instead of resolving the Item type through the Iterable trait. User-defined iterable types silently get Int as the element type.
**Why it happens:** The current code has a fallback `Unknown -> bind as Int` path that activates for any type not recognized as List/Map/Set/Range. If the Iterable check is not added BEFORE this fallback, it gets bypassed.
**How to avoid:** In `infer_for_in`, after the existing collection type checks and before the Unknown fallback, add a check: `if trait_registry.has_impl("Iterable", &iter_ty)` then resolve `Item` via `resolve_associated_type("Iterable", "Item", &iter_ty)` and bind the loop variable accordingly.
**Warning signs:** Compiling `for x in my_custom_iterable do x end` where x should be String, but the type checker infers x as Int.

### Pitfall 7: Iter.from() Name Conflicts with Type Constructors
**What goes wrong:** `Iter.from(list)` is parsed as a field access on `Iter` followed by a call, but `Iter` could be interpreted as a type name (triggering struct field access codegen) or a module name.
**Why it happens:** The parser/lowerer interprets `X.method()` as either field access on a variable `X`, or a module-qualified call `Module.function()`, or a type constructor `Type.variant()`. `Iter` is not a variable, module, or sum type.
**How to avoid:** Implement `Iter.from()` as a static module function in the stdlib, not as a method on a type. The lowerer's stdlib method fallback path (lower.rs line 5345-5374) already handles patterns like `String.method()` by mapping to `mesh_string_method`. Add `Iter.from` -> `mesh_iter_from` in the same way.
**Warning signs:** `Iter.from(list)` produces "unknown variable Iter" or "field access on non-struct type" errors.

## Code Examples

### User-Facing Syntax: Custom Iterator

```mesh
struct Counter do
  current :: Int
  max :: Int
end

impl Iterator for Counter do
  type Item = Int
  fn next(self) -> Option<Int> do
    if self.current >= self.max do
      None
    else
      let val = self.current
      # Note: returns updated counter implicitly via runtime handle
      Some(val)
    end
  end
end

fn main() do
  let c = Counter { current: 0, max: 5 }
  for x in c do
    println(x.to_string())
  end
end
```

### User-Facing Syntax: Custom Iterable Collection

```mesh
struct NumberRange do
  start :: Int
  stop :: Int
end

struct NumberRangeIter do
  current :: Int
  stop :: Int
end

impl Iterator for NumberRangeIter do
  type Item = Int
  fn next(self) -> Option<Int> do
    if self.current >= self.stop do
      None
    else
      Some(self.current)
    end
  end
end

impl Iterable for NumberRange do
  type Item = Int
  type Iter = NumberRangeIter
  fn iter(self) -> NumberRangeIter do
    NumberRangeIter { current: self.start, stop: self.stop }
  end
end

fn main() do
  let r = NumberRange { start: 1, stop: 4 }
  for x in r do
    println(x.to_string())
  end
  # prints: 1, 2, 3
end
```

### User-Facing Syntax: Iter.from() Pipe Entry Point

```mesh
fn main() do
  let list = [1, 2, 3, 4, 5]
  # Iter.from() creates an iterator from a collection
  let result = Iter.from(list)
  # In Phase 78+, this enables: list |> Iter.from() |> Iter.map(fn(x) x * 2 end) |> Iter.collect()
end
```

### Backward Compatibility: All Existing For-In Must Still Work

```mesh
fn main() do
  # Range (ForInRange path -- UNCHANGED)
  for i in 0..5 do println(i.to_string()) end

  # List (ForInList path -- UNCHANGED)
  for x in [1, 2, 3] do println(x.to_string()) end

  # Map (ForInMap path -- UNCHANGED)
  let m = Map.new() |> Map.put("a", 1) |> Map.put("b", 2)
  for {k, v} in m do println(k) end

  # Set (ForInSet path -- UNCHANGED)
  let s = Set.new() |> Set.add(1) |> Set.add(2)
  for x in s do println(x.to_string()) end

  # Comprehension semantics (UNCHANGED)
  let squares = for x in [1, 2, 3] do x * x end
  # squares == [1, 4, 9]
end
```

## Requirement Mapping

| Requirement | What It Needs | Implementation Approach |
|-------------|---------------|------------------------|
| ITER-01: Iterator interface with `type Item` and `fn next(self) -> Option<Self.Item>` | Trait definition with associated type and method | Register in `builtins.rs` with `AssocTypeDef { name: "Item" }` and `TraitMethodSig { name: "next", has_self: true, param_count: 0 }`. Users can define `impl Iterator for MyType` with `type Item = T` and `fn next(self) -> Option<T>` |
| ITER-02: Iterable interface with `type Iter` and `fn iter(self) -> Self.Iter` | Trait definition with two associated types | Register in `builtins.rs` with associated types `Item` and `Iter`, method `iter`. The Iter type must implement Iterator. |
| ITER-03: for-in over any Iterable type | Typeck + MIR + codegen support | Add Iterable check in `infer_for_in` (resolve Item), add `ForInIterator` MIR node, add fallback in `lower_for_in_expr`, add `codegen_for_in_iterator` |
| ITER-04: Built-in types implement Iterable | Compiler-provided impls | Register Iterable impls for List/Map/Set/Range in `builtins.rs`. Add runtime iterator handle functions (`mesh_list_iter_new`, `mesh_list_iter_next`, etc.) in `mesh-rt` |
| ITER-05: Existing for-in loops continue working | Zero regressions | Preserve all four existing ForIn* MIR nodes and codegen paths. Iterator path is a FALLBACK only, checked after all existing collection type checks |
| ITER-06: Iter.from() pipe-friendly entry point | Module function | Add `Iter.from(collection)` as a stdlib function that calls the collection's `iter()` method. Map in lowerer's module method resolution |

## File Touch Points

Complete list of files that need modification in Phase 76:

### mesh-typeck (Type System)
1. **`builtins.rs`** -- Register Iterator trait (with `type Item`, `fn next`), Iterable trait (with `type Item`, `type Iter`, `fn iter`), and built-in Iterable impls for List/Map/Set/Range + corresponding Iterator impls for iterator handle types (ListIterator, MapIterator, etc.)
2. **`infer.rs`** -- Modify `infer_for_in` (line 4104): after existing collection type checks and before `CollectionType::Unknown` fallback, check `has_impl("Iterable", &iter_ty)`, resolve `Item` via `resolve_associated_type`, bind loop variable to the resolved Item type

### mesh-codegen (Code Generation)
3. **`mir/mod.rs`** -- Add `ForInIterator` MIR node variant (var, iterator, filter, body, elem_ty, body_ty, next_fn, option_ty, ty); add match arm in `MirExpr::ty()`
4. **`mir/lower.rs`** -- Add `lower_for_in_iterator` function; modify `lower_for_in_expr` (line 5999) to check for Iterable/Iterator impls after existing collection checks; generate ForInIterator node with resolved next_fn name
5. **`mir/mono.rs`** -- Add `ForInIterator` arm in `collect_function_refs` (around line 249) to traverse iterator, filter, body sub-expressions and collect next_fn as reachable
6. **`codegen/expr.rs`** -- Add `codegen_for_in_iterator` function; add match arm for `MirExpr::ForInIterator` in main `codegen_expr` dispatch (around line 146). Generates: iter call + next() loop + Option tag check + element extraction + list builder

### mesh-rt (Runtime)
7. **`collections/list.rs`** -- Add `ListIterator` struct, `mesh_list_iter_new(list) -> iter_ptr`, `mesh_list_iter_next(iter) -> option_ptr`
8. **`collections/map.rs`** -- Add `MapIterator` struct, `mesh_map_iter_new(map) -> iter_ptr`, `mesh_map_iter_next(iter) -> option_ptr` (yields key-value tuples)
9. **`collections/set.rs`** -- Add `SetIterator` struct, `mesh_set_iter_new(set) -> iter_ptr`, `mesh_set_iter_next(iter) -> option_ptr`
10. **`collections/range.rs`** -- Add `RangeIterator` struct, `mesh_range_iter_new(start, end) -> iter_ptr`, `mesh_range_iter_next(iter) -> option_ptr`

### Test Files
11. **`tests/e2e/iterator_basic.mpl`** -- E2E test: user-defined Iterator with for-in
12. **`tests/e2e/iterator_iterable.mpl`** -- E2E test: user-defined Iterable + Iterator pair with for-in
13. **`tests/e2e/iterator_builtin.mpl`** -- E2E test: built-in collection iteration via Iterable trait (if distinguishable from existing paths)
14. **`crates/meshc/tests/e2e.rs`** -- New test functions for iterator E2E tests

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Hardcoded per-collection for-in (List/Map/Set/Range only) | Adding Iterator/Iterable trait-based fallback | Phase 76 (now) | User-defined types can participate in for-in loops |
| No Iterator trait | Adding Iterator with type Item + fn next | Phase 76 (now) | Foundation for lazy evaluation, pipe chains, and combinators |
| No Iterable trait | Adding Iterable with type Iter + fn iter | Phase 76 (now) | Collections can produce iterators without being iterators themselves |
| No Iter.from() | Adding pipe-friendly entry point | Phase 76 (now) | Enables `list |> Iter.from() |> ...` pipe chains |

## Open Questions

1. **How should iterator handle types be represented in the type system?**
   - What we know: At MIR level, iterator handles are `MirType::Ptr` (like all collections). At the typeck level, they need type names (ListIterator, MapIterator, etc.) for trait resolution.
   - What's unclear: Whether these should be registered as real types in the TypeRegistry or be phantom types known only to the trait system. The `extract_list_elem_type` function currently recognizes `List<T>` by matching `Ty::App(Con("List"), [T])`. Iterator handle types might need similar registration.
   - Recommendation: Register them as opaque types in the TypeRegistry (like existing collections), mapped to `MirType::Ptr` during MIR lowering. The planner should investigate the exact registration mechanism.

2. **Should built-in collections ALSO use the Iterator path for for-in?**
   - What we know: ITER-04 requires built-in types to implement Iterable. ITER-05 requires existing for-in to work unchanged. These are compatible: register the impls (for future use by pipe chains and Iter.from()), but keep the existing optimized ForInList/Map/Set/Range paths for for-in.
   - What's unclear: Whether there should be a way to force the Iterator path for testing purposes (e.g., a compiler flag or a test mode).
   - Recommendation: Register Iterable impls for built-in types (needed for Iter.from() and future pipe chains), but always use existing optimized paths for for-in. Test the Iterator path exclusively through user-defined Iterable types.

3. **How does the next() return type (Option<Self.Item>) interact with type inference?**
   - What we know: The type checker currently does not validate method return types against associated type projections (it uses `return_type: None` for trait methods). The actual return type is checked at the impl level. `resolve_associated_type` returns the concrete Item type.
   - What's unclear: Whether `infer_for_in` needs to construct `Ty::option(item_ty)` and unify it with the next() return type, or whether it can simply use the resolved Item type directly.
   - Recommendation: Use the resolved Item type directly for binding the loop variable. The Option wrapping is a runtime concern handled by codegen (tag-check loop). The type checker only needs to know the element type, not the Option wrapper.

4. **Iter.from() vs method syntax for creating iterators**
   - What we know: ITER-06 requires `Iter.from()` as a pipe-friendly entry point. The Iterable trait has `fn iter(self)` as the method-call equivalent.
   - What's unclear: Whether `Iter.from()` should be a separate runtime function or whether it should desugar to calling `Iterable__iter__Type` on the collection.
   - Recommendation: `Iter.from(collection)` desugars to `Iterable__iter__Type(collection)` in the MIR lowerer. No separate runtime function needed. The lowerer recognizes `Iter.from` as a special module function pattern and dispatches through the Iterable trait.

5. **Map iterator element type: tuple or destructured?**
   - What we know: The existing `ForInMap` codegen uses destructured `{key, value}` bindings with two separate variables. The Iterator protocol yields single elements via `next()`.
   - What's unclear: What type `mesh_map_iter_next` returns. A tuple `(K, V)`? Two separate values?
   - Recommendation: `mesh_map_iter_next` returns a tuple (pair) value `(K, V)`. The for-in body receives the tuple, and the user destructures with `let {k, v} = elem`. This is consistent with how Map iterators work in every language. However, the existing `for {k, v} in map` syntax expects two separate bindings, which requires special handling in the lowerer/typeck for Map-typed Iterables.

## Sources

### Primary (HIGH confidence)
- `crates/mesh-typeck/src/builtins.rs` lines 821-895 -- existing trait registration pattern (Add/Sub/Mul/Div/Mod/Neg with Output associated type)
- `crates/mesh-typeck/src/traits.rs` full file -- TraitDef, ImplDef, TraitRegistry, resolve_associated_type (verified: Phase 74 infrastructure working)
- `crates/mesh-typeck/src/infer.rs` lines 4099-4248 -- `infer_for_in` function (verified: dispatches by collection type, Unknown fallback to Int)
- `crates/mesh-codegen/src/mir/mod.rs` lines 146-434 -- MIR expression types including ForInRange/List/Map/Set (verified: structural pattern for new ForInIterator)
- `crates/mesh-codegen/src/mir/lower.rs` lines 5997-6089 -- `lower_for_in_expr` and `lower_for_in_range`/`lower_for_in_list` (verified: dispatch chain, insertion point for Iterator fallback)
- `crates/mesh-codegen/src/mir/mono.rs` lines 227-249 -- `collect_function_refs` ForIn* handling (verified: explicit match arms needed per MIR variant)
- `crates/mesh-codegen/src/codegen/expr.rs` lines 146-162 -- codegen_expr dispatch for ForIn* (verified: each MIR variant has its own codegen function)
- `crates/mesh-codegen/src/codegen/expr.rs` lines 3544-3707 -- `codegen_for_in_list` (verified: counter loop + list_get + list_builder pattern)
- `crates/mesh-codegen/src/codegen/expr.rs` lines 2410-2557 -- `codegen_for_in_range` (verified: counter loop + list_builder pattern)
- `crates/mesh-rt/src/option.rs` full file -- MeshOption { tag: u8, value: *mut u8 }, alloc_option (verified: tag 0 = Some, tag 1 = None)
- `crates/mesh-rt/src/collections/list.rs` -- runtime list functions (verified: mesh_list_get, mesh_list_length, mesh_list_builder_new/push pattern)
- `crates/mesh-rt/src/collections/map.rs` -- runtime map functions (verified: mesh_map_size, mesh_map_entry_key/value pattern)
- `crates/mesh-rt/src/collections/set.rs` -- runtime set functions (verified: mesh_set_size, mesh_set_element_at pattern)
- `crates/mesh-rt/src/collections/range.rs` -- runtime range functions (verified: mesh_range_new, mesh_range_length)
- `crates/mesh-codegen/src/mir/lower.rs` lines 5300-5388 -- `resolve_trait_callee` (verified: Trait__Method__Type mangling, handles built-in and user types uniformly)
- `.planning/REQUIREMENTS.md` ITER-01 through ITER-06 -- Phase 76 requirements (all Pending, assigned to Phase 76)

### Secondary (MEDIUM confidence)
- `.planning/research/ARCHITECTURE.md` lines 167-256 -- Iterator protocol architecture (detailed ForInIterator MIR node, codegen pattern, state management)
- `.planning/research/FEATURES.md` lines 55-100 -- Iterator protocol feature analysis (two-trait design, Iterable vs Iter.from() recommendation)
- `.planning/research/PITFALLS.md` lines 35-55, 152-169, 201-211 -- Pitfalls 2, 7, 10 (iterator MIR representation, comprehension semantics, IntoIterator vs Iterator)
- `.planning/research/STACK.md` lines 72-125 -- Stack decisions (two-trait design, lazy combinators)
- `.planning/phases/75-numeric-traits/75-RESEARCH.md` -- Phase 75 research (pattern for associated type resolution, Output type lookup)
- `tests/e2e/assoc_type_basic.mpl`, `assoc_type_multiple.mpl` -- Associated type test fixtures (verify Self.Item syntax works)
- 19 existing for-in E2E tests in `crates/meshc/tests/e2e.rs` (lines 1159-1445) -- regression test safety net

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- all changes to existing crates verified against source code with exact line numbers; integration points fully mapped
- Architecture: HIGH -- dual-path design confirmed by ARCHITECTURE.md research, existing ForIn* patterns provide clear templates, all compiler passes traced end-to-end
- Pitfalls: HIGH -- 7 pitfalls identified from codebase analysis; 3 cross-referenced with domain PITFALLS.md (Pitfalls 2, 7, 10); Option tag encoding verified against runtime source
- Code examples: HIGH -- patterns derived from existing codebase conventions and Phase 74/75 patterns; backward compatibility examples match existing E2E test fixtures

**Research date:** 2026-02-13
**Valid until:** 2026-03-13 (stable -- compiler internals don't change externally)
