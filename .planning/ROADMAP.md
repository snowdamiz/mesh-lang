# Roadmap: Mesh

## Milestones

- [x] **v1.0 MVP** - Phases 1-10 (shipped 2026-02-07)
- [x] **v1.1 Language Polish** - Phases 11-15 (shipped 2026-02-08)
- [x] **v1.2 Runtime & Type Fixes** - Phases 16-17 (shipped 2026-02-08)
- [x] **v1.3 Traits & Protocols** - Phases 18-22 (shipped 2026-02-08)
- [x] **v1.4 Compiler Polish** - Phases 23-25 (shipped 2026-02-08)
- [x] **v1.5 Compiler Correctness** - Phases 26-29 (shipped 2026-02-09)
- [x] **v1.6 Method Dot-Syntax** - Phases 30-32 (shipped 2026-02-09)
- [x] **v1.7 Loops & Iteration** - Phases 33-36 (shipped 2026-02-09)
- [x] **v1.8 Module System** - Phases 37-42 (shipped 2026-02-09)
- [x] **v1.9 Stdlib & Ergonomics** - Phases 43-48 (shipped 2026-02-10)
- [x] **v2.0 Database & Serialization** - Phases 49-54 (shipped 2026-02-12)
- [x] **v3.0 Production Backend** - Phases 55-58 (shipped 2026-02-12)
- [x] **v4.0 WebSocket Support** - Phases 59-62 (shipped 2026-02-12)
- [x] **v5.0 Distributed Actors** - Phases 63-69 (shipped 2026-02-13)
- [x] **v6.0 Website & Documentation** - Phases 70-73 (shipped 2026-02-13)
- [x] **v7.0 Iterator Protocol & Trait Ecosystem** - Phases 74-79 (shipped 2026-02-14)

## Phases

<details>
<summary>v1.0 MVP (Phases 1-10) - SHIPPED 2026-02-07</summary>

See milestones/v1.0-ROADMAP.md for full phase details.
55 plans across 10 phases. 52,611 lines of Rust. 213 commits.

</details>

<details>
<summary>v1.1 Language Polish (Phases 11-15) - SHIPPED 2026-02-08</summary>

See milestones/v1.1-ROADMAP.md for full phase details.
10 plans across 5 phases. 56,539 lines of Rust (+3,928). 45 commits.

</details>

<details>
<summary>v1.2 Runtime & Type Fixes (Phases 16-17) - SHIPPED 2026-02-08</summary>

See milestones/v1.2-ROADMAP.md for full phase details.
6 plans across 2 phases. 57,657 lines of Rust (+1,118). 22 commits.

</details>

<details>
<summary>v1.3 Traits & Protocols (Phases 18-22) - SHIPPED 2026-02-08</summary>

See milestones/v1.3-ROADMAP.md for full phase details.
18 plans across 5 phases. 63,189 lines of Rust (+5,532). 65 commits.

</details>

<details>
<summary>v1.4 Compiler Polish (Phases 23-25) - SHIPPED 2026-02-08</summary>

See milestones/v1.4-ROADMAP.md for full phase details.
5 plans across 3 phases. 64,548 lines of Rust (+1,359). 13 commits.

</details>

<details>
<summary>v1.5 Compiler Correctness (Phases 26-29) - SHIPPED 2026-02-09</summary>

See milestones/v1.5-ROADMAP.md for full phase details.
6 plans across 4 phases. 66,521 lines of Rust (+1,973). 29 commits.

</details>

<details>
<summary>v1.6 Method Dot-Syntax (Phases 30-32) - SHIPPED 2026-02-09</summary>

See milestones/v1.6-ROADMAP.md for full phase details.
6 plans across 3 phases. 67,546 lines of Rust (+1,025). 24 commits.

</details>

<details>
<summary>v1.7 Loops & Iteration (Phases 33-36) - SHIPPED 2026-02-09</summary>

See milestones/v1.7-ROADMAP.md for full phase details.
8 plans across 4 phases. 70,501 lines of Rust (+2,955). 34 commits.

</details>

<details>
<summary>v1.8 Module System (Phases 37-42) - SHIPPED 2026-02-09</summary>

