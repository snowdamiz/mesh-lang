# Architecture Research: Trait System & Monomorphization

**Domain:** Compiler pipeline modifications for user-defined traits with static dispatch
**Researched:** 2026-02-07
**Confidence:** HIGH (based on direct codebase analysis + established compiler design patterns)

---

## Current Pipeline (Baseline)

```
Source
  |
  v
[snow-lexer]     tokens with spans
  |
  v
[snow-parser]    rowan CST -> AST
  |               - InterfaceDef, ImplDef already parsed
  |               - InterfaceMethod with params, return type
  |
  v
[snow-typeck]    Hindley-Milner inference (Algorithm J)
  |               - TraitRegistry: trait defs + impl registrations
  |               - where-clause constraint checking at call sites
  |               - infer_interface_def: registers TraitDef
  |               - infer_impl_def: type-checks methods, registers ImplDef
  |               - Methods registered as callable: env.insert(method_name, fn_ty)
  |               - Compiler-known traits: Add, Sub, Mul, Div, Mod, Eq, Ord, Not
  |
  v
[snow-codegen::mir::lower]   AST -> MIR
  |               - Item::ImplDef: lowers methods as standalone functions
  |               - Item::InterfaceDef: SKIPPED ("interfaces are erased")
  |               - Ty -> MirType via resolve_type() (all types concrete)
  |               - Closures lifted, pipes desugared, strings interpolated
  |
  v
[snow-codegen::mir::mono]    Monomorphization pass
  |               - Currently: reachability analysis only (dead code elimination)
  |               - No generic function specialization yet
  |               - Comment: "In future: creates specialized copies"
  |
  v
[snow-codegen::codegen]      MIR -> LLVM IR via Inkwell
  |               - Direct function calls: MirExpr::Call
  |               - Closure calls: MirExpr::ClosureCall
  |               - No trait-aware dispatch
  |
  v
[linker]         object + libsnow_rt.a -> native binary
```

### What Already Works

1. **Parser:** `interface` and `impl` syntax fully parsed. InterfaceMethod nodes carry params, return types, self parameter detection.

2. **TraitRegistry (snow-typeck/traits.rs):** Complete data model for trait defs, impl registrations, method signature validation, where-clause constraint checking. Keys impls by `(trait_name, type_key)` where `type_key = type_to_key(ty)`.

3. **Type inference for impl methods:** `infer_impl_def` pushes scope with `self` bound to impl type, infers method body, unifies return type, registers method as callable function in env.

4. **Where-clause enforcement:** At call sites, `infer_call` resolves type args from argument types, maps them back to type parameter names via `param_type_param_names`, and calls `trait_registry.check_where_constraints`.

5. **MIR lowering of impl methods:** `Item::ImplDef` lowers each method via `lower_fn_def` -- methods become standalone MIR functions.

6. **Name mangling infrastructure:** `mangle_type_name` in mir/types.rs produces names like `Option_Int`, `Result_Int_String`.

### What Is Missing (The Gap)

1. **Trait method dispatch in MIR lowering:** When `to_string(42)` is called, the call resolves to the function name `to_string` -- but there is no mechanism to disambiguate which `to_string` impl to call when multiple types implement the same trait. The method is registered in env as a flat function name, so only one impl per method name can exist.

2. **Method name mangling for trait impls:** Impl methods are lowered with their bare names. `impl Display for Int do def to_string(self) ... end end` produces a MIR function named `to_string`, not `Display_Int_to_string` or `to_string__Int`.

3. **Monomorphization of generic functions:** The mono.rs pass only does reachability analysis. Generic functions that are called with different type arguments do not get specialized copies. The type checker resolves concrete types at each call site (HM produces concrete types after unification), but MIR lowering does not create separate function instances.

4. **Codegen awareness of trait methods:** Codegen has no concept of trait-dispatched calls. All calls are either direct (`MirExpr::Call`) or through closures (`MirExpr::ClosureCall`).

5. **Default method implementations:** InterfaceDef parsing supports method signatures but not method bodies. No mechanism for default implementations that impls can inherit.

6. **Stdlib protocol definitions:** No trait definitions exist for Display, Iterator, From/Into, Hash, Default, Serialize/Deserialize beyond the compiler-known operator traits.

---

## Changes Per Compiler Stage

### Stage 1: Parser (snow-parser) -- Minimal Changes

