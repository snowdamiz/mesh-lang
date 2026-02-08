# Project State

## Project Reference

See: .planning/PROJECT.md (updated 2026-02-08)

**Core value:** Expressive, readable concurrency -- writing concurrent programs should feel as natural as sequential code, with supervision and fault tolerance built in.
**Current focus:** v1.4 Compiler Polish -- Phase 23 complete, Phase 24 next

## Current Position

Phase: 23 of 25 (Pattern Matching Codegen)
Plan: 2 of 2 in phase
Status: Phase complete
Last activity: 2026-02-08 -- Completed 23-02-PLAN.md (Ordering type, compare function, e2e tests)

Progress: ██░░░░░░░░ ~20% (2/2 plans in phase 23; phases 24-25 not yet planned)

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
- Plans completed: 2
- Phases: 1 (23)
- Commits: 4
- Tests: 1,196 passing (+9 new)

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

### Pending Todos

None.

### Blockers/Concerns

PATM-01 and PATM-02 are now resolved. Remaining v1.4 items: TGEN-01, TGEN-02, TSND-01.

## Session Continuity

Last session: 2026-02-08
Stopped at: Completed 23-02-PLAN.md (Phase 23 complete)
Resume file: None
Next action: Plan and execute Phase 24 (TGEN-01/TGEN-02)
