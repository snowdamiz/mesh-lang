# Roadmap: Snow

## Milestones

- [x] **v1.0 MVP** - Phases 1-10 (shipped 2026-02-07)
- [x] **v1.1 Language Polish** - Phases 11-15 (shipped 2026-02-08)
- [x] **v1.2 Runtime & Type Fixes** - Phases 16-17 (shipped 2026-02-08)
- [x] **v1.3 Traits & Protocols** - Phases 18-22 (shipped 2026-02-08)
- [x] **v1.4 Compiler Polish** - Phases 23-25 (shipped 2026-02-08)
- [x] **v1.5 Compiler Correctness** - Phases 26-29 (shipped 2026-02-09)
- [ ] **v1.6 Method Dot-Syntax** - Phases 30-32 (in progress)

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

### v1.6 Method Dot-Syntax (In Progress)

**Milestone Goal:** Add method call syntax so values can call their impl/trait methods via dot notation (`value.method(args)`), with self-parameter desugaring, chaining, and clear diagnostics.

- [ ] **Phase 30: Core Method Resolution** - Basic `value.method(args)` works end-to-end through type checker and MIR lowering
- [ ] **Phase 31: Extended Method Support** - Primitives, generics, chaining, and mixed field/method access
- [ ] **Phase 32: Diagnostics and Integration** - Ambiguity errors, deterministic ordering, and non-regression for existing syntax

## Phase Details

### Phase 30: Core Method Resolution
**Goal**: Users can call trait impl methods on struct values using dot syntax, with the receiver automatically passed as the first argument
**Depends on**: Phase 29 (v1.5 complete)
**Requirements**: METH-01, METH-02, METH-03, DIAG-01
**Success Criteria** (what must be TRUE):
  1. User can write `point.to_string()` and it resolves to the Display impl's `to_string` method for the Point type
  2. The receiver (`point`) is automatically passed as the first argument -- `point.to_string()` produces identical results to `to_string(point)`
  3. Method resolution searches all trait impl blocks for the receiver's concrete type and finds the correct method
  4. Calling a nonexistent method on a type produces a clear "no method `x` on type `Y`" error, not a confusing "no field" error
**Plans**: TBD

### Phase 31: Extended Method Support
**Goal**: Method dot-syntax works with primitive types, generic types, and supports chaining and mixed field/method access
**Depends on**: Phase 30
**Requirements**: METH-04, METH-05, CHAIN-01, CHAIN-02
**Success Criteria** (what must be TRUE):
  1. User can call methods on primitive types -- `42.to_string()` returns `"42"`, `true.to_string()` returns `"true"`
  2. User can call methods on generic types -- `my_list.to_string()` works where `List<T>` implements Display
  3. User can chain method calls -- `point.to_string().length()` compiles and returns the length of the string representation
  4. User can mix field access and method calls -- `person.name.length()` accesses the `name` field then calls `length()` on the result
**Plans**: TBD

### Phase 32: Diagnostics and Integration
**Goal**: Method dot-syntax has polished diagnostics for edge cases and all existing syntax forms continue to work unchanged
**Depends on**: Phase 31
**Requirements**: DIAG-02, DIAG-03, INTG-01, INTG-02, INTG-03, INTG-04, INTG-05
**Success Criteria** (what must be TRUE):
  1. When multiple traits provide the same method for a type, the compiler produces an error listing the conflicting traits and suggests qualified syntax (e.g., `Display.to_string(point)`)
  2. Ambiguity errors use deterministic ordering (alphabetical by trait name), not random HashMap iteration order
  3. Struct field access (`point.x`), module-qualified calls (`String.length(s)`), pipe operator (`value |> method(args)`), sum type variant access (`Shape.Circle`), and actor `self` in receive blocks all continue to work exactly as before
  4. Existing test suite passes with zero regressions
**Plans**: TBD

## Progress

**Execution Order:** 30 -> 31 -> 32

| Phase | Milestone | Plans Complete | Status | Completed |
|-------|-----------|----------------|--------|-----------|
| 30. Core Method Resolution | v1.6 | 0/TBD | Not started | - |
| 31. Extended Method Support | v1.6 | 0/TBD | Not started | - |
| 32. Diagnostics and Integration | v1.6 | 0/TBD | Not started | - |