**Status:** Mostly complete. The parser already handles `interface` and `impl` syntax.

**Required changes:**

| Change | Rationale | Complexity |
|--------|-----------|------------|
| Default method bodies in interfaces | `interface Display do def to_string(self) :: String ... end end` needs body parsing in InterfaceMethod | Low |
| Associated type syntax (future) | Not needed for v1.3; defer | N/A |

**Key insight:** InterfaceMethod currently only has `name`, `param_list`, `return_type`. Adding an optional `body()` accessor mirrors FnDef's structure. The CST node type INTERFACE_METHOD may already support child BLOCK nodes -- needs verification.

**Build order:** Do this first if default methods are needed, otherwise skip entirely.

### Stage 2: Type Checker (snow-typeck) -- Moderate Changes

**Status:** TraitRegistry and where-clause infrastructure exist. Primary gap is method name disambiguation and richer type information flowing to MIR.

**Required changes:**

| Change | Rationale | Complexity |
|--------|-----------|------------|
| Mangled method names in env | Register impl methods as `TraitName_TypeName_method` instead of bare `method` | Medium |
| Trait method resolution in calls | When calling `to_string(x)`, resolve via TraitRegistry based on x's type | Medium |
| Pass trait/impl info to TypeckResult | Codegen needs to know trait defs and impl registrations | Low |
| Default method fallback | If impl does not provide a method, use the interface's default body | Medium |
| Self type substitution | Properly handle `Self` as a type parameter in trait method signatures | Low |

**Detailed design for method resolution:**

The current flow is:
```
infer_impl_def -> env.insert("to_string", Scheme::mono(fn_ty))
infer_call -> looks up "to_string" in env -> finds the function
```

The problem: if both `impl Display for Int` and `impl Display for String` define `to_string`, the second registration overwrites the first in the flat env.

**Solution: Qualified method names.** Register methods with mangled names that encode both the impl type and method name:

```
impl Display for Int:
  env.insert("Display__Int__to_string", scheme)

impl Display for String:
  env.insert("Display__String__to_string", scheme)
```

At call sites, when the type checker sees `to_string(x)` and resolves `x: Int`, it:
1. Checks if `to_string` is in env (backward compat for non-trait functions)
2. If not found, searches TraitRegistry for a trait that has method `to_string`
3. Uses `resolve_trait_method` with the resolved arg type to find the correct impl
4. Rewrites the call to the mangled name `Display__Int__to_string`

**Alternative (simpler):** Since Snow uses static dispatch, the type checker already knows concrete types at every call site. The type information flows through `types: FxHashMap<TextRange, Ty>` to MIR lowering. Instead of rewriting in typeck, let MIR lowering handle the name mangling by looking at the concrete argument types.

**Recommendation:** Use the MIR lowering approach. Keep typeck changes minimal -- just ensure trait/impl information is available in `TypeckResult`, and let MIR lowering resolve the mangled function names. This is simpler and keeps the typeck focused on type correctness rather than name generation.

**New fields in TypeckResult:**

```rust
pub struct TypeckResult {
    pub types: FxHashMap<TextRange, Ty>,
    pub errors: Vec<TypeError>,
    pub warnings: Vec<TypeError>,
    pub result_type: Option<Ty>,
    pub type_registry: TypeRegistry,
    // NEW: trait information for codegen
    pub trait_registry: TraitRegistry,  // trait defs + impl registrations
}
```

### Stage 3: MIR Lowering (snow-codegen/mir/lower.rs) -- Major Changes

**Status:** This is the primary integration point. MIR lowering transforms the typed AST into concrete, monomorphized MIR.

**Required changes:**

| Change | Rationale | Complexity |
|--------|-----------|------------|
| Mangled names for impl methods | `impl Display for Int do def to_string(self)` -> MirFunction name `Display__Int__to_string` | Medium |
| Trait method call resolution | `to_string(x)` with `x: Int` -> `Call { func: "Display__Int__to_string", args: [x] }` | High |
| Self parameter lowering | `self` in impl methods -> first parameter with concrete type | Low |
| Generic function specialization | `def identity<T>(x :: T) :: T` called with Int and String -> two MirFunctions | High |
| Where-clause-guided dispatch | At generic call sites, use resolved types to select correct impl | Medium |

**Detailed design for method call resolution in MIR lowering:**

