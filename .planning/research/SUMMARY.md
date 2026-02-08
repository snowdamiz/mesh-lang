# Project Research Summary

**Project:** Snow Programming Language -- v1.3 Traits & Protocols
**Domain:** Compiler trait system completion, monomorphization, stdlib protocol implementation
**Researched:** 2026-02-07
**Confidence:** HIGH

## Executive Summary

Snow v1.3 completes the trait/protocol system for a statically-typed, LLVM-compiled language with Elixir-style syntax. The existing compiler pipeline already handles parsing of `interface`/`impl` blocks, type-checks trait method signatures via `TraitRegistry`, and enforces where-clause constraints at call sites. However, trait methods currently produce no executable code -- codegen explicitly skips `InterfaceDef` and `ImplDef`, making traits purely decorative. The v1.3 milestone bridges this gap by wiring trait method dispatch through MIR lowering via monomorphization (static dispatch, no vtables), implementing stdlib protocols (Display, Debug, Eq, Ord, Hash, Default), and extending the existing compiler-known trait pattern to user-defined traits.

The recommended approach is to treat MIR lowering as the critical integration point. The type checker already resolves concrete types at every call site via Hindley-Milner inference, so trait dispatch reduces to name mangling at the MIR level: `to_string(42)` becomes `Call("Display__to_string__Int")`. No new Rust crate dependencies are needed -- the entire implementation is internal compiler work using the existing Inkwell/LLVM, ena, and rustc-hash stack. The name mangling convention (`Trait__Method__Type` with double-underscore separators) extends the existing `mangle_type_name` infrastructure. By the time code reaches LLVM codegen, trait method calls are indistinguishable from regular function calls.

The primary risks are: (1) the `type_to_key` string-based impl lookup cannot handle generic impls like `impl Display for List<T>`, requiring a rewrite to structural type matching before any parameterized type impls work; (2) the codegen gap where impl method bodies are type-checked but never lowered to MIR, which is the single biggest blocker; and (3) method name collision between traits sharing method names (e.g., Display and Debug both defining `to_string`), which currently resolves nondeterministically via HashMap iteration order. All three must be addressed in Phase 1 before any downstream protocol work begins.

## Key Findings

### Recommended Stack

Zero new Rust crate dependencies are needed for v1.3. The existing workspace dependencies (Inkwell 0.8.0, ena 0.14, rustc-hash 2, rowan 0.16, ariadne 0.6, serde_json 1) are all at their latest compatible versions and require no changes. This is the correct outcome -- trait systems are core compiler semantics, not library-shaped problems. Monomorphized trait methods compile to regular LLVM functions; the runtime needs only leaf functions for primitive type operations (int-to-string, hash, etc.).

**Core technologies (no changes):**
- **Inkwell 0.8.0 (LLVM 21):** LLVM IR generation -- monomorphized trait methods are regular LLVM functions, no changes needed
- **ena 0.14:** Union-find for type unification -- trait resolution uses TraitRegistry, not unification directly
- **rustc-hash 2:** FxHashMap for compiler internals -- already used throughout, extends naturally to new registries
- **serde_json 1:** Already in snow-rt -- reused for Serialize/Deserialize protocol runtime (deferred to v1.4+)

**New internal components (no external deps):**
- **FNV-1a hash (~30 lines in snow-rt):** Deterministic, platform-independent hashing for the Hash protocol. FNV-1a beats SipHash on short keys (ints, small strings) and avoids platform-dependent AES instructions (ahash). No crate needed.
- **TraitRegistry extensions:** Default methods, `impls_for_type`/`impls_for_trait` queries, structural type matching to replace string-based `type_to_key`
- **MIR trait dispatch:** `Trait__Method__Type` name mangling, trait method call resolution in `lower_call_expr`, impl method body lowering

### Expected Features

**Must have (table stakes -- users cannot write polymorphic code without these):**
- User-defined traits with full codegen (interface/impl compile to native binaries)
- Trait method dispatch via monomorphization (static, no vtables)
- Where clause enforcement threaded through to codegen
- Display protocol with string interpolation integration (`"${value}"` calls `to_string`)
- Debug protocol for developer inspection (`inspect(value)` shows structure)
- Eq extended to structs and sum types (structural equality)
- Ord extended to structs and sum types (with new Ordering sum type)
- Impl for user-defined structs and sum types (end-to-end)
- Default method implementations in traits

**Should have (differentiators -- competitive advantage):**
- Hash protocol for user types as Map keys/Set elements
- Default protocol for zero-initialization
- Auto-derive mechanism (`deriving(Eq, Ord, Display, Debug, Hash)`)
- Coherent auto-impls for all primitives (Display, Debug, Eq, Ord, Hash, Default)
- Display/Debug for stdlib collections (List, Map, Set)

