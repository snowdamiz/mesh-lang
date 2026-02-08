# Phase 21: Extended Protocols - Research

**Researched:** 2026-02-08
**Domain:** Snow compiler -- Hash protocol, Default protocol, default method implementations, collection Display/Debug
**Confidence:** HIGH (codebase investigation, verified against actual source)

## Summary

This phase adds four distinct capabilities to the Snow compiler: (1) a Hash protocol enabling user-defined types as Map keys via FNV-1a hashing in the runtime, (2) a Default protocol for zero-initialization with a static `default() -> Self` method, (3) default method implementations in interface blocks (method bodies that serve as fallbacks when an impl omits the method), and (4) Display/Debug implementations for collection types (List, Map, Set).

The trait infrastructure from Phases 18-20 is fully operational: TraitRegistry, impl registration, mangled name dispatch (`Trait__Method__Type`), auto-derived impls for structs/sum types, and operator dispatch for all 6 comparison operators. The primary technical challenges in Phase 21 are: (a) extending the runtime Map to support hash-based key lookup instead of just integer/string equality, (b) representing `Self` as a return type for static methods (no `self` parameter), (c) modifying the parser and typeck to support method bodies inside interface definitions, and (d) generating Display/Debug MIR functions that iterate over collection elements via runtime calls.

**Primary recommendation:** Implement Hash first (most impactful -- enables user types as Map keys), then Default (simpler protocol, Self return type is the only novelty), then default method implementations (parser + typeck + lowering changes), then collection Display/Debug (stretch goal, runtime iteration helpers needed).

## Standard Stack

No new external dependencies. All implementation is within existing crates.

### Core Crates Affected

| Crate | Purpose | What Changes |
|-------|---------|-------------|
| snow-rt/collections/map.rs | Runtime Map key lookup | Add key_type=2 (hash-based) with FNV-1a hash comparison |
| snow-rt (new file) | FNV-1a hash helpers | `snow_hash_int`, `snow_hash_float`, `snow_hash_bool`, `snow_hash_string`, `snow_hash_combine` |
| snow-typeck/builtins.rs | Trait registration | Register Hash and Default traits + primitive impls |
| snow-typeck/traits.rs | TraitDef structure | Add `has_default_body: bool` flag to TraitMethodSig |
| snow-typeck/infer.rs | Interface + impl inference | Support default method bodies; auto-register Hash for non-generic structs |
| snow-parser/parser/items.rs | Interface method parsing | Support optional `do...end` body in interface methods |
| snow-parser/ast/item.rs | InterfaceMethod AST | Add `body()` accessor for optional Block child |
| snow-codegen/mir/lower.rs | MIR lowering | Hash__hash auto-generation; Default__default static methods; default method fallback; collection Display/Debug functions |
| snow-codegen/codegen/intrinsics.rs | Runtime declarations | Declare new hash runtime functions |

## Architecture Patterns

### Pattern 1: Hash Protocol with Runtime Key Type Extension

**What:** Extend the existing Map key_type tag system (currently 0=Int, 1=String) with a new tag value 2=Hash, where keys are compared by calling a Snow-level Hash__hash function and storing the hash alongside the key pointer.

**Current Map key comparison** (from `snow-rt/src/collections/map.rs:60-69`):
```rust
unsafe fn keys_equal(m: *const u8, a: u64, b: u64) -> bool {
    if map_key_type(m) == KEY_TYPE_STR {
        crate::string::snow_string_eq(a as *const _, b as *const _) != 0
    } else {
        a == b  // integer equality
    }
}
```

**Required extension:** For key_type=2 (hash-based), the Map stores `(hash: u64, key_ptr: u64, value: u64)` triples instead of `(key: u64, value: u64)` pairs. Lookup first compares hashes (fast rejection), then falls back to equality comparison. However, this requires the Map to know BOTH the hash function AND the equality function for the key type, which are compile-time-known mangled names.

