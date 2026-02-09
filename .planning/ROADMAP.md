# Roadmap: Snow

## Milestones

- [x] **v1.0 MVP** - Phases 1-10 (shipped 2026-02-07)
- [x] **v1.1 Language Polish** - Phases 11-15 (shipped 2026-02-08)
- [x] **v1.2 Runtime & Type Fixes** - Phases 16-17 (shipped 2026-02-08)
- [x] **v1.3 Traits & Protocols** - Phases 18-22 (shipped 2026-02-08)
- [x] **v1.4 Compiler Polish** - Phases 23-25 (shipped 2026-02-08)
- [x] **v1.5 Compiler Correctness** - Phases 26-29 (shipped 2026-02-09)
- [x] **v1.6 Method Dot-Syntax** - Phases 30-32 (shipped 2026-02-09)
- [ ] **v1.7 Loops & Iteration** - Phases 33-36 (in progress)

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

### v1.7 Loops & Iteration (In Progress)

**Milestone Goal:** Add for..in loops, while loops, and break/continue as expression-level constructs, enabling natural iteration over Lists, Maps, Sets, and Ranges.

- [x] **Phase 33: While Loop + Loop Control Flow** - Establish loop infrastructure with while loops, break/continue, and reduction checks
- [x] **Phase 34: For-In over Range** - Range iteration with zero-allocation integer arithmetic
- [x] **Phase 35: For-In over Collections** - List/Map/Set iteration with list builder, expression semantics, and destructuring
- [ ] **Phase 36: Filter Clause + Integration** - Filter clause (`when`) and comprehensive integration validation

## Phase Details

### Phase 33: While Loop + Loop Control Flow
**Goal**: Users can write conditional loops with early exit and skip, and the actor scheduler remains responsive during long-running loops
**Depends on**: Phase 32 (v1.6 complete)
**Requirements**: WHILE-01, WHILE-02, WHILE-03, BRKC-01, BRKC-02, BRKC-04, BRKC-05, RTIM-01
**Success Criteria** (what must be TRUE):
  1. User can write `while condition do body end` and the body executes repeatedly while the condition is true
  2. A while loop whose condition is initially false executes zero times and the program continues normally
  3. User can write `break` inside a while loop to exit early, and `continue` to skip to the next iteration
  4. Writing `break` or `continue` outside any loop produces a compile-time error; writing them inside a closure within a loop also produces a compile-time error
  5. A tight while loop (e.g., 1 million iterations with no function calls) does not starve other actors in the runtime
**Plans:** 2 plans
Plans:
- [x] 33-01-PLAN.md -- Keywords, parser, AST, and type checker for while/break/continue
- [x] 33-02-PLAN.md -- MIR lowering, LLVM codegen, formatter, and e2e tests

### Phase 34: For-In over Range
**Goal**: Users can iterate over integer ranges with for-in syntax, producing collected results, with zero heap allocation for the range itself
**Depends on**: Phase 33
**Requirements**: FORIN-02, FORIN-07, FORIN-08
**Success Criteria** (what must be TRUE):
  1. User can write `for i in 0..10 do body end` and the body executes once for each integer in the range
  2. Range iteration compiles to pure integer arithmetic with no heap allocation for the range counter
  3. The loop variable is scoped to the loop body and does not leak into the surrounding scope; each iteration gets a fresh binding
**Plans:** 2 plans
Plans:
- [x] 34-01-PLAN.md -- Parser, AST, and type checker for for-in over range
- [x] 34-02-PLAN.md -- MIR lowering, LLVM codegen, formatter, and e2e tests

### Phase 35: For-In over Collections
**Goal**: Users can iterate over Lists, Maps, and Sets with for-in syntax, with expression semantics that return a collected List of body results
**Depends on**: Phase 34
**Requirements**: FORIN-01, FORIN-03, FORIN-04, FORIN-05, FORIN-06, RTIM-02, BRKC-03
**Success Criteria** (what must be TRUE):
  1. User can write `for x in list do body end`, `for {k, v} in map do body end`, and `for x in set do body end` to iterate each collection type
  2. For-in loop returns `List<T>` containing the evaluated body expression for each element (comprehension semantics)
  3. For-in over an empty collection returns an empty list without error
  4. `break` inside a for-in loop returns the partially collected list of results gathered so far
  5. For-in collection uses O(N) list builder allocation, not O(N^2) append chains
**Plans:** 2 plans
Plans:
- [x] 35-01-PLAN.md -- Runtime list builder + indexed access, parser destructuring, typeck collection detection, MIR variants
- [x] 35-02-PLAN.md -- LLVM codegen for collection iteration, range comprehension update, e2e tests

### Phase 36: Filter Clause + Integration
**Goal**: Users can filter elements during for-in iteration, and all loop forms work correctly with closures, nesting, pipes, and tooling
**Depends on**: Phase 35
**Requirements**: FILT-01, FILT-02
**Success Criteria** (what must be TRUE):
  1. User can write `for x in list when condition do body end` and only elements satisfying the condition are processed
  2. Filtered elements are excluded from the collected result list (the returned List contains only results from iterations where the condition was true)
  3. Nested loops, loops containing closures, and loops inside pipe chains all work correctly
  4. The formatter and LSP handle all loop syntax forms without errors
**Plans:** 2 plans
Plans:
- [ ] 36-01-PLAN.md -- Filter clause pipeline: parser, AST, typeck, MIR, lowering, codegen, formatter
- [ ] 36-02-PLAN.md -- E2E tests, parser tests, formatter tests for filter and integration

## Progress

**Execution Order:**
Phases execute in numeric order: 33 -> 34 -> 35 -> 36

| Phase | Milestone | Plans Complete | Status | Completed |
|-------|-----------|----------------|--------|-----------|
| 1-10 | v1.0 | 55/55 | Complete | 2026-02-07 |
| 11-15 | v1.1 | 10/10 | Complete | 2026-02-08 |
| 16-17 | v1.2 | 6/6 | Complete | 2026-02-08 |
| 18-22 | v1.3 | 18/18 | Complete | 2026-02-08 |
| 23-25 | v1.4 | 5/5 | Complete | 2026-02-08 |
| 26-29 | v1.5 | 6/6 | Complete | 2026-02-09 |
| 30-32 | v1.6 | 6/6 | Complete | 2026-02-09 |
| 33. While Loop + Loop Control Flow | v1.7 | 2/2 | Complete | 2026-02-08 |
| 34. For-In over Range | v1.7 | 2/2 | Complete | 2026-02-09 |
| 35. For-In over Collections | v1.7 | 2/2 | Complete | 2026-02-09 |
| 36. Filter Clause + Integration | v1.7 | 0/TBD | Not started | - |

**Total: 35 phases shipped, 1 phase remaining (v1.7).**