```
Input AST: CallExpr("to_string", [NameRef("x")])
Types map: CallExpr range -> String, NameRef("x") range -> Int

MIR lowering steps:
1. lower_call_expr encounters "to_string" with arg x
2. Resolve x's type from types map: Int
3. Check if "to_string" is a known function -> NO (it is only in trait impls)
4. Search trait_registry: find that Display trait has method "to_string"
5. Check if Display has impl for Int: trait_registry.has_impl("Display", &Ty::int()) -> YES
6. Generate mangled name: "Display__Int__to_string"
7. Emit: MirExpr::Call { func: Var("Display__Int__to_string"), args: [lower(x)] }
```

**The MIR lowering context needs access to TraitRegistry.** Currently `LowerCtx` has `types`, `registry` (TypeRegistry), and `known_functions`. Add `trait_registry: &TraitRegistry`.

**For generic function specialization:**

The current mono.rs is a reachability pass. True monomorphization requires:

1. During MIR lowering, when a generic function is called with concrete types, generate a specialized MirFunction with a mangled name.
2. The type checker already resolves concrete types at each call site (HM unification produces concrete types from type variables).
3. MIR lowering can use the `types` map to determine the concrete types at each call site.

**However**, the current architecture already handles this implicitly because:
- The type checker resolves all types to concrete types before MIR lowering
- `resolve_type()` converts `Ty` to `MirType`, and `Ty::Var` falls back to `MirType::Unit`
- Functions are not actually generic in MIR -- they are always lowered with whatever types the checker resolved

The gap is: if the **same function** is called with **different concrete types**, only one MIR function is generated (with the types from one call site). This needs to be addressed for true generic function support.

**Recommendation for v1.3:** Since the primary goal is trait methods (not arbitrary generic functions), focus on:
1. Impl method name mangling (each impl produces a uniquely-named MirFunction)
2. Call-site resolution (map bare method names to mangled impl function names)
3. Defer full generic function monomorphization to a future version unless specific stdlib protocols require it

### Stage 4: Monomorphization Pass (snow-codegen/mir/mono.rs) -- Moderate Changes

**Status:** Currently a reachability pass. Needs to become a true monomorphization pass if generic functions must be specialized.

**Required changes for v1.3:**

| Change | Rationale | Complexity |
|--------|-----------|------------|
| Track impl method functions | Ensure mangled impl method names are considered reachable | Low |
| Generic function specialization (if needed) | Clone and specialize generic MirFunctions for each concrete type | High |

**For trait methods specifically:** If MIR lowering produces correctly-named impl method functions and call sites reference those names, the existing reachability pass already handles this correctly. The mangled function names (`Display__Int__to_string`) will be discovered as reachable through normal call graph analysis.

**For generic function specialization:** This is the harder case. If `def identity<T>(x :: T) :: T` is called as `identity(42)` and `identity("hello")`, MIR lowering currently produces one function. The monomorphization pass would need to:

1. Detect that `identity` is generic (has type parameters in its definition)
2. Find all call sites and their concrete type arguments
3. Clone the MIR function body, substituting types
4. Rename to `identity__Int`, `identity__String`
5. Rewrite call sites to reference specialized names

**Recommendation:** For v1.3, most stdlib protocols (Display, Default, Hash) do not require generic function specialization -- they operate on concrete types. Iterator and From/Into may need it. Implement basic specialization for these cases.

### Stage 5: Codegen (snow-codegen/codegen/) -- Minimal Changes

**Status:** Codegen translates MIR to LLVM IR. If MIR correctly represents trait method calls as direct function calls with mangled names, codegen needs no trait-specific changes.

**Required changes:**

| Change | Rationale | Complexity |
|--------|-----------|------------|
| None for basic trait dispatch | Mangled names resolve to concrete LLVM functions | None |
| String formatting runtime functions | `Display.to_string` impls for primitives need runtime support | Low |
| Iterator runtime support | `Iterator.next` for collections needs runtime functions | Medium |

**Key insight:** Static dispatch via monomorphization means trait method calls become ordinary direct function calls by the time they reach codegen. The `MirExpr::Call { func: Var("Display__Int__to_string"), args: [...] }` is no different from any other function call. Codegen is already equipped to handle this.

---

## Monomorphization Strategy

### When Monomorphization Happens