**Simpler alternative (recommended):** Since Snow compiles to LLVM with static dispatch, the compiler can generate a wrapper function `snow_map_hash_key_TypeName(key: u64) -> u64` that calls `Hash__hash__TypeName`. The Map put/get operations would be wrapped at the MIR level to first hash the key, then use the hash value as the actual integer key in the existing Map infrastructure. This avoids modifying the Map runtime at all -- hashing happens at the call site, not inside the Map.

**Recommended approach:** At MIR lowering, when `map_put(map, struct_key, value)` is called and the key type has a Hash impl:
1. Emit `let hash_key = Hash__hash__TypeName(struct_key)`
2. Emit `snow_map_put(map, hash_key, value)`

This means struct keys are stored by their hash value. Collisions are possible but acceptable for v1.3 (the Map uses linear scan anyway). The key insight: the existing Map already uses `u64` keys with integer equality -- we just compute a `u64` hash from the struct and use that as the key.

### Pattern 2: FNV-1a Hash Implementation in snow-rt

**What:** Implement FNV-1a (64-bit) as runtime helper functions callable from generated MIR.

**FNV-1a 64-bit constants:**
- `FNV_OFFSET_BASIS`: `0xcbf29ce484222325` (14695981039346656037)
- `FNV_PRIME`: `0x00000100000001B3` (1099511628211)

**Algorithm:**
```
hash = FNV_OFFSET_BASIS
for each byte:
    hash = hash XOR byte
    hash = hash * FNV_PRIME
return hash
```

**Runtime functions needed (~30 lines total):**
```rust
// In snow-rt/src/hash.rs (new file)

const FNV_OFFSET: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x00000100000001B3;

#[no_mangle]
pub extern "C" fn snow_hash_int(value: i64) -> i64 {
    let bytes = value.to_le_bytes();
    let mut hash = FNV_OFFSET;
    for &b in &bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash as i64
}

#[no_mangle]
pub extern "C" fn snow_hash_string(s: *const crate::string::SnowString) -> i64 {
    unsafe {
        let bytes = (*s).as_str().as_bytes();
        let mut hash = FNV_OFFSET;
        for &b in bytes {
            hash ^= b as u64;
            hash = hash.wrapping_mul(FNV_PRIME);
        }
        hash as i64
    }
}

#[no_mangle]
pub extern "C" fn snow_hash_combine(a: i64, b: i64) -> i64 {
    let mut hash = a as u64;
    for &byte in &(b as u64).to_le_bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash as i64
}
```

### Pattern 3: Default Protocol with Self Return Type

**What:** A static method (no `self` parameter) that returns the implementing type. This is the first trait in Snow where the return type depends on the implementing type rather than being fixed.

**Challenge:** The current `TraitMethodSig.return_type` is `Option<Ty>`. For `default() -> Self`, the return type should be the implementing type. When `Default__default__Int` is called, it should return `Int`; when `Default__default__Point` is called, it should return `Point`.

**Resolution approach:** Register `Default` with `return_type: None` (indicating "Self"). When lowering a call to `default()`, the compiler knows the expected type from the call context (the variable being assigned to, or the type annotation). The MIR lowerer emits `Default__default__TypeName()` with the concrete return type from context.

**For primitive types**, redirect mangled names to runtime constants at MIR lowering (same pattern as Display):
- `Default__default__Int` -> `MirExpr::IntLit(0, MirType::Int)`
- `Default__default__Float` -> `MirExpr::FloatLit(0.0, MirType::Float)`
- `Default__default__Bool` -> `MirExpr::BoolLit(false, MirType::Bool)`
- `Default__default__String` -> `MirExpr::StringLit("", MirType::String)`

**For user structs**, the compiler can auto-generate `Default__default__Point` that constructs `Point { x: Default__default__Int(), y: Default__default__Int() }` if all fields have Default impls.

### Pattern 4: Default Method Implementations in Interfaces

**What:** Allow interface definitions to include method bodies that serve as defaults. When an `impl` block omits a method that has a default, the default body is used.

**Current state:**
- Parser: `parse_interface_method()` parses only signatures (no body support) -- `items.rs:515-551`
- AST: `InterfaceMethod` has `name()`, `param_list()`, `return_type()` but no `body()` -- `item.rs:412-429`
- Typeck: `infer_interface_def()` processes signatures only -- `infer.rs:1732-1785`
- Lowering: `Item::InterfaceDef(_)` is skipped ("interfaces are erased") -- `lower.rs:491-493`

