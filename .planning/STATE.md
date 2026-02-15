# Project State

## Project Reference

See: .planning/PROJECT.md (updated 2026-02-14)

**Core value:** Expressive, readable concurrency -- writing concurrent programs should feel as natural and clean as writing sequential code, with the safety net of supervision and fault tolerance built into the language.
**Current focus:** v9.0 Mesher Phase 95 (React Frontend)

## Current Position

Phase: 95 of 95 (React Frontend)
Plan: 2 of 7 in current phase -- COMPLETE
Status: Plan 02 complete -- Dashboard page with Recharts charts, health stats, issue list, WebSocket live updates
Last activity: 2026-02-15 - Split-view dashboard with volume/level charts, health stat cards, compact issue list, time range selector, WS live updates

Progress: [########----------------------] 29% phase (2/7 plans shipped)

## Performance Metrics

**All-time Totals:**
- Plans completed: 271
- Phases completed: 98
- Milestones shipped: 18 (v1.0-v8.0)
- Lines of Rust: ~98,800
- Lines of website: ~5,500
- Lines of Mesh: ~4020 (first Mesh application code, refactored into modules, ingestion pipeline wired with health monitoring, error grouping pipeline, issue lifecycle API, streaming state management, backpressure buffer drain, subscription protocol and event broadcasting, search/filter/pagination REST API, dashboard aggregation and event detail endpoints, team membership and API token management, refactored with shared helpers, pipe-chained router and data transforms, alerting data foundation, alert evaluation engine, alert HTTP API routes, retention data foundation, settings API and ingestion sampling, forward-reference fixes for clean compilation, actor spawn ABI fix, distributed node startup and global service registration, cross-node service discovery via get_registry, load monitoring and remote processor spawning, Node.spawn and Node.monitor gap closure)
- Timeline: 11 days (2026-02-05 -> 2026-02-15)

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
- [88-06] HTTP.response_with_headers requires entries in builtins.rs, stdlib_modules() in infer.rs, AND intrinsics.rs for full compiler pipeline support
- [88-06] Bulk payload routed as single JSON string to EventProcessor (Json.array_get not exposed in Mesh for per-element parsing)
- [89-01] SQL-based fingerprint computation (extract_event_fields) instead of Mesh-level parsing to avoid cross-module from_json limitation
- [89-01] Triple-pipe delimiter (|||) for enriched event entries -- safe separator never appearing in JSON or UUIDs
- [89-01] issue_id and fingerprint passed as separate SQL params to insert_event (avoids JSON field injection in Mesh)
- [89-01] Mesh fingerprint module (Ingestion.Fingerprint) kept as reference; runtime path uses PostgreSQL SQL approach
- [89-02] POST routes for all issue state transitions instead of PUT/DELETE -- avoids untested HTTP method support
- [89-02] Default to 'unresolved' status filter for list endpoint -- Mesh lacks query string parsing
- [89-02] PostgreSQL jsonb extraction for assign request body parsing -- consistent with SQL-based field extraction
- [89-02] Extracted log_spike_result helper for single-expression case arm constraint in spike_checker actor
- [90-01] Map.delete for map entry removal (Map.remove not in runtime); Map.has_key extracted to let bindings before if conditions (parser limitation)
- [90-01] both_match helper for AND logic instead of && operator (LLVM PHI node codegen issue in nested if blocks)
- [90-01] Connection handle typed as Int at Mesh level consistent with Ws.send pattern (pointer cast to i64)
- [90-03] Helper functions ordered bottom-up (leaf first) for Mesh define-before-use requirement in cross-referencing chains
- [90-03] Cast handler if/else guard logic extracted to helper functions (parser limitation with branching in cast bodies)
- [90-03] 250ms drain ticker interval for responsive WS buffer flushing (cheaper than DB writes, shorter than flush_ticker)
- [90-02] Broadcast after process_event returns (in route handler), not inside EventProcessor service actor (avoids I/O bottleneck on single-threaded actor)
- [90-02] Success helper per action (resolve_success, archive_success, etc.) for single-expression case arm constraint in issue state transition handlers
- [90-02] stream_drain_ticker defined in pipeline.mpl (actors cannot be imported across modules in Mesh)
- [90-02] ModuleGraph::add_dependency rejects self-dependencies and duplicates (fixes circular dependency from 90-01/90-03 imports)
- [91-01] Search queries return raw Map rows (not typed structs) for flexible JSON serialization without cross-module issues
- [91-01] Inline to_tsvector in WHERE clause (not stored column) avoids partition complications on events table
- [91-01] Tag JSON constructed from key/value params in handler (not raw user JSON) prevents JSONB injection
- [91-02] JSONB fields (exception, stacktrace, breadcrumbs, tags, extra, user_context) embedded raw in JSON response without double-quoting
- [91-02] Two-query pattern in event detail handler: detail then neighbors, combined via helper functions for case arm constraint
- [91-02] Null neighbor IDs formatted as JSON null (not empty string) for clean API contract
- [91-02] Health summary returns numeric values without string quoting for direct consumption
- [91-03] PostgreSQL jsonb extraction (COALESCE($1::jsonb->>$2, '')) for request body field parsing, consistent with routes.mpl pattern
- [91-03] SQL-side role validation (AND $2 IN ('owner','admin','member')) ensures only valid roles at database level
- [91-03] API key revoked_at formatted as JSON null for nullable timestamps in API responses
- [91-03] extract_json_field reusable helper for PostgreSQL-based JSON body field extraction
- [91.1-01] Cross-module import of query_or_default with inferred request type works without explicit annotation (TyVar normalization from 87.1-02)
- [91.1-01] Json.encode(issue) includes project_id and fingerprint fields not in manual version -- backward compatible (additive)
- [91.1-01] Shared helper module pattern: from Api.Helpers import query_or_default, to_json_array
- [91.1-02] Multi-line pipe chains require parentheses (Mesh parser treats newlines as statement terminators at zero delimiter depth)
- [91.1-02] Comments between pipe steps break parsing -- must be removed or placed on same line
- [91.1-02] Router pipe chain inlined into HTTP.serve() to avoid let-binding scoping issue with parenthesized expressions
- [92-01] 8 pre-existing compilation errors baseline (not 7 as plan estimated) -- no new errors from alerting data foundation
- [92-02] restart_all_services moved after alert_evaluator actor for define-before-use compliance (nothing calls restart_all_services from within pipeline.mpl so safe to reorder)
- [92-03] No new compilation errors from alert HTTP routes -- 7 pre-existing errors unchanged
- [93-01] Use 'actor' not 'pub actor' -- Mesh grammar doesn't support pub before actor keyword
- [93-02] ~~Actors cannot be imported across modules in Mesh -- retention_cleaner duplicated in pipeline.mpl~~ FIXED: actors now exported/imported like services (always exported, no pub prefix)
- [93-02] Separate bulk sampling path (handle_bulk_sampled) preserves 5MB size limit vs 1MB single-event limit
- [93.1-01] main.mpl handle_top_issues type mismatch was cascading inference failure from broken Api.Detail -- no explicit return type annotation needed
- [93.2-01] Actor wrapper keeps original name, body renamed to __actor_{name}_body -- spawn references resolve to wrapper
- [93.2-01] TCE rewrite uses original actor name for matching (recursive calls use original name, not body name)
- [93.2-01] Monomorphize pass explicitly marks __actor_*_body as reachable from wrapper functions (same pattern as service dispatch)
- [94-01] Global.register/whereis type signatures fixed to Pid<()> for consistency with Process.register/whereis (runtime u64)
- [94-01] Helper functions extracted for nested case arms in start_node (Mesh single-expression case arm constraint)
- [94-01] Node startup placed after PG pool but before schema creation (HTTP.serve blocks, per research pitfall 4)
- [94-01] StreamManager kept node-local with Process.register only (connection handles are local pointers)
- [94-01] Global.register first-writer-wins for well-known "mesher_registry"; node-specific names for targeted cross-node lookup
- [94-02] Node.self() check for cluster/standalone mode instead of Pid-to-Int comparison (Pid type constraint, decision [88-05])
- [94-02] Global.whereis for cluster mode, Process.whereis for standalone mode -- both return valid Pid in their respective modes
- [94-02] StreamManager kept node-local (Process.whereis only) -- connection handles are local pointers
- [94-03] try_remote_spawn uses Global.whereis unconditionally (no Pid-to-Int null check) -- service call on null Pid returns harmless default
- [94-03] Event count threshold: 100 events per 5-second window for remote spawning consideration
- [94-03] PoolHandle never sent across nodes; remote spawning uses Global.whereis to find remote node's own registry
- [94-03] Bulk event requests count as 1 event for load rate tracking
- [94-04] Node.spawn return value (Pid) discarded -- Pid is {} in LLVM, cannot pass to String.from
- [94-04] codegen_node_spawn reloads alloca as i64 when MIR type is Unit (Ty::Var -> MirType::Unit workaround for unresolved type variables)
- [94-04] Zero-arg remote worker pattern: Node.spawn sends function name, worker uses Process.whereis for local resources (no PoolHandle across nodes)
- [95-02] Health endpoint returns single snapshot not time-series -- rendered as stat cards instead of LineChart
- [95-02] Semantic colors only for error/warning severity; info/debug/resolved/archived use monochrome theme variants
- [95-02] WebSocket updates applied optimistically to local dashboard state without full API refetch
- [95-02] formatRelativeTime extracted to lib/format.ts for cross-component reuse

### Roadmap Evolution

- Phase 87.1 inserted after Phase 87: Issues Encountered (URGENT)
- Phase 87.2 inserted after Phase 87.1: Refactor Phase 87 code to use cross-module services (URGENT)
- Phase 91.1 inserted after Phase 91: Refactor Mesher to use pipe operators and idiomatic Mesh features (URGENT)
- Phase 93.1 inserted after Phase 93: Issues Encountered (URGENT)
- Phase 93.2 inserted after Phase 93: Fix actor spawn segfault in project mode (URGENT)

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
| 3 | Validate codegen bug fixes (LLVM type coercion for service args, returns, actor messages) | 2026-02-15 | 7f429957 | [3-ensure-all-tests-still-pass-after-applyi](./quick/3-ensure-all-tests-still-pass-after-applyi/) |
| 4 | Build mesher and fix existing warnings (353 MIR false-positives + 15 Rust warnings) | 2026-02-15 | 2101b179 | [4-build-mesher-and-fix-existing-warnings-e](./quick/4-build-mesher-and-fix-existing-warnings-e/) |

## Session Continuity

Last session: 2026-02-15
Stopped at: Completed 95-02-PLAN.md (Dashboard page with charts, stats, issue list, WebSocket live updates)
Resume file: None
Next action: Continue with 95-03-PLAN.md (Issues list and Events list pages).
