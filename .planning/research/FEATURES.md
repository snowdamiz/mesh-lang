# Feature Landscape

**Domain:** Trait ecosystem expansion for the Mesh programming language -- associated types, iterator protocol, From/Into conversions, numeric traits with Output, and Collect.
**Researched:** 2026-02-13
**Confidence:** HIGH for associated types, iterator, numeric traits, From/Into. MEDIUM for Collect (type inference for target collection is the hardest unsolved problem).

## Existing System Baseline

Before defining features, here is what Mesh already has (verified from codebase inspection):

- **Trait system:** `interface` definitions, `impl ... for ... do ... end` blocks, where clauses (`where T: Trait`), structural type matching via unification, default method bodies, `deriving(Eq, Ord, Display, Debug, Hash, Json, Row)`
- **Compiler-known traits:** Add, Sub, Mul, Div, Mod (Int + Float only), Eq, Ord, Not, Display, Debug, Hash, Default
- **TraitDef / ImplDef:** Methods have `has_self`, `param_count`, `return_type: Option<Ty>`, `has_default_body`. TraitDef has `methods: Vec<TraitMethodSig>` but no associated type declarations. ImplDef stores `methods: FxHashMap<String, ImplMethodSig>` but no associated type bindings.
- **Type system:** HM inference with `Ty::Var`, `Ty::Con`, `Ty::App`, `Ty::Fun`, `Ty::Tuple`, `Ty::Never`. Generic type params use single uppercase letter convention (freshened to fresh `Ty::Var` during impl lookup). Polymorphic schemes via `Scheme { vars, ty }`.
- **For-in loops:** Hardcoded per-collection-type lowering: `ForInRange`, `ForInList`, `ForInMap`, `ForInSet` -- each a separate MIR node with dedicated codegen. Not extensible to user types.
- **Collections:** List (polymorphic via `Ty::App`), Map (polymorphic), Set (monomorphic Int-only), Range, Queue. Eagerly evaluated. Pipe-based `List.map/filter/reduce` are eager functions returning new Lists.
- **Operator dispatch:** Binary operators check trait impls in the trait registry. Return type convention: `return_type: None` on ImplMethodSig means "Self" (the implementing type). No `Output` associated type.
- **Monomorphization:** The MIR pass (`mono.rs`) does reachability-based dead code elimination. Generic functions are monomorphized by the type checker resolving all types before MIR lowering.

---

## Table Stakes

Features users expect from a language with these capabilities. Missing = the trait system feels incomplete or broken.

### 1. Associated Types: Declaration, Specification, and Resolution

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| `type Item` declaration inside `interface` | Every modern trait-based language has this (Rust, Swift, Scala). Without it, Iterator and Collect cannot express their element type. | **High** | Parser: new `type Name` AST node inside INTERFACE_DEF. Type checker: new `AssociatedTypeDef` in TraitDef. Must validate impls provide all required assoc types. |
| `type Item = ConcreteType` in `impl` blocks | Implementors must specify what the associated type resolves to. | **High** | Parser: new `type Name = TypeAnnotation` AST node inside IMPL_DEF. ImplDef must store `assoc_types: FxHashMap<String, Ty>`. |
| `Self.Item` references in method signatures | Methods must reference associated types. `fn next(self) -> Option<Self.Item>` is the canonical example. | **High** | Requires a new `Ty::Projection(Box<Ty>, String)` variant or resolution during trait method type-checking. When checking a method in an impl block, `Self.Item` resolves to the concrete binding. |
| Projection normalization during type inference | When calling `iter.next()` on a value of type `ListIterator`, the type checker must resolve `<ListIterator as Iterator>::Item` to the concrete type (e.g., `Int`). | **High** | This is the hardest part. The trait registry's `find_impl` already does structural matching. Extend it to extract associated type bindings from the matched impl. |
| Duplicate impl detection still works | Associated types must not break the existing overlap checking (tested in `traits.rs` tests). | **Low** | Already works -- structural overlap checking via temporary unification is independent of associated types. |

