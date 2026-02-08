# Requirements: Snow v1.2

**Defined:** 2026-02-07
**Core Value:** Expressive, readable concurrency -- writing concurrent programs should feel as natural as sequential code, with supervision and fault tolerance built in.

## v1.2 Requirements

### Type System

- [ ] **TYPE-01**: Fun() parsed as function type annotation (e.g., `Fun(Int) -> String`)
- [ ] **TYPE-02**: Function type annotations work in function signatures, struct fields, and type aliases
- [ ] **TYPE-03**: Function type annotations integrate with HM type inference (unify with inferred function types)

### Runtime

- [ ] **RT-01**: Mark-sweep garbage collector for per-actor heaps replacing arena/bump allocation
- [ ] **RT-02**: GC triggers automatically based on heap pressure threshold
- [ ] **RT-03**: GC runs per-actor (no stop-the-world pauses across actors)
- [ ] **RT-04**: Long-running actors reclaim unreachable memory (no unbounded growth)

## Out of Scope

| Feature | Reason |
|---------|--------|
| Generational GC | Mark-sweep is sufficient for v1.2; generational optimization is future work |
| Concurrent/incremental GC | Per-actor isolation means GC pauses only affect one actor; concurrent collection is unnecessary complexity |
| Cross-actor cycle detection | Actors are isolated; cross-actor references use Pid, not direct pointers |
| Compacting GC | Adds complexity; mark-sweep with free-list is sufficient |
| Function type syntax alternatives (arrows only) | Keep Fun() keyword syntax consistent with existing type constructors |

## Traceability

| Requirement | Phase | Status |
|-------------|-------|--------|
| TYPE-01 | Phase 16 | Pending |
| TYPE-02 | Phase 16 | Pending |
| TYPE-03 | Phase 16 | Pending |
| RT-01 | Phase 17 | Pending |
| RT-02 | Phase 17 | Pending |
| RT-03 | Phase 17 | Pending |
| RT-04 | Phase 17 | Pending |

**Coverage:**
- v1.2 requirements: 7 total
- Mapped to phases: 7
- Unmapped: 0

---
*Requirements defined: 2026-02-07*
*Last updated: 2026-02-07 after roadmap creation*
