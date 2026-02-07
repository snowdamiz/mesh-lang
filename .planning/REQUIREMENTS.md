# Requirements: Snow v1.1 Language Polish

**Defined:** 2026-02-07
**Core Value:** Expressive, readable concurrency -- writing concurrent programs should feel as natural as sequential code, with supervision and fault tolerance built in.

## v1.1 Requirements

Requirements for v1.1 release. Each maps to roadmap phases.

### Syntax & Parsing

- [ ] **SYN-01**: User can define functions with multiple pattern-matched clauses (`fn fib(0) = 0`, `fn fib(1) = 1`, `fn fib(n) = fib(n-1) + fib(n-2)`)
- [ ] **SYN-02**: Multi-clause functions participate in exhaustiveness checking
- [ ] **SYN-03**: Type inference unifies across all clauses of a multi-clause function
- [ ] **SYN-04**: User can pipe into inline closures (`list |> Enum.map(fn x -> x * 2 end)`)
- [ ] **SYN-05**: Nested `do/end` and `fn/end` blocks parse correctly inside pipe chains

### Pattern Matching

- [ ] **PAT-01**: User can match on string literals in `case` expressions with compile-time code generation
- [ ] **PAT-02**: String patterns participate in exhaustiveness checking (wildcard required for non-exhaustive string matches)

### Standard Library — Maps

- [ ] **MAP-01**: Map type supports generic key and value types (`Map<K, V>`)
- [ ] **MAP-02**: `Map.put`, `Map.get`, and other Map functions work with string keys
- [ ] **MAP-03**: Map literal syntax works with string keys (`%{"name" => "Alice"}`)

### Standard Library — HTTP

- [ ] **HTTP-01**: HTTP server spawns a lightweight actor per incoming connection instead of an OS thread
- [ ] **HTTP-02**: HTTP connections benefit from actor supervision (crash isolation per connection)

## Future Requirements

Deferred to later milestones.

### Language Features

- **LANG-01**: Distributed actors across nodes
- **LANG-02**: Hot code reloading
- **LANG-03**: Macro system

## Out of Scope

Explicitly excluded from v1.1. Documented to prevent scope creep.

| Feature | Reason |
|---------|--------|
| Distributed actors | Major new feature, not a polish item |
| Hot code reloading | Requires runtime architecture changes beyond polish |
| Macros | New language feature, not fixing existing limitations |
| New standard library modules | v1.1 focuses on fixing existing modules, not adding new ones |
| Mark-sweep GC replacement | Significant runtime change, separate milestone |

## Traceability

Which phases cover which requirements. Updated during roadmap creation.

| Requirement | Phase | Status |
|-------------|-------|--------|
| SYN-01 | Phase 11 | Pending |
| SYN-02 | Phase 11 | Pending |
| SYN-03 | Phase 11 | Pending |
| SYN-04 | Phase 12 | Pending |
| SYN-05 | Phase 12 | Pending |
| PAT-01 | Phase 13 | Pending |
| PAT-02 | Phase 13 | Pending |
| MAP-01 | Phase 14 | Pending |
| MAP-02 | Phase 14 | Pending |
| MAP-03 | Phase 14 | Pending |
| HTTP-01 | Phase 15 | Pending |
| HTTP-02 | Phase 15 | Pending |

**Coverage:**
- v1.1 requirements: 12 total
- Mapped to phases: 12
- Unmapped: 0

---
*Requirements defined: 2026-02-07*
*Last updated: 2026-02-07 after roadmap creation*
