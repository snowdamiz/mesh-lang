# Requirements: Snow v1.5 Compiler Correctness

**Defined:** 2026-02-08
**Core Value:** Expressive, readable concurrency -- writing concurrent programs should feel as natural as sequential code, with supervision and fault tolerance built in.

## v1.5 Requirements

Requirements for resolving all three remaining known limitations.

### Polymorphic Collections

- [ ] **LIST-01**: List literal `[1, 2, 3]` continues to work as List<Int> (backward compatibility)
- [ ] **LIST-02**: User can create and manipulate List<String> (append, access, iterate)
- [ ] **LIST-03**: User can create and manipulate List<Bool>
- [ ] **LIST-04**: User can create and manipulate List<MyStruct> for user-defined struct types
- [ ] **LIST-05**: User can create and manipulate nested lists (List<List<Int>>)
- [ ] **LIST-06**: Display/Debug works for List<T> where T implements Display/Debug
- [ ] **LIST-07**: Eq/Ord works for List<T> where T implements Eq/Ord
- [ ] **LIST-08**: Pattern matching on List<T> works for all element types (head :: tail destructuring)

### Trait Deriving Safety

- [ ] **DERIVE-01**: deriving(Ord) without Eq emits compile-time error with clear diagnostic
- [ ] **DERIVE-02**: deriving(Eq, Ord) continues to work correctly (no regression)
- [ ] **DERIVE-03**: Error message suggests adding Eq to the deriving list

### Qualified Types

- [ ] **QUAL-01**: Constrained function passed as argument works (e.g., apply(show, 42) where show requires Display)
- [ ] **QUAL-02**: Constraints propagate through multiple levels of higher-order passing
- [ ] **QUAL-03**: Type error emitted when constrained function passed to context that doesn't satisfy the constraint

## Future Requirements

None -- this is a focused correctness milestone.

## Out of Scope

| Feature | Reason |
|---------|--------|
| Polymorphic Map<K,V> beyond String keys | Map already supports generic K,V; String key limitation is separate from List |
| Set<T> polymorphism | Set uses same underlying representation as List; address if needed after List<T> |
| Associated types | Required for Iterator protocol; separate milestone |
| Method dot-syntax | Ergonomic feature, not a correctness fix |
| Blanket impls | Requires infrastructure not yet built |
| Full qualified type syntax in annotations | QUAL fixes inference-level propagation; explicit `where` in function types is future work |

## Traceability

| Requirement | Phase | Status |
|-------------|-------|--------|
| LIST-01 | Phase 26 | Pending |
| LIST-02 | Phase 26 | Pending |
| LIST-03 | Phase 26 | Pending |
| LIST-04 | Phase 26 | Pending |
| LIST-05 | Phase 26 | Pending |
| LIST-06 | Phase 27 | Pending |
| LIST-07 | Phase 27 | Pending |
| LIST-08 | Phase 27 | Pending |
| DERIVE-01 | Phase 28 | Pending |
| DERIVE-02 | Phase 28 | Pending |
| DERIVE-03 | Phase 28 | Pending |
| QUAL-01 | Phase 29 | Pending |
| QUAL-02 | Phase 29 | Pending |
| QUAL-03 | Phase 29 | Pending |

**Coverage:**
- v1.5 requirements: 14 total
- Mapped to phases: 14
- Unmapped: 0

---
*Requirements defined: 2026-02-08*
*Last updated: 2026-02-08 after roadmap creation*
