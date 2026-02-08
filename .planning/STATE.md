# Project State

## Project Reference

See: .planning/PROJECT.md (updated 2026-02-08)

**Core value:** Expressive, readable concurrency -- writing concurrent programs should feel as natural as sequential code, with supervision and fault tolerance built in.
**Current focus:** v1.4 Compiler Polish -- Phase 24 complete (Trait System Generics)

## Current Position

Phase: 24 of 25 (Trait System Generics)
Plan: 2 of 2 in phase
Status: Phase complete
Last activity: 2026-02-08 -- Completed 24-02-PLAN.md (Generic Type Deriving -- TGEN-02)

Progress: ████░░░░░░ ~40% (2/2 plans in phase 23 + 2/2 in phase 24; phase 25 not yet planned)

## Performance Metrics

**v1.0 Totals:**
- Plans completed: 55
- Average duration: 9min
- Total execution time: 505min
- Commits: 213
- Lines of Rust: 52,611

**v1.1 Totals:**
- Plans completed: 10
- Phases: 5 (11-15)
- Average duration: 8min
- Commits: 45
- Lines of Rust: 56,539 (+3,928)

**v1.2 Totals:**
- Plans completed: 6
- Phases: 2 (16, 17)
- Commits: 22
- Lines of Rust: 57,657 (+1,118)

**v1.3 Totals:**
- Plans completed: 18
- Phases: 5 (18-22)
- Commits: 65
- Lines of Rust: 63,189 (+5,532)
- Tests: 1,187 passing (+130 new)

**v1.4 Totals (in progress):**
- Plans completed: 4
- Phases: 2 (23, 24)
- Commits: 11
- Tests: 1,203 passing (+16 new)

## Accumulated Context

### Decisions

Decisions logged in PROJECT.md Key Decisions table.
Full decision history archived in milestones/v1.0-ROADMAP.md, milestones/v1.1-ROADMAP.md, milestones/v1.2-ROADMAP.md, and milestones/v1.3-ROADMAP.md.

| Decision | Phase | Rationale |
|----------|-------|-----------|
| Thread sum_type_defs as parameter, not in PatMatrix | 23-01 | PatMatrix is cloned frequently; reference parameter avoids data duplication |
| Fallback to appearance-order tags when type not in map | 23-01 | Preserves backward compatibility for tests using ad-hoc type names |
| Ordering as non-generic built-in sum type | 23-02 | Simpler than Option/Result; no type parameters needed |
| Primitive compare uses BinOp directly | 23-02 | Int/Float/String don't have generated Ord__lt__ functions; use hardware ops |
| Synthetic wrapper functions for nested collection Display callbacks | 24-01 | Runtime expects fn(u64)->ptr; wrappers bridge two-arg calls to one-arg callback signature |
| codegen_var module.get_function() fallback for intrinsic fn ptrs | 24-01 | Intrinsics not in self.functions map; need LLVM module lookup for callback references |
| Parametric Ty::App registration for generic struct deriving | 24-02 | TraitRegistry structural matching handles concrete instantiation lookup automatically |
| Lazy monomorphization at struct literal sites | 24-02 | Generate trait functions on demand when generic type is instantiated with concrete args |
| known_functions fallback for monomorphized type dispatch | 24-02 | Trait registry has parametric impl but dispatch needs mangled name; check known_functions |
| display_name separation for monomorphized Display/Debug | 24-02 | Show "Box(42)" not "Box_Int(42)" for human-readable output |

### Pending Todos

None.

### Blockers/Concerns

Phase 24 complete. Remaining v1.4 item: TSND-01 (Phase 25 - Tooling Stretch Goals, not yet planned).

## Session Continuity

Last session: 2026-02-08
Stopped at: Completed 24-02-PLAN.md (Generic Type Deriving -- TGEN-02)
Resume file: None
Next action: Plan and execute Phase 25 (Tooling Stretch Goals -- TSND-01)