```
                    Typeck                 MIR Lowering              Mono Pass
                    ------                 -----------               ---------
Generic fn def  ->  Scheme with vars   ->  [deferred]            ->  [not reached]
Call site 1     ->  concrete types     ->  mangled call          ->  reachability
Call site 2     ->  concrete types     ->  mangled call          ->  reachability
Impl method     ->  validated body     ->  mangled MirFunction   ->  reachability
```

### Strategy: Eager Resolution at MIR Lowering

Rather than deferring monomorphization to a separate pass, resolve concrete types eagerly during MIR lowering. This is natural because:

1. **HM inference produces concrete types.** After unification, every expression has a concrete type (no remaining type variables in a well-typed program).
2. **The types map is available.** `types: FxHashMap<TextRange, Ty>` maps every AST node to its resolved type.
3. **MIR is already monomorphic.** The MirType enum has no type variable variant. Everything is concrete.

The existing pipeline already does eager resolution for everything except:
- Multiple impls of the same trait method (name collision)
- Multiple instantiations of generic functions (single function produced)

### Name Mangling Convention

Proposed convention for trait impl methods:

```
{TraitName}__{TypeName}__{method_name}

Examples:
  Display__Int__to_string
  Display__String__to_string
  Display__Point__to_string
  Add__Vec2__add
  Iterator__Range__next
  From__String__Int__from    (From<String> for Int)
  Hash__Int__hash
  Default__Config__default
```

For generic function specialization (if needed):

```
{function_name}__{TypeArgs}

Examples:
  identity__Int
  identity__String
  map__Int__String        (map<A,B> specialized for A=Int, B=String)
```

This aligns with the existing `mangle_type_name` function in mir/types.rs which produces `Option_Int`, `Result_Int_String` for type names.

### Interaction with HM Inference

The key insight: **HM inference and monomorphization are complementary, not conflicting.**

1. HM inference operates on polymorphic `Scheme` types during type checking. `identity : forall a. a -> a` is a polymorphic scheme.
2. At each call site, `instantiate` creates fresh type variables, and unification resolves them to concrete types. `identity(42)` produces `identity : Int -> Int`.
3. The types map records the concrete types at each call site.
4. MIR lowering reads the types map and produces concrete MIR.
5. If the same generic function is called with different types, MIR lowering must produce separate specialized functions (or the mono pass must clone and specialize).

**Snow's current approach already handles step 1-4.** Step 5 is the gap for generic functions, but NOT for trait methods (which are already separate functions per impl).

### What Does NOT Need Monomorphization

Trait methods do not need generic function specialization because each `impl Trait for ConcreteType` already produces a separate, concrete function. The method body is type-checked with `self: ConcreteType`, and all types in the body are concrete.

What needs monomorphization is trait-constrained generic functions:

```snow
def print_it<T>(x :: T) :: String where T: Display
  to_string(x)
end
```

Here, `print_it` is generic over T, and the call `to_string(x)` must resolve to different impls depending on T. When called as `print_it(42)`, T=Int, so `to_string` -> `Display__Int__to_string`. When called as `print_it("hi")`, T=String, so `to_string` -> `Display__String__to_string`.

**Resolution:** Since HM inference resolves T to a concrete type at each call site, MIR lowering can look up the concrete type and generate the correct mangled call. No separate monomorphization pass is needed -- just awareness of trait dispatch during lowering.

---

## Stdlib Protocol Architecture

### Where Protocols Live

**Recommendation:** Define stdlib protocol traits in **snow-typeck/builtins.rs**, alongside the existing compiler-known traits.

**Rationale:**
- Compiler-known traits (Add, Eq, Ord, Not) are already defined in `register_compiler_known_traits()` in builtins.rs
- Stdlib protocols follow the same pattern: trait definition + built-in impls for primitive types
- No separate crate needed -- the existing architecture supports this naturally
- Snow does not have a user-facing standard library module system (stdlib functions are registered as builtins)

### Protocol Definitions

Each protocol consists of:
1. **TraitDef** registration in builtins.rs
2. **ImplDef** registrations for primitive types in builtins.rs
3. **Runtime functions** in snow-rt (Rust) for any operations that need runtime support
4. **MIR lowering** awareness of the mangled method names

