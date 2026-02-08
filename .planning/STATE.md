# Project State

## Project Reference

See: .planning/PROJECT.md (updated 2026-02-08)

**Core value:** Expressive, readable concurrency -- writing concurrent programs should feel as natural as sequential code, with supervision and fault tolerance built in.
**Current focus:** v1.4 Compiler Polish -- Phase 23 (Pattern Matching Codegen)

## Current Position

Phase: 23 of 25 (Pattern Matching Codegen)
Plan: 1 of 2 in phase
Status: In progress
Last activity: 2026-02-08 -- Completed 23-01-PLAN.md (pattern compiler tag/type fix)

Progress: █░░░░░░░░░ ~10% (1/2 plans in phase 23; phases 24-25 not yet planned)

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

## Accumulated Context

### Decisions

Decisions logged in PROJECT.md Key Decisions table.
Full decision history archived in milestones/v1.0-ROADMAP.md, milestones/v1.1-ROADMAP.md, milestones/v1.2-ROADMAP.md, and milestones/v1.3-ROADMAP.md.

| Decision | Phase | Rationale |
|----------|-------|-----------|
| Thread sum_type_defs as parameter, not in PatMatrix | 23-01 | PatMatrix is cloned frequently; reference parameter avoids data duplication |
| Fallback to appearance-order tags when type not in map | 23-01 | Preserves backward compatibility for tests using ad-hoc type names |

### Pending Todos

None.

### Blockers/Concerns

None -- all five v1.3 limitations are now tracked as v1.4 requirements (PATM-01, PATM-02, TGEN-01, TGEN-02, TSND-01).

## Session Continuity

Last session: 2026-02-08
Stopped at: Completed 23-01-PLAN.md
Resume file: None
Next action: Execute 23-02-PLAN.md (Ordering type registration, compare method, e2e tests)