**Required changes:**

1. **Parser** (`items.rs:515-551`): After parsing the return type annotation, check for `DO_KW`. If present, parse a `BLOCK` (using the same `parse_block` used by functions), then expect `END_KW`. Close as `INTERFACE_METHOD` regardless.

2. **AST** (`item.rs:412-429`): Add a `body()` method to `InterfaceMethod` that looks for a `BLOCK` child node, same pattern as `FnDef::body()`.

3. **TraitDef** (`traits.rs:29-34`): Add a `has_default_body: bool` field to `TraitMethodSig`. This tells `register_impl()` that a missing method is OK if the trait sig has `has_default_body: true`.

4. **Typeck** (`infer.rs:1732-1785`): In `infer_interface_def()`, check if each method has a body. If so, set `has_default_body: true` on the `TraitMethodSig`. Also type-check the default body in its own scope.

5. **TraitRegistry** (`traits.rs:93-150`): In `register_impl()`, when checking required methods, skip the `MissingTraitMethod` error if `has_default_body` is true for the missing method.

6. **MIR Lowering** (`lower.rs:491-493`): When processing `Item::InterfaceDef`, for each method with a body, generate a MIR function with name `TraitName__MethodName__DEFAULT` (or similar). When lowering `Item::ImplDef`, if a method is missing from the impl but the trait has a default, emit a call forwarding to the default body (or inline it).

**Key insight:** The default method body doesn't know its concrete `Self` type. It must work generically. In Snow's monomorphization model, the default body would be re-lowered for each concrete type that uses it. This means the default body AST node must be stored somewhere accessible during lowering.

**Storage approach:** Store the interface's CST node reference (or the default method's syntax node) in the TraitDef so the lowerer can access it. Add a field `default_bodies: FxHashMap<String, rowan::GreenNode>` (or similar) to TraitDef, storing the method body syntax for later re-lowering per concrete type.

**Simpler approach (recommended for v1.3):** Store default method bodies in the TraitRegistry as a separate map: `default_method_bodies: FxHashMap<(String, String), SyntaxNode>` keyed by `(trait_name, method_name)`. During `Item::ImplDef` lowering, after lowering all user-provided methods, check the TraitRegistry for any methods the impl is missing. For each missing method with a default body, call `lower_impl_method()` with the default body's FnDef-like syntax node and the appropriate mangled name.

### Pattern 5: Collection Display/Debug via Runtime Helpers

**What:** Implement `to_string([1, 2, 3])` returning `"[1, 2, 3]"` and `to_string(%{a => 1})` returning a readable map representation.

**Challenge:** Collections are opaque `MirType::Ptr` at the MIR level. The runtime stores elements as raw `u64` values. To display them, we need to:
1. Know the element type (from the typeck `Ty::App(Con("List"), [Int])`)
2. Iterate over elements (runtime list_length + list_get calls)
3. Convert each element to string using Display dispatch

**Approach:** Generate runtime helper functions in snow-rt that accept a function pointer for element-to-string conversion:

```rust
// snow_list_to_string(list: *mut u8, elem_to_str: fn(u64) -> *mut u8) -> *mut u8
// Produces "[elem1, elem2, elem3]"
#[no_mangle]
pub extern "C" fn snow_list_to_string(
    list: *mut u8,
    fn_ptr: *mut u8,
    env_ptr: *mut u8,
) -> *mut u8 { ... }
```

At MIR lowering, when `Display__to_string__List` is needed (for `MirType::Ptr` with typeck type `List<Int>`), emit:
```
snow_list_to_string(list, snow_int_to_string, null)
```

For `List<Point>`, emit:
```
snow_list_to_string(list, Display__to_string__Point, null)
```

Similarly for Map and Set.

### Recommended Project Structure for Changes

