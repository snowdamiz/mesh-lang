# Requirements: Snow v1.4 Compiler Polish

**Defined:** 2026-02-08
**Core Value:** Expressive, readable concurrency -- writing concurrent programs should feel as natural as sequential code, with supervision and fault tolerance built in.

## v1.4 Requirements

Fix all five known limitations carried from v1.3.

### Pattern Matching Codegen

- [x] **PATM-01**: Sum type pattern matching extracts field values from non-nullary variants in LLVM codegen (e.g., `case opt do Some(x) -> x end` binds `x` to the inner value)
- [x] **PATM-02**: Ordering sum type (Less | Equal | Greater) is user-visible and can be used in pattern matching and return values from Ord trait methods

### Trait System Generics

- [x] **TGEN-01**: Nested collection Display renders recursively — `to_string([[1, 2], [3, 4]])` produces `[[1, 2], [3, 4]]` instead of falling back to `snow_int_to_string` for inner elements
- [x] **TGEN-02**: `deriving(Eq, Ord, Display, Debug, Hash)` works on generic types (e.g., `type Box<T> do value :: T end`) with monomorphization-aware trait impl registration

### Type System Soundness

- [x] **TSND-01**: Higher-order constrained functions preserve trait constraints when captured as values — `let f = show` retains the `T: Display` constraint, preventing calls with non-Display types (compiler error instead of silent unsoundness)

## Future Requirements

None — v1.4 is a focused bug-fix milestone.

## Out of Scope

| Feature | Reason |
|---------|--------|
| Iterator/From protocols | Requires associated types; separate feature milestone |
| Method dot-syntax | Separate feature; not a bug fix |
| Blanket impls | Requires coherence infrastructure; separate milestone |
| Distributed actors | Major feature; not a limitation fix |
| Generational GC | Optimization, not a correctness fix |

## Traceability

| Requirement | Phase | Status |
|-------------|-------|--------|
| PATM-01 | Phase 23 | Complete |
| PATM-02 | Phase 23 | Complete |
| TGEN-01 | Phase 24 | Complete |
| TGEN-02 | Phase 24 | Complete |
| TSND-01 | Phase 25 | Complete |

**Coverage:**
- v1.4 requirements: 5 total
- Mapped to phases: 5
- Unmapped: 0

---
*Requirements defined: 2026-02-08*
*Last updated: 2026-02-08 after roadmap creation*