**Defer to v1.4+:**
- Iterator/Iterable protocol (requires associated types or type parameters on traits, plus lazy evaluation infrastructure)
- From/Into conversion protocol (blanket impl `From implies Into` requires infrastructure that does not exist)
- Supertraits / trait inheritance
- Method dot-syntax (`value.method()` as alternative to `method(value)`)
- Serialize/Deserialize protocols
- TryFrom/TryInto (fallible conversions)
- Blanket impls

**Never build:**
- Dynamic dispatch / vtables / trait objects (use sum types instead)
- Higher-kinded types (Functor/Monad hierarchy)
- Multi-parameter type classes
- Specialization (overlapping impls)
- Implicit trait resolution (Scala 2-style)

### Architecture Approach

The trait system integrates into the existing 6-stage pipeline (lexer -> parser -> typeck -> MIR lowering -> mono pass -> codegen) with changes concentrated in MIR lowering (major), typeck (moderate), and codegen (minimal). The key architectural insight is that MIR lowering is the natural resolution point: HM inference already produces concrete types at every expression, so the lowerer can look up the concrete argument type in TraitRegistry and emit a direct call to the mangled function name. By the time code reaches LLVM codegen, there is no concept of "trait dispatch" -- only concrete function calls.

**Major components and responsibilities:**

1. **snow-typeck/traits.rs (TraitRegistry)** -- Trait/impl data model, validation, impl lookup. Must be extended with structural type matching (replacing `type_to_key` string keys), duplicate impl detection, and query methods for codegen.
2. **snow-typeck/builtins.rs** -- Registration of stdlib protocols (Display, Debug, Hash, Default) alongside existing compiler-known traits (Add, Eq, Ord). Same pattern: TraitDef + ImplDef for primitives.
3. **snow-codegen/mir/lower.rs (MIR Lowering)** -- The critical integration point. Must lower impl method bodies to mangled MirFunctions, resolve trait method calls to mangled names at call sites, and handle `self` parameter as first argument with concrete type.
4. **snow-codegen/mir/mono.rs (Monomorphization)** -- Remains a reachability pass for v1.3. Mangled trait method names are discovered through normal call graph analysis. Full generic function specialization deferred.
5. **snow-codegen/codegen/ (LLVM Codegen)** -- No trait-specific changes needed. Monomorphized calls are regular `MirExpr::Call` with mangled function names.
6. **snow-rt (Runtime)** -- Leaf functions for built-in protocol impls: `snow_int_to_string`, `snow_string_hash`, `snow_int_default`, etc. Follows existing `#[no_mangle] extern "C"` pattern.

### Critical Pitfalls

1. **type_to_key string-based impl lookup (Pitfall 2)** -- The current string-key HashMap lookup cannot match generic impls (`impl Display for List<T>` vs query `List<Int>`). Must be replaced with structural type matching using temporary unification before any parameterized type impls work. This is foundational -- every downstream feature depends on it.

2. **Codegen gap: impl bodies never lowered (Pitfall 5)** -- MIR lowering explicitly skips `InterfaceDef` and `ImplDef` (line 431 of lower.rs). Impl method bodies are type-checked but produce no MIR functions. This is the single biggest blocker for v1.3. Without fixing this, traits have zero runtime effect.

3. **Method name collision between traits (Pitfall 4)** -- Two traits defining the same method name resolve nondeterministically via FxHashMap iteration order. Must detect ambiguity and either error or require qualified syntax (`Display.to_string(x)` vs `Debug.to_string(x)`). Critical because Display and Debug are near-certain to share method names.

4. **Duplicate impl silent overwrite (Pitfall 7)** -- `register_impl` uses HashMap insert which silently replaces previous entries. Two `impl Display for Int` blocks compile without error. One-line fix: check before insert.

5. **Compiler-known and user traits use different dispatch paths (Pitfall 12)** -- Binary operator inference has hardcoded Int/Float logic alongside TraitRegistry lookup. `impl Add for MyStruct` is registered but `my_a + my_b` still fails with "expected Int". Must unify dispatch before adding more protocols.

## Implications for Roadmap

Based on research, suggested phase structure:

### Phase 1: Trait Infrastructure (Type System Foundation)

**Rationale:** Every other phase depends on correct trait resolution. The type system must be fixed first: `type_to_key` rewrite, overlap detection, method collision handling, and dispatch path unification. Without this foundation, all downstream codegen and protocol work builds on broken ground.

**Delivers:** Correct trait/impl resolution for all type shapes, including parameterized types. Duplicate impl detection. Deterministic method dispatch. Unified compiler-known and user-trait dispatch paths.

**Addresses features:** None directly user-visible, but enables all of them.

**Avoids pitfalls:** 2 (type_to_key), 4 (method collision), 7 (duplicate overwrite), 12 (dispatch path divergence)

**Estimated effort:** 3-5 days