See milestones/v1.8-ROADMAP.md for full phase details.
12 plans across 6 phases. 73,384 lines of Rust (+2,883). 52 commits.

</details>

<details>
<summary>v1.9 Stdlib & Ergonomics (Phases 43-48) - SHIPPED 2026-02-10</summary>

See milestones/v1.9-ROADMAP.md for full phase details.
13 plans across 6 phases. 76,100 lines of Rust (+2,716). 56 commits.

</details>

<details>
<summary>v2.0 Database & Serialization (Phases 49-54) - SHIPPED 2026-02-12</summary>

See milestones/v2.0-ROADMAP.md for full phase details.
13 plans across 6 phases. 81,006 lines of Rust (+4,906). 52 commits.

</details>

<details>
<summary>v3.0 Production Backend (Phases 55-58) - SHIPPED 2026-02-12</summary>

See milestones/v3.0-ROADMAP.md for full phase details.
8 plans across 4 phases. 83,451 lines of Rust (+2,445). 33 commits.

</details>

<details>
<summary>v4.0 WebSocket Support (Phases 59-62) - SHIPPED 2026-02-12</summary>

See milestones/v4.0-ROADMAP.md for full phase details.
8 plans across 4 phases. ~84,400 lines of Rust (+~950). 38 commits.

</details>

<details>
<summary>v5.0 Distributed Actors (Phases 63-69) - SHIPPED 2026-02-13</summary>

See milestones/v5.0-ROADMAP.md for full phase details.
20 plans across 7 phases. 93,515 lines of Rust (+9,115). 75 commits.

</details>

<details>
<summary>v6.0 Website & Documentation (Phases 70-73) - SHIPPED 2026-02-13</summary>

See milestones/v6.0-ROADMAP.md for full phase details.
11 plans across 4 phases. 5,134 LOC website (Vue/TS/CSS/MD). 32 commits.

</details>

### v7.0 Iterator Protocol & Trait Ecosystem (In Progress)

**Milestone Goal:** Add associated types to the trait system, then build a comprehensive trait-based protocol ecosystem: lazy iterators with pipe-style composition, From/Into conversion, numeric traits for generic math, and the Collect trait for iterator materialization.

- [x] **Phase 74: Associated Types** - Type system foundation for trait-level type members (completed 2026-02-13)
- [x] **Phase 75: Numeric Traits** - User-extensible arithmetic operators via Add/Sub/Mul/Div/Neg (completed 2026-02-13)
- [x] **Phase 76: Iterator Protocol** - Iterator/Iterable traits with built-in implementations and for-in desugaring (completed 2026-02-13)
- [x] **Phase 77: From/Into Conversion** - Type conversion traits with automatic Into generation and ? operator integration (completed 2026-02-13)
- [x] **Phase 78: Lazy Combinators & Terminals** - Lazy iterator pipeline composition and consuming operations (completed 2026-02-14)
- [x] **Phase 79: Collect** - Generic iterator materialization into List, Map, Set, String (completed 2026-02-14)
- [x] **Phase 80: Documentation Update** - Website docs for all v7.0 features (completed 2026-02-14)

#### Phase 74: Associated Types
**Goal**: Users can declare type members in traits and the compiler resolves them through inference
**Depends on**: Nothing (first phase of v7.0)
**Requirements**: ASSOC-01, ASSOC-02, ASSOC-03, ASSOC-04, ASSOC-05
**Success Criteria** (what must be TRUE):
  1. User can write `interface Foo do type Item end` and `impl Foo for Bar do type Item = Int end` and the program compiles
  2. User can reference `Self.Item` in trait method signatures and the compiler resolves it to the concrete type from the impl
  3. Compiler infers concrete associated types through generic function calls without explicit annotation (HM integration works)
  4. Compiler reports clear error when an impl is missing an associated type binding or provides an extra one
**Plans:** 3 plans
Plans:
- [x] 74-01-PLAN.md -- Parser and AST support for associated type syntax
- [x] 74-02-PLAN.md -- Type checker: TraitDef/ImplDef extension, validation, Self.Item resolution
- [x] 74-03-PLAN.md -- MIR integration, cross-module export, E2E and compile-fail tests

