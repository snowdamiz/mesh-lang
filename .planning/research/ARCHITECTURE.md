# Architecture Patterns

**Domain:** Compiler trait ecosystem extension -- associated types, iterator protocol, From/Into, numeric traits, Collect
**Researched:** 2026-02-13
**Overall confidence:** HIGH (based on direct source analysis of all relevant compiler passes)

## Executive Summary

The Mesh compiler has a clean four-stage pipeline (Parser -> Typeck -> MIR -> Codegen) with an existing trait system that uses string-keyed `TraitDef`/`ImplDef` registrations, structural type matching via temporary unification for impl lookup, and `Trait__Method__Type` name mangling for static dispatch. The new features -- associated types, Iterator protocol, From/Into, numeric traits, and Collect -- each require changes at specific stages but share a common integration pattern: extend `TraitDef`/`TraitMethodSig` with associated type slots, add resolution logic in typeck, generate concrete MIR functions during lowering, and let existing codegen handle the monomorphized calls.

The critical architectural insight is that **associated types are the foundational feature**. Iterator, From/Into, and Collect all require associated types to express their signatures correctly (e.g., `Iterator` has `type Item`, `Collect` needs `type Output`). Without associated types, these traits would need hardcoded return types or lose type safety.

The second key insight is that **the existing for-in loop codegen already uses an indexed iteration pattern** (counter + `list_get` in a loop), which means the Iterator protocol can be implemented as a new MIR node (`ForInIterator`) that generates a different loop shape (`next()` + `Option::Some/None` matching) while reusing the same codegen patterns (alloca for state, basic block loop, conditional branch on `None`).

## Recommended Architecture

### High-Level Data Flow for Associated Types

```
Parser                    Typeck                     MIR                        Codegen
------                    ------                     ---                        -------
interface Iterator do     TraitDef {                 (no direct MIR repr --     (no change --
  type Item               name: "Iterator",          associated types are       operates on
  fn next(self)           assoc_types: ["Item"],     resolved to concrete       concrete
    -> Option<Self.Item>  methods: [...]             types during MIR lowering) MirTypes)
end                       }

impl Iterator for         ImplDef {                  Functions generated:
  Range do                trait_name: "Iterator",    Iterator__next__Range
  type Item = Int         impl_type: Range,          with concrete return
  fn next(self)           assoc_types: {             type Option_Int
    -> Option<Int>          "Item" -> Ty::Int
  ...                     },
end                       methods: { "next": ... }
                          }

val.next()                resolve_trait_method ->    MirExpr::Call {
                          resolves Self.Item to        func: "Iterator__next__Range"
                          concrete type via impl       args: [val]
                          lookup; unifies result       ty: SumType("Option_Int")
                          with Option<Int>           }
```

### Component Boundaries

| Component | Responsibility | Communicates With |
|-----------|---------------|-------------------|
| `mesh-parser` | Parse `type Item` in interface defs, `type Item = Ty` in impl blocks | Produces CST nodes for typeck |
| `mesh-typeck::traits` | Store associated types in `TraitDef`/`ImplDef`, resolve `Self.Item` during type checking | `InferCtx` for unification, `TraitRegistry` for lookups |
| `mesh-typeck::infer` | Resolve `Self.AssocType` projections during inference, enforce associated type constraints | `TraitRegistry` for impl lookup, `InferCtx` for unification |
| `mesh-typeck::builtins` | Register Iterator, From, Into, Neg, Collect trait definitions with associated types | `TraitRegistry`, `TypeEnv` |
| `mesh-codegen::mir::lower` | Resolve associated types to concrete types during MIR lowering, generate iterator loop code, generate From/Into wrapper functions | `TraitRegistry`, `TypeRegistry` |
| `mesh-codegen::mir::mod` | New MIR node `ForInIterator` (or extend existing ForInList pattern) | Consumed by codegen |
| `mesh-codegen::codegen::expr` | Generate LLVM IR for iterator-based loops, From/Into calls | Existing patterns suffice |

### New vs Modified Components

