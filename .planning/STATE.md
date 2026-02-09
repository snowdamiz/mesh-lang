# Project State

## Project Reference

See: .planning/PROJECT.md (updated 2026-02-08)

**Core value:** Expressive, readable concurrency -- writing concurrent programs should feel as natural as sequential code, with supervision and fault tolerance built in.
**Current focus:** v1.5 Compiler Correctness -- Phase 28 complete (Trait Deriving Safety)

## Current Position

Phase: 28 of 29 (Trait Deriving Safety)
Plan: 1 of 1 in current phase
Status: Phase complete
Last activity: 2026-02-09 -- Completed 28-01-PLAN.md

Progress: ######░░░░ ~63% (v1.5: 5/8 plans)

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
- Phases: 3 (23-25)
- Commits: 13
- Lines of Rust: 64,548 (+1,359)
- Tests: 1,206 passing (+19 new)

## Accumulated Context

### Decisions

Decisions logged in PROJECT.md Key Decisions table.
Full decision history archived in milestones/v1.0-ROADMAP.md through milestones/v1.4-ROADMAP.md.

| ID | Decision | Rationale |
|----|----------|-----------|
| 26-01-D1 | Use ConstraintOrigin::Annotation for list literal unification | Matches map literal pattern |
| 26-01-D2 | Desugar list literals to list_new + list_append chain | Simplest lowering, matches map literal pattern |
| 26-01-D3 | Fix Map.keys/values to return List<K>/List<V> | Correct typing now that List is polymorphic |
| 26-02-D1 | ListLit MIR variant + snow_list_from_array replaces append chain | Single allocation O(n) vs O(n^2) append chain |
| 26-02-D2 | known_functions return Ptr, actual type from typeck resolve_range | Enables polymorphic return type conversion in codegen |
| 26-02-D3 | Uniform u64 storage with codegen-level type conversion | No runtime type tags needed, all conversion at compile time |
| 27-01-D1 | Callback-based element comparison for snow_list_eq/snow_list_compare | Matches snow_list_to_string callback pattern |
| 27-01-D2 | Parametric Eq/Ord impls for List<T> via single-letter type param | freshen_type_params unification enables matching any List<Concrete> |
| 27-01-D3 | Reuse wrap_collection_to_string for debug/inspect on collections | Same [elem1, elem2, ...] format for both Display and Debug on lists |
| 27-02-D1 | ListDecons decision tree node for cons patterns | Runtime length check + head/tail extraction doesn't fit Switch or Test nodes |
| 27-02-D2 | AccessPath::ListHead/ListTail for list sub-value navigation | Enables pattern compiler to express list element/tail access paths |
| 27-02-D3 | Local variable bindings take precedence over builtin name mappings | Pattern binding `head` was incorrectly mapped to snow_list_head |
| 27-02-D4 | Conservative exhaustiveness for cons patterns (treated as wildcards) | Lists are infinite types; cons alone is never exhaustive |
| 28-01-D1 | Emit error and early-return instead of silently adding Eq | User opted into selective deriving; respect that with a clear error and suggestion |

### Pending Todos

None.

### Blockers/Concerns

None.

## Session Continuity

Last session: 2026-02-09T01:30:27Z
Stopped at: Completed 28-01-PLAN.md (Phase 28 complete)
Resume file: None
Next action: Execute Phase 29
