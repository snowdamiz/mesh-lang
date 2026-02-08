# Roadmap: Snow

## Milestones

- ✅ **v1.0 MVP** - Phases 1-10 (shipped 2026-02-07)
- ✅ **v1.1 Language Polish** - Phases 11-15 (shipped 2026-02-08)
- ✅ **v1.2 Runtime & Type Fixes** - Phases 16-17 (shipped 2026-02-08)
- 🚧 **v1.3 Traits & Protocols** - Phases 18-22 (in progress)

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

<details>
<summary>✅ v1.2 Runtime & Type Fixes (Phases 16-17) - SHIPPED 2026-02-08</summary>

6 plans across 2 phases. Fun() type annotation parsing (full pipeline from parser through codegen)
and mark-sweep garbage collector for per-actor heaps (conservative stack scanning, cooperative collection).
See milestones/v1.2-ROADMAP.md for details.

</details>

### 🚧 v1.3 Traits & Protocols (In Progress)

**Milestone Goal:** Complete the trait/protocol system for user-defined interfaces and impls with static dispatch via monomorphization, and ship stdlib protocols (Display, Debug, Eq, Ord, Hash, Default) that enable server-oriented abstractions.

See milestones/v1.3-ROADMAP.md for full phase details, success criteria, and pitfall tracking.

- [x] **Phase 18: Trait Infrastructure** - Fix type resolution foundation (structural matching, duplicate detection, dispatch unification)
- [x] **Phase 19: Trait Method Codegen** - Lower impl method bodies to MIR, resolve trait calls to mangled names, where clause enforcement
- [x] **Phase 20: Essential Stdlib Protocols** - Display, Debug, Eq, Ord with string interpolation integration and struct/sum-type support
- [ ] **Phase 21: Extended Protocols** - Hash, Default, default method implementations, collection Display/Debug
- [ ] **Phase 22: Auto-Derive (Stretch)** - `deriving(Eq, Ord, Display, Debug, Hash)` from struct/sum-type metadata

**Phase 21 Plans:** 4 plans

Plans:
- [ ] 21-01-PLAN.md -- Hash protocol: FNV-1a runtime, trait registration, auto-derive for structs, Map key integration
- [ ] 21-02-PLAN.md -- Default protocol: static method trait, primitive short-circuits, call-site type resolution
- [ ] 21-03-PLAN.md -- Default method implementations: parser/AST/typeck/lowering for optional interface method bodies
- [ ] 21-04-PLAN.md -- Collection Display/Debug: runtime callback helpers for List, Map, Set to_string

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
| 16. Fun() Type Parsing | v1.2 | 2/2 | Complete | 2026-02-08 |
| 17. Mark-Sweep GC | v1.2 | 4/4 | Complete | 2026-02-08 |
| 18. Trait Infrastructure | v1.3 | 3/3 | Complete | 2026-02-08 |
| 19. Trait Method Codegen | v1.3 | 4/4 | Complete | 2026-02-08 |
| 20. Essential Stdlib Protocols | v1.3 | 5/5 | Complete | 2026-02-08 |
| 21. Extended Protocols | v1.3 | 0/4 | Not started | - |
| 22. Auto-Derive (Stretch) | v1.3 | 0/2 | Not started | - |