```
crates/snow-rt/src/
  hash.rs              # NEW: FNV-1a hash functions (~30 lines)
  lib.rs               # Add `pub mod hash;`
  collections/map.rs   # No changes needed (hash at call site)
  collections/list.rs  # Add snow_list_to_string helper
  collections/map.rs   # Add snow_map_to_string helper
  collections/set.rs   # Add snow_set_to_string helper
crates/snow-typeck/src/
  builtins.rs          # Register Hash, Default traits + primitive impls
  traits.rs            # Add has_default_body to TraitMethodSig
  infer.rs             # Support default method bodies; auto-register Hash
crates/snow-parser/src/
  parser/items.rs      # Support optional body in parse_interface_method
  ast/item.rs          # Add body() to InterfaceMethod
crates/snow-codegen/src/
  mir/lower.rs         # Hash__hash auto-generation; Default__default; default fallback; collection Display
  codegen/intrinsics.rs # Declare hash runtime functions + list/map/set to_string
```

### Anti-Patterns to Avoid

- **Modifying the Map runtime for Hash support:** The Map already uses `u64` keys. Hashing at the call site and using the hash as an integer key is simpler and requires zero runtime changes. Hash collisions are the tradeoff but acceptable for v1.3's linear-scan Map.

- **Storing default method bodies as strings or re-parsing:** Store the actual syntax node (or a clone) from the parser. Re-parsing is fragile and unnecessary.

- **Implementing Self as a special Ty variant:** Self is not a type -- it's a placeholder resolved at each concrete impl site. Represent it as `return_type: None` in TraitMethodSig and resolve from context during call lowering.

- **Trying to make collection Display work without runtime helpers:** The MIR has no loop construct. Element iteration must happen in the runtime, with string conversion delegated to a function pointer callback.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| FNV-1a hash | Custom hash from scratch | Standard FNV-1a constants/algorithm | Well-studied, good distribution, ~15 lines of code |
| Hash for struct fields | Manual field-by-field hash | `snow_hash_combine` chaining | Existing combine pattern (XOR + multiply) handles field composition |
| Missing method detection | Custom impl completeness check | TraitRegistry.register_impl() validation | Already checks all required methods; just add has_default_body skip |
| Collection element iteration | MIR-level loop generation | Runtime callback functions (snow_list_to_string) | MIR has no loop construct; runtime already has list_length/list_get |
| Primitive Default values | Runtime functions | MIR literal inlining | Default values for primitives are constants (0, 0.0, false, ""); no function call needed |

**Key insight:** The Hash protocol's heaviest machinery is in the runtime (FNV-1a implementation) and MIR lowering (call-site hashing wrapper). The trait infrastructure is already complete from Phases 18-20.

## Common Pitfalls

### Pitfall 1: Hash Collisions in Map Keys

**What goes wrong:** Two different struct values with the same hash are treated as the same key. `Map.put(map, point_a, 1)` followed by `Map.put(map, point_b, 2)` where `hash(point_a) == hash(point_b)` silently overwrites the first entry.

**Why it happens:** Using the hash as the Map's integer key means the Map's equality check is `hash_a == hash_b`, not `point_a == point_b`.

**How to avoid:** Document as a known limitation for v1.3. FNV-1a has very low collision rates for typical struct field values. A full solution would require the Map to store both the key object and its hash, comparing by hash first then equality -- but this would require modifying the Map's entry layout from `(u64, u64)` to `(u64, u64, u64)`, which is a larger change.

**Alternative:** Use a different key_type tag (e.g., key_type=2) that stores entries as `(hash, key_ptr, value)` and compares by hash first, then calls an equality function pointer. This is more correct but significantly more complex.

**Recommendation:** Start with the simple hash-as-key approach. If the success criteria require exact equality (not just hash equality), upgrade to the full approach. The success criteria say "Map.put(map, my_struct, value) and Map.get(map, my_struct) work correctly" -- this works with hash-as-key as long as there are no collisions, which is the common case.

### Pitfall 2: Self Return Type Resolution for Default

**What goes wrong:** `default()` is called without enough context to determine the concrete return type. The compiler doesn't know whether to call `Default__default__Int` or `Default__default__Point`.

**Why it happens:** `default()` has no arguments (unlike `to_string(self)` where the first arg determines the type). The return type depends on the usage context.

