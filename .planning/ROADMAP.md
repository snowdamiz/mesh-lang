# Roadmap: Snow

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
- [ ] **v1.9 Stdlib & Ergonomics** - Phases 43-48 (in progress)

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

### v1.9 Stdlib & Ergonomics (In Progress)

**Milestone Goal:** Make Snow practical for real programs by adding math stdlib, error propagation sugar, receive timeouts, timer primitives, collection operations, and tail-call optimization.

- [x] **Phase 43: Math Stdlib** - Numeric functions and type conversions via LLVM intrinsics (completed 2026-02-09)
- [x] **Phase 44: Receive Timeouts & Timers** - Actor timeout codegen completion and timer primitives (completed 2026-02-09)
- [x] **Phase 45: Error Propagation** - ? operator for Result/Option early return (completed 2026-02-10)
- [x] **Phase 46: Core Collection Operations** - Sort, find, contains, String split/join/parse (completed 2026-02-10)
- [ ] **Phase 47: Extended Collection Operations** - Zip, flat_map, enumerate, take/drop, Map/Set conversions
- [ ] **Phase 48: Tail-Call Elimination** - Self-recursive tail calls transformed to loops

## Phase Details

### Phase 43: Math Stdlib
**Goal**: Users can perform standard math operations on Int and Float values
**Depends on**: Nothing (first phase of v1.9)
**Requirements**: MATH-01, MATH-02, MATH-03, MATH-04, MATH-05, MATH-06, MATH-07
**Success Criteria** (what must be TRUE):
  1. User can call Math.abs, Math.min, Math.max on both Int and Float and get correct results
  2. User can call Math.pow, Math.sqrt and get correct numeric results (sqrt returns Float)
  3. User can call Math.floor, Math.ceil, Math.round to convert Float to Int
  4. User can reference Math.pi as a Float constant in expressions
  5. User can convert between Int and Float with Int.to_float(x) and Float.to_int(x)
**Plans:** 2 plans
Plans:
- [ ] 43-01-PLAN.md -- Core math ops (abs, min, max, pi) + Int/Float type conversions
- [ ] 43-02-PLAN.md -- Advanced math ops (pow, sqrt, floor, ceil, round) + integration tests

### Phase 44: Receive Timeouts & Timers
**Goal**: Actors can time out on message receives and use timer primitives for delayed operations
**Depends on**: Nothing (independent of Phase 43)
**Requirements**: RECV-01, RECV-02, TIMER-01, TIMER-02
**Success Criteria** (what must be TRUE):
  1. User can write `receive { ... } after ms -> body` and the timeout body executes when no message arrives within ms (no segfault)
  2. Compiler type-checks timeout body against receive arm types, rejecting type mismatches
  3. User can call Timer.sleep(ms) to suspend the current actor for the specified duration without blocking other actors
  4. User can call Timer.send_after(pid, ms, msg) and the target actor receives msg after ms milliseconds
**Plans:** 2 plans
Plans:
- [ ] 44-01-PLAN.md -- Receive-with-timeout codegen (null-check branching + e2e tests)
- [ ] 44-02-PLAN.md -- Timer.sleep and Timer.send_after stdlib module

### Phase 45: Error Propagation
**Goal**: Users can propagate errors concisely using the ? operator instead of explicit pattern matching
**Depends on**: Nothing (independent of Phases 43-44)
**Requirements**: ERR-01, ERR-02, ERR-03
**Success Criteria** (what must be TRUE):
  1. User can write `expr?` on a Result<T,E> value: unwraps Ok(v) to v, early-returns Err(e) on error
  2. User can write `expr?` on an Option<T> value: unwraps Some(v) to v, early-returns None on absence
  3. Compiler emits a clear error when ? is used in a function whose return type is not Result or Option
**Plans:** 3 plans
Plans:
- [x] 45-01-PLAN.md -- Parser + typeck: TRY_EXPR parsing, fn_return_type_stack, E0036 diagnostic
- [x] 45-02-PLAN.md -- MIR lowering: ? desugaring to Match+Return + comprehensive e2e tests
- [x] 45-03-PLAN.md -- Gap closure: compile_expect_error e2e tests for ERR-03 (E0036/E0037)

### Phase 46: Core Collection Operations
**Goal**: Users have essential collection manipulation functions for lists and strings
**Depends on**: Nothing (independent, but benefits from existing Ord infrastructure)
**Requirements**: COLL-01, COLL-02, COLL-03, COLL-04, COLL-09, COLL-10
**Success Criteria** (what must be TRUE):
  1. User can sort a list with List.sort(list, cmp_fn) using an explicit comparator function
  2. User can search lists with List.find (returns Option), List.any/List.all (returns Bool), and List.contains (returns Bool)
  3. User can split strings with String.split(s, delim) and join lists of strings with String.join(list, sep)
  4. User can parse strings to numbers with String.to_int(s) and String.to_float(s) returning Option
**Plans:** 2 plans
Plans:
- [x] 46-01-PLAN.md -- List operations (sort, find, any/all, contains) + SnowOption shared module
- [x] 46-02-PLAN.md -- String operations (split, join, to_int, to_float)

### Phase 47: Extended Collection Operations
**Goal**: Users have the full complement of functional collection transformations across List, Map, and Set
**Depends on**: Phase 46 (builds on collection operation patterns established there)
**Requirements**: COLL-05, COLL-06, COLL-07, COLL-08, COLL-11, COLL-12
**Success Criteria** (what must be TRUE):
  1. User can zip two lists with List.zip(a, b) returning List<(A, B)> truncated to shorter length
  2. User can call List.flat_map(list, fn) and List.flatten(list) for nested list processing
  3. User can call List.enumerate(list) returning List<(Int, T)> and List.take/List.drop for subsequences
  4. User can convert between Map and List with Map.merge, Map.to_list, Map.from_list, and between Set and List with Set.difference, Set.to_list, Set.from_list
**Plans**: TBD

### Phase 48: Tail-Call Elimination
**Goal**: Self-recursive functions execute in constant stack space, making actor receive loops safe from stack overflow
**Depends on**: Nothing (independent, but scheduled last due to highest complexity)
**Requirements**: TCE-01, TCE-02
**Success Criteria** (what must be TRUE):
  1. A self-recursive function in tail position runs for 1,000,000+ iterations without stack overflow
  2. Tail position is correctly detected through if/else branches, case arms, receive arms, blocks, and let-chains
  3. Actor receive loops using self-recursive tail calls run indefinitely without growing the stack
**Plans**: TBD

## Progress

**Execution Order:**
Phases execute in numeric order: 43 -> 44 -> 45 -> 46 -> 47 -> 48

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
| 43 | v1.9 | 2/2 | Complete | 2026-02-09 |
| 44 | v1.9 | 2/2 | Complete | 2026-02-09 |
| 45 | v1.9 | 3/3 | Complete | 2026-02-10 |
| 46 | v1.9 | 2/2 | Complete | 2026-02-10 |
| 47 | v1.9 | 0/TBD | Not started | - |
| 48 | v1.9 | 0/TBD | Not started | - |

**Total: 46 phases shipped across 9 milestones. 137 plans completed. 2 phases remaining.**
