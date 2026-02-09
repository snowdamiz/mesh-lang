# Project State

## Project Reference

See: .planning/PROJECT.md (updated 2026-02-08)

**Core value:** Expressive, readable concurrency -- writing concurrent programs should feel as natural as sequential code, with supervision and fault tolerance built in.
**Current focus:** v1.6 Method Dot-Syntax -- Phase 32 (Diagnostics & Integration)

## Current Position

Phase: 32 of 32 (Diagnostics & Integration)
Plan: 1 of 2 in current phase
Status: In Progress
Last activity: 2026-02-09 -- Completed 32-01 (Ambiguous method diagnostics)

Progress: [==========] 100%

## Performance Metrics

**v1.0-v1.5 Totals:**
- Plans completed: 100
- Phases completed: 29
- Lines of Rust: 66,521
- Tests: 1,248 passing

**v1.6 Progress:**
- Plans completed: 5
- Phases: 3 (30-32)

| Phase | Plan | Duration | Tasks | Files |
|-------|------|----------|-------|-------|
| 30-01 | Core Method Resolution | 6min | 2 | 4 |
| 30-02 | MIR Method Lowering | 23min | 2 | 3 |
| 31-01 | Extended Method Support (typeck) | 3min | 2 | 2 |
| 31-02 | MIR Stdlib Fallback + E2E Tests | 5min | 2 | 2 |
| 32-01 | Ambiguous Method Diagnostics | 6min | 2 | 6 |

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
- Non-struct concrete types (Ty::Con, Ty::App) return Err(NoSuchField) to trigger method retry
- Stdlib module method fallback maps receiver type to module name generically (String, List, Map, Set, Range)
- Display registered for List<T>, Map<K,V>, Set in builtins for collection to_string via dot-syntax
- MIR stdlib module fallback maps MirType::String to string_ prefix, MirType::Ptr to list_ prefix
- True chaining (p.to_string().length()) and mixed field+method (p.name.length()) work end-to-end
- AmbiguousMethod span field uses TextRange, consistent with other span-bearing error variants
- Help text lists per-trait qualified syntax joined by "or" separator
- Display impl ignores span (span: _) following existing conventions

### Pending Todos

None.

### Blockers/Concerns

None. Research confidence HIGH across all areas.

## Session Continuity

Last session: 2026-02-09
Stopped at: Completed 32-01-PLAN.md
Resume file: None
Next action: Execute 32-02-PLAN.md (remaining diagnostics polish)
