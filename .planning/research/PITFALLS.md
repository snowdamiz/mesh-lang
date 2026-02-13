# Domain Pitfalls

**Domain:** Adding associated types, iterator protocol, From/Into traits, numeric traits, and Collect trait to existing Mesh compiler
**Researched:** 2026-02-13
**Confidence:** HIGH (codebase-informed, architecture-verified)

## Critical Pitfalls

Mistakes that cause rewrites, inference regressions, or correctness failures across the existing 93K LOC compiler.

---

### Pitfall 1: Associated Types Break HM Principal Types Property

**What goes wrong:** The current `Ty` enum has no representation for associated type projections (e.g., `<T as Iterator>::Item`). Adding them requires a new `Ty` variant that participates in unification, but associated type projections are not first-class terms in standard HM -- they are type-level functions that may not be injective. This means the unifier cannot always determine a principal type, and inference may produce ambiguous results where it previously succeeded.

**Why it happens:** Standard HM unification treats all type constructors as injective: `List<A> = List<B>` implies `A = B`. But `<T as Iterator>::Item = Int` does NOT imply `T = SomeSpecificType`, because multiple types can have `Item = Int`. The unifier in `unify.rs` currently assumes all `Ty::App` arguments can be structurally decomposed. A projection variant breaks this assumption.

**Consequences:**
- Programs that previously inferred types unambiguously may now require type annotations
- The occurs check in `unify.rs:170-195` does not understand projections and may loop or give false negatives
- `resolve()` in `unify.rs:128-162` would need to handle normalization (reducing `<Vec<Int> as Iterator>::Item` to `Int`), which requires trait resolution during unification -- creating a circular dependency between inference and trait solving

**Prevention:**
1. Do NOT add a `Ty::Projection` variant that participates in general unification. Instead, normalize projections eagerly: whenever an associated type is referenced, immediately look up the concrete impl and substitute the concrete type. This keeps the unifier unchanged.
2. Require that all associated type references occur in contexts where the implementing type is already known (fully resolved). The `TraitRegistry::find_impl()` already does structural matching -- use it to resolve the associated type to a concrete `Ty` before it enters the unification table.
3. Add a `Ty::AssocProjection { trait_name, type_name, assoc_name }` for deferred cases, but treat it as an error if it survives past the normalization pass. Never unify two projections directly.

**Detection:** Test that all programs in the existing test suite still compile with zero annotation changes. Any new "ambiguous type" errors indicate a regression.

**Phase mapping:** Must be addressed FIRST, before Iterator or From/Into. Associated types are a prerequisite for `type Item` in Iterator and `type Output` in From.

---

### Pitfall 2: Iterator State Machine Representation in MIR

**What goes wrong:** The current for-in loop codegen uses specialized MIR nodes (`ForInList`, `ForInMap`, `ForInSet`, `ForInRange`) that are each lowered to custom LLVM IR with indexed iteration. An iterator protocol requires a fundamentally different representation: each iterator is a struct holding mutable state, and `next()` is a method call that returns `Option<Item>`. The MIR has no concept of "mutable iterator state" -- all for-in loops currently use ephemeral counter variables generated inline during codegen.

**Why it happens:** The existing codegen in `codegen/expr.rs:146-160` emits basic blocks with integer counters (`i = 0; while i < len { elem = get(coll, i); ... ; i += 1 }`). This is efficient but hardcoded per-collection-type. An Iterator trait requires: (a) constructing an iterator struct from a collection, (b) calling `next()` which mutates the iterator and returns `Option<Item>`, (c) pattern matching the Option to extract the element or break.

**Consequences:**
- The existing `ForInList`/`ForInMap`/`ForInSet`/`ForInRange` MIR nodes must continue working for backward compatibility (existing programs must not break)
- Adding iterator-based for-in alongside indexed for-in creates two parallel codegen paths that must produce identical semantics
- Iterator structs need heap or stack allocation, GC integration, and the monomorphization pass (`mono.rs`) must handle iterator type specialization
- The comprehension semantics (all for-in loops return `List<T>` of collected body results) must work identically with both indexed and iterator-based iteration

