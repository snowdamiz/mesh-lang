# Research Summary: Mesh v7.0 -- Iterator Protocol & Trait Ecosystem

**Domain:** Compiler trait ecosystem extension (associated types, iterators, conversions, numeric traits)
**Researched:** 2026-02-13
**Overall confidence:** HIGH

## Executive Summary

Mesh v7.0 adds five interconnected features to the compiler's trait system: associated types, the Iterator/Iterable protocol, From/Into conversion traits, user-extensible numeric traits (Add/Sub/Mul/Div/Neg), and the Collect trait for materializing lazy pipelines. All five features are well-understood in the programming language design space, with Rust, Haskell, Swift, and Scala providing established reference implementations. The research confirms that Mesh's existing infrastructure -- Hindley-Milner type inference, monomorphization-based static dispatch, trait system with TraitDef/ImplDef/TraitRegistry, and MIR-level desugaring -- can be extended surgically to support all five features without new crates or fundamental architectural changes.

The critical insight is that **associated types are the foundation for everything else**. Iterator needs `type Item` to express its element type. Numeric traits need `type Output` for operator return types (though v7.0 keeps Output=Self for simplicity). Collect needs `type Output` for target collection types. From/Into is the exception -- it uses generic type parameters rather than associated types, so it can be built in parallel.

The second key finding is that Mesh's monomorphization model significantly simplifies the design versus Rust. Since every trait method call is statically dispatched to a concrete `Trait__Method__Type` function, there is no need for Rust's complex trait solver with negative reasoning, specialization, or placeholder types. Every associated type projection MUST normalize to a concrete type before MIR lowering -- if it doesn't, that's a type checker error, not something codegen needs to handle. This simplification makes the implementation tractable within a single milestone.

The third finding is that the existing for-in loop implementation (indexed iteration with separate ForInList/ForInMap/ForInSet/ForInRange MIR nodes) MUST be preserved for backward compatibility. The Iterator-based for-in path should be added as a new fallback, not a replacement. This dual-path approach ensures zero regressions in existing programs while enabling iteration over user-defined types.

## Key Findings

**Stack:** No new Rust dependencies. All changes are internal to mesh-parser, mesh-typeck, mesh-codegen, and mesh-rt. The primary new data structures are `Ty::Projection` for associated type references, associated type storage in `TraitDef`/`ImplDef`, and deferred projection constraints in `InferCtx`.

**Architecture:** Parser adds `type Item` / `type Item = T` syntax. Type checker adds projection normalization with deferred constraints. MIR lowering adds ForInIterator node and From->Into synthetic generation. Codegen adds iterator loop codegen (next() + tag check). Runtime adds collection iterator handles.

**Critical pitfall:** Projection resolution can create infinite loops if projections reference other projections circularly. Must have depth limit (64) and cycle detection. Second critical pitfall: breaking existing for-in loops -- keep ALL existing indexed iteration paths.

## Implications for Roadmap

Based on research, suggested phase structure:

1. **Associated Types** - Foundation (everything depends on this)
   - Parser: `type Item` in interface, `type Item = T` in impl
   - Type system: `Ty::Projection`, deferred normalization in InferCtx, TraitDef/ImplDef storage
   - Validation: missing/extra associated type errors, cross-module export
   - Addresses: FEATURES table stakes 1-4 (declaration, specification, Self.Item, projection)
   - Avoids: PITFALLS 1 (HM principal types), 5 (mangling collision), 8 (no storage), 9 (freshening)
   - Risk: HIGH complexity -- this is the most technically challenging phase

2. **Numeric Traits (Add/Sub/Mul/Div/Neg)** - Low risk, quick win
   - Verify existing user-defined Add/Sub/Mul/Div impls work end-to-end
   - Add Neg trait for unary minus dispatch
   - Ensure resolve_trait_callee handles user types correctly
   - Addresses: FEATURES table stakes (operator overloading for user types)
   - Avoids: PITFALLS 4 (Output type breaks chaining -- keep Output=Self), 11 (UnaryOp codegen path)
   - Risk: LOW -- 90% of infrastructure already exists

3. **Iterator Protocol** - Core lazy iteration
   - Define Iterator + Iterable traits with associated types
   - Runtime iterator handles (list_iter, map_iter, set_iter, range_iter, iter_next)
   - ForInIterator MIR node and codegen
   - For-in desugaring: Iterable check as LAST fallback after existing indexed paths
   - Built-in Iterable impls for List, Map, Set, Range
   - Addresses: FEATURES table stakes 5-8 (Iterator, Iterable, for-in, built-in impls)
   - Avoids: PITFALLS 2 (break existing for-in), 7 (comprehension semantics), 10 (two-trait design)
   - Risk: HIGH -- touches parser, typeck, MIR, codegen, and runtime