```
Protocol          Method(s)                 Built-in Impls
--------          ---------                 ---------------
Display           to_string(self) :: String Int, Float, Bool, String
Iterator          next(self) :: Option<T>   Range, List (via runtime)
From<S>           from(value :: S) :: Self  Int<->String, String<->Int
Into<T>           into(self) :: T           Derived from From (auto)
Hash              hash(self) :: Int         Int, String, Bool
Default           default() :: Self         Int(0), Float(0.0), String(""), Bool(false)
Serialize         serialize(self) :: String (Derived from Display initially)
Deserialize       deserialize(s :: String)  (Derived from From initially)
```

### Registration Pattern

```rust
// In builtins.rs, extend register_compiler_known_traits():

fn register_stdlib_protocols(registry: &mut TraitRegistry) {
    // Display protocol
    registry.register_trait(TraitDef {
        name: "Display".to_string(),
        methods: vec![TraitMethodSig {
            name: "to_string".to_string(),
            has_self: true,
            param_count: 0,
            return_type: Some(Ty::string()),
        }],
    });

    // Built-in impls for primitives
    for (ty, ty_name) in &[(Ty::int(), "Int"), (Ty::float(), "Float"),
                            (Ty::string(), "String"), (Ty::bool(), "Bool")] {
        let mut methods = FxHashMap::default();
        methods.insert("to_string".to_string(), ImplMethodSig {
            has_self: true,
            param_count: 0,
            return_type: Some(Ty::string()),
        });
        registry.register_impl(ImplDef {
            trait_name: "Display".to_string(),
            impl_type: ty.clone(),
            impl_type_name: ty_name.to_string(),
            methods,
        });
    }
}
```

### Runtime Support

Built-in Display impls for primitives need runtime functions:

```rust
// In snow-rt:
#[no_mangle]
pub extern "C" fn snow_int_to_string(value: i64) -> *mut SnowString { ... }

#[no_mangle]
pub extern "C" fn snow_float_to_string(value: f64) -> *mut SnowString { ... }

#[no_mangle]
pub extern "C" fn snow_bool_to_string(value: i8) -> *mut SnowString { ... }
```

MIR lowering maps `Display__Int__to_string` to `snow_int_to_string` (or generates inline code).

### Iterator Protocol Considerations

Iterator is the most complex stdlib protocol because it is inherently stateful and generic:

```snow
interface Iterator<T>
  def next(self) :: Option<T>
end
```

For v1.3, recommend implementing Iterator for:
- `Range` (already exists as a runtime type, `range_to_list` exists)
- `List<T>` (needs an iterator wrapper with index state)

The statefulness requires either:
- A wrapper struct holding the collection and current position
- Runtime support for iterator state management

**Recommendation:** Use wrapper structs. `RangeIterator { range: Range, current: Int }` with `next` returning `Option<Int>`.

---

## Data Flow: Trait Information Through Pipeline

```
Parser
  |
  |  InterfaceDef { name, methods: [InterfaceMethod { name, params, ret_type }] }
  |  ImplDef { trait_path, methods: [FnDef { name, params, body }] }
  |
  v
Type Checker
  |
  |  TraitRegistry:
  |    traits: { "Display" -> TraitDef { methods: [to_string] } }
  |    impls:  { ("Display", "Int") -> ImplDef { methods: { "to_string": sig } } }
  |
  |  TypeckResult.trait_registry  (NEW: expose to codegen)
  |  TypeckResult.types           (existing: concrete types at every node)
  |
  v
MIR Lowering  (LowerCtx gains trait_registry access)
  |
  |  Item::ImplDef -> for each method:
  |    Generate MirFunction with mangled name: "Display__Int__to_string"
  |    First param is self with concrete type (Int)
  |    Body lowered normally
  |
  |  CallExpr("to_string", [x]) where x: Int ->
  |    Resolve via trait_registry: "Display__Int__to_string"
  |    MirExpr::Call { func: Var("Display__Int__to_string"), args: [x] }
  |
  v
Mono Pass  (reachability -- no changes needed)
  |
  |  Discovers "Display__Int__to_string" through call graph
  |
  v
Codegen  (no trait-specific changes)
  |
  |  "Display__Int__to_string" -> LLVM function, called directly
  |  Maps to runtime: snow_int_to_string or inline codegen
```

---

## Component Boundaries