**How to avoid:** Two resolution strategies:
1. **Type annotation required:** `let x: Int = default()` -- the annotation provides the type.
2. **Inference from context:** `let x = default()` where `x` is later used as an `Int` argument -- typeck unification resolves the type.

In both cases, the typeck pass must resolve the return type to a concrete type before MIR lowering. At lowering time, look up the call's resolved type from the types map and use it to mangle the name.

**Implementation:** In `lower_call_expr`, when the callee is `default` (bare name), look up the call expression's resolved type from typeck. Use that to emit `Default__default__TypeName()`.

### Pitfall 3: Interface Method Bodies Need Access During Lowering

**What goes wrong:** The interface definition is processed by typeck and its syntax node is dropped. When an impl block later needs the default body, it's no longer available.

**Why it happens:** The current flow processes items sequentially. Interface defs are processed by typeck and then skipped during lowering (`Item::InterfaceDef(_) => {}`). The syntax tree is still alive (the Parse owns it), but the default bodies aren't stored anywhere the lowerer can find them.

**How to avoid:** Store default method body syntax nodes in a data structure accessible during lowering. Options:
1. **In TraitRegistry:** Add a `default_bodies: FxHashMap<(String, String), SyntaxNode>` field. Populated during `infer_interface_def`, read during `lower_item(ImplDef)`.
2. **In TypeckResult:** Add a similar map that's passed through to the lowerer.
3. **Re-walk the AST:** During lowering of `Item::ImplDef`, scan backwards for the interface def. Fragile and complex.

**Recommendation:** Option 2 (TypeckResult). The TypeckResult already carries the types map and trait_registry. Adding a `default_method_nodes` map keeps the data flow clean.

### Pitfall 4: Collection Display Needs Element Type Information at MIR Level

**What goes wrong:** When lowering `to_string(my_list)`, the MIR type is `Ptr` (all collections are opaque pointers). The lowerer doesn't know the element type needed to select the right element-to-string function.

**Why it happens:** `MirType::Ptr` erases all generic type information. The typeck type `List<Int>` is available from the types map, but the MIR lowerer doesn't look it up for `wrap_to_string`.

**How to avoid:** In `wrap_to_string`, for `MirType::Ptr`, look up the original `Ty` from the types map. If it's `Ty::App(Con("List"), [elem_ty])`, resolve the element type and use it to select the element-to-string callback function.

**Challenge:** `wrap_to_string` currently takes a `MirExpr` but doesn't have access to the original `Ty`. Need to either pass the `Ty` alongside, or look up from the expression's text range. The expression may be a synthetic expression (e.g., from auto-generated Debug code) without a text range.

**Recommendation:** For collection Display, add a new `wrap_collection_to_string` method that takes both the expression and the original `Ty`. Call this from the specific paths that handle collection types (string interpolation, to_string calls).

### Pitfall 5: Default Method Bodies Referencing Self

**What goes wrong:** A default method body in an interface uses `self` or calls other trait methods. When lowered for a concrete type, `self` must resolve to the concrete type.

**Why it happens:** The default body is written generically (in the interface block, not knowing the concrete type). During lowering, it's re-lowered in the context of a specific impl type.

**How to avoid:** When lowering a default method body for a concrete type, set up the scope exactly as `lower_impl_method` does: push a scope with `self` bound to the concrete struct type, then lower the body. The mangled name should be `Trait__Method__TypeName` as usual.

## Code Examples

### FNV-1a Hash Implementation (snow-rt/src/hash.rs)