**New components (files/modules that do not exist yet):**

None required. All changes extend existing files. This is by design -- the new features integrate into existing structures rather than creating parallel systems.

**Modified components:**

| File | What Changes | Scope |
|------|-------------|-------|
| `mesh-parser/src/parser/items.rs` | Add `type Name = Type` parsing in interface/impl bodies | Small: ~40 lines |
| `mesh-parser/src/syntax_kind.rs` | Add `ASSOC_TYPE_DEF` syntax kind | Trivial |
| `mesh-parser/src/ast/item.rs` | Add `AssocTypeDef` AST node accessor | Small |
| `mesh-typeck/src/traits.rs` | Add `assoc_types` to `TraitDef`, `assoc_type_values` to `ImplDef`, add `resolve_assoc_type()` method | Medium: ~80 lines |
| `mesh-typeck/src/ty.rs` | Add `Ty::Projection` variant for associated type projections | Small: ~20 lines |
| `mesh-typeck/src/unify.rs` | Handle `Ty::Projection` in unification (resolve before unifying) | Small: ~30 lines |
| `mesh-typeck/src/infer.rs` | Handle `Self.Item` type projection during inference, validate assoc type constraints | Medium: ~100 lines |
| `mesh-typeck/src/builtins.rs` | Register Iterator, From, Into, Neg, Collect traits | Medium: ~200 lines |
| `mesh-codegen/src/mir/mod.rs` | Add `ForInIterator` MIR node | Small: ~30 lines |
| `mesh-codegen/src/mir/lower.rs` | Lower for-in with Iterator protocol, generate From/Into bridge functions, numeric operator fallback | Large: ~400 lines |
| `mesh-codegen/src/mir/mono.rs` | Handle `ForInIterator` in reachability scan | Trivial |
| `mesh-codegen/src/codegen/expr.rs` | Codegen for `ForInIterator` loop | Medium: ~150 lines |

## Detailed Integration Points Per Feature

### 1. Associated Types

**Parser changes:**

The interface body parser (`parse_interface_method` in `items.rs`) currently only expects `fn` keyword. Extend to also accept `type` keyword:

```
interface Iterator do
  type Item              # associated type declaration (no default)
  fn next(self) -> Option<Self.Item>
end
```

New syntax kind `ASSOC_TYPE_DEF` wrapping a `NAME` and optional `= TYPE` (for defaults in the future). In impl blocks, `type Item = Int` is parsed as an `ASSOC_TYPE_BINDING` within the `IMPL_DEF` node.

**Typeck changes:**

`TraitDef` gains:
```rust
pub struct TraitDef {
    pub name: String,
    pub methods: Vec<TraitMethodSig>,
    pub assoc_types: Vec<String>,  // NEW: ["Item"] for Iterator
}
```

`ImplDef` gains:
```rust
pub struct ImplDef {
    // ... existing fields ...
    pub assoc_type_values: FxHashMap<String, Ty>,  // NEW: {"Item" -> Ty::Int}
}
```

`TraitRegistry` gains:
```rust
impl TraitRegistry {
    /// Resolve an associated type for a concrete impl.
    /// Given trait "Iterator", type Range, assoc_name "Item" -> returns Ty::Int
    pub fn resolve_assoc_type(
        &self,
        trait_name: &str,
        impl_ty: &Ty,
        assoc_name: &str,
    ) -> Option<Ty> {
        let impl_def = self.find_impl(trait_name, impl_ty)?;
        impl_def.assoc_type_values.get(assoc_name).cloned()
    }
}
```

The critical new operation is **associated type projection resolution**. When the type checker encounters `Self.Item`:

1. Look up which trait is in scope (from the `impl` being processed or from where-clause context)
2. Find the impl for the concrete type
3. Return the `assoc_type_values["Item"]` from that impl
4. Unify with the expected type

**Recommended representation for projections:**

Add a new `Ty` variant rather than overloading `Ty::App`:

```rust
pub enum Ty {
    // ... existing variants ...
    /// Associated type projection: <Type as Trait>::AssocType
    /// During inference, resolved to concrete type via impl lookup.
    Projection {
        base_ty: Box<Ty>,
        trait_name: String,
        assoc_name: String,
    },
}
```

This is cleaner than encoding `Self.Item` as `Ty::App(Con("Self"), ...)` which would collide with struct field access semantics. The `Projection` variant is resolved (replaced with the concrete type) during type checking, so it never reaches MIR.

**Unification handling:** When `resolve()` encounters `Ty::Projection`, it attempts to resolve it to a concrete type via `TraitRegistry::resolve_assoc_type`. If the base type is still a variable (not yet resolved), the projection stays as-is until more information is available. This is a form of lazy projection resolution that integrates with the existing HM inference algorithm.

**MIR/Codegen changes:** None. Projections are fully resolved before MIR lowering. MIR only sees concrete types.

**Validation errors to add:**
- `MissingAssocType { trait_name, assoc_name, impl_ty }` -- impl block does not define required associated type
- `UnexpectedAssocType { trait_name, assoc_name }` -- impl block defines an associated type not in the trait
- `AssocTypeMismatch { expected, found, trait_name, assoc_name }` -- associated type value conflicts with usage

### 2. Iterator Protocol

**Trait definition (registered in builtins.rs):**

```rust
// Iterator trait with associated type Item
registry.register_trait(TraitDef {
    name: "Iterator".to_string(),
    assoc_types: vec!["Item".to_string()],
    methods: vec![TraitMethodSig {
        name: "next".to_string(),
        has_self: true,
        param_count: 0,
        return_type: None,  // Option<Self.Item> -- resolved per impl
        has_default_body: false,
    }],
});
```

**For-in loop integration:**

The existing `lower_for_in_expr` in `lower.rs` dispatches on collection type (List, Map, Set, Range). Add a new fallback branch:

```rust
fn lower_for_in_expr(&mut self, for_in: &ForInExpr) -> MirExpr {
    // Existing: check for DotDot range -> ForInRange
    // Existing: check for Map -> ForInMap
    // Existing: check for Set -> ForInSet
    // Existing: check for List -> ForInList
    // NEW: check if iterable type implements Iterator trait
    if self.trait_registry.has_impl("Iterator", &iterable_ty) {
        return self.lower_for_in_iterator(for_in, &iterable_ty);
    }
    // Existing: fallback to list
}
```

**New MIR node:**

```rust
/// For-in loop over any type implementing Iterator.
/// Desugared to repeated next() calls with Option tag checking.
ForInIterator {
    var: String,              // loop variable bound to each element
    iterator: Box<MirExpr>,   // the iterator expression
    filter: Option<Box<MirExpr>>,  // optional when clause
    body: Box<MirExpr>,       // loop body
    elem_ty: MirType,         // resolved Iterator::Item type
    body_ty: MirType,         // type of body expression
    next_fn: String,          // mangled name: "Iterator__next__TypeName"
    ty: MirType,              // result type (List of body results)
}
```

**Codegen pattern for ForInIterator:**

```
entry:
  %iter_alloca = alloca IteratorType
  store codegen(iterator_expr) -> %iter_alloca
  %result_list = call mesh_list_builder_new(0)
  br loop_header

loop_header:
  %iter = load %iter_alloca
  %next_result = call Iterator__next__Type(%iter)
  %tag = extractvalue %next_result, 0    // 0 = None, 1 = Some
  %is_some = icmp eq %tag, 1
  br %is_some, loop_body, loop_exit

loop_body:
  %elem = extractvalue %next_result, 1   // Some's payload
  store %elem -> %var_alloca
  [optional: codegen filter, br to loop_header if false]
  %body_val = codegen(body)
  call mesh_list_builder_push(%result_list, %body_val)
  br loop_header

loop_exit:
  %final_list = call mesh_list_builder_finish(%result_list)
```

This mirrors the existing `codegen_for_in_list` pattern (counter + list_get + branch) but replaces indexed access with `next()` + tag check.

**Iterator state management:**