#### Phase 75: Numeric Traits
**Goal**: Users can implement arithmetic operators for custom types and write generic numeric code
**Depends on**: Phase 74 (associated types for `type Output`)
**Requirements**: NUM-01, NUM-02, NUM-03
**Success Criteria** (what must be TRUE):
  1. User can `impl Add for Vec2 do type Output = Vec2 fn add(self, other) ... end` and use `v1 + v2` with their custom type
  2. Binary operators (+, -, *, /) infer the result type from the Output associated type (not hardcoded to operand type)
  3. User can implement Neg for a type and use `-value` with unary minus dispatching to the trait method
**Plans:** 2 plans
Plans:
- [x] 75-01-PLAN.md -- Add Output to arithmetic traits, register Neg, update type inference
- [x] 75-02-PLAN.md -- Fix MIR dispatch, add Neg codegen dispatch, E2E tests

#### Phase 76: Iterator Protocol
**Goal**: Users can iterate over any type that implements Iterable, including all built-in collections
**Depends on**: Phase 74 (associated types for `type Item` and `type Iter`)
**Requirements**: ITER-01, ITER-02, ITER-03, ITER-04, ITER-05, ITER-06
**Success Criteria** (what must be TRUE):
  1. User can define a custom iterator by implementing the Iterator trait with `type Item` and `fn next(self) -> Option<Self.Item>`
  2. User can write `for x in my_custom_iterable do ... end` and it desugars through the Iterable/Iterator protocol
  3. All existing for-in loops over List, Map, Set, and Range continue to compile and produce identical results (zero regressions)
  4. User can create an iterator from a collection via `Iter.from(list)` and call `next()` manually
  5. Built-in List, Map, Set, and Range all implement Iterable with compiler-provided iterator types
**Plans:** 2 plans
Plans:
- [x] 76-01-PLAN.md -- Iterator/Iterable trait registration, runtime iterator handles, typeck Iterable resolution
- [x] 76-02-PLAN.md -- ForInIterator MIR/codegen pipeline, Iter.from(), E2E tests

#### Phase 77: From/Into Conversion
**Goal**: Users can define type conversions and the ? operator auto-converts error types
**Depends on**: Phase 74 (associated types used in trait definitions)
**Requirements**: CONV-01, CONV-02, CONV-03, CONV-04
**Success Criteria** (what must be TRUE):
  1. User can write `impl From<Int> for MyType` with a `from` function and call `MyType.from(42)` to convert
  2. Writing `From<A> for B` automatically makes `Into<B>` available on values of type A without additional code
  3. Built-in conversions work: `Float.from(42)` produces `42.0`, `String.from(42)` produces `"42"`
  4. The `?` operator auto-converts error types: a function returning `Result<T, AppError>` can use `?` on `Result<T, String>` if `From<String> for AppError` exists
**Plans:** 3 plans
Plans:
- [x] 77-01-PLAN.md -- Parameterized trait registry, From/Into trait registration, built-in impls, type checking
- [x] 77-02-PLAN.md -- MIR/codegen From dispatch, ? operator error conversion, E2E tests
- [x] 77-03-PLAN.md -- Gap closure: Fix struct error types in Result for From-based ? conversion

#### Phase 78: Lazy Combinators & Terminals
**Goal**: Users can compose lazy iterator pipelines and consume them with terminal operations
**Depends on**: Phase 76 (Iterator trait and built-in iterators)
**Requirements**: COMB-01, COMB-02, COMB-03, COMB-04, COMB-05, COMB-06, TERM-01, TERM-02, TERM-03, TERM-04, TERM-05
**Success Criteria** (what must be TRUE):
  1. User can write `Iter.from(list) |> Iter.map(fn x -> x * 2 end) |> Iter.filter(fn x -> x > 5 end)` and no intermediate list is allocated
  2. User can chain take/skip/enumerate/zip combinators to build multi-step pipelines that evaluate lazily
  3. User can consume an iterator with `Iter.count`, `Iter.sum`, `Iter.any`, `Iter.all`, `Iter.find`, and `Iter.reduce` producing the expected scalar results
  4. A pipeline like `Iter.from(1..1000000) |> Iter.filter(...) |> Iter.take(10) |> Iter.count` stops after finding 10 matches (short-circuit evaluation)
