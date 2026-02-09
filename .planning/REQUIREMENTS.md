# Requirements: Snow v1.7 Loops & Iteration

**Defined:** 2026-02-08
**Core Value:** Expressive, readable concurrency -- writing concurrent programs should feel as natural as sequential code

## v1.7 Requirements

Requirements for loops and iteration. Each maps to roadmap phases.

### While Loop

- [ ] **WHILE-01**: User can write `while condition do body end` to loop while condition is true
- [ ] **WHILE-02**: While loop body executes zero times if condition is initially false
- [ ] **WHILE-03**: While loop returns Unit type

### For-In Loop

- [ ] **FORIN-01**: User can write `for x in list do body end` to iterate List elements
- [ ] **FORIN-02**: User can write `for i in 0..10 do body end` to iterate Range values
- [ ] **FORIN-03**: User can write `for {k, v} in map do body end` to iterate Map entries with destructuring
- [ ] **FORIN-04**: User can write `for x in set do body end` to iterate Set elements
- [ ] **FORIN-05**: For-in loop returns `List<T>` where T is the body expression type (comprehension semantics)
- [ ] **FORIN-06**: For-in over empty collection returns empty list without error
- [ ] **FORIN-07**: Range iteration compiles to zero-allocation integer arithmetic
- [ ] **FORIN-08**: Loop variable is scoped to loop body and fresh per iteration

### Break/Continue

- [ ] **BRKC-01**: User can write `break` to exit the innermost enclosing loop
- [ ] **BRKC-02**: User can write `continue` to skip to the next iteration of the innermost loop
- [ ] **BRKC-03**: `break` in for-in returns the partially collected list
- [ ] **BRKC-04**: `break` and `continue` outside a loop produce a compile error
- [ ] **BRKC-05**: `break` and `continue` inside closures within loops produce a compile error (cannot cross closure boundary)

### Filter Clause

- [ ] **FILT-01**: User can write `for x in list when condition do body end` to filter during iteration
- [ ] **FILT-02**: Filtered elements are excluded from the collected result list

### Runtime Integration

- [ ] **RTIM-01**: Loops insert reduction checks at back-edges to prevent actor scheduler starvation
- [ ] **RTIM-02**: For-in collection uses O(N) list builder (not O(N^2) append chains)

## Future Requirements

Deferred to future release. Tracked but not in current roadmap.

### Loop Enhancements

- **LOOP-01**: `break value` returns a value from while loops (requires type unification research)
- **LOOP-02**: `while condition do body else default end` (Zig-style while-else)
- **LOOP-03**: Labeled breaks and continues (`break :outer`)
- **LOOP-04**: String character iteration (requires Unicode infrastructure)
- **LOOP-05**: Iterator protocol / Iterable trait with lazy evaluation

## Out of Scope

Explicitly excluded. Documented to prevent scope creep.

| Feature | Reason |
|---------|--------|
| `loop` keyword (infinite loop) | `while true do ... end` is sufficient and clear |
| `do..while` (post-test loop) | Rare use case; achievable with `while true do body; if not cond do break end end` |
| Comprehension guards with commas | Ambiguous with tuple syntax; use `when` keyword instead |
| Parallel for-loops | Actor-based parallelism via `Job.map` already exists |
| Generator/yield | Requires coroutine state machines; too complex for v1.7 |
| Mutable loop accumulators | Conflicts with Snow's immutability model; use `List.reduce` instead |
| for-else (Python-style) | Widely considered confusing semantics |
| `break value` in for-in | Type conflict between List<T> and value type; deferred |

## Traceability

Which phases cover which requirements. Updated during roadmap creation.

| Requirement | Phase | Status |
|-------------|-------|--------|
| WHILE-01 | Phase 33 | Pending |
| WHILE-02 | Phase 33 | Pending |
| WHILE-03 | Phase 33 | Pending |
| FORIN-01 | Phase 35 | Pending |
| FORIN-02 | Phase 34 | Pending |
| FORIN-03 | Phase 35 | Pending |
| FORIN-04 | Phase 35 | Pending |
| FORIN-05 | Phase 35 | Pending |
| FORIN-06 | Phase 35 | Pending |
| FORIN-07 | Phase 34 | Pending |
| FORIN-08 | Phase 34 | Pending |
| BRKC-01 | Phase 33 | Pending |
| BRKC-02 | Phase 33 | Pending |
| BRKC-03 | Phase 35 | Pending |
| BRKC-04 | Phase 33 | Pending |
| BRKC-05 | Phase 33 | Pending |
| FILT-01 | Phase 36 | Pending |
| FILT-02 | Phase 36 | Pending |
| RTIM-01 | Phase 33 | Pending |
| RTIM-02 | Phase 35 | Pending |

**Coverage:**
- v1.7 requirements: 20 total
- Mapped to phases: 20
- Unmapped: 0

---
*Requirements defined: 2026-02-08*
*Last updated: 2026-02-08 after roadmap creation*
