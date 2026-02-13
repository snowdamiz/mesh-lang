# Technology Stack

**Project:** Mesh v7.0 -- Iterator Protocol & Trait Ecosystem
**Researched:** 2026-02-13

## Recommended Stack

No new Rust crate dependencies required. All v7.0 features are compiler-internal changes to existing crates (mesh-parser, mesh-typeck, mesh-codegen, mesh-rt). This is consistent with Mesh's zero-new-deps philosophy for compiler features.

### Compiler Internals: What Changes

| Component | Crate | Purpose | Changes Required |
|-----------|-------|---------|-----------------|
| Parser | mesh-parser | Parse associated type declarations in interfaces | New `ASSOC_TYPE` syntax kind, `type Item` in interface bodies |
| Type Representation | mesh-typeck/ty.rs | Represent associated type projections | New `Ty::Projection` variant for `Self.Item` |
| Unification Engine | mesh-typeck/unify.rs | Handle projection types during unification | Deferred normalization when encountering projections |
| Trait Registry | mesh-typeck/traits.rs | Store associated type info in trait/impl defs | `associated_types` field on `TraitDef` and `ImplDef` |
| Type Inference | mesh-typeck/infer.rs | Infer associated types, resolve projections | Projection normalization pass, `Self.Item` resolution |
| Builtins | mesh-typeck/builtins.rs | Register Iterator/Iterable/From/Into/Add/.../Collect | New trait registrations with associated types |
| MIR Lowering | mesh-codegen/mir/lower.rs | Desugar for-in to Iterable protocol, iterator state | New iterator state machine lowering, for-in rewrite |
| MIR Types | mesh-codegen/mir/types.rs | Resolve projection types to concrete MIR types | Projection normalization in `resolve_type` |
| Runtime | mesh-rt | Iterator runtime helpers | `mesh_iter_*` functions for lazy evaluation support |

### Core Framework (No Changes)

| Technology | Version | Purpose | Why No Change |
|------------|---------|---------|---------------|
| Rust | stable | Compiler language | Existing toolchain sufficient |
| LLVM 21 | via Inkwell 0.8 | Code generation | No new LLVM features needed |
| ena | existing | Union-find for unification | Works with projection extensions |
| rowan | existing | CST for parsing | Existing node types extensible |
| rustc_hash | existing | FxHashMap throughout | No change |

## Detailed Technical Decisions

### 1. Associated Types: Projection-Based Representation

**Decision:** Add `Ty::Projection { trait_name: String, assoc_name: String, self_ty: Box<Ty> }` variant to the `Ty` enum.

**Why:** Mesh's existing `Ty` enum (Var, Con, Fun, App, Tuple, Never) has no way to represent "the Item type of T's Iterator impl." A projection type is the standard approach used by Rust (chalk), Haskell (type families), and Swift (associated types). It keeps the type representation self-contained -- no need for external lookup during pure type operations.

**Alternative considered:** Eagerly resolving associated types to concrete types at trait-impl registration time. Rejected because this fails when the Self type is still a type variable during inference (e.g., in generic functions like `fn sum<T>(iter: T) where T: Iterator`).

**Confidence:** HIGH -- this is the established pattern in every language with associated types and type inference.

### 2. Deferred Projection Normalization

**Decision:** When unification encounters a `Ty::Projection` against another type T, succeed immediately and emit a deferred `ProjectionEq(projection, T)` constraint. Resolve these constraints after the main unification pass when more type information is available.

