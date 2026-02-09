# Project State

## Project Reference

See: .planning/PROJECT.md (updated 2026-02-08)

**Core value:** Expressive, readable concurrency -- writing concurrent programs should feel as natural as sequential code, with supervision and fault tolerance built in.
**Current focus:** v1.6 Method Dot-Syntax -- Phase 30 (Core Method Resolution)

## Current Position

Phase: 30 of 32 (Core Method Resolution) -- COMPLETE
Plan: 2 of 2 in current phase
Status: Phase Complete
Last activity: 2026-02-09 -- Completed 30-02 (MIR method lowering + e2e tests)

Progress: [===.......] 33%

## Performance Metrics

**v1.0-v1.5 Totals:**
- Plans completed: 100
- Phases completed: 29
- Lines of Rust: 66,521
- Tests: 1,242 passing

**v1.6 Progress:**
- Plans completed: 2
- Phases: 3 (30-32)

| Phase | Plan | Duration | Tasks | Files |
|-------|------|----------|-------|-------|
| 30-01 | Core Method Resolution | 6min | 2 | 4 |
| 30-02 | MIR Method Lowering | 23min | 2 | 3 |

## Accumulated Context

### Decisions

Decisions logged in PROJECT.md Key Decisions table.
Full decision history archived in milestones/v1.0-ROADMAP.md through milestones/v1.5-ROADMAP.md.

v1.6 decisions:
- Method dot-syntax is pure desugaring at two integration points (type checker + MIR lowering)
- No new CST nodes, MIR nodes, or runtime mechanisms needed
- Resolution priority: module > service > variant > struct field > method (method is last)
- Retry-based method resolution in infer_call: normal inference first, method-call fallback on NoSuchField
- build_method_fn_type uses fresh type vars for non-self params (ImplMethodSig has param_count only)
- find_method_sig added as public accessor on TraitRegistry (maintains encapsulation)
- Shared resolve_trait_callee helper eliminates duplication between bare-name and dot-syntax dispatch
- Guard chain in method interception: STDLIB_MODULES > services > sum types > struct types > method call
- E2e tests use deriving() for trait impl registration (user-defined interface+impl has typeck pipeline limitation)

### Pending Todos

None.

### Blockers/Concerns

None. Research confidence HIGH across all areas.

## Session Continuity

Last session: 2026-02-09
Stopped at: Completed 30-02-PLAN.md (Phase 30 complete)
Resume file: None
Next action: Execute Phase 31 (if exists) or next milestone phase