4. **From/Into Traits** - Conversion protocol
   - Define From<T> and Into<T> traits
   - Synthetic Into impl generation from From registrations (two-phase: collect then synthesize)
   - Built-in From impls for primitive conversions (Int->Float, Int->String, etc.)
   - Extend ? operator with From-based error conversion
   - Addresses: FEATURES table stakes 9 (From), differentiators (auto Into, ? conversion)
   - Avoids: PITFALLS 3 (blanket impl recursion -- use synthetic generation)
   - Risk: MEDIUM -- synthetic generation is non-standard

5. **Lazy Combinators** - Iterator pipeline composition
   - Struct-based combinator types (MapIterator, FilterIterator, TakeIterator)
   - map, filter, take, skip, enumerate, zip as compiler-generated iterator structs
   - Terminal operations: count, sum, any, all, find, reduce
   - Addresses: FEATURES differentiators (lazy composition without intermediate allocations)
   - Avoids: PITFALLS 12 (monomorphization explosion -- limit combinator set)
   - Risk: MEDIUM -- each combinator is mechanically similar but there are many

6. **Collect Trait** - Pipeline materialization (capstone)
   - Define Collect trait with type Output associated type
   - Built-in Collect impls for List, Map, Set, String
   - Type-directed dispatch via annotation or module-qualified call (List.collect())
   - Addresses: FEATURES table stakes (collect), differentiators (collect to String)
   - Avoids: PITFALLS 6 (HKT-like dispatch -- require type annotation)
   - Risk: MEDIUM -- type inference for target collection is the hardest unsolved problem

**Phase ordering rationale:**
- Phase 1 (associated types) must be first because Iterator, numeric Output, and Collect all require it
- Phase 2 (numeric traits) can go early because it is low-risk and extends existing infrastructure
- Phase 3 (Iterator) depends on Phase 1 for associated types and is the highest-impact user feature
- Phase 4 (From/Into) is independent of Iterator and can be built after associated types
- Phase 5 (combinators) depends on Phase 3 Iterator trait
- Phase 6 (Collect) depends on both Phase 3 (Iterator) and Phase 1 (associated types), and benefits from Phase 5 (combinators exist to collect from)

**Research flags for phases:**
- Phase 1 (associated types): Likely needs deeper research on deferred projection constraint resolution loop termination
- Phase 3 (Iterator): Needs careful integration testing -- for-in backward compatibility is non-negotiable
- Phase 6 (Collect): Type inference for return-type-directed dispatch may need phase-specific research
- Phase 2 (numeric traits): Standard patterns, unlikely to need research
- Phase 4 (From/Into): Synthetic impl generation is straightforward

## Confidence Assessment

| Area | Confidence | Notes |
|------|------------|-------|
| Stack | HIGH | No new dependencies needed. All changes to existing crates verified against source code. |
| Features | HIGH | Surveyed Rust, Swift, Haskell, Scala. Feature set is well-understood. Mesh-specific adaptations (GC model, no ownership, no dynamic dispatch) are straightforward. |
| Architecture | HIGH | All integration points verified against actual source code. Parser, typeck, MIR, and codegen extension points identified. |
| Pitfalls | HIGH | 15 pitfalls identified from codebase analysis and language design literature. Phase-specific mitigations provided. |

## Gaps to Address

- **Deferred projection constraint termination:** The fixed-point loop for resolving pending projections needs careful design. The research identifies the need for progress tracking but the exact algorithm needs to be designed during Phase 1 implementation.
- **Iterator state management at runtime:** The opaque-handle approach is recommended but the exact C runtime API (mesh_list_iter, mesh_iter_next, etc.) needs to be designed during Phase 3.
- **Collect type inference edge cases:** When no type annotation is present on collect(), the error message and recovery strategy need design during Phase 6. The research recommends requiring annotations.
- **Performance testing:** Iterator chains with 5+ combinators may reveal compile-time or binary-size concerns. Should be measured during Phase 5.
- **Multi-letter type parameter freshening:** The existing `freshen_type_params` heuristic (single uppercase letter = type param) needs to be fixed when adding associated types, since associated type names like "Item" and "Output" are multi-character.

---
*Research completed: 2026-02-13*
*Ready for roadmap: yes*