**Why:** Mesh's existing unification (Algorithm J via ena union-find) resolves types eagerly. But projection types often can't be resolved until the Self type is fully known. Chalk (Rust's trait solver) and OutsideIn(X) (GHC) both use deferred constraint strategies for exactly this reason.

**Integration with existing InferCtx:** Add a `pending_projections: Vec<(Ty, Ty)>` field to `InferCtx`. After each top-level inference pass, drain pending projections and attempt normalization. If Self is resolved, look up the impl and substitute. If still unresolved, keep deferred.

**Alternative considered:** Immediately failing when projection can't be resolved. Rejected because this would break inference for generic code where types flow in from multiple directions.

**Confidence:** HIGH -- deferred constraint resolution is standard for associated types.

### 3. Projection Normalization Algorithm

**Decision:** Normalize `Ty::Projection { trait_name, assoc_name, self_ty }` by:
1. Resolve `self_ty` through the unification table
2. Look up `trait_name` impl for the resolved self_ty in `TraitRegistry`
3. If found, substitute the associated type value from the impl
4. If self_ty is still a type variable, defer

**Why:** This is the minimal algorithm that handles Mesh's use case. Mesh doesn't need Rust's full trait solver (no lifetime parameters, no higher-ranked trait bounds, no specialization, no negative impls). The existing `TraitRegistry::find_impl` + structural unification already handles step 2 correctly.

**Key simplification vs. Rust:** Mesh uses monomorphization exclusively (no trait objects, no dynamic dispatch). This means every projection MUST normalize to a concrete type before codegen. No need for placeholder/applicative types as in Chalk -- unresolved projections at codegen time are errors.

**Confidence:** HIGH -- simplification is valid given Mesh's static dispatch model.

### 4. Iterator Protocol: Two-Trait Design (Iterator + Iterable)

**Decision:** Define two traits:
```
interface Iterator do
  type Item
  fn next(self) :: Option<Self.Item>
end

interface Iterable do
  type Item
  type Iter        # must implement Iterator
  fn iter(self) :: Self.Iter
end
```

**Why:** This mirrors Rust's `Iterator`/`IntoIterator` split, which is the battle-tested design. The separation allows:
- Types that ARE iterators (stateful, have next()) to be used directly
- Types that CAN PRODUCE iterators (collections) to be iterable via for-in
- Multiple iterator types per collection (e.g., values vs. entries for Map)

**Mesh-specific adaptation:** Use `iter()` instead of Rust's `into_iter()` because Mesh has no ownership/move semantics. All values are GC-managed, so "consuming" vs. "borrowing" iteration is not a concern.

**For-in desugaring:** `for x in collection do body end` desugars to:
```
let __iter = Iterable.iter(collection)
while true do
  case Iterator.next(__iter) do
    Some(x) -> body
    None -> break
  end
end
```

This reuses the existing while/break/case infrastructure. The desugaring happens in MIR lowering (lower.rs), replacing the current indexed iteration paths.

**Confidence:** HIGH -- Rust's design is proven, and the adaptation to Mesh's GC model is straightforward.

### 5. Lazy Iterator Combinators: Struct-Based State Machines

**Decision:** Each combinator (map, filter, take, etc.) returns a new struct that implements Iterator. These structs are compiler-generated during monomorphization.

Example: `list.iter().map(fn x -> x * 2 end).filter(fn x -> x > 5 end)` produces:
- `ListIterator<Int>` (from list.iter())
- `MapIterator<ListIterator<Int>, Int>` (wrapping the ListIterator + closure)
- `FilterIterator<MapIterator<...>, Int>` (wrapping the MapIterator + closure)

Each struct's `next()` calls the inner iterator's `next()` and applies the transformation.

**Why:** This is how Rust, C++, and every compiled language with monomorphized iterators works. The monomorphization pass already generates specialized code per type, so each combinator chain produces a unique, fully-inlined call sequence. No heap allocation for the iterator pipeline itself (closures may still capture to heap, but the iterator structs are stack-allocated or register-promoted).

**Mesh-specific consideration:** Mesh's GC manages all heap objects. Iterator structs will be GC-allocated (like all Mesh structs) but are typically short-lived and collected quickly. The existing mark-sweep GC handles this fine.

**Alternative considered:** Implementing combinators as built-in runtime functions (like current List.map). Rejected because this defeats laziness -- each step would materialize an intermediate list. The whole point of the iterator protocol is lazy, fused evaluation.

**Confidence:** HIGH -- this is the standard compiled-language approach.

### 6. From/Into: Synthetic Impl Generation (Not Blanket Impls)

**Decision:** Implement From/Into as:
```
interface From<T> do
  fn from(value :: T) :: Self
end

interface Into<T> do
  fn into(self) :: T
end
```

When the user writes `impl From<String> for Int`, the compiler automatically generates the reverse `impl Into<Int> for String`. This is a synthetic impl, generated during trait registration.

**Why:** Mesh does not support blanket impls (overlapping impls are out of scope per PROJECT.md). Instead of the Rust approach (`impl<T, U> Into<U> for T where U: From<T>`), Mesh synthesizes concrete Into impls from each From impl. This is simpler and avoids the need for a blanket impl system.

**Coherence:** Since Mesh generates the Into impl deterministically from each From impl, there is no coherence concern. The existing `TraitRegistry::register_impl` duplicate detection catches any conflicts.

**Integration with ? operator:** Currently, `?` desugars Result/Option with identical error types. With From/Into, `?` can convert error types: if the function returns `Result<T, TargetError>` and the expression is `Result<T, SourceError>`, the Err arm calls `From.from(err)` to convert. This extends the existing `lower_try_result` in MIR lowering.

**Confidence:** MEDIUM -- synthetic impl generation is non-standard but simpler than blanket impls. The ? operator integration adds complexity to the existing desugaring.

### 7. Numeric Traits: Extend Existing Operator Dispatch

**Decision:** The existing Add/Sub/Mul/Div/Mod traits (already in builtins.rs) become the user-facing numeric traits. Add Neg for unary minus. Users can `impl Add for MyType` to enable `+` on their types.

**What changes:** Currently, `infer_trait_binary_op` checks `trait_registry.has_impl(trait_name, &resolved)` and returns the resolved type. This already works for user types. The key addition is:
1. Making the existing compiler-known Add/Sub/Mul/Div traits user-implementable (they already are -- users just haven't had this documented/tested)
2. Adding Neg trait for unary `-` dispatch (currently unary minus returns operand_ty without trait check)
3. Verifying that `resolve_trait_callee` in MIR correctly mangles user-defined numeric trait impls

**Simplification decision:** Keep Mesh's current same-type constraint for arithmetic (`a + b` requires a and b to be the same type). This avoids mixed-type arithmetic complexity while still enabling `impl Add for Vector2D`.

**Why:** The infrastructure is already 90% there. The existing `register_compiler_known_traits` registers Add/Sub/Mul/Div/Mod with impls for Int and Float. The `infer_trait_binary_op` function already dispatches through the trait registry. The `resolve_trait_callee` in MIR already mangles to `Add__add__TypeName`. Adding user-defined impls is extending what exists, not building new.

**Confidence:** HIGH -- builds directly on existing infrastructure with minimal new code.

### 8. Collect Trait: Type-Directed Materialization

**Decision:** Define Collect as:
```
interface Collect do
  type Item
  type Output
  fn from_iter(iter) :: Self.Output
end
```

Usage: `iter.collect()` where the target type is inferred from context (let binding annotation or return type).

**Implementation approach:** The `collect()` method on Iterator calls `Collect.from_iter(self)`. The type checker resolves which Collect impl to use based on the expected return type. Built-in impls provided for:
- Collect for List -- produces `List<T>` from any `Iterator<Item=T>`
- Collect for Map -- produces `Map<K,V>` from `Iterator<Item=(K,V)>`
- Collect for Set -- produces `Set<T>` from `Iterator<Item=T>`
- Collect for String -- produces String from `Iterator<Item=String>` (join)

**Type inference integration:** `collect()` returns a type determined by context. The type checker uses the target type annotation to select the appropriate Collect impl. Example:
```
let result :: List<Int> = numbers.iter().map(fn x -> x * 2 end).collect()
```
The `List<Int>` annotation drives inference, selecting the List collect impl.

**Confidence:** MEDIUM -- type-directed dispatch through projection normalization is the right approach but requires the associated types infrastructure to be solid first.

## Algorithms and Data Structures Needed

### New in mesh-typeck

| Algorithm/DS | Purpose | Complexity |
|-------------|---------|------------|
| Projection normalization | Resolve `Self.Item` to concrete type | O(1) per lookup via TraitRegistry |
| Deferred projection constraints | Queue unresolved projections for later resolution | Vec<(Ty, Ty)> on InferCtx |
| Associated type storage in TraitDef | Store `type Item` declarations | FxHashMap<String, Option<Ty>> on TraitDef |
| Associated type storage in ImplDef | Store `type Item = ConcreteType` | FxHashMap<String, Ty> on ImplDef |
| Synthetic Into impl generation | Auto-generate Into from From | Deterministic rewrite at registration time |

### New in mesh-codegen

| Algorithm/DS | Purpose | Complexity |
|-------------|---------|------------|
| Iterator struct generation | Create MapIterator, FilterIterator, etc. | Monomorphization produces structs per chain |
| For-in desugaring to Iterable | Rewrite for-in to iter()+next() loop | Replaces indexed iteration in lower_for_in_expr |
| Collect dispatch in MIR | Route collect() to correct from_iter impl | resolve_trait_callee with projection resolution |
| From-based ? error conversion | Extend lower_try_expr with From::from call | Additional match arm in Err case |

### New in mesh-rt

| Function | Purpose | Signature |
|----------|---------|-----------|
| mesh_list_iter | Create ListIterator from List | (list_ptr) -> iter_ptr |
| mesh_map_iter | Create MapIterator from Map | (map_ptr) -> iter_ptr |
| mesh_set_iter | Create SetIterator from Set | (set_ptr) -> iter_ptr |
| mesh_range_iter | Create RangeIterator from Range | (start, end) -> iter_ptr |
| mesh_iter_next_* | Advance specific iterator type | (iter_ptr) -> option_ptr |

## Alternatives Considered

| Category | Recommended | Alternative | Why Not |
|----------|-------------|-------------|---------|
| Associated type representation | Ty::Projection variant | External lookup table | Projection variant is self-contained, works with existing resolve/unify |
| Projection resolution | Deferred constraints | Eager resolution | Fails for generic code where Self is still a type variable |
| Iterator design | Two-trait (Iterator + Iterable) | Single-trait Iterable | Cannot distinguish stateful iterators from iterable collections |
| Lazy evaluation | Struct-based state machines | Generator/coroutine-based | Struct approach needs no new runtime machinery, monomorphizes cleanly |
| From/Into blanket impl | Synthetic concrete impls | Full blanket impl system | Blanket impls require specialization/negative-reasoning; out of scope |
| Collect dispatch | Type-directed via context | Explicit target type parameter | Context-driven approach gives better ergonomics |
| Numeric traits | Extend existing Add/Sub/Mul/Div | New separate numeric trait hierarchy | Existing traits already work; just need user-facing exposure |

## Sources

- [Rust Chalk: Type Equality and Unification](https://rust-lang.github.io/chalk/book/clauses/type_equality.html) -- projection normalization algorithm (HIGH confidence)
- [Niko Matsakis: Unification in Chalk Part 2](https://smallcultfollowing.com/babysteps/blog/2017/04/23/unification-in-chalk-part-2/) -- deferred projection constraints (HIGH confidence)
- [Rust Compiler Dev Guide: Trait Resolution](https://rustc-dev-guide.rust-lang.org/traits/resolution.html) -- candidate assembly for associated types (HIGH confidence)
- [Rust Compiler Dev Guide: Type Inference](https://rustc-dev-guide.rust-lang.org/type-inference.html) -- InferCtxt and union-find integration (HIGH confidence)
- [Rust IntoIterator PR #20790](https://github.com/rust-lang/rust/pull/20790) -- for-loop desugaring design (HIGH confidence)
- [Rust RFC 0195: Associated Items](https://rust-lang.github.io/rfcs/0195-associated-items.html) -- original associated types design (HIGH confidence)
- [Rust RFC 0235: Collections Conventions](https://rust-lang.github.io/rfcs/0235-collections-conventions.html) -- collect/from_iter patterns (HIGH confidence)
- [C# Iterator Block State Machines](https://csharpindepth.com/articles/IteratorBlockImplementation) -- state machine compilation pattern (MEDIUM confidence)
- [Swift Associated Type Inference](https://forums.swift.org/t/recent-improvements-to-associated-type-inference/70265) -- practical challenges (MEDIUM confidence)
- [Rust Coherence and Orphan Rules](https://ohadravid.github.io/posts/2023-05-coherence-and-errors/) -- From/Into coherence patterns (HIGH confidence)
