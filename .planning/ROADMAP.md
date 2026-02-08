# Roadmap: Snow

## Milestones

- ✅ **v1.0 MVP** - Phases 1-10 (shipped 2026-02-07)
- ✅ **v1.1 Language Polish** - Phases 11-15 (shipped 2026-02-08)
- ✅ **v1.2 Runtime & Type Fixes** - Phases 16-17 (shipped 2026-02-08)
- ✅ **v1.3 Traits & Protocols** - Phases 18-22 (shipped 2026-02-08)
- 🚧 **v1.4 Compiler Polish** - Phases 23-25 (in progress)

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

### 🚧 v1.4 Compiler Polish (In Progress)

**Milestone Goal:** Fix all five known limitations carried from v1.3 -- making the compiler fully correct across pattern matching codegen, trait system generics, and type system soundness.

#### Phase 23: Pattern Matching Codegen
**Goal**: Sum type pattern matching fully works in LLVM codegen -- users can destructure non-nullary variant fields and use the Ordering type directly in Snow programs
**Depends on**: Phase 22 (v1.3 complete)
**Requirements**: PATM-01, PATM-02
**Success Criteria** (what must be TRUE):
  1. `case opt do Some(x) -> x end` binds `x` to the inner value at runtime (not just discriminates the variant)
  2. `case compare(a, b) do Less -> ... | Equal -> ... | Greater -> ... end` compiles and runs correctly
  3. Ordering is importable/usable as a first-class sum type in user Snow code (return values, variable bindings, pattern matches)
  4. Nested constructor patterns work (e.g., `Some(Some(x))` extracts the doubly-wrapped value)
**Plans:** 2 plans
Plans:
- [x] 23-01-PLAN.md -- Fix pattern compiler tag assignment and field type resolution
- [x] 23-02-PLAN.md -- Register Ordering type, add compare(), end-to-end tests

#### Phase 24: Trait System Generics
**Goal**: Display and auto-derive work correctly with generic and nested types -- users see proper string representations and can derive traits on parameterized structs
**Depends on**: Phase 23
**Requirements**: TGEN-01, TGEN-02
**Success Criteria** (what must be TRUE):
  1. `to_string([[1, 2], [3, 4]])` produces `[[1, 2], [3, 4]]` (recursive Display through nested collections)
  2. `to_string([Some(1), None])` produces `[Some(1), None]` (Display dispatches correctly through collection elements that are sum types)
  3. A generic struct `type Box<T> do value :: T end` with `deriving(Display, Eq)` works for `Box<Int>`, `Box<String>`, and other concrete instantiations
  4. Auto-derived trait impls are registered per-monomorphization so `Box<Int>` and `Box<String>` get independent Display/Eq implementations
**Plans**: TBD

#### Phase 25: Type System Soundness
**Goal**: Higher-order constrained functions preserve their trait constraints when captured as values -- the type system prevents unsound calls at compile time
**Depends on**: Phase 23 (independent of Phase 24; numbered last because it is self-contained)
**Requirements**: TSND-01
**Success Criteria** (what must be TRUE):
  1. `let f = show` retains the `T: Display` constraint -- calling `f(some_non_display_value)` produces a compile-time error, not a runtime crash or silent unsoundness
  2. Constrained functions passed as arguments to higher-order functions propagate their constraints to the call site
  3. The constraint preservation works for user-defined traits (not just stdlib Display) -- e.g., a function `where T: MyTrait` captured as a value still enforces `MyTrait`
**Plans**: TBD

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
| 24. Trait System Generics | v1.4 | 0/TBD | Not started | - |
| 25. Type System Soundness | v1.4 | 0/TBD | Not started | - |