| Component | Responsibility | What Changes |
|-----------|---------------|-------------|
| snow-parser | Parse interface/impl syntax | Possibly: default method bodies |
| snow-typeck/traits.rs | Trait/impl data model, validation | Expose TraitRegistry in TypeckResult |
| snow-typeck/builtins.rs | Register compiler-known traits + stdlib protocols | Add Display, Iterator, From, Hash, Default registrations |
| snow-typeck/infer.rs | Type-check impl methods, enforce where clauses | Minimal: method resolution guidance for MIR |
| snow-codegen/mir/lower.rs | AST -> MIR with trait method resolution | Major: mangled names, trait-dispatched calls |
| snow-codegen/mir/mono.rs | Reachability analysis | Minimal: ensure mangled names reachable |
| snow-codegen/codegen/ | MIR -> LLVM IR | Minimal: no trait-specific logic needed |
| snow-rt | Runtime functions for built-in trait impls | Add: to_string, hash, default for primitives |

---

## Suggested Build Order

Based on dependency analysis, the recommended implementation order:

### Phase A: Foundation (Trait Method Dispatch)

1. **Expose TraitRegistry in TypeckResult** (snow-typeck/lib.rs)
   - Add `trait_registry: TraitRegistry` to TypeckResult
   - Pass it from infer() to the result
   - Dependency: None. Enables all downstream work.

2. **Mangled impl method names in MIR lowering** (snow-codegen/mir/lower.rs)
   - When lowering `Item::ImplDef`, use mangled name for MirFunction
   - Extract trait name and type name, format as `Trait__Type__method`
   - Dependency: TraitRegistry in TypeckResult

3. **Trait method call resolution in MIR lowering** (snow-codegen/mir/lower.rs)
   - In `lower_call_expr`, detect trait method calls
   - Use argument types + TraitRegistry to resolve mangled function name
   - Register mangled names in `known_functions`
   - Dependency: Mangled names (step 2)

4. **Self parameter lowering** (snow-codegen/mir/lower.rs)
   - Ensure `self` in impl methods lowers as first parameter
   - Concrete type from impl block, not a generic Self type
   - Dependency: Mangled names (step 2)

### Phase B: Stdlib Protocol Definitions

5. **Register stdlib protocol traits** (snow-typeck/builtins.rs)
   - Define TraitDefs for Display, Hash, Default, Eq (extend), Ord (extend)
   - Dependency: Phase A complete

6. **Register built-in impls for primitives** (snow-typeck/builtins.rs)
   - ImplDefs for Int, Float, String, Bool implementing Display, Hash, Default
   - Dependency: Protocol trait defs (step 5)

7. **Runtime functions for built-in impls** (snow-rt)
   - `snow_int_to_string`, `snow_float_to_string`, etc.
   - `snow_int_hash`, `snow_string_hash`, etc.
   - `snow_int_default`, `snow_float_default`, etc.
   - Dependency: Can be done in parallel with steps 5-6

8. **Map built-in impl methods to runtime functions** (snow-codegen/mir/lower.rs)
   - `Display__Int__to_string` -> calls `snow_int_to_string`
   - Similar to how `string_length` maps to `snow_string_length`
   - Dependency: Steps 5-7

### Phase C: User-Defined Traits (End-to-End)

9. **User-defined interface + impl end-to-end test**
   - Write Snow code: `interface Greetable`, `impl Greetable for MyStruct`
   - Verify parser -> typeck -> MIR -> codegen -> execution
   - Dependency: Phase A complete

10. **Operator overloading via user impl**
    - `impl Add for Vec2 do def add(self, other :: Vec2) :: Vec2 ... end end`
    - Type checker already uses TraitRegistry for operator dispatch
    - MIR lowering needs to map binary ops to mangled impl methods
    - Dependency: Phase A + user-defined traits working

### Phase D: Complex Protocols

11. **Iterator protocol** (stateful, generic)
    - Define Iterator trait, RangeIterator struct, impl
    - for-loop desugaring (if desired) or explicit `.next()` calls
    - Dependency: Phase A + B

12. **From/Into protocols**
    - From<S> for T with auto-derived Into
    - Requires: trait with type parameter for the source type
    - Dependency: Phase A + B

13. **Serialize/Deserialize protocols**
    - Built on Display (serialize) and From<String> (deserialize)
    - Dependency: Display + From working

---

## Anti-Patterns to Avoid

### Anti-Pattern 1: Vtable-Based Dynamic Dispatch

