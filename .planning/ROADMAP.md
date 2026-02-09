# Roadmap: Snow

## Milestones

- ✅ **v1.0 MVP** - Phases 1-10 (shipped 2026-02-07)
- ✅ **v1.1 Language Polish** - Phases 11-15 (shipped 2026-02-08)
- ✅ **v1.2 Runtime & Type Fixes** - Phases 16-17 (shipped 2026-02-08)
- ✅ **v1.3 Traits & Protocols** - Phases 18-22 (shipped 2026-02-08)
- ✅ **v1.4 Compiler Polish** - Phases 23-25 (shipped 2026-02-08)
- 🚧 **v1.5 Compiler Correctness** - Phases 26-29 (in progress)

## Phases

<details>
<summary>✅ v1.0 MVP (Phases 1-10) - SHIPPED 2026-02-07</summary>

55 plans across 10 phases. Full compiler pipeline, actor runtime, supervision trees,
standard library, and developer tooling. See milestones/v1.0-ROADMAP.md for details.

</details>

<details>
<summary>✅ v1.1 Language Polish (Phases 11-15) - SHIPPED 2026-02-08</summary>

10 plans across 5 phases. Fixed all five v1.0 limitations: multi-clause functions,
pipe operator closures, string pattern matching, generic map types, and actor-per-connection HTTP.
See milestones/v1.1-ROADMAP.md for details.

</details>

<details>
<summary>✅ v1.2 Runtime & Type Fixes (Phases 16-17) - SHIPPED 2026-02-08</summary>

6 plans across 2 phases. Fun() type annotation parsing (full pipeline from parser through codegen)
and mark-sweep garbage collector for per-actor heaps (conservative stack scanning, cooperative collection).
See milestones/v1.2-ROADMAP.md for details.

</details>

<details>
<summary>✅ v1.3 Traits & Protocols (Phases 18-22) - SHIPPED 2026-02-08</summary>

18 plans across 5 phases. Complete trait/protocol system with structural type matching,
static dispatch via monomorphization, six stdlib protocols (Display, Debug, Eq, Ord, Hash, Default),
default method implementations, collection Display/Debug, and auto-derive system.
See milestones/v1.3-ROADMAP.md for details.

</details>

<details>
<summary>✅ v1.4 Compiler Polish (Phases 23-25) - SHIPPED 2026-02-08</summary>

5 plans across 3 phases. Fixed all five v1.3 known limitations: sum type pattern matching
codegen with field destructuring, recursive nested collection Display, generic type deriving
with monomorphization, and higher-order constrained function soundness.
See milestones/v1.4-ROADMAP.md for details.

</details>

### 🚧 v1.5 Compiler Correctness (In Progress)

**Milestone Goal:** Resolve all three remaining known limitations -- polymorphic List<T>, Ord-requires-Eq compile-time enforcement, and higher-order constraint propagation (qualified types).

#### Phase 26: Polymorphic List Foundation ✓
**Goal**: Users can create and use List<T> with any element type, not just Int
**Depends on**: Phase 25 (v1.4 complete)
**Requirements**: LIST-01, LIST-02, LIST-03, LIST-04, LIST-05
**Success Criteria** (what must be TRUE):
  1. ✓ `[1, 2, 3]` continues to compile and work as List<Int> with no changes to existing code
  2. ✓ User can create `["hello", "world"]` as List<String> and access/append elements
  3. ✓ User can create `[true, false]` as List<Bool> and iterate over elements
  4. ✓ User can create a list of user-defined struct instances and manipulate them
  5. ✓ User can create `[[1, 2], [3, 4]]` as List<List<Int>> and access nested elements
**Plans**: 2 plans
**Completed**: 2026-02-08

Plans:
- [x] 26-01-PLAN.md -- Parser list literal syntax + polymorphic type signatures
- [x] 26-02-PLAN.md -- MIR lowering + LLVM codegen + list concatenation

#### Phase 27: List Trait & Pattern Integration ✓
**Goal**: Trait protocols and pattern matching work correctly with polymorphic List<T>
**Depends on**: Phase 26
**Requirements**: LIST-06, LIST-07, LIST-08
**Success Criteria** (what must be TRUE):
  1. ✓ `to_string([1, 2, 3])` and `to_string(["a", "b"])` both produce correct Display output
  2. ✓ `debug(my_struct_list)` renders each element using its Debug implementation
  3. ✓ `[1, 2] == [1, 2]` returns true and `[1, 3] > [1, 2]` returns true via Eq/Ord
  4. ✓ `case my_list do head :: tail -> ... end` destructures List<String>, List<Bool>, and List<MyStruct>
**Plans**: 2 plans
**Completed**: 2026-02-09