### Phase 2: Trait Method Codegen (MIR Lowering + Name Mangling)

**Rationale:** This is the primary v1.3 deliverable. Trait methods must produce executable code. Depends on Phase 1 for correct trait resolution. Concentrated in MIR lowering -- define the name mangling scheme, lower impl method bodies to MirFunctions, resolve trait method calls to mangled names, handle self parameter.

**Delivers:** User-defined traits compile and run end-to-end. `interface Greetable`, `impl Greetable for MyStruct`, and calls to trait methods produce correct native code. Monomorphization depth limit prevents infinite instantiation.

**Addresses features:** User-defined traits with codegen, trait method dispatch (static), where clause enforcement in codegen, impl for user-defined structs and sum types.

**Avoids pitfalls:** 5 (MIR gap), 13 (name mangling), 3 (infinite monomorphization)

**Estimated effort:** 5-8 days

### Phase 3: Essential Stdlib Protocols (Display, Debug, Eq, Ord)

**Rationale:** Once trait dispatch works, the most impactful protocols are Display (enables string interpolation for user types), Debug (enables developer inspection), and Eq/Ord extensions (enables == and < on user types). These are the protocols users will reach for immediately.

**Delivers:** `to_string(42)` works via Display, `"${my_struct}"` calls to_string, `inspect(value)` shows structure, `==` and `<` work on structs and sum types, new Ordering sum type for Ord.

**Addresses features:** Display protocol, Debug protocol, string interpolation integration, Eq for structs/sum types, Ord with Ordering type.

**Uses stack:** snow-rt runtime functions (`snow_int_to_string`, etc.), builtins.rs registration pattern.

**Avoids pitfalls:** 9 (Self type -- Display/Debug/Eq/Ord all avoid Self returns, sidestepping this issue)

**Estimated effort:** 5-7 days

### Phase 4: Extended Protocols (Hash, Default, Default Methods)

**Rationale:** Hash and Default are important for practical use (Map keys, zero-initialization) but are less critical than Display/Eq/Ord. Default methods reduce boilerplate in trait definitions. Default protocol requires static method support (no `self` parameter) which is a design decision point.

**Delivers:** User types as Map keys (Hash), zero-initialization (Default), default method implementations in traits, Display/Debug impls for collections.

**Addresses features:** Hash protocol, Default protocol, default method implementations, collection Display/Debug.