**What:** Creating vtable structs with function pointers for trait method dispatch.
**Why bad:** Snow chose static dispatch via monomorphization. Vtables add runtime overhead, complicate the type system, and conflict with the language's design philosophy. The actor system already provides dynamic routing where needed.
**Instead:** All trait method calls resolve to concrete function names at compile time.

### Anti-Pattern 2: Monomorphization in a Separate Pass

**What:** Collecting all generic instantiations in mono.rs and cloning/specializing functions there.
**Why bad for Snow:** The existing pipeline already resolves concrete types during type checking. Adding a separate specialization pass duplicates work and creates complexity. Rust does this because rustc's MIR is still generic; Snow's MIR is already concrete.
**Instead:** Resolve trait method dispatch during MIR lowering, where concrete types are available from the types map. Keep mono.rs as a reachability pass.

### Anti-Pattern 3: Flat Method Name Registration

**What:** Registering `to_string` as a flat function name in env, allowing only one impl per method name.
**Why bad:** Multiple types implementing the same trait would overwrite each other's method registrations. This is the current state and must be fixed.
**Instead:** Use mangled names that encode trait + type + method.

### Anti-Pattern 4: Trait Objects / Type Erasure

**What:** Erasing concrete types behind a trait reference (like Rust's `dyn Trait`).
**Why bad:** Not needed for Snow v1.3. Static dispatch handles all use cases. Type erasure requires vtables, wide pointers, and complicates the type system.
**Instead:** All trait method calls are resolved to concrete types at compile time. If dynamic dispatch is ever needed, the actor message system already provides it.

### Anti-Pattern 5: Separate Crate for Stdlib Protocols

**What:** Creating a new `snow-stdlib` crate for protocol definitions.
**Why bad:** Adds workspace complexity. The existing architecture already has protocols (operator traits) registered in snow-typeck/builtins.rs. Stdlib protocols are the same pattern.
**Instead:** Extend builtins.rs with new protocol registrations. Runtime functions go in snow-rt where they already live.

### Anti-Pattern 6: Modifying Codegen for Trait Dispatch

**What:** Adding trait-dispatch logic to the LLVM codegen layer.
**Why bad:** Codegen should only see concrete, fully-resolved function calls. Trait dispatch is a semantic concept that belongs in MIR lowering. Pushing it to codegen mixes concerns and makes the codegen layer aware of type system concepts.
**Instead:** Resolve all trait dispatch in MIR lowering. By the time code reaches codegen, `Display__Int__to_string(x)` is just a regular function call.

---

## Risk Assessment

| Risk | Severity | Mitigation |
|------|----------|------------|
| Method name collision with mangling | Medium | Use double-underscore separator `__` which is unlikely in user code |
| Backward compatibility with existing tests | Medium | Existing compiler-known traits (Add, Eq, Ord) use inline codegen, not trait dispatch. Keep this path and layer new traits alongside. |
| Iterator statefulness | High | Start with simple Range iterator; defer complex iterators |
| Generic function monomorphization complexity | High | Defer to future version; v1.3 trait impls do not require it since each impl is concrete |
| TraitRegistry clone cost | Low | TraitRegistry is small (tens of entries); cloning is negligible |

---

## Sources

- Direct analysis of Snow compiler source code (57,657 lines, 1,018 tests)
- [Rust Monomorphization - Compiler Dev Guide](https://rustc-dev-guide.rust-lang.org/backend/monomorph.html)
- [Rust MIR Lowering - Compiler Dev Guide](https://rustc-dev-guide.rust-lang.org/backend/lowering-mir.html)
- [Rust Special Types and Traits - Reference](https://doc.rust-lang.org/reference/special-types-and-traits.html)
- [Rust Lang Items - Compiler Dev Guide](https://rustc-dev-guide.rust-lang.org/lang-items.html)
- [Hindley-Milner Type System - Wikipedia](https://en.wikipedia.org/wiki/Hindley%E2%80%93Milner_type_system)
- [HM Inference with Constraints - Kwang Yul Seo](https://kseo.github.io/posts/2017-01-02-hindley-milner-inference-with-constraints.html)
- [Rust Dispatch Explained: Enums vs dyn Trait](https://www.somethingsblog.com/2025/04/20/rust-dispatch-explained-when-enums-beat-dyn-trait/)

---
*Architecture research for: Snow v1.3 Trait System & Monomorphization*
*Researched: 2026-02-07*