**Plans:** 3 plans
Plans:
- [x] 78-01-PLAN.md -- Runtime adapter infrastructure: type-tagged iterator handles, generic dispatch, combinator adapters, terminal operations
- [x] 78-02-PLAN.md -- Compiler wiring: type checker signatures, MIR lowerer mappings, intrinsic declarations, adapter type registration
- [x] 78-03-PLAN.md -- E2E tests for all combinators and terminals with multi-combinator pipeline verification

#### Phase 79: Collect
**Goal**: Users can materialize iterator pipelines into concrete collections
**Depends on**: Phase 78 (lazy combinators produce iterators to collect)
**Requirements**: COLL-01, COLL-02, COLL-03, COLL-04
**Success Criteria** (what must be TRUE):
  1. User can materialize an iterator into a List via `List.collect(iter)` or `iter |> List.collect()`
  2. User can materialize an iterator of `{key, value}` tuples into a Map via `Map.collect(iter)`
  3. User can materialize an iterator into a Set via `Set.collect(iter)` and into a String via `String.collect(char_iter)`
**Plans:** 2 plans
Plans:
- [x] 79-01-PLAN.md -- Runtime collect functions + compiler wiring for List, Map, Set, String
- [x] 79-02-PLAN.md -- E2E tests for all four collect functions

## Progress

**Execution Order:** 74 -> 75 -> 76 -> 77 -> 78 -> 79 -> 80

| Phase | Milestone | Plans Complete | Status | Completed |
|-------|-----------|----------------|--------|-----------|
| 1-10 | v1.0 | 55/55 | Complete | 2026-02-07 |
| 11-15 | v1.1 | 10/10 | Complete | 2026-02-08 |
| 16-17 | v1.2 | 6/6 | Complete | 2026-02-08 |
| 18-22 | v1.3 | 18/18 | Complete | 2026-02-08 |
| 23-25 | v1.4 | 5/5 | Complete | 2026-02-08 |
| 26-29 | v1.5 | 6/6 | Complete | 2026-02-09 |
| 30-32 | v1.6 | 6/6 | Complete | 2026-02-09 |
| 33-36 | v1.7 | 8/8 | Complete | 2026-02-09 |
| 37-42 | v1.8 | 12/12 | Complete | 2026-02-09 |
| 43-48 | v1.9 | 13/13 | Complete | 2026-02-10 |
| 49-54 | v2.0 | 13/13 | Complete | 2026-02-12 |
| 55-58 | v3.0 | 8/8 | Complete | 2026-02-12 |
| 59-62 | v4.0 | 8/8 | Complete | 2026-02-12 |
| 63-69 | v5.0 | 20/20 | Complete | 2026-02-13 |
| 70-73 | v6.0 | 11/11 | Complete | 2026-02-13 |
| 74 | v7.0 | 3/3 | Complete | 2026-02-13 |
| 75 | v7.0 | 2/2 | Complete | 2026-02-13 |
| 76 | v7.0 | 2/2 | Complete | 2026-02-13 |
| 77 | v7.0 | 3/3 | Complete | 2026-02-13 |
| 78 | v7.0 | 3/3 | Complete | 2026-02-14 |
| 79 | v7.0 | 2/2 | Complete | 2026-02-14 |
| 80 | v7.0 | 2/2 | Complete | 2026-02-14 |

**Total: 80 phases shipped across 17 milestones. 218 plans completed.**

#### Phase 80: Documentation Update for v7.0 APIs

**Goal:** Update the website documentation to cover all v7.0 features: custom interfaces, associated types, numeric traits, From/Into conversion, iterator protocol, lazy combinators, terminal operations, and collect
**Depends on:** Phase 79
**Plans:** 2 plans

Plans:
- [x] 80-01-PLAN.md -- Create Iterators documentation page + sidebar config (completed 2026-02-14)
- [x] 80-02-PLAN.md -- Update Type System, Cheatsheet, and Language Basics pages (completed 2026-02-14)