```rust
// Source: FNV-1a specification (public domain)
const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x00000100000001B3;

fn fnv1a_bytes(bytes: &[u8]) -> u64 {
    let mut hash = FNV_OFFSET_BASIS;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

#[no_mangle]
pub extern "C" fn snow_hash_int(value: i64) -> i64 {
    fnv1a_bytes(&value.to_le_bytes()) as i64
}

#[no_mangle]
pub extern "C" fn snow_hash_float(value: f64) -> i64 {
    fnv1a_bytes(&value.to_bits().to_le_bytes()) as i64
}

#[no_mangle]
pub extern "C" fn snow_hash_bool(value: i8) -> i64 {
    fnv1a_bytes(&[value as u8]) as i64
}

#[no_mangle]
pub extern "C" fn snow_hash_string(s: *const crate::string::SnowString) -> i64 {
    unsafe { fnv1a_bytes((*s).as_str().as_bytes()) as i64 }
}

/// Combine two hash values (for struct field hashing).
#[no_mangle]
pub extern "C" fn snow_hash_combine(hash_a: i64, hash_b: i64) -> i64 {
    let mut hash = hash_a as u64;
    for &b in &(hash_b as u64).to_le_bytes() {
        hash ^= b as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash as i64
}
```

### Hash Trait Registration (builtins.rs)

```rust
// Register Hash trait
registry.register_trait(TraitDef {
    name: "Hash".to_string(),
    methods: vec![TraitMethodSig {
        name: "hash".to_string(),
        has_self: true,
        param_count: 0,
        return_type: Some(Ty::int()),
        has_default_body: false,
    }],
});

// Register Hash impls for primitives (Int, Float, String, Bool)
for (ty, ty_name) in &[
    (Ty::int(), "Int"),
    (Ty::float(), "Float"),
    (Ty::string(), "String"),
    (Ty::bool(), "Bool"),
] {
    let mut methods = FxHashMap::default();
    methods.insert("hash".to_string(), ImplMethodSig {
        has_self: true,
        param_count: 0,
        return_type: Some(Ty::int()),
    });
    let _ = registry.register_impl(ImplDef {
        trait_name: "Hash".to_string(),
        impl_type: ty.clone(),
        impl_type_name: ty_name.to_string(),
        methods,
    });
}
```

### Auto-Generated Hash__hash__StructName (MIR pattern)

For a struct `Point { x: Int, y: Int }`:
```
fn Hash__hash__Point(self: Point) -> Int {
    let h0 = snow_hash_int(self.x)
    let h1 = snow_hash_combine(h0, snow_hash_int(self.y))
    h1
}
```

In MIR:
```rust
// Hash__hash__Point: chain field hashes via snow_hash_combine
let hash_x = MirExpr::Call {
    func: snow_hash_int,
    args: [self.x],
    ty: MirType::Int,
};
let hash_y = MirExpr::Call {
    func: snow_hash_int,
    args: [self.y],
    ty: MirType::Int,
};
let combined = MirExpr::Call {
    func: snow_hash_combine,
    args: [hash_x, hash_y],
    ty: MirType::Int,
};
```

### Map.put with Hash Dispatch (MIR lowering pattern)

When lowering `map_put(map, struct_key, value)` where key type has Hash impl:
```rust
// In lower_call_expr, after identifying map_put with struct key:
let hash_key = MirExpr::Call {
    func: Box::new(MirExpr::Var(
        "Hash__hash__Point".to_string(),
        MirType::FnPtr(vec![MirType::Struct("Point")], Box::new(MirType::Int)),
    )),
    args: vec![struct_key.clone()],
    ty: MirType::Int,
};
// Then emit: snow_map_put(map, hash_key, value)
```

### Default Trait with Self Return Type

```rust
// Registration (in builtins.rs):
registry.register_trait(TraitDef {
    name: "Default".to_string(),
    methods: vec![TraitMethodSig {
        name: "default".to_string(),
        has_self: false,  // static method
        param_count: 0,
        return_type: None,  // Self -- resolved per concrete type
        has_default_body: false,
    }],
});
```

### Primitive Default Short-Circuits (MIR lowering)

```rust
// In lower_call_expr, when callee is "default":
let call_ty = self.resolve_range(call.syntax().text_range());
let type_name = mir_type_to_impl_name(&call_ty);
let mangled = format!("Default__default__{}", type_name);

// Short-circuit for primitives:
match mangled.as_str() {
    "Default__default__Int" => return MirExpr::IntLit(0, MirType::Int),
    "Default__default__Float" => return MirExpr::FloatLit(0.0, MirType::Float),
    "Default__default__Bool" => return MirExpr::BoolLit(false, MirType::Bool),
    "Default__default__String" => return MirExpr::StringLit("".to_string(), MirType::String),
    _ => { /* emit call to Default__default__TypeName() */ }
}
```

