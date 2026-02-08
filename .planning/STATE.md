# Project State

## Project Reference

See: .planning/PROJECT.md (updated 2026-02-07)

**Core value:** Expressive, readable concurrency -- writing concurrent programs should feel as natural as sequential code, with supervision and fault tolerance built in.
**Current focus:** v1.2 Runtime & Type Fixes

## Current Position

Phase: 17 of 17 (Mark-Sweep Garbage Collector)
Plan: 4 of 4 in phase 17
Status: Phase complete
Last activity: 2026-02-08 -- Completed 17-04-PLAN.md

Progress: ██████████ 100% (2/2 phases complete in v1.2)

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

**v1.2 Progress:**
- Plans completed: 6
- Phases: 2 (16, 17) -- both complete
- Commits: 11

## Accumulated Context

### Decisions

Decisions logged in PROJECT.md Key Decisions table.
Full decision history archived in milestones/v1.0-ROADMAP.md and milestones/v1.1-ROADMAP.md.

| Decision | Phase | Rationale |
|----------|-------|-----------|
| Fun remains IDENT, not keyword | 16-01 | Type-position disambiguation only; avoid breaking existing code using Fun as variable name |
| FUN_TYPE placed after RESULT_TYPE | 16-01 | Groups type annotation nodes together in SyntaxKind enum |
| Fun-typed params as MirType::Closure | 16-02 | LLVM signatures must accept {ptr, ptr} structs for closure parameters |
| No closure splitting for user functions | 16-02 | Only runtime intrinsics expect split (fn_ptr, env_ptr); user functions take struct |
| GcHeader 16 bytes with dual-purpose next pointer | 17-01 | Keeps header compact; next links all-objects (when live) or free-list (when freed) |
| Global arena unchanged -- no headers | 17-01 | Per Research Pitfall 1; main thread allocates little, focus GC on actor heaps only |
| Free-list first-fit without block splitting | 17-01 | KISS for v1.2; optimize to size-segregated if profiling shows need |
| Conservative stack scanning (no type info) | 17-02 | No type maps yet; every 8-byte word treated as potential pointer -- safe but may retain some garbage |
| Worklist on system heap (Rust Vec) | 17-02 | Avoids re-entrancy: GC heap allocation during GC would be circular |
| GC at yield points only (cooperative) | 17-02 | Runs when actor yields, never interrupts other actors |
| Test code retains snow_gc_alloc | 17-03 | Tests run outside actor context; explicit global arena usage is clearer |
| Read stack_base from Process, not thread-local | 17-04 | STACK_BASE thread-local is stale when coroutines share a worker thread |
| Multi-message GC test pattern | 17-04 | 64 KiB coroutine stack limits recursion depth; use many shallow rounds instead |

### Pending Todos

None.

### Blockers/Concerns

None.

## Session Continuity

Last session: 2026-02-08
Stopped at: Completed 17-04-PLAN.md -- Phase 17 complete, v1.2 complete
Resume file: None
Next action: v1.2 is complete. All phases (16, 17) delivered.