**Prevention:**
1. Keep the existing `ForInList`/`ForInMap`/`ForInSet`/`ForInRange` MIR nodes as-is. They are optimized codegen paths for known collection types.
2. Add a NEW `ForInIterator { var, iterator_expr, body, elem_ty, body_ty, ty }` MIR node for trait-based iteration. This is used when for-in encounters a type that implements `Iterable`/`IntoIterator` but is not a known collection.
3. In the MIR lowerer, check: if the collection type is a known type (List, Map, Set, Range), emit the existing specialized nodes. Otherwise, desugar to: `let iter = IntoIterator.into_iter(collection); while let Some(elem) = Iterator.next(iter) do body end`.
4. The iterator struct itself should be stack-allocated (like closures: `MakeClosure` already handles this pattern with `{fn_ptr, env_ptr}`). Use the same alloca + GEP pattern.

**Detection:** Run the full for-in test suite after adding iterator support. Every existing for-in test must produce identical output. Add new tests for custom iterables.

**Phase mapping:** Iterator MIR representation should come AFTER associated types are working, since `type Item` is an associated type on the Iterator trait.

---

### Pitfall 3: From/Into Blanket Impl Creates Inference Ambiguity

**What goes wrong:** The standard From/Into pattern requires a blanket impl: `impl Into<U> for T where T: From<U>`. But the current trait system (`traits.rs`) has no concept of blanket impls (impls parameterized over type variables with trait bounds). Adding one creates a chicken-and-egg problem: to resolve `Into<String> for Int`, the system must first check if `From<Int> for String` exists, which requires trait resolution during impl lookup -- the exact circular dependency that the current `TraitRegistry::has_impl()` avoids by doing simple structural matching.

**Why it happens:** The current `register_impl()` in `traits.rs:97-157` stores concrete impls like `ImplDef { trait_name: "Add", impl_type: Ty::int() }`. A blanket impl would be `ImplDef { trait_name: "Into", impl_type: Ty::Var(?), ... }` with an additional constraint `where T: From<U>`. The `find_impl()` method does structural unification against stored impls, but it has no mechanism to check where-clause constraints during the match. Adding this check means `find_impl` becomes recursive (checking `has_impl("From", ...)` inside `find_impl("Into", ...)`), which can loop.

**Consequences:**
- Naive implementation causes infinite recursion: `Into` lookup triggers `From` lookup, which could trigger another `Into` lookup
- Even with cycle detection, the search space explodes: for each type, the solver must check all `From` impls to determine available `Into` impls
- Error messages become confusing: "type X does not implement Into<Y>" when the real issue is "type X does not implement From<Y>"

**Prevention:**
1. Do NOT implement a general blanket impl mechanism for v7.0. Instead, use a compiler-known auto-derivation: whenever `impl From<A> for B` is registered, ALSO automatically register `impl Into<B> for A` as a concrete impl. This is the same pattern used for compiler-known traits in `builtins.rs:821-1230` (Add/Sub/Mul/etc are registered with explicit impls for Int and Float, not derived from a blanket).
2. In `register_impl()`, add a post-registration hook: if `trait_name == "From"`, extract `A` (the impl type) and `B` (the `From<B>` type parameter), then register a synthetic `Into<B> for A` impl with a method that calls the `From` method.
3. This avoids recursive trait solving entirely while giving users the expected behavior: write `From`, get `Into` for free.

**Detection:** Test that `impl From<Int> for String` automatically enables `Into<String>` for `Int`. Test that explicit `impl Into<X> for Y` still works and does not conflict with auto-derived ones. Test that circular `From` impls (A -> B -> A) produce a clear error rather than infinite recursion.

**Phase mapping:** From/Into should come AFTER associated types (since `From<T>` may want `type Output`), but can be implemented before Iterator since it is simpler.

---

### Pitfall 4: Numeric Trait Output Type Breaks Operator Chaining