Plans:
- [x] 27-01-PLAN.md -- Display/Debug and Eq/Ord trait dispatch for List<T>
- [x] 27-02-PLAN.md -- Cons pattern (head :: tail) destructuring for List<T>

#### Phase 28: Trait Deriving Safety ✓
**Goal**: Compiler enforces trait dependency rules at compile time instead of failing at runtime
**Depends on**: Phase 25 (v1.4 complete, independent of Phases 26-27)
**Requirements**: DERIVE-01, DERIVE-02, DERIVE-03
**Success Criteria** (what must be TRUE):
  1. ✓ `deriving(Ord)` without `Eq` on a struct emits a compile-time error (not a runtime crash)
  2. ✓ The error message explicitly suggests adding `Eq` to the deriving list
  3. ✓ `deriving(Eq, Ord)` compiles and works correctly with no regression from v1.4 behavior
**Plans**: 1 plan
**Completed**: 2026-02-09

Plans:
- [x] 28-01-PLAN.md -- MissingDerivePrerequisite error variant + validation checks + e2e tests

#### Phase 29: Qualified Types
**Goal**: Trait constraints propagate correctly when constrained functions are passed as higher-order arguments
**Depends on**: Phase 25 (v1.4 complete, independent of Phases 26-28)
**Requirements**: QUAL-01, QUAL-02, QUAL-03
**Success Criteria** (what must be TRUE):
  1. `apply(show, 42)` works where `show` requires Display and `42` satisfies it
  2. Constraints propagate through nested higher-order calls (e.g., `wrap(apply, show, value)`)
  3. Passing a constrained function to a context that violates the constraint produces a clear type error
**Plans**: TBD

Plans:
- [ ] 29-01: TBD
- [ ] 29-02: TBD

## Progress

| Phase | Milestone | Plans Complete | Status | Completed |
|-------|-----------|----------------|--------|-----------|
| 1. Foundation & Lexer | v1.0 | 3/3 | Complete | 2026-02-05 |
| 2. Parser & AST | v1.0 | 5/5 | Complete | 2026-02-05 |
| 3. Type System | v1.0 | 5/5 | Complete | 2026-02-05 |
| 4. Pattern Matching & ADTs | v1.0 | 5/5 | Complete | 2026-02-06 |
| 5. LLVM Codegen | v1.0 | 5/5 | Complete | 2026-02-06 |
| 6. Actor Runtime | v1.0 | 7/7 | Complete | 2026-02-06 |
| 7. Supervision & Fault Tolerance | v1.0 | 3/3 | Complete | 2026-02-06 |
| 8. Standard Library | v1.0 | 7/7 | Complete | 2026-02-06 |
| 9. Concurrency Standard Library | v1.0 | 5/5 | Complete | 2026-02-07 |
| 10. Developer Tooling | v1.0 | 10/10 | Complete | 2026-02-07 |
| 11. Multi-Clause Functions | v1.1 | 3/3 | Complete | 2026-02-07 |
| 12. Pipe Operator Closures | v1.1 | 3/3 | Complete | 2026-02-07 |
| 13. String Pattern Matching | v1.1 | 1/1 | Complete | 2026-02-07 |
| 14. Generic Map Types | v1.1 | 2/2 | Complete | 2026-02-08 |
| 15. HTTP Actor Model | v1.1 | 1/1 | Complete | 2026-02-08 |
| 16. Fun() Type Parsing | v1.2 | 2/2 | Complete | 2026-02-08 |
| 17. Mark-Sweep GC | v1.2 | 4/4 | Complete | 2026-02-08 |
| 18. Trait Infrastructure | v1.3 | 3/3 | Complete | 2026-02-08 |
| 19. Trait Method Codegen | v1.3 | 4/4 | Complete | 2026-02-08 |
| 20. Essential Stdlib Protocols | v1.3 | 5/5 | Complete | 2026-02-08 |
| 21. Extended Protocols | v1.3 | 4/4 | Complete | 2026-02-08 |
| 22. Auto-Derive (Stretch) | v1.3 | 2/2 | Complete | 2026-02-08 |
| 23. Pattern Matching Codegen | v1.4 | 2/2 | Complete | 2026-02-08 |
| 24. Trait System Generics | v1.4 | 2/2 | Complete | 2026-02-08 |
| 25. Type System Soundness | v1.4 | 1/1 | Complete | 2026-02-08 |
| 26. Polymorphic List Foundation | v1.5 | 2/2 | Complete | 2026-02-08 |
| 27. List Trait & Pattern Integration | v1.5 | 2/2 | Complete | 2026-02-09 |
| 28. Trait Deriving Safety | v1.5 | 1/1 | Complete | 2026-02-09 |
| 29. Qualified Types | v1.5 | 0/TBD | Not started | - |