**Recommended Mesh syntax** (matches existing Elixir-like style):
```
interface Iterator do
  type Item
  fn next(self) -> Option<Self.Item>
end

impl Iterator for ListIter<T> do
  type Item = T
  fn next(self) -> Option<T> do
    # ...
  end
end
```

**Design note -- `Self.Item` vs `Self::Item`:** Mesh uses `.` for field access and module qualification (e.g., `point.x`, `List.map`). Using `Self.Item` is consistent with existing syntax. Rust uses `::` but Mesh has no `::` path separator -- it uses `.` everywhere. `Self.Item` reads naturally.

**Confidence: HIGH** -- Rust, Swift, and Scala all converge on this pattern. The semantics are well-understood from decades of type theory research on type families.

### 2. Iterator Protocol: Trait with `next` + Lazy Combinators

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| `Iterator` trait with `type Item` + `fn next(self) -> Option<Self.Item>` | Universal iterator interface. Rust, Python, Java, C#, Scala all have one. Users need a single protocol for for-in and pipe chains. | **High** | Requires associated types. Must define the trait and register it as compiler-known. |
| `Iterable` trait with `fn iter(self) -> I where I: Iterator` | Separates "things that can produce an iterator" from "the iterator itself". Equivalent to Rust's `IntoIterator`. Enables for-in desugaring. | **Med** | A simpler alternative: skip `Iterable` entirely and have `for x in expr` check if expr's type implements `Iterator` or has a known `Iter.from()` conversion. |
| `for x in collection` works via protocol | For-in must desugar to Iterator method calls rather than hardcoded per-type MIR nodes. | **High** | Currently ForInList/ForInMap/ForInSet/ForInRange are separate MIR nodes. Must generalize to a single Iterator-based lowering, OR add an Iterator-based fallback path that activates when the type is not a known builtin. |
| Built-in `impl Iterator` for List, Map, Set, Range | All existing iterable types must work with the new protocol. | **Med** | Registration in builtins.rs. Runtime support: per-element `next()` functions for each collection type. List needs index-based cursor; Map needs entry iteration; Set/Range need position tracking. |
| Iterator combinators: `map`, `filter`, `take`, `skip`, `enumerate`, `zip` | Users writing `iter |> Iter.map(fn) |> Iter.filter(fn)` expect lazy composition. | **High** | Each combinator wraps the source iterator in an adapter struct (MapIter, FilterIter, etc.). Each adapter implements Iterator. This means the compiler must handle iterator adapter types through monomorphization. |
| Terminal operations: `count`, `sum`, `any`, `all`, `find`, `reduce` | Standard consuming operations that drain an iterator to produce a value. | **Med** | Default methods on the Iterator trait. Each calls `next()` in a loop. |

**Key design decision: Lazy vs Eager.**

Mesh already has eager `List.map`, `List.filter`, etc. The Iterator protocol should be **lazy** (computed on-demand), matching Rust's approach. This gives Mesh both paradigms:
- **Eager (existing):** `list |> List.map(fn)` -- returns a new List immediately
- **Lazy (new):** `list |> Iter.from() |> Iter.map(fn) |> Iter.filter(fn) |> Iter.collect()` -- deferred computation

This dual approach follows Scala's `collection.map()` (eager) vs `collection.view.map()` (lazy) pattern. Research shows keeping both is the ideal for debuggability (eager is simpler to trace) and performance (lazy avoids intermediate allocations).

**Design decision: Iterable vs Iter.from().**

Two options for the "convert to iterator" step:

| Approach | Pros | Cons |
|----------|------|------|
| `Iterable` trait (like Rust's `IntoIterator`) | Compiler can desugar `for x in expr` uniformly; type-safe | Another trait to learn; needs associated type for the iterator type |
| `Iter.from()` module function | Pipe-friendly; no extra trait; simple | Less type-safe; requires per-type dispatch in the function |

**Recommendation:** Use `Iterable` trait with `type Iter` associated type for for-in desugaring (compiler-internal), but also provide `Iter.from()` as a user-facing convenience function for pipe chains. This mirrors Rust's approach where `for` uses `IntoIterator` implicitly but users can also call `.into_iter()` explicitly.

**Confidence: HIGH** -- This is the standard approach across all researched languages.

### 3. From/Into Conversion Traits

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| `From<T>` trait: `fn from(value :: T) -> Self` | Standard conversion interface. Rust, Scala, Kotlin all have this pattern. Enables `Float.from(42)` syntax. | **Med** | Requires generic type parameters on traits (Mesh already supports `<T>` on interfaces). Define as a normal interface. |
| Automatic `Into` from `From` | If `impl From<Int> for Float` exists, then `42 |> into()` should produce a Float when context demands it. | **Med** | Requires either blanket impls (not in Mesh) or compiler-special-cased synthetic impl generation. The pragmatic path: when `From<A> for B` is registered, automatically register `Into<B> for A`. |
| `From` for common primitive conversions | `Int -> Float`, `Int -> String`, `Float -> String`, `Bool -> String`, `Bool -> Int` | **Low** | Just impl registrations in builtins.rs + runtime intrinsics. |
| `From`-based error conversion in `?` operator | `result?` should auto-convert error types via From. If function returns `Result<T, AppError>` and expression returns `Result<T, IoError>`, and `From<IoError> for AppError` exists, `?` inserts the conversion. | **Med** | Extends existing `lower_try_expr` in lower.rs. |

**Important design note:** `From` uses a **generic type parameter** (not an associated type) because you can implement `From<Int> for String` AND `From<Float> for String` -- multiple impls per implementing type, varying on the source type. This matches Rust's design: associated types = unique output per type, generics = multiple inputs per type.

**Recommended Mesh syntax:**
```
interface From<T> do
  fn from(value :: T) -> Self
end

impl From<Int> for Float do
  fn from(value :: Int) -> Float do
    # compiler intrinsic
  end
end

# Usage:
let f = Float.from(42)
let f2: Float = 42 |> into()
```

**Confidence: HIGH** -- Rust's design is well-proven and maps cleanly to Mesh's existing trait system.

### 4. Numeric Traits with `Output` Associated Type

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| `type Output` associated type on Add/Sub/Mul/Div/Mod | Currently these traits use `return_type: None` to mean "Self". Real `Output` allows `Matrix * Vector = Vector` (different output type than either operand). | **Med** | Requires associated types to exist first. Then refactor existing trait defs in `register_compiler_known_traits()`. |
| User-defined `impl Add for MyType` with custom Output | `impl Add for Vector2D do type Output = Vector2D; fn add(self, other) do ... end end` | **Low** | Already partially works -- operator dispatch checks the trait registry. Just needs Output resolution. |
| Default `RHS = Self` type parameter on arithmetic traits | So `impl Add for Int` means `Int + Int` without specifying RHS. | **Med** | Requires default type parameters on generic traits. Could defer this: just have `Add` take no generic param, always operating on `Self + Self`. |
| Binary operator inference uses Output type | When checking `a + b`, the result type should be `<typeof(a) as Add>::Output`, not hardcoded to typeof(a). | **Med** | Update `infer_binary_op` to query the trait registry for the Output associated type binding. |
| `Neg` trait for unary minus | `impl Neg for Vector` enables `-vector`. Currently Not trait handles `!`, but unary `-` is hardcoded. | **Low** | New trait registration + update `infer_unary_op`. |

**Current state to refactor:** In `register_compiler_known_traits()`, Add is registered as:
```rust
TraitMethodSig { name: "add", return_type: None, ... }  // None = Self
```
This must become:
```rust
TraitDef { assoc_types: [("Output", ...)], methods: [...] }
ImplDef { assoc_type_bindings: {"Output": Ty::int()}, ... }
```

**Confidence: HIGH** -- Rust's `Add<RHS=Self> { type Output; }` is the gold standard for this pattern.

### 5. Collect Trait (FromIterator)

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| `Collect` (or `FromIterator`) trait | The counterpart to Iterator. Turns a lazy iterator back into a concrete collection. Essential for `iter |> Iter.map(fn) |> Iter.collect()` pipeline. | **High** | Requires associated types and Iterator to exist. Also needs type inference to determine target collection. |
| `impl Collect for List<T>` | Lists must be collectible from iterators. | **Med** | Runtime function that builds a List by calling `next()` in a loop. |
| `impl Collect for Map<K,V>` from Iterator of tuples | Maps collectible from `Iterator<Item = (K, V)>`. | **Med** | Needs tuple element type extraction. |
| `impl Collect for Set<T>` | Sets collectible from iterators. | **Med** | Similar to List. |
| Type-directed collect | `let xs: List<Int> = iter |> Iter.collect()` -- the return type annotation tells `collect()` which impl to use. | **High** | Mesh's HM inference propagates type constraints forward during unification. If the let binding has a type annotation, the expected type flows to `collect()`. This should work with existing HM machinery. |

**Recommended Mesh syntax:**
```
interface Collect<A> do
  fn collect(iter :: Iterator) -> Self
end

impl Collect<T> for List<T> do
  fn collect(iter :: Iterator) -> List<T> do
    # builds list from repeated next() calls
  end
end

# Usage (type annotation drives dispatch):
let doubled: List<Int> = [1, 2, 3]
  |> Iter.from()
  |> Iter.map(fn(x) -> x * 2 end)
  |> Iter.collect()

# Alternative: explicit turbofish-style
let doubled = [1, 2, 3]
  |> Iter.from()
  |> Iter.map(fn(x) -> x * 2 end)
  |> List.collect()       # Module-qualified makes target explicit
```

**Design note:** The `List.collect()` module-function approach sidesteps the type inference problem entirely. If `Iter.collect()` is ambiguous, `List.collect()` is unambiguous. Both should be supported.

**Confidence: MEDIUM** -- The trait is straightforward, but type inference for "what collection should `collect()` produce?" is genuinely hard. Rust requires turbofish or annotations. Mesh should follow suit.

### 6. Backward Compatibility with Existing For-In

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| Existing `for x in [1,2,3]` still works | Breaking existing programs is unacceptable. | **High** | Dual-path lowering: if the iterable type is List/Map/Set/Range, use existing optimized ForInList/ForInMap/ForInSet/ForInRange MIR nodes. If the type implements Iterator/Iterable, use the new protocol-based path. |
| Existing eager List.map/filter/reduce unchanged | These must continue to work. The lazy iterator is an addition, not a replacement. | **Low** | No changes needed to existing functions. |

**Confidence: HIGH** -- keeping existing fast paths is standard engineering practice.

---

## Differentiators

Features that set Mesh apart. Not expected by every user, but signal quality and thoughtful design.

| Feature | Value Proposition | Complexity | Notes |
|---------|-------------------|------------|-------|
| Pipe-friendly `Iter.from()` entry point | Cleaner than Rust's `.iter()`. Fits Mesh idiom: `list |> Iter.from() |> Iter.map(fn) |> Iter.collect()` | **Low** | Module function. |
| Both eager and lazy paths in same language | Most languages pick one. Mesh keeps both: `List.map(fn)` (eager, simple) and `Iter.map(fn)` (lazy, composable). | **Low** | No extra work -- just documentation. |
| `From`-based `?` error conversion | Elixir-like ergonomics with Rust-like type safety. `result?` with automatic error type adaptation. | **Med** | Extends existing try expression lowering. |
| `Neg` trait for unary minus overloading | Enables mathematical DSLs: `let neg_v = -vector`. | **Low** | Simple trait + dispatch. |
| `Zero` and `One` traits | `fn zero() -> Self`, `fn one() -> Self`. Essential for generic sum/product. | **Low** | Static trait methods, similar to existing Default. |
| `collect()` to String from char/string iterators | `["hello", " ", "world"] |> Iter.from() |> Iter.collect()` produces `"hello world"`. | **Low** | Collect impl for String. |

---

## Anti-Features

Features to explicitly NOT build in this milestone.

| Anti-Feature | Why Avoid | What to Do Instead |
|--------------|-----------|-------------------|
| **Higher-kinded types (HKTs)** | Enormous complexity. Mesh's monomorphization does not support type constructors as parameters. Would require fundamental type system redesign. | Use concrete adapter structs for iterator combinators (MapIter, FilterIter, etc.). |
| **Generic Associated Types (GATs)** | Rust needed years to stabilize GATs. They enable lending iterators and streaming patterns but add massive inference complexity. | Not needed for basic Iterator/Collect. Defer indefinitely. |
| **Trait inheritance / supertraits** | `interface Ord: Eq do ... end` adds complexity to trait resolution. Where clauses provide equivalent constraint power. | Use multiple where clause bounds: `where T: Eq, T: Ord`. |
| **Blanket impls (user-defined)** | `impl<T: Display> ToString for T` creates coherence problems and makes the trait registry much harder to reason about. | Compiler-special-case Into generation from From. Do not expose general blanket impls. |
| **Lazy iterator fusion** | Fusing chained iterators into single loops is a complex optimization pass. | Straightforward `next()` call chains. Optimize in a future milestone. |
| **`TryFrom` / `TryInto`** | Fallible conversions returning `Result`. Useful but not blocking for this milestone. | Users write functions returning `Result` manually. |
| **Full bidirectional type inference** | Rust-level collect inference from return position requires bidirectional checking beyond HM. | Require annotations on `collect()` or use module-qualified `List.collect()`. |
| **Infinite iterators** | `Iter.repeat(x)`, `Iter.count()`, etc. | Defer to future milestone. |
| **Dynamic dispatch / trait objects** | Mesh uses monomorphization only, per project philosophy. | Sum types for runtime polymorphism. |
| **Async iterators / streams** | Mesh uses actors for concurrency, not async/await. | Actor message streams are the Mesh idiom. |
| **Mixed-type arithmetic (Int + Float -> Float)** | Adds significant inference complexity -- which type "wins"? | Explicit conversion: `Float.from(x) + y`. |
| **Specialization / overlapping impls** | Unsound without careful design. One impl per type per trait is Mesh's coherence model. | Keep strict coherence. |
| **Mutable iterators** | Mesh has no mutable references; all values are GC-managed immutable. | Collect to new collection after transformation. |

---

## Feature Dependencies

```
Associated Types (type Item, Self.Item, projection resolution)
  |
  +-> Iterator trait (uses type Item in next() return type)
  |     |
  |     +-> Iterable trait (uses type Iter :: Iterator as associated type)
  |     |     |
  |     |     +-> For-in desugaring (calls Iterable.iter() then Iterator.next())
  |     |     |
  |     |     +-> Built-in Iterable impls (List, Map, Set, Range)
  |     |
  |     +-> Lazy combinators (map, filter, take, skip, enumerate, zip)
  |     |     |
  |     |     +-> Collect trait (materializes lazy pipeline to concrete collection)
  |     |
  |     +-> Terminal operations (count, sum, any, all, find, reduce)
  |
  +-> Numeric Output type (Add { type Output }, Sub { type Output }, etc.)
  |     |
  |     +-> Operator dispatch refactor (infer_binary_op uses Output)
  |     |
  |     +-> Neg trait (unary minus)
  |
  +-> From/Into traits (independent of associated types -- uses generic param)
        |
        +-> Synthetic Into generation from From
        |
        +-> ? operator error conversion via From
```

**Key ordering insight:** From/Into does NOT depend on associated types (it uses a generic type parameter, not an associated type). It can be built in parallel with associated types. However, Collect depends on both Iterator AND associated types, so it must come last.

---

## MVP Recommendation

Prioritize in dependency order:

1. **Associated types** (type declarations in interface/impl, Self.Item resolution, projection normalization) -- unlocks everything else. This is the foundation.

2. **Iterator + Iterable traits** (core protocol with next(), built-in impls for List/Map/Set/Range) -- highest-impact user-facing feature after associated types.

3. **For-in desugaring via Iterator** (dual-path: keep existing fast paths, add Iterator fallback) -- makes the protocol immediately useful.

4. **Numeric traits with Output** (refactor Add/Sub/Mul/Div/Mod to use type Output, add Neg) -- relatively low effort once associated types exist. High impact for user-defined types.

5. **From/Into traits** (conversion protocol + synthetic Into + ? operator integration) -- can be built in parallel with steps 2-4.

6. **Lazy combinators** (map, filter, take, skip, enumerate, zip on Iterator) -- enables pipe-style lazy composition.

7. **Collect trait** (materializes lazy pipelines to List/Map/Set/String) -- the capstone that completes the pipeline.

**Defer to post-milestone:**
- Terminal operations beyond basic (sum, count) -- trivial additions once Iterator exists
- Additional combinators (flat_map, chain, skip_while, take_while) -- incremental
- Bounded associated types (`type Iter: Iterator`) -- nice constraint, not blocking
- Zero/One traits -- simple but lower priority
- Infinite iterators -- out of scope

## Sources

- [Rust Advanced Traits: Associated Types](https://doc.rust-lang.org/book/ch20-02-advanced-traits.html) -- HIGH confidence
- [Rust Associated Types by Example](https://doc.rust-lang.org/rust-by-example/generics/assoc_items/types.html) -- HIGH confidence
- [Rust RFC 0195: Associated Items](https://rust-lang.github.io/rfcs/0195-associated-items.html) -- HIGH confidence
- [Rust Iterator Trait](https://doc.rust-lang.org/std/iter/trait.Iterator.html) -- HIGH confidence
- [Rust IntoIterator Trait](https://doc.rust-lang.org/std/iter/trait.IntoIterator.html) -- HIGH confidence
- [Rust FromIterator Trait](https://doc.rust-lang.org/std/iter/trait.FromIterator.html) -- HIGH confidence
- [Rust From/Into Traits](https://doc.rust-lang.org/rust-by-example/conversion/from_into.html) -- HIGH confidence
- [Rust std::ops Traits](https://doc.rust-lang.org/std/ops/index.html) -- HIGH confidence
- [Rust RFC 0235: Collections Conventions](https://rust-lang.github.io/rfcs/0235-collections-conventions.html) -- HIGH confidence
- [Haskell Foldable and Traversable](https://wiki.haskell.org/Foldable_and_Traversable) -- HIGH confidence
- [Haskell mono-traversable (type families for monomorphic iteration)](https://hackage.haskell.org/package/mono-traversable) -- HIGH confidence
- [Typeclasses in Haskell, Scala, and Rust](https://gist.github.com/DarinM223/f6adf64569b55408886313cd3032c7e6) -- MEDIUM confidence
- [Collect in Rust, traverse in Haskell and Scala](https://academy.fpblock.com/blog/collect-rust-traverse-haskell-scala/) -- MEDIUM confidence
- [Lazy vs Eager Evaluation Tradeoffs](https://brontowise.com/2025/06/27/lazy-evaluation-vs-eager-evaluation-compute-now-or-compute-when-needed/) -- MEDIUM confidence
- [Stanford CS 242: Traits](https://stanford-cs242.github.io/f19/lectures/07-1-traits.html) -- HIGH confidence
- Mesh codebase: `traits.rs`, `builtins.rs`, `ty.rs`, `mir/types.rs`, `mir/lower.rs`, `codegen/expr.rs` -- verified directly
