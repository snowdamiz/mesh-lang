# Project State

## Project Reference

See: .planning/PROJECT.md (updated 2026-02-14)

**Core value:** Expressive, readable concurrency -- writing concurrent programs should feel as natural and clean as writing sequential code, with the safety net of supervision and fault tolerance built into the language.
**Current focus:** v9.0 Mesher Phase 88 (Ingestion Pipeline)

## Current Position

Phase: 88 of 95 (Ingestion Pipeline)
Plan: 5 of 5 in current phase
Status: Phase complete (including gap closure plans 04-05)
Last activity: 2026-02-15 -- Completed 88-05 (Health Checker Actor)

Progress: [####################..........] 97% overall (253/260 plans shipped)

## Performance Metrics

**All-time Totals:**
- Plans completed: 253
- Phases completed: 90
- Milestones shipped: 18 (v1.0-v8.0)
- Lines of Rust: ~98,800
- Lines of website: ~5,500
- Lines of Mesh: ~1165 (first Mesh application code, refactored into modules, ingestion pipeline wired with health monitoring)
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
- [87.2-02] Co-locate flush_ticker actor with StorageWriter to avoid untested cross-module actor-to-service references
- [87.2-02] Apply ? operator only in flush_loop; keep explicit case in retry functions where Err branch calls retry logic
- [88-01] Headers field added as third field in MeshHttpResponse (after status, body) -- backward compatible with null default
- [88-01] Direct MeshMap entries array iteration (offset 16, [u64;2] per entry) for header extraction -- avoids mesh_map_to_list allocation
- [88-02] Map.get returns 0 for missing keys -- correct default for Int counter tracking in RateLimiter
- [88-02] EventProcessor delegates validation to caller due to cross-module from_json not resolving for imported types
- [88-02] Service call handlers use inferred return types; explicit :: Result annotation causes type check failure
- [88-02] Mesh parser single-expression case arms: extract multi-line logic into helper functions
- [88-03] PipelineRegistry service pattern for HTTP handler context -- closures not supported in HTTP routing
- [88-03] MeshString-based Process.register/whereis runtime functions for compiler-generated string args
- [88-03] SEND_KW added to parser field access for Ws.send module-qualified access
- [88-03] ~~Ws.serve deferred -- type inference cascade with callback signatures needs investigation~~ FIXED in 88-04
- [88-04] FnPtr splitting: bare function refs to runtime intrinsics emit (fn_ptr, null_env_ptr) pairs
- [88-04] Non-blocking mesh_ws_serve: OS thread for accept loop (Mesh spawn expects Pid return, incompatible with void server loops)
- [88-04] Cross-module callback wrappers isolate type inference when passing imported functions to Ws.serve
- [88-04] ws_write returns nil (Unit) to satisfy Ws.serve on_message fn(Int,String)->() signature
- [88-04] Map.has_key instead of Map.get==0 for header checks (avoids String==Int type mismatch under Ws.serve unification)
- [88-05] Timer.sleep + recursive call for health_checker actor -- established pattern per 87-02, Timer.send_after incompatible with typed dispatch
- [88-05] PipelineRegistry.get_pool as liveness probe -- service call success implies all services responsive
- [88-05] Pid liveness comparison deferred -- Process.whereis returns Pid type, Pid > 0 comparison needs future Pid.to_int support

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
Stopped at: Completed 88-04-PLAN.md (gap closure -- Ws.serve codegen fix and wiring)
Resume file: None
Next action: Phase 88 fully complete (all 5 plans including gap closure). Next phase: 89 (Dashboard)