The iterator is an opaque runtime handle (like List, Map, Set are today -- `MirType::Ptr`). Internal state is managed by C functions in `mesh-rt`. From Mesh's perspective it is a value passed to `next()`. The `next()` function returns a new iterator state implicitly (the runtime mutates the handle internally).

This is consistent with how all collection operations work in Mesh: `list_append(list, elem)` takes and returns a Ptr, with the runtime managing the underlying data structure.

### 3. From/Into Traits

**Trait definitions:**

```rust
// From -- convert from source type to target type
// impl From<SourceType> for TargetType
TraitDef {
    name: "From",
    assoc_types: vec![],
    methods: vec![TraitMethodSig {
        name: "from",
        has_self: false,  // static method (like Default)
        param_count: 1,   // takes the source value
        return_type: None, // Self -- resolved per impl
        has_default_body: false,
    }],
}

// Into -- convert self into target type
TraitDef {
    name: "Into",
    assoc_types: vec![],
    methods: vec![TraitMethodSig {
        name: "into",
        has_self: true,
        param_count: 0,
        return_type: None, // target type -- inferred from context
        has_default_body: false,
    }],
}
```

**Key design decision: From implies Into.**

When `impl From<String> for Int` is registered, the system automatically provides `impl Into<Int> for String`. This is done at `register_impl` level: when a `From` impl is registered, synthesize and register the corresponding `Into` impl.

**Static method dispatch for `from()`:**

`from()` is a static trait method (no `self`), identical in dispatch pattern to the existing `default()`. The call site looks like `Int.from(some_string)` which the lowerer already handles for static methods:

1. `lower_field_access` detects `Type.method_name` pattern
2. Checks if it is a known static trait method
3. Resolves to mangled name `From__from__Int`

**Into inference from call-site context:**

`into()` requires special handling because the target type must be inferred from context, similar to how `default()` works today. In `builtins.rs`, `into()` is registered as a polymorphic function:

```rust
// into() -> T, where T is inferred from context
let t_var = TyVar(99100);
let t = Ty::Var(t_var);
env.insert("into".into(), Scheme {
    vars: vec![t_var],
    ty: Ty::fun(vec![], t),  // no explicit params besides self
});
```

When `val.into()` is called and the expected return type is known (from a `let` annotation or function parameter context), the type variable `T` unifies with the expected type, and MIR lowering resolves to `Into__into__SourceType(val)`.

**MIR lowering:**

From/Into calls lower to mangled function calls:
- `Int.from(s)` -> `From__from__Int(s)`
- `s.into()` -> `Into__into__String(s)` (where target type is inferred)

### 4. Numeric Traits (Neg + User-Extensible Arithmetic)

**Current state:** Add, Sub, Mul, Div, Mod are already registered as compiler-known traits with impls for Int and Float in `builtins.rs`. Binary operator dispatch in `lower_binary_expr` already routes `+` to `Add__add__Type` via `has_impl` check.

**What changes:**

1. **User impls for arithmetic traits already work.** The existing binary operator dispatch checks `trait_registry.has_impl("Add", &ty)` and generates the mangled call. A user writing `impl Add for Vector` will have `+` dispatch to `Add__add__Vector`. The only gap is that `infer_binary` in `infer.rs` may hardcode numeric types for `+` -- this check needs to be relaxed to also accept any type with an `Add` impl.

2. **Add `Neg` trait** for unary minus:
```rust
TraitDef {
    name: "Neg",
    assoc_types: vec![],
    methods: vec![TraitMethodSig {
        name: "neg",
        has_self: true,
        param_count: 0,
        return_type: None, // Self
        has_default_body: false,
    }],
}
```

Register impls for Int and Float. Wire unary `-` operator to `Neg__neg__Type` dispatch in `lower_unary_expr`, mirroring how binary operators dispatch through trait impls.

3. **Numeric supertraits (deferred).** A `Num` supertrait requiring `Add + Sub + Mul + Div` would need supertrait resolution in the trait system. This can be deferred because where-clauses already support `where T: Add, T: Sub` which achieves the same constraint without new infrastructure.