**What goes wrong:** The current arithmetic operators (Add, Sub, Mul, Div, Mod) in `builtins.rs:824-855` use `return_type: None` (meaning "Self" -- the implementing type). The actual return type is stored per-impl as `Some(Ty::int())` or `Some(Ty::float())`. This works because `Int + Int = Int` and `Float + Float = Float`. But if we introduce `type Output` as an associated type on Add (like Rust's `Add<Rhs = Self, Output = Self>`), the output type can differ from the input types. This breaks the assumption in `infer.rs` where `infer_binary_op` uses the LHS type as the result type of the operation.

**Why it happens:** The MIR lowerer (`lower.rs`) and LLVM codegen (`codegen/expr.rs`) both assume `BinOp { op: Add, lhs, rhs, ty }` where `ty` equals the type of `lhs`. The `codegen_int_binop` / `codegen_float_binop` functions produce LLVM instructions whose result type matches the operand type. If `Add::Output` is a different type (e.g., `Vec + Vec = Matrix`), the MIR type annotation is wrong, and the LLVM IR will be mistyped.

**Consequences:**
- `a + b + c` may fail: if `a + b` returns type `C` (not type `A`), then `C + c` requires `Add` impl for `C`, not `A`
- The type recorded in `MirExpr::BinOp { ty }` must be the OUTPUT type, not the LHS type
- `codegen_binop` in `expr.rs` selects the codegen path based on `lhs_ty` -- if output differs, it selects the wrong path
- Compound assignment (`+=`) must unify `lhs_type` with `Add::Output`, not with `rhs_type`

**Prevention:**
1. For v7.0, keep the simple model: `Add<Self> -> Self`. The output type equals the implementing type. Do NOT introduce a separate `Output` associated type on arithmetic traits yet.
2. When extending existing Add/Sub/Mul/Div/Mod trait defs in `builtins.rs`, keep `return_type: Some(ty.clone())` where `ty` is the implementing type. This preserves the invariant that `BinOp.ty == lhs_ty == rhs_ty == result_ty` for primitives.
3. For user-defined operator overloading (e.g., `impl Add for Point`), the MIR lowerer already rewrites `BinOp` to `Call` for non-primitive types (Phase 18-03 established this). The `Call` node carries its own return type from the function signature, so the output type can differ safely.
4. If `type Output` is added later, it requires updating: (a) `infer_binary_op` to use `TraitRegistry.resolve_trait_method("add", lhs_ty)` for the result type instead of assuming LHS type, (b) `MirExpr::BinOp { ty }` to carry the resolved output type, (c) `codegen_binop` to handle mismatched input/output types.

**Detection:** Test `a + b + c` with user-defined types. Test that the inferred type of `x + y` matches the declared return type of the `add` method, not the type of `x`.

**Phase mapping:** Numeric traits (Add/Sub/Mul/Div/Neg) should be extended BEFORE Iterator, because iterator adapters like `sum()` depend on numeric traits.

---

### Pitfall 5: Monomorphization Name Mangling Collision with Associated Types

**What goes wrong:** The current name mangling scheme uses single underscores for generic type instantiation (`Option_Int`, `Result_Int_String` in `types.rs:148-155`) and double underscores for trait method dispatch (`Add__add__Int` in `lower.rs:1137`). Associated types add a third dimension: the same trait method may be instantiated for different associated type bindings. If mangling does not account for associated types, two different monomorphizations may collide.

**Why it happens:** Consider `Iterator__next__List_Int` (Iterator.next() on List<Int>) and `Iterator__next__List_String` (Iterator.next() on List<String>). These already mangle differently because the implementing type differs. But if a generic function takes `T: Iterator` and calls `T.next()`, monomorphization must create a version for each concrete `T` AND each concrete `Item`. The current `mangle_type_name` in `types.rs:148` only handles the base type's generic args, not trait-associated type bindings.

**Consequences:**
- Linker errors from duplicate symbols if two different instantiations produce the same mangled name
- Incorrect function calls if monomorphization picks the wrong specialization
- The `mir_type_to_impl_name` function in `types.rs:210-220` returns `"Unknown"` for complex types, which would cause ALL complex-type iterator instances to collide

**Prevention:**
1. Extend `mangle_type_name` to include the associated type bindings when mangling trait method calls. Format: `Trait__method__ImplType__AssocName_ConcreteType` (e.g., `Iterator__next__ListInt__Item_Int`).
2. Fix `mir_type_to_impl_name` to handle `MirType::Tuple`, `MirType::Closure`, and `MirType::Ptr` cases instead of returning `"Unknown"`. Every MIR type must produce a unique, deterministic name suffix.
3. In the monomorphization pass (`mono.rs:24-30`), when specializing generic functions that use associated types, include the associated type bindings in the function's mangled name. Currently mono.rs only does reachability analysis -- it needs to be extended to actually specialize when associated types are involved.

**Detection:** Compile a program with two different iterator types whose `Item` types differ. Verify that the linker produces no duplicate symbol errors and that each iterator correctly yields its own `Item` type.

**Phase mapping:** Must be addressed during associated types implementation, before Iterator, since Iterator is the first trait that uses associated types.

---

### Pitfall 6: Collect Trait Requires Higher-Kinded-Like Dispatch

**What goes wrong:** A `Collect` trait (like Rust's `FromIterator`/`collect()`) needs to convert an iterator into a collection. The return type depends on the TARGET collection, not the iterator: `iter.collect::<Vec<_>>()` vs `iter.collect::<HashSet<_>>()`. This requires the caller to specify the output type, and the trait solver to find the right impl based on the return type context. HM inference flows type information from arguments to return types (bottom-up), not from return types to arguments (top-down). Selecting a trait impl based on the expected return type requires bidirectional type flow.

**Why it happens:** The current `InferCtx` in `unify.rs` works bottom-up: it infers the type of each expression from its subexpressions, then unifies with any annotation. For `collect()`, the return type is a fresh variable that gets unified with the context's expected type. But selecting which `Collect` impl to use requires knowing the return type FIRST, to find the right impl, to determine the `collect` method's behavior. This is a "type-directed dispatch" problem that standard HM does not handle.

**Consequences:**
- Without type annotations, `let result = iter.collect()` cannot be inferred -- which collection type?
- The inference engine would need to enumerate all `Collect` impls and try each one, which is expensive and may produce multiple valid solutions (ambiguity)
- Error messages become opaque: "could not infer type for collect()" gives no hint about which collection types are available

**Prevention:**
1. For v7.0, require explicit type annotations on `collect()` calls: `let xs: List<Int> = iter.collect()` or `iter.collect::<List<Int>>()`. The annotation constrains the return type, making impl selection unambiguous.
2. Implement collect as a simpler pattern first: `iter.to_list()`, `iter.to_set()`, `iter.to_map()` -- named methods that each return a specific collection type. This avoids the return-type-directed dispatch problem entirely.
3. If a general `Collect` trait is desired, implement it as: when the return type annotation is present, use it to select the impl. When absent, emit a specific error: "collect() requires a type annotation -- try `let x: List<T> = ...`".
4. The `Scheme` type already supports polymorphic functions (see `builtins.rs:70-77` for `default()`). Use the same pattern: `collect` is polymorphic in its return type, and the call-site type annotation drives instantiation.

**Detection:** Test `collect()` with explicit annotations (must work). Test `collect()` without annotations (must produce a clear error, not a cryptic inference failure).

**Phase mapping:** Collect should be the LAST feature implemented, after Iterator and From/Into, since it depends on both.

---

### Pitfall 7: For-In Backward Compatibility -- Existing Comprehension Semantics

**What goes wrong:** All existing for-in loops in Mesh have comprehension semantics: they return `List<T>` of collected body results (see `MirExpr::ForInList { ty: MirType::Ptr }` and the list builder pattern in codegen). If the new iterator-based for-in changes this semantic (e.g., returning `Unit` like Rust's for loops), existing programs that depend on the return value silently change behavior.

**Why it happens:** The current for-in implementation (`codegen_for_in_list`, etc.) creates a list builder, pushes each body result into it, and returns the built list. The MIR type of for-in is `Ptr` (a list). If iterator-based for-in returns the last `next()` result or `Unit` instead, the type changes and existing code breaks.

**Consequences:**
- `let squares = for x in [1, 2, 3] do x * x end` currently evaluates to `[1, 4, 9]`. If iterator-based for-in returns Unit, this becomes `()`.
- The break-returns-partial-list semantics would also need to be preserved
- Filter clauses (`when condition`) work by skipping the list builder push, not by skipping the iterator -- the two mechanisms interact differently

**Prevention:**
1. Preserve comprehension semantics for ALL for-in loops, regardless of whether they use indexed or iterator-based codegen.
2. For iterator-based for-in, the codegen must STILL wrap the loop body in a list builder. The desugaring is: `let builder = list_builder_new(); let iter = into_iter(coll); while let Some(elem) = next(iter) do builder.push(body(elem)) end; builder.finish()`.
3. Do NOT change the return type of for-in. It remains `List<T>` where `T` is the body expression type.
4. Add integration tests that verify `let result = for x in custom_iterable do x * 2 end` returns a list, identical to `let result = for x in [1, 2, 3] do x * 2 end`.

**Detection:** Any existing for-in test that captures the return value will catch this regression immediately.

**Phase mapping:** Must be verified during Iterator implementation. Regression tests from the v1.7 loop system milestone are the safety net.

---

## Moderate Pitfalls

### Pitfall 8: TraitMethodSig Has No Associated Type Declarations

**What goes wrong:** The current `TraitMethodSig` in `traits.rs:15-29` stores method signatures but has no field for associated type declarations. A trait like `Iterator` needs to declare `type Item` as part of the trait definition, and impls must provide the concrete type for `Item`. Without storage for associated types in `TraitDef`, the trait registry cannot validate that impls provide all required associated types, and method signatures cannot reference associated types.

**Prevention:**
1. Add `associated_types: Vec<AssocTypeDef>` to `TraitDef` where `AssocTypeDef = { name: String, bounds: Vec<String> }`.
2. Add `associated_types: FxHashMap<String, Ty>` to `ImplDef` mapping associated type names to their concrete types.
3. During `register_impl`, validate that all associated types declared in the trait are provided by the impl, similar to how missing methods are currently detected in `traits.rs:102-114`.

**Phase mapping:** Must be implemented as part of associated types, before Iterator.

---

### Pitfall 9: freshen_type_params Only Recognizes Single-Letter Type Params

**What goes wrong:** The `freshen_type_params` function in `traits.rs:333-381` uses a heuristic: `Ty::Con` names that are a single uppercase ASCII letter (A-Z) are treated as type parameters. This breaks for associated type names like `Item`, `Output`, or `Error`, which are multi-character. These would NOT be freshened during structural matching, causing false negatives when matching generic impls that reference associated types.

**Prevention:**
1. When associated types are added to `TraitDef`, pass the list of declared associated type names to `freshen_type_params` (or its replacement). These names should also be freshened.
2. Alternatively, switch to an explicit tracking approach: when registering an impl, record which `Ty::Con` names are type parameters (from the trait's generic params list) and which are associated types (from the trait's associated type list). Freshen both categories.
3. The safest long-term fix: represent type parameters as `Ty::Var` from the start (during parsing/inference of trait definitions), not as `Ty::Con`. This eliminates the need for heuristic freshening entirely.

**Phase mapping:** Must be fixed when adding associated types. The heuristic is already fragile for multi-letter type params (e.g., `Key`, `Value`).

---

### Pitfall 10: IntoIterator vs Iterator -- Two Traits or One?

**What goes wrong:** Rust separates `Iterator` (has `next()`, holds mutable state) from `IntoIterator` (has `into_iter()`, converts a collection into an iterator). If Mesh conflates these (one `Iterable` trait with both `next()` and `into_iter()`), then collections would need mutable iteration state embedded in themselves, which breaks value semantics and makes collections non-reentrant (iterating twice simultaneously is impossible).

**Prevention:**
1. Define TWO traits: `Iterator` with `type Item` and `fn next(self) -> Option<Item>`, and `Iterable` with `type Item` and `type Iter: Iterator` and `fn iter(self) -> Iter`.
2. For-in desugars to: `let iter = Iterable.iter(collection); while let Some(elem) = Iterator.next(iter) do ... end`.
3. Collections (List, Map, Set) implement `Iterable` but NOT `Iterator`. The iterator struct is a separate type with mutable state.
4. Iterators implement both `Iterator` AND `Iterable` (trivially -- `iter(self)` returns `self`), so passing an iterator directly to for-in works.

**Phase mapping:** Architecture decision that must be made before implementing Iterator. Affects trait definitions, MIR representation, and codegen.

---

### Pitfall 11: Neg Trait for Unary Minus -- UnaryOp Codegen Path

**What goes wrong:** The current `UnaryOp::Neg` in MIR (`mir/mod.rs:553-558`) is hardcoded to numeric types. Adding a `Neg` trait for user-defined types requires the same MIR lowerer rewrite that was done for BinOp in Phase 18-03: detect non-primitive types and emit a `Call` to `Neg__neg__TypeName` instead of `UnaryOp`.

**Prevention:**
1. Follow the exact same pattern as BinOp dispatch unification from Phase 18-03: in the MIR lowerer, when lowering a unary negation, check if the operand type is a primitive. If yes, emit `MirExpr::UnaryOp`. If no, emit `MirExpr::Call` to the mangled Neg trait method.
2. Register `Neg` as a compiler-known trait in `builtins.rs` with impls for Int and Float, following the same pattern as Add/Sub/Mul/Div/Mod.

**Phase mapping:** Should be done alongside numeric trait extension (Add/Sub/Mul/Div/Neg together).

---

### Pitfall 12: Generic Iterator Adapters Cause Monomorphization Explosion

**What goes wrong:** Iterator adapters like `map`, `filter`, `take`, `zip` each create a new iterator type wrapping the previous one. A chain like `list.iter().map(f).filter(g).take(n)` creates a deeply nested type: `Take<Filter<Map<ListIter<Int>, F>, G>>`. Monomorphization must specialize ALL generic functions for each unique nested type, which can cause exponential code generation.

**Prevention:**
1. For v7.0, limit the iterator adapter set to the essentials: `map`, `filter`, `take`, `zip`, `enumerate`. Do not implement the full Rust `itertools` suite.
2. Monitor binary sizes during development. If a simple iterator chain causes >2x code size increase, consider boxing intermediate iterators (erasing the type) as an escape hatch.
3. The current monomorphization pass (`mono.rs`) only does reachability analysis. It does NOT actually specialize functions. This means generic iterator adapter functions are currently emitted as-is, which would fail at LLVM codegen (LLVM needs concrete types). The mono pass must be extended to actually stamp out specialized copies.

**Phase mapping:** Becomes relevant when implementing iterator adapters, which is after the basic Iterator trait.

---

## Minor Pitfalls

### Pitfall 13: AST Parsing for Associated Type Syntax

**What goes wrong:** The parser (`mesh-parser`) currently has no syntax for `type Item = Int` inside trait or impl definitions. Adding associated type syntax requires new parser rules, new CST/AST node types, and coordination between the parser and type checker.

**Prevention:** Design the syntax before implementing. Suggested: `type Item = ConcreteType` in impl blocks, `type Item` in interface blocks. This is a small parser change (add a new item variant) but must be done carefully to avoid ambiguity with existing `type` alias syntax.

**Phase mapping:** Must be the very first implementation step -- parser changes enable everything else.

---

### Pitfall 14: Option Type Required for Iterator.next()

**What goes wrong:** `Iterator.next()` returns `Option<Item>`. The existing `Option` type is a built-in sum type with `Some(T)` and `None` variants. However, the current type checker registers Option as a built-in in `builtins.rs` and the codegen has special handling for Option pattern matching. If the Iterator's `next()` method returns `Option<Item>` where `Item` is an associated type, the type checker must unify `Option<associated_type_projection>` with the expected `Option<ConcreteType>`, which requires associated type normalization to work correctly first.

**Prevention:** Ensure associated type normalization happens BEFORE Option type checking. When type-checking `iter.next()`, first resolve `Item` to its concrete type (e.g., `Int`), then construct `Option<Int>`, then proceed with normal type checking. Do not attempt to construct `Option<Item>` as a symbolic type.

**Phase mapping:** Automatically handled if associated type normalization (Pitfall 1) is implemented correctly.

---

### Pitfall 15: Cross-Module Trait Export for Associated Types

**What goes wrong:** The `ExportedSymbols` type in `lib.rs:98-112` exports trait defs and trait impls. But `TraitDef` has no associated type declarations (Pitfall 8), and `ImplDef` has no associated type bindings. When a module exports an Iterator impl, the importing module needs to know what `Item` type it provides. Without this information in the export structure, cross-module iterator usage is impossible.

**Prevention:** When extending `TraitDef` and `ImplDef` with associated types (Pitfall 8), also update `collect_exports` in `lib.rs:191-318` to include associated type information in exported trait defs and impls. The `ImportContext` in `lib.rs:52-69` carries `all_trait_defs` and `all_trait_impls` -- these will automatically include the new associated type fields if the structs are updated.

**Phase mapping:** Must be done alongside associated type implementation. Verify with a multi-module test where one module defines an Iterator impl and another module uses it.

---

## Phase-Specific Warnings

| Phase Topic | Likely Pitfall | Mitigation |
|-------------|---------------|------------|
| Associated types (type system) | Breaks principal types, unifier loops | Eager normalization, no projection in unification (Pitfall 1) |
| Associated types (parser) | Syntax ambiguity with type aliases | Design syntax carefully, test against existing type alias parsing (Pitfall 13) |
| Associated types (trait registry) | No storage, no validation | Extend TraitDef/ImplDef structs (Pitfall 8) |
| Associated types (mangling) | Name collisions | Include assoc type bindings in mangled names (Pitfall 5) |
| Associated types (freshening) | Single-letter heuristic fails | Pass explicit param/assoc type name lists (Pitfall 9) |
| Iterator protocol (MIR) | No iterator state representation | New ForInIterator MIR node alongside existing nodes (Pitfall 2) |
| Iterator protocol (semantics) | Comprehension semantics broken | Always wrap iterator loop in list builder (Pitfall 7) |
| Iterator protocol (design) | IntoIterator vs Iterator confusion | Two separate traits (Pitfall 10) |
| Iterator protocol (Option) | Associated type in return position | Normalize before Option construction (Pitfall 14) |
| From/Into traits | Blanket impl recursion | Auto-derive Into from From registrations (Pitfall 3) |
| From/Into (cross-module) | Export structure missing assoc types | Update ExportedSymbols (Pitfall 15) |
| Numeric traits (Add/Sub/Mul/Div) | Output type breaks chaining | Keep Output = Self for v7.0 (Pitfall 4) |
| Numeric traits (Neg) | Unary codegen path hardcoded | Same pattern as BinOp dispatch (Pitfall 11) |
| Collect trait | Return-type-directed dispatch | Require annotations or use named methods (Pitfall 6) |
| Iterator adapters | Monomorphization explosion | Limit adapter set, monitor binary sizes (Pitfall 12) |

## Recommended Phase Ordering (Based on Pitfall Dependencies)

```
Phase 1: Associated Types (foundation)
  - Parser syntax for `type X` in interface/impl
  - TraitDef/ImplDef storage extension
  - Eager normalization in type checker
  - freshen_type_params fix
  - Name mangling extension
  - Cross-module export updates
  Addresses: Pitfalls 1, 5, 8, 9, 13, 15

Phase 2: Numeric Traits Extension (low risk, unblocks Phase 3)
  - Add Neg trait to builtins
  - Extend Add/Sub/Mul/Div with user-defined type support
  - UnaryOp dispatch in MIR lowerer
  Addresses: Pitfalls 4, 11

Phase 3: Iterator Protocol (core)
  - Iterator + Iterable trait definitions
  - Iterator struct representation
  - ForInIterator MIR node
  - Codegen for trait-based iteration
  - Comprehension semantics preservation
  Addresses: Pitfalls 2, 7, 10, 14

Phase 4: From/Into Traits (depends on associated types)
  - From/Into trait definitions
  - Auto-derive Into from From
  Addresses: Pitfall 3

Phase 5: Collect Trait (depends on Iterator + From)
  - Collect trait with annotation requirement
  - to_list/to_set/to_map convenience methods
  Addresses: Pitfalls 6, 12
```

## Sources

### Primary (HIGH confidence -- direct codebase analysis)
- `crates/mesh-typeck/src/ty.rs` -- Ty enum, TyCon, Scheme, TyVar (no associated type variant)
- `crates/mesh-typeck/src/unify.rs` -- InferCtx, unification, occurs check, generalization (no projection handling)
- `crates/mesh-typeck/src/traits.rs` -- TraitRegistry, TraitDef, ImplDef, freshen_type_params (single-letter heuristic)
- `crates/mesh-typeck/src/builtins.rs` -- Compiler-known traits (Add/Sub/Mul/Div/Mod/Eq/Ord/Not/Display/Debug/Hash/Default)
- `crates/mesh-typeck/src/lib.rs` -- TypeckResult, ExportedSymbols, ImportContext
- `crates/mesh-typeck/src/error.rs` -- TypeError variants
- `crates/mesh-codegen/src/mir/mod.rs` -- MIR types, ForInList/ForInMap/ForInSet/ForInRange nodes
- `crates/mesh-codegen/src/mir/types.rs` -- resolve_type, mangle_type_name, mir_type_to_impl_name
- `crates/mesh-codegen/src/mir/mono.rs` -- Monomorphization pass (reachability only, no specialization)
- `crates/mesh-codegen/src/mir/lower.rs` -- MIR lowering, trait method dispatch mangling
- `crates/mesh-codegen/src/codegen/expr.rs` -- LLVM codegen, for-in codegen, BinOp/UnaryOp dispatch
- `.planning/phases/18-trait-infrastructure/18-RESEARCH.md` -- Phase 18 trait infrastructure research

### Secondary (MEDIUM confidence -- web research, multiple sources agree)
- [Unification in Chalk, part 2](https://smallcultfollowing.com/babysteps/blog/2017/04/23/unification-in-chalk-part-2/) -- Associated type projection in unification, termination concerns
- [Rust Coherence (Chalk)](https://rust-lang.github.io/chalk/book/clauses/coherence.html) -- Coherence rules for trait impls
- [RFC 1023: Rebalancing Coherence](https://rust-lang.github.io/rfcs/1023-rebalancing-coherence.html) -- Orphan rules and blanket impl constraints
- [RFC 2451: Re-rebalancing Coherence](https://rust-lang.github.io/rfcs/2451-re-rebalancing-coherence.html) -- Updated coherence rules
- [Rust orphan rules documentation](https://github.com/Ixrec/rust-orphan-rules) -- Comprehensive orphan rule analysis
- [Rust Monomorphization Dev Guide](https://rustc-dev-guide.rust-lang.org/backend/monomorph.html) -- Monomorphization and code bloat
- [Code bloat from monomorphization (rust-lang/rust#77767)](https://github.com/rust-lang/rust/issues/77767) -- Concrete code bloat examples
- [Generics and Compile-Time in Rust (PingCAP)](https://www.pingcap.com/blog/generics-and-compile-time-in-rust/) -- Monomorphization compilation time impact
- [GHC Type Families User Guide](https://ghc.gitlab.haskell.org/ghc/doc/users_guide/exts/type_families.html) -- Type family inference pitfalls, injectivity
- [Simple unification-based type inference for GADTs (Microsoft Research)](https://www.microsoft.com/en-us/research/wp-content/uploads/2016/02/gadt-pldi.pdf) -- Extending HM with type-level features
- [LLVM IR introduction (mcyoung)](https://mcyoung.xyz/2023/08/01/llvm-ir/) -- Iterator overhead in compiled IR