**Avoids pitfalls:** 9 (Self type -- needed for Default's `default() -> Self`; decide here whether to add `Ty::SelfType` or defer Default)

**Estimated effort:** 4-6 days

### Phase 5: Auto-Derive (Stretch Goal)

**Rationale:** `deriving(Eq, Ord, Display, Debug, Hash)` is the single biggest ergonomic improvement. Without it, every struct requires 5-10 lines of boilerplate per trait. However, it requires compiler-level struct metadata and generated impl AST/MIR, making it the most complex feature. Positioned last because it is a usability improvement, not a capability enabler.

**Delivers:** `type Point do x :: Int, y :: Int end deriving(Eq, Display, Debug, Hash)` generates correct implementations automatically.

**Addresses features:** Auto-derive mechanism for structs and sum types.

**Estimated effort:** 3-5 days

### Phase Ordering Rationale

- **Phase 1 before Phase 2:** Codegen cannot produce correct trait method calls if the trait resolver returns wrong results for parameterized types or nondeterministic results for name collisions.
- **Phase 2 before Phase 3-5:** All protocols depend on trait method dispatch working. Phase 2 is the gate.
- **Phase 3 before Phase 4:** Display/Eq/Ord are more commonly needed than Hash/Default. Also, Phase 3 validates the full pipeline before adding more protocols.
- **Phase 5 last:** Auto-derive is an ergonomic multiplier that is most valuable after the protocols it derives are already working.
- **Iterator, From/Into, Serialize/Deserialize deferred to v1.4+:** Iterator requires associated types or trait type parameters plus lazy evaluation infrastructure. From/Into requires blanket impl support. These are correctly deferred -- they add significant complexity for features that have workarounds (existing eager collection functions, explicit conversion functions).

### Research Flags

Phases likely needing deeper research during planning:
- **Phase 1:** The `type_to_key` replacement needs careful design -- structural type matching with temporary unification is well-documented in PL literature but must be adapted to Snow's specific TraitRegistry data structures. Research the interaction with Snow's existing `InferCtx` unification.
- **Phase 2:** MIR lowering changes are concentrated in `lower.rs` which is a large, complex file. Research the specific call-site resolution flow: how does the lowerer distinguish a trait method call from a regular function call when both look like `CallExpr("to_string", [x])`?
- **Phase 4:** Default protocol requires static methods (no `self` parameter) in traits. Current trait infrastructure assumes all methods have `self`. Research whether to add `Ty::SelfType` or to use a simpler approach (well-known type variable sentinel).

Phases with standard patterns (skip research-phase):
- **Phase 3:** Display, Debug, Eq, Ord are extremely well-documented across Rust, Haskell, Swift, and Elixir. Registration follows the exact pattern already established in builtins.rs for Add/Sub/Mul/etc. Runtime functions follow the existing `#[no_mangle] extern "C"` pattern.
- **Phase 5:** Auto-derive follows Rust's `#[derive]` and Haskell's `deriving` with extensive prior art. The compiler generates impl bodies from struct metadata -- field-by-field comparison for Eq, lexicographic for Ord, field concatenation for Display.

## Confidence Assessment

| Area | Confidence | Notes |
|------|------------|-------|
| Stack | HIGH | All dependency versions verified on crates.io (2026-02-07). Zero new dependencies confirmed through analysis of what trait systems actually need. |
| Features | HIGH | Extremely well-established domain with extensive prior art from Rust, Haskell, Elixir, Swift, Scala 3. Feature prioritization validated across all five ecosystems. |
| Architecture | HIGH | Based on direct analysis of Snow compiler source code (57,657 lines, 1,018 tests). Pipeline stages, data flow, and integration points verified against actual code. |
| Pitfalls | HIGH | 13 specific pitfalls identified from codebase analysis with line-number precision. Validated against established PL research (coherence theory, monomorphization analysis, Rust compiler issues). |

**Overall confidence:** HIGH

### Gaps to Address

- **Self type representation:** If Default protocol (with `default() -> Self`) is in v1.3 scope, the type system needs a Self type or sentinel. Research required during Phase 4 planning to choose between `Ty::SelfType` variant, well-known type variable, or deferring Default entirely.
- **Higher-order constrained functions:** The `Scheme` struct has no `constraints` field. Passing a trait-constrained function as a value (`let f = show`) drops the constraint. For v1.3, this should be documented as a known limitation with a compiler error ("cannot capture constrained function as value") rather than a silent unsoundness. Full qualified types (Scheme with constraints) can be added in v1.4.
- **Generic function monomorphization:** True generic function specialization (same function called with different type args produces different copies) is not needed for v1.3 trait impls (each impl is already concrete) but will be needed for trait-bounded generic functions like `fn print_all<T: Display>(list :: List<T>)`. The current architecture handles this implicitly for single-instantiation cases but not for multiple instantiations. Monitor during Phase 2 and implement if required by test cases.
- **Coherence in multi-file programs:** Snow is currently single-file. The orphan rule (preventing impl of foreign trait for foreign type) is not needed yet but must be designed when packages/modules are added. Document the intended coherence story during v1.3 to avoid painting into a corner.

## Sources

### Codebase Analysis (HIGH confidence)
- `snow-typeck/src/traits.rs` -- TraitRegistry, type_to_key, register_impl, has_impl
- `snow-typeck/src/infer.rs` -- infer_interface_def, infer_impl_def, where-clause checking
- `snow-typeck/src/builtins.rs` -- compiler-known trait registration pattern
- `snow-typeck/src/ty.rs` -- Ty enum, Scheme struct
- `snow-codegen/src/mir/lower.rs` -- MIR lowering, impl skip at line 431
- `snow-codegen/src/mir/mono.rs` -- reachability-only monomorphization pass
- `snow-codegen/src/mir/types.rs` -- existing mangle_type_name function
- `snow-codegen/src/codegen/expr.rs` -- LLVM call emission

### Primary References (HIGH confidence)
- [Rust Monomorphization -- Compiler Dev Guide](https://rustc-dev-guide.rust-lang.org/backend/monomorph.html)
- [Tour of Rust's Standard Library Traits](https://github.com/pretzelhammer/rust-blog/blob/master/posts/tour-of-rusts-standard-library-traits.md)
- [FNV Hash Function -- Wikipedia](https://en.wikipedia.org/wiki/Fowler%E2%80%93Noll%E2%80%93Vo_hash_function)
- [Coherence of Type Class Resolution (Bottu et al.)](https://xnning.github.io/papers/coherence-class.pdf)
- [Recursive Monomorphization in Rust (issue #50043)](https://github.com/rust-lang/rust/issues/50043)

### Secondary References (MEDIUM confidence)
- [Effective Rust: Standard Traits](https://effective-rust.com/std-traits.html)
- [Elixir Protocols Documentation](https://hexdocs.pm/elixir/protocols.html)
- [Swift Standard Library Protocols](https://bugfender.com/blog/swift-standard-library-protocols/)
- [The Dark Side of Inlining and Monomorphization](https://nickb.dev/blog/the-dark-side-of-inlining-and-monomorphization/)
- [Demystifying Type Classes (Kiselyov)](https://okmij.org/ftp/Computation/typeclass.html)

---
*Research completed: 2026-02-07*
*Ready for roadmap: yes*