### 5. Collect Trait

**Trait definition:**

```rust
TraitDef {
    name: "Collect",
    assoc_types: vec!["Output".to_string()],
    methods: vec![TraitMethodSig {
        name: "collect",
        has_self: true,
        param_count: 0,
        return_type: None, // Self.Output -- the collection type
        has_default_body: false,
    }],
}
```

**How it works:**

`collect()` is called on an iterator and produces a collection. The `Output` associated type determines what collection is produced:

```
impl Collect for RangeIterator do
  type Output = List<Int>
  fn collect(self) -> List<Int> do ... end
end
```

**MIR lowering:** `iter.collect()` becomes `Collect__collect__RangeIterator(iter)` -- a function that exhausts the iterator and builds the target collection.

**Initial scope:** Provide built-in Collect impls for Iterator -> List (the most common case). Collect to Map, Set can be added as follow-up work.

## Suggested Build Order

The features have a strict dependency chain:

```
1. Associated Types  (foundation -- everything else needs this)
   |
   +-> 2. Iterator Protocol  (needs assoc type Item)
   |     |
   |     +-> 5. Collect Trait  (needs Iterator + assoc type Output)
   |
   +-> 3. From/Into Traits  (benefits from assoc type resolution for Into)
   |
   +-> 4. Numeric Traits  (least dependent; mostly wiring existing infra)
```

**Phase 1: Associated Types**
- Parser: `type Name` in interface, `type Name = Type` in impl
- Typeck: `TraitDef.assoc_types`, `ImplDef.assoc_type_values`
- Typeck: `Ty::Projection` resolution during inference
- Validation: missing associated type errors, duplicate checks
- No MIR/codegen changes needed

**Phase 2: Iterator Protocol**
- Register Iterator trait in builtins (with assoc type Item)
- Add `ForInIterator` MIR node
- Lower for-in over Iterator types
- Codegen for ForInIterator (next() + tag-check loop)
- Built-in iterators for Range, List (replacing current indexed approach or as alternative)

**Phase 3: From/Into Traits**
- Register From/Into traits in builtins
- Auto-synthesize Into from From
- Wire static method dispatch (`Type.from(val)`)
- Into inference from call-site context

**Phase 4: Numeric Traits (Neg + user-extensible arithmetic)**
- Add Neg trait, register for Int/Float
- Wire unary minus to Neg dispatch
- Relax infer_binary to accept any type with matching trait impl
- Ensure user-defined Add/Sub/Mul/Div impls work end-to-end

**Phase 5: Collect Trait**
- Register Collect trait with associated type Output
- Provide Iterator -> List collect implementation
- Wire `iter.collect()` to trait dispatch

## Patterns to Follow

### Pattern 1: Trait Registration via Builtins

**What:** All compiler-known traits are registered in `builtins.rs::register_compiler_known_traits`. New traits follow this exact pattern.

**When:** Adding Iterator, From, Into, Neg, Collect.

**Example (from existing code in builtins.rs):**
```rust
registry.register_trait(TraitDef {
    name: "Add".to_string(),
    methods: vec![TraitMethodSig {
        name: "add".to_string(),
        has_self: true,
        param_count: 1,
        return_type: None,
        has_default_body: false,
    }],
});
for (ty, ty_name) in &[(Ty::int(), "Int"), (Ty::float(), "Float")] {
    let mut methods = FxHashMap::default();
    methods.insert("add".to_string(), ImplMethodSig { ... });
    let _ = registry.register_impl(ImplDef {
        trait_name: "Add".to_string(),
        impl_type: ty.clone(),
        impl_type_name: ty_name.to_string(),
        methods,
    });
}
```

### Pattern 2: Mangled Function Names for Static Dispatch

**What:** All trait method calls compile to `Trait__Method__Type` function names. No vtables. No dynamic dispatch.

**When:** Every trait method call in MIR.

**Example:**
```
x.next()        ->  Iterator__next__Range(x)
Int.from(s)     ->  From__from__Int(s)
v.add(w)        ->  Add__add__Vector(v, w)
iter.collect()  ->  Collect__collect__RangeIterator(iter)
```

