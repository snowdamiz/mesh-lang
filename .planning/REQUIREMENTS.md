# Requirements: Snow

**Defined:** 2026-02-08
**Core Value:** Expressive, readable concurrency -- writing concurrent programs should feel as natural as sequential code, with supervision and fault tolerance built in.

## v1.6 Requirements

Requirements for method dot-syntax milestone. Each maps to roadmap phases.

### Method Resolution

- [ ] **METH-01**: User can call trait impl methods via dot syntax (`value.method(args)`)
- [ ] **METH-02**: Receiver is automatically passed as the first argument to the resolved impl method
- [ ] **METH-03**: Method resolution searches all trait impl blocks for the receiver's type
- [ ] **METH-04**: Methods on primitive types work via dot syntax (`42.to_string()`, `true.to_string()`)
- [ ] **METH-05**: Methods on generic types work via dot syntax (`my_list.to_string()` where `List<T>` implements Display)

### Chaining

- [ ] **CHAIN-01**: User can chain method calls (`point.to_string().length()`)
- [ ] **CHAIN-02**: User can mix field access and method calls (`person.name.to_string()`)

### Diagnostics

- [ ] **DIAG-01**: Calling a nonexistent method produces "no method `x` on type `Y`" error (not "no field")
- [ ] **DIAG-02**: Ambiguous method (multiple traits) produces error listing conflicting traits with qualified syntax suggestion
- [ ] **DIAG-03**: Ambiguity uses deterministic ordering (not HashMap iteration order)

### Integration

- [ ] **INTG-01**: Struct field access (`point.x`) continues to work unchanged
- [ ] **INTG-02**: Module-qualified calls (`String.length(s)`) continue to work unchanged
- [ ] **INTG-03**: Pipe operator (`value |> method(args)`) continues to work unchanged
- [ ] **INTG-04**: Sum type variant access (`Shape.Circle`) continues to work unchanged
- [ ] **INTG-05**: `self` inside actor receive blocks is not affected by method calls

## Future Requirements

### Method Extensions

- **METH-06**: Inherent methods (`impl Type do ... end` without a trait)
- **METH-07**: Method references as values (`let f = value.method`)
- **METH-08**: IDE autocomplete for available methods after `.`

## Out of Scope

| Feature | Reason |
|---------|--------|
| UFCS (any function callable via dot) | Pipe operator covers this use case; UFCS blurs method/function distinction |
| Auto-ref/auto-deref on receiver | Snow has no references; all values are value-typed |
| Method overloading by parameter count | Snow does not support function overloading |
| Implicit self in method bodies | Ambiguity between locals and fields; explicit `self.x` is clearer |
| Extension methods without traits | Breaks coherence; use pipe + module functions instead |
| Dynamic dispatch / vtables | Snow uses static dispatch via monomorphization |

## Traceability

Which phases cover which requirements. Updated during roadmap creation.

| Requirement | Phase | Status |
|-------------|-------|--------|
| METH-01 | Phase 30 | Pending |
| METH-02 | Phase 30 | Pending |
| METH-03 | Phase 30 | Pending |
| METH-04 | Phase 31 | Pending |
| METH-05 | Phase 31 | Pending |
| CHAIN-01 | Phase 31 | Pending |
| CHAIN-02 | Phase 31 | Pending |
| DIAG-01 | Phase 30 | Pending |
| DIAG-02 | Phase 32 | Pending |
| DIAG-03 | Phase 32 | Pending |
| INTG-01 | Phase 32 | Pending |
| INTG-02 | Phase 32 | Pending |
| INTG-03 | Phase 32 | Pending |
| INTG-04 | Phase 32 | Pending |
| INTG-05 | Phase 32 | Pending |

**Coverage:**
- v1.6 requirements: 15 total
- Mapped to phases: 15
- Unmapped: 0

---
*Requirements defined: 2026-02-08*
*Last updated: 2026-02-08 after roadmap creation*
