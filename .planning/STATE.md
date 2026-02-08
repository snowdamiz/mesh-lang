# Project State

## Project Reference

See: .planning/PROJECT.md (updated 2026-02-08)

**Core value:** Expressive, readable concurrency -- writing concurrent programs should feel as natural as sequential code, with supervision and fault tolerance built in.
**Current focus:** v1.4 Compiler Polish -- Phase 25 complete (Type System Soundness)

## Current Position

Phase: 25 of 25 (Type System Soundness)
Plan: 1 of 1 in phase
Status: Phase complete
Last activity: 2026-02-08 -- Completed 25-01-PLAN.md (Type System Soundness -- TSND-01)

Progress: ██████████ 100% (2/2 plans in phase 23 + 2/2 in phase 24 + 1/1 in phase 25)

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

**v1.4 Totals:**
- Plans completed: 5
- Phases: 3 (23, 24, 25)
- Commits: 13
- Tests: 1,206 passing (+19 new)

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
| Clone-locally strategy for fn_constraints in infer_block | 25-01 | Avoids &mut cascade to 10+ callers; cloning small constraints map is cheap |
| Propagate constraints only for NameRef initializers | 25-01 | Closures already work via inner call; only bare function aliases need propagation |

### Pending Todos

None.

### Blockers/Concerns

v1.4 Compiler Polish milestone is complete. All 5 known limitations addressed. Higher-order function constraint propagation (e.g., `apply(show, value)`) remains a known limitation for future work.

## Session Continuity

Last session: 2026-02-08
Stopped at: Completed 25-01-PLAN.md (Type System Soundness -- TSND-01)
Resume file: None
Next action: v1.4 milestone complete. Plan next milestone or release.