### Pattern 3: Type-Driven For-In Dispatch

**What:** `lower_for_in_expr` checks the iterable's type and dispatches to the appropriate lowering function. Each collection type has its own MIR node and codegen.

**When:** Adding Iterator-based for-in loops.

**How it works (existing code in lower.rs:5959-5986):**
```rust
fn lower_for_in_expr(&mut self, for_in: &ForInExpr) -> MirExpr {
    // Check for DotDot range first
    if let Some(Expr::BinaryExpr(ref bin)) = for_in.iterable() {
        if bin.op() == Some(DOT_DOT) {
            return self.lower_for_in_range(for_in, bin);
        }
    }
    // Then check collection type via typeck results
    if let Some(ref ty) = iterable_ty {
        if extract_map_types(ty).is_some() { return self.lower_for_in_map(...); }
        if extract_set_elem_type(ty).is_some() { return self.lower_for_in_set(...); }
        if extract_list_elem_type(ty).is_some() { return self.lower_for_in_list(...); }
    }
    // NEW: fallback to Iterator protocol check
    self.lower_for_in_list(for_in, &Ty::int()) // current fallback
}
```

### Pattern 4: Derive-Like Auto-Generation at MIR Level

**What:** The `deriving()` clause generates trait impl functions during MIR lowering by synthesizing MIR expressions from struct/sum type metadata. No source-level code generation.

**When:** Auto-deriving Iterator for built-in types, Collect for common types.

**Existing pattern (from lower.rs:1665-1697):**
```rust
let has_deriving = struct_def.has_deriving_clause();
let derive_list = struct_def.deriving_traits();
let derive_all = !has_deriving;

if derive_all || derive_list.iter().any(|t| t == "Debug") {
    self.generate_debug_inspect_struct(&name, &fields);
}
if derive_all || derive_list.iter().any(|t| t == "Eq") {
    self.generate_eq_struct(&name, &fields);
}
```

### Pattern 5: Static Method Dispatch (like Default)

**What:** Static trait methods (no `self` parameter) are called via `Type.method()` syntax and dispatch to `Trait__method__Type`.

**When:** `from()` in From trait, `default()` in Default trait.

**Existing pattern (from lower.rs, builtins.rs):**

The `default()` function is registered as a polymorphic function in the type environment. At call sites, the return type is inferred from context (type annotation). MIR lowering resolves the concrete type and emits `Default__default__Int` (or whatever type was inferred). This exact pattern applies to `From::from()`.

## Anti-Patterns to Avoid

### Anti-Pattern 1: Encoding Associated Types as Type Parameters

**What:** Representing `Iterator<Item = Int>` as `Iterator<Int>` (a type application).

**Why bad:** Conflates associated types (determined by the impl) with generic parameters (determined by the caller). A type implementing Iterator has its Item type fixed by the impl, not chosen by the caller. Using `Ty::App(Con("Iterator"), [Con("Int")])` would require the type checker to track which positions are associated types vs parameters, creating complexity throughout unification.

**Instead:** Use a dedicated `Ty::Projection` variant for associated type references, and store associated type values in `ImplDef.assoc_type_values`. Projections are resolved eagerly during type checking so they never reach MIR.

### Anti-Pattern 2: Desugaring Iterator Loops to While+Match at MIR Level

**What:** Lowering `for x in iter do ... end` to a `MirExpr::While` containing a `MirExpr::Match` on `next()`.

**Why bad:** Forces the MIR match compilation (pattern -> decision tree) to handle the `Option<T>` tag check, adding unnecessary complexity. The codegen for while + match generates suboptimal LLVM IR compared to a direct tag-check + branch.

**Instead:** Use a dedicated `ForInIterator` MIR node that codegen compiles directly to a tag-check loop (analogous to how `ForInList` compiles directly to a counter loop).

### Anti-Pattern 3: Separate Dispatch Paths for Built-in vs User Traits