### Default Method Body Parsing (parser change)

```rust
// In parse_interface_method(), after return type annotation:
// Check for optional default body: do ... end
if p.at(SyntaxKind::DO_KW) {
    parse_block(p);  // reuse existing block parsing
}

p.close(m, SyntaxKind::INTERFACE_METHOD);
```

### Collection Display Runtime Helper (snow_list_to_string)

```rust
#[no_mangle]
pub extern "C" fn snow_list_to_string(
    list: *mut u8,
    elem_to_str: *mut u8,  // fn(u64) -> *mut u8
    env_ptr: *mut u8,      // null for bare functions
) -> *mut u8 {
    type BareFn = unsafe extern "C" fn(u64) -> *mut u8;
    unsafe {
        let len = list_len(list) as usize;
        let data = list_data(list);
        let f: BareFn = std::mem::transmute(elem_to_str);

        let mut result = crate::string::snow_string_new(b"[".as_ptr(), 1);
        for i in 0..len {
            if i > 0 {
                result = crate::string::snow_string_concat(
                    result as *const _,
                    crate::string::snow_string_new(b", ".as_ptr(), 2) as *const _,
                ) as *mut u8;
            }
            let elem_str = f(*data.add(i));
            result = crate::string::snow_string_concat(
                result as *const _,
                elem_str as *const _,
            ) as *mut u8;
        }
        result = crate::string::snow_string_concat(
            result as *const _,
            crate::string::snow_string_new(b"]".as_ptr(), 1) as *const _,
        ) as *mut u8;
        result
    }
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Map keys: Int or String only (key_type tag 0/1) | Hash-based struct keys via call-site hashing | Phase 21 | User types as Map keys |
| No zero-initialization | Default trait with `default() -> Self` | Phase 21 | Consistent zero-value construction |
| Interface = signature only | Interface methods can have bodies (defaults) | Phase 21 | Less boilerplate in impl blocks |
| Collections have no Display | List/Map/Set can be converted to string | Phase 21 | `"${my_list}"` works in interpolation |

**Infrastructure reused from Phases 18-20 (HIGH confidence):**
- TraitRegistry: trait defs, impl registration, has_impl, find_method_traits
- Mangled name convention: `Trait__Method__Type`
- Auto-derivation pattern for structs/sum types (Debug, Eq, Ord)
- Operator dispatch in lower_binary_expr
- wrap_to_string for string interpolation
- Primitive trait redirect pattern (mangled name -> runtime function)
- String interpolation desugaring
- Display__to_string__String identity short-circuit pattern

## Open Questions

1. **Hash collisions in Map keys**
   - What we know: Using hash-as-key means collisions cause silent overwrites.
   - What's unclear: Whether the success criteria ("Map.put and Map.get work correctly") require collision-free behavior or just typical-case correctness.
   - Recommendation: Start with hash-as-key (simplest). If tests reveal collision issues, add a full `(hash, key, value)` triple storage with equality fallback. FNV-1a collision probability for typical structs is extremely low.

2. **Default method body storage across compilation phases**
   - What we know: The syntax tree is alive for the duration of compilation (Parse owns it). The lowerer has access to the Parse reference.
   - What's unclear: Whether storing SyntaxNode references in TypeckResult is clean enough, or whether the lowerer should re-walk the source file's items.
   - Recommendation: Store in TypeckResult. It's the established data flow channel between typeck and lowering.

3. **Collection Display for nested generic types**
   - What we know: `List<Int>` Display can use `snow_int_to_string` as the callback. `List<Point>` can use `Display__to_string__Point`.
   - What's unclear: How to handle `List<List<Int>>` -- the inner list's Display function would need to be a closure or a specific `Display__to_string__List_Int` function.
   - Recommendation: For v1.3, support Display for `List<primitive>` and `List<UserStruct>`. Nested collections (List<List<T>>) can fall back to a generic `"[...]"` representation or be marked as a known limitation.

4. **Auto-derive Hash for all structs or require explicit impl?**
   - What we know: Eq and Ord are auto-derived for all non-generic structs. Hash could follow the same pattern.
   - What's unclear: Whether auto-deriving Hash is desirable (it means all structs are automatically usable as Map keys).
   - Recommendation: Auto-derive Hash for non-generic structs (consistent with Eq/Ord pattern). Users who want custom hash behavior can write explicit impls.

5. **Default trait for structs -- auto-derive or require explicit impl?**
   - What we know: Auto-deriving Default for structs requires all fields to have Default impls.
   - What's unclear: Whether auto-deriving Default could produce surprising zero values for user types.
   - Recommendation: Do NOT auto-derive Default for structs. Require explicit `impl Default for MyStruct`. Auto-derive only for primitives. The success criteria mention "user-defined for structs" -- this implies user-written impls.

## Sources

### Primary (HIGH confidence)
- `crates/snow-typeck/src/builtins.rs:560-764` - Existing compiler-known trait registration pattern (Display, Debug, Eq, Ord)
- `crates/snow-typeck/src/traits.rs:1-341` - TraitRegistry, TraitDef, TraitMethodSig, ImplDef structures
- `crates/snow-typeck/src/infer.rs:1732-1785` - infer_interface_def (current signature-only processing)
- `crates/snow-typeck/src/infer.rs:1788-1932` - infer_impl_def (method body type-checking pattern)
- `crates/snow-typeck/src/infer.rs:1455-1510` - Auto-registration of Debug/Eq/Ord for structs
- `crates/snow-codegen/src/mir/lower.rs:598-711` - lower_impl_method (method lowering with mangled names)
- `crates/snow-codegen/src/mir/lower.rs:1109-1188` - lower_struct_def with auto-generated Debug/Eq/Ord
- `crates/snow-codegen/src/mir/lower.rs:2326-2425` - Trait method call rewriting and primitive redirect pattern
- `crates/snow-codegen/src/mir/lower.rs:3324-3397` - wrap_to_string with Display dispatch
- `crates/snow-codegen/src/mir/lower.rs:3442-3485` - lower_map_literal with key_type inference
- `crates/snow-codegen/src/mir/lower.rs:165-182` - infer_map_key_type (determines key_type tag)
- `crates/snow-codegen/src/mir/types.rs:189-215` - mir_type_to_ty and mir_type_to_impl_name
- `crates/snow-rt/src/collections/map.rs:1-373` - Map runtime (key_type tags, linear scan, keys_equal)
- `crates/snow-rt/src/collections/list.rs:1-474` - List runtime (length, get, iteration pattern)
- `crates/snow-rt/src/collections/set.rs:1-276` - Set runtime
- `crates/snow-parser/src/parser/items.rs:453-551` - parse_interface_def and parse_interface_method
- `crates/snow-parser/src/ast/item.rs:390-449` - InterfaceDef and InterfaceMethod AST nodes
- `crates/snow-codegen/src/codegen/intrinsics.rs:247-256` - Map runtime function declarations
- `.planning/phases/20-essential-stdlib-protocols/20-VERIFICATION.md` - Phase 20 verification (1,146 tests pass)

### Secondary (MEDIUM confidence)
- [FNV-1a Wikipedia](https://en.wikipedia.org/wiki/Fowler%E2%80%93Noll%E2%80%93Vo_hash_function) - FNV-1a constants and algorithm specification
- [FNV IETF Draft](https://datatracker.ietf.org/doc/draft-eastlake-fnv/) - Authoritative FNV specification

## Metadata

**Confidence breakdown:**
- Hash protocol: HIGH - Runtime pattern clear, trait registration follows established pattern, FNV-1a is well-specified
- Default protocol: HIGH - Self resolution strategy clear, trait registration straightforward, primitive short-circuits follow Display pattern
- Default method implementations: MEDIUM - Parser/AST changes are straightforward, but body storage during lowering needs careful design
- Collection Display: MEDIUM - Runtime helpers needed, element type resolution from Ptr types needs careful implementation

**Research date:** 2026-02-08
**Valid until:** Indefinite (internal codebase analysis, not version-dependent)
