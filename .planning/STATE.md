# Project State

## Project Reference

See: .planning/PROJECT.md (updated 2026-02-14)

**Core value:** Expressive, readable concurrency -- writing concurrent programs should feel as natural and clean as writing sequential code, with the safety net of supervision and fault tolerance built into the language.
**Current focus:** v9.0 Mesher Phase 87.2 (Refactor Phase 87 code to use cross-module services)

## Current Position

Phase: 87.2 of 95 (Refactor Phase 87 code to use cross-module services)
Plan: 1 of 2 in current phase
Status: In progress
Last activity: 2026-02-15 -- Completed 87.2-01 (Extract Entity Services)

Progress: [####################..........] 91% overall (245/260 plans shipped)

## Performance Metrics

**All-time Totals:**
- Plans completed: 245
- Phases completed: 88
- Milestones shipped: 18 (v1.0-v8.0)
- Lines of Rust: ~98,800
- Lines of website: ~5,500
- Lines of Mesh: ~950 (first Mesh application code)
- Timeline: 10 days (2026-02-05 -> 2026-02-14)

## Accumulated Context

### Decisions

Cleared at milestone boundary. v8.0 decisions archived in PROJECT.md.

- [87-01] Row structs use all-String fields for DB text protocol; JSONB parsed with from_json() separately
- [87-01] Recursive helper functions for iteration (Mesh has no mutable variable assignment)
- [87-01] UUID columns cast to ::text in SELECT for deriving(Row) compatibility
- [87-01] User struct excludes password_hash -- never exposed to application code
- [87-01] Flat org -> project hierarchy with org_memberships for roles
- [87-02] ~~All services in main.mpl -- cross-module service export not supported in Mesh~~ FIXED in 87.1-02
- [87-02] Explicit case matching instead of ? operator -- LLVM codegen bug with ? in Result functions
- [87-02] JSON string buffer for StorageWriter -- ~~polymorphic type variables can't cross module boundaries~~ FIXED in 87.1-02 (normalized TyVar export)
- [87-02] Timer actor pattern (recursive sleep + cast) for periodic flush -- Timer.send_after incompatible with service dispatch
- [87.1-01] Entry-block alloca placement in codegen_leaf matches existing codegen_guard pattern
- [87.1-01] Re-store to existing alloca when same variable name reused across case expressions
- [87.1-01] Defensive ptr-to-struct load in codegen_return even though current code returns struct values
- [87.1-02] Normalize TyVar IDs to sequential 0-based in exported Schemes for cross-module safety
- [87.1-02] Services exported by default (no pub prefix) since grammar lacks pub service syntax
- [87.1-02] Check service_modules before user_modules in MIR field access for generated function names
- [87.2-01] Service module convention: services/X.mpl -> from Services.X import XService
- [87.2-01] Service modules only depend on Storage.Queries and Types.*, never on each other

### Roadmap Evolution

- Phase 87.1 inserted after Phase 87: Issues Encountered (URGENT)
- Phase 87.2 inserted after Phase 87.1: Refactor Phase 87 code to use cross-module services (URGENT)

### Pending Todos

None.

### Blockers/Concerns

Research flags from research/SUMMARY.md:
- ~~List.find Option pattern matching codegen bug~~ -- FIXED in 87.1-01
- Map.collect integer key assumption -- workaround: manual Map building with fold
- Timer.send_after spawns OS thread per call -- use single recurring timer actor for alerting
- Phase 94 (Multi-Node Clustering) may need research-phase for split-brain handling

### Quick Tasks Completed

| # | Description | Date | Commit | Directory |
|---|-------------|------|--------|-----------|
| 1 | Rename project from Snow to Mesh, change .snow file extension to .mpl | 2026-02-13 | 3fe109e1 | [1-rename-project-from-snow-to-mesh-change-](./quick/1-rename-project-from-snow-to-mesh-change-/) |
| 2 | Write article: How Opus 4.6 and I Built a Production-Ready Programming Language in 9 Days | 2026-02-13 | (current) | [2-mesh-story-article](./quick/2-mesh-story-article/) |

## Session Continuity

Last session: 2026-02-15
Stopped at: Completed 87.2-01-PLAN.md (Extract Entity Services)
Resume file: None
Next action: Execute 87.2-02-PLAN.md (Extract StorageWriter)