**What:** Having different code paths for built-in traits (Add, Eq) vs user-defined traits.

**Why bad:** The codebase already unified this -- `resolve_trait_callee` (lower.rs:5260) handles both via `find_method_traits`. Adding special cases for Iterator/From/Into would create maintenance burden and edge-case bugs.

**Instead:** Register new traits through the same `TraitRegistry` and let the unified dispatch path handle them. Only add special-case handling for genuinely unique behavior (e.g., `into()` needs call-site type inference, `from()` needs static method dispatch).

### Anti-Pattern 4: Making Iterator Stateful via Mutation of Self

**What:** Having `next()` mutate the iterator in place (like Rust's `&mut self`).

**Why bad:** Mesh has no `&mut self` -- all values are either immutable or GC-managed. Mutation-based iterators require either (a) adding mutable references (huge scope creep) or (b) breaking Mesh's value semantics.

**Instead:** The iterator is an opaque runtime handle (like List/Map/Set -- `MirType::Ptr`). Internal state is managed by C functions in `mesh-rt`. From Mesh's perspective it is a value passed to `next()`. The `next()` function internally advances the iterator state through the runtime handle. This is consistent with how all collection operations already work in Mesh.

## Data Flow Changes

### Type Checker Data Flow (modified)

```
BEFORE:
  infer_interface_def -> register TraitDef (methods only)
  infer_impl_def      -> register ImplDef (methods only)
  method call          -> find_method_traits -> mangled name

AFTER:
  infer_interface_def -> register TraitDef (methods + assoc_types)
  infer_impl_def      -> register ImplDef (methods + assoc_type_values)
  Self.AssocType ref   -> resolve via TraitRegistry::resolve_assoc_type -> concrete Ty
  method call          -> find_method_traits -> mangled name (unchanged)
  into() call          -> infer target type from context -> Into__into__SourceType
  from() call          -> resolve from Type.from() -> From__from__TargetType
```

### MIR Lowering Data Flow (modified)

```
BEFORE:
  for-in expr -> detect collection type -> ForInList/ForInRange/ForInMap/ForInSet

AFTER:
  for-in expr -> detect collection type -> ForInList/ForInRange/ForInMap/ForInSet
              -> check Iterator impl     -> ForInIterator (NEW)

  impl blocks     -> generate Trait__Method__Type functions (unchanged)
  From impl       -> also generate Into__into__SourceType wrapper (NEW)
  unary minus     -> check Neg impl -> Neg__neg__Type call (NEW, extends existing)
```

### Codegen Data Flow (modified)

```
BEFORE:
  ForInList  -> counter loop with list_get + list_builder
  ForInRange -> counter loop with start..end

AFTER:
  ForInIterator -> next() call + tag check loop + list_builder (NEW)
  All other paths unchanged
```

## Scalability Considerations

| Concern | Current State | With This Milestone | Future (100+ traits) |
|---------|---------------|--------------------|--------------------|
| Trait lookup | Linear scan of impls per trait | Same, but impls carry assoc_type_values | May need indexing by impl type |
| Method resolution | `find_method_traits` scans all traits | Same | May need reverse index: method_name -> [trait_name] |
| Monomorphization | Reachability pass in mono.rs | More MIR functions from Iterator/From/Into impls | Dead code elimination already handles this |
| Compile time | Fast (linear in program size) | Slightly more work in typeck for projection resolution | Only concerns with deep generic nesting (already has depth limit of 64) |
| Associated type resolution | N/A | O(1) per projection (hashmap lookup in ImplDef) | Scales well -- resolution is per-impl, not per-usage |

## Integration Checklist Per Compiler Pass

### Parser (mesh-parser)
- [ ] New syntax kind: `ASSOC_TYPE_DEF`
- [ ] `parse_interface_def` body: accept `type Name` in addition to `fn`
- [ ] `parse_impl_def` body: accept `type Name = Type` in addition to `fn`
- [ ] AST accessor: `InterfaceDef::assoc_types() -> impl Iterator<Item = AssocTypeDef>`
- [ ] AST accessor: `ImplDef::assoc_type_bindings() -> impl Iterator<Item = (Name, Type)>`

### Type Checker (mesh-typeck)
- [ ] `Ty::Projection { base_ty, trait_name, assoc_name }` variant in ty.rs
- [ ] `TraitDef.assoc_types: Vec<String>` in traits.rs
- [ ] `ImplDef.assoc_type_values: FxHashMap<String, Ty>` in traits.rs
- [ ] `TraitRegistry::resolve_assoc_type(trait, ty, assoc) -> Option<Ty>` in traits.rs
- [ ] Handle `Ty::Projection` in `InferCtx::resolve()` in unify.rs
- [ ] Handle `Ty::Projection` in `InferCtx::occurs_in()` in unify.rs
- [ ] Handle `Ty::Projection` in `InferCtx::unify()` in unify.rs
- [ ] Handle `Ty::Projection` in `freshen_type_params()` in traits.rs
- [ ] Validate: error if impl missing required associated types
- [ ] Register Iterator, From, Into, Neg, Collect in builtins.rs
- [ ] `into()` call-site type inference in infer.rs
- [ ] `from()` static method pattern in infer.rs

### MIR (mesh-codegen::mir)
- [ ] `MirExpr::ForInIterator` node in mod.rs
- [ ] `ForInIterator` in `MirExpr::ty()` match in mod.rs
- [ ] `lower_for_in_iterator()` in lower.rs
- [ ] From -> Into auto-synthesis during impl lowering in lower.rs
- [ ] Neg trait dispatch in `lower_unary_expr` in lower.rs
- [ ] `collect_function_refs` handles ForInIterator in mono.rs

### Codegen (mesh-codegen::codegen)
- [ ] `codegen_for_in_iterator()` in expr.rs
- [ ] Match arm in main `codegen_expr` dispatch for `ForInIterator`
- [ ] Iterator runtime support functions declared in intrinsics.rs (if needed)

### Runtime (mesh-rt)
- [ ] Iterator handle type (opaque struct with internal cursor/state)
- [ ] `mesh_range_iterator_new(start, end) -> IterPtr`
- [ ] `mesh_list_iterator_new(list) -> IterPtr`
- [ ] `mesh_iterator_next(iter) -> Option<Item>` (returns tagged union)

## Sources

- Direct source analysis: `mesh-typeck/src/traits.rs` -- trait registry, impl lookup, structural matching, duplicate detection
- Direct source analysis: `mesh-typeck/src/ty.rs` -- Ty enum variants, TyCon, Scheme, ena integration
- Direct source analysis: `mesh-typeck/src/infer.rs` -- HM inference engine, struct/sum type registries, where-clause enforcement
- Direct source analysis: `mesh-typeck/src/unify.rs` -- unification, occurs check, generalization, instantiation
- Direct source analysis: `mesh-typeck/src/builtins.rs` -- compiler-known trait registration (Add, Eq, Ord, Not, Display, Debug, Hash, Default)
- Direct source analysis: `mesh-codegen/src/mir/mod.rs` -- MirModule, MirFunction, MirExpr (all node types), MirType, ForInList/Range/Map/Set nodes
- Direct source analysis: `mesh-codegen/src/mir/lower.rs` -- Lowerer struct, trait dispatch via resolve_trait_callee, for-in lowering, impl method lowering, deriving system
- Direct source analysis: `mesh-codegen/src/mir/types.rs` -- Ty->MirType resolution, name mangling, mir_type_to_ty reverse mapping
- Direct source analysis: `mesh-codegen/src/mir/mono.rs` -- monomorphization/reachability pass, function reference collection
- Direct source analysis: `mesh-codegen/src/codegen/mod.rs` -- CodeGen struct, LLVM context, type caches
- Direct source analysis: `mesh-codegen/src/codegen/expr.rs` -- expression codegen, for-in codegen patterns
- Direct source analysis: `mesh-parser/src/parser/items.rs` -- interface and impl parsing grammar
- Confidence: HIGH -- all architectural claims based on direct reading of current source code
