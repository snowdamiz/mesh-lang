# Roadmap: Snow

## Milestones

- ✅ **v1.0 MVP** - Phases 1-10 (shipped 2026-02-07)
- ✅ **v1.1 Language Polish** - Phases 11-15 (shipped 2026-02-08)
- 🚧 **v1.2 Runtime & Type Fixes** - Phases 16-17 (in progress)

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

### 🚧 v1.2 Runtime & Type Fixes (In Progress)

**Milestone Goal:** Fix the two remaining known issues -- Fun() type annotation parsing and mark-sweep GC for long-running actors.

#### Phase 16: Fun() Type Parsing
**Goal**: Users can annotate function types in Snow code and the compiler parses and type-checks them correctly
**Depends on**: Nothing (independent workstream)
**Requirements**: TYPE-01, TYPE-02, TYPE-03
**Success Criteria** (what must be TRUE):
  1. User can write `Fun(Int, String) -> Bool` as a type annotation and the compiler parses it as a function type, not a type constructor
  2. User can use function type annotations in function parameters, return types, struct fields, and type aliases
  3. The compiler unifies explicit function type annotations with inferred function types during type checking (e.g., passing a closure where `Fun(Int) -> String` is expected works without extra annotation)
**Plans**: TBD

Plans:
- [ ] 16-01: TBD

#### Phase 17: Mark-Sweep Garbage Collector
**Goal**: Long-running actors reclaim unused memory automatically without affecting other actors
**Depends on**: Nothing (independent workstream, can run in parallel with Phase 16)
**Requirements**: RT-01, RT-02, RT-03, RT-04
**Success Criteria** (what must be TRUE):
  1. Per-actor heap uses mark-sweep collection instead of arena/bump allocation
  2. GC triggers automatically when an actor's heap exceeds a pressure threshold without manual invocation
  3. GC pauses are scoped to the individual actor -- other actors continue executing uninterrupted
  4. A long-running actor that allocates and discards data over time maintains bounded memory usage (no unbounded growth)
**Plans**: TBD

Plans:
- [ ] 17-01: TBD

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
| 16. Fun() Type Parsing | v1.2 | 0/? | Not started | - |
| 17. Mark-Sweep GC | v1.2 | 0/? | Not started | - |
