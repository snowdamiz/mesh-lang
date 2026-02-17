---
phase: 103-refactor-orm-to-eliminate-raw-sql-and-rewrite-mesher
plan: 01
subsystem: runtime
tags: [json, serde_json, intrinsics, stdlib, mesh-rt]

# Dependency graph
requires:
  - phase: 97-schema-metadata-sql-generation
    provides: "ORM pattern: pure Rust helpers + extern C wrappers + compiler pipeline registration"
  - phase: 102-mesher-rewrite
    provides: "5 JSONB-parsing Pool.query call sites identified for replacement"
provides:
  - "Json.get(body, key) intrinsic for top-level JSON field extraction"
  - "Json.get_nested(body, path1, path2) intrinsic for nested JSON field extraction"
  - "5 non-storage JSONB Pool.query calls eliminated from mesher"
affects: [103-02, 103-03, 103-04, 103-05]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Json.get/Json.get_nested: serde_json-backed field extraction without DB roundtrip"
    - "COALESCE-equivalent: empty string return on missing/invalid JSON (matches PostgreSQL behavior)"

key-files:
  created:
    - "crates/mesh-rt/src/db/json.rs"
  modified:
    - "crates/mesh-rt/src/db/mod.rs"
    - "crates/mesh-rt/src/lib.rs"
    - "crates/mesh-codegen/src/codegen/intrinsics.rs"
    - "crates/mesh-codegen/src/mir/lower.rs"
    - "crates/mesh-typeck/src/infer.rs"
    - "crates/mesh-repl/src/jit.rs"
    - "mesher/ingestion/routes.mpl"
    - "mesher/ingestion/ws_handler.mpl"
    - "mesher/ingestion/pipeline.mpl"
    - "mesher/api/alerts.mpl"
    - "mesher/api/team.mpl"

key-decisions:
  - "Json.get returns empty string on missing/invalid JSON (matches COALESCE($1::jsonb->>$2, '') pattern from all 5 call sites)"
  - "Json.get_nested traverses exactly two levels (sufficient for ws_handler.mpl nested extraction; deeper nesting not needed)"
  - "Pure Rust helpers separated from extern C wrappers for unit testability (follows orm.rs, migration.rs pattern)"

patterns-established:
  - "db::json module: serde_json field extraction without GC allocation for intermediates"
  - "value_to_string: PG ->> operator behavior (strings returned bare, numbers/bools stringified, null returns empty)"

# Metrics
duration: 7min
completed: 2026-02-17
---

# Phase 103 Plan 01: JSON Field Extraction Summary

**Mesh-native Json.get/Json.get_nested intrinsics backed by serde_json, replacing 5 PostgreSQL JSONB-parsing roundtrips with in-process extraction**

## Performance

- **Duration:** 7 min
- **Started:** 2026-02-17T00:08:58Z
- **Completed:** 2026-02-17T00:15:49Z
- **Tasks:** 2
- **Files modified:** 12

## Accomplishments
- Created db/json.rs runtime module with mesh_json_get (2-arg) and mesh_json_get_nested (3-arg) intrinsics
- Registered both functions through the full compiler pipeline (intrinsics.rs, lower.rs, infer.rs, jit.rs)
- Replaced all 5 non-storage JSONB Pool.query calls with Json.get/Json.get_nested across mesher
- 15 unit tests covering all extraction patterns (string, number, bool, null, missing, invalid, nested, dynamic key)

## Task Commits

Each task was committed atomically:

1. **Task 1: Implement Json runtime module with serde_json extraction** - `76a39b73` (feat)
2. **Task 2: Replace all 5 non-storage JSONB Pool.query calls** - `1ee8cfde` (feat)

## Files Created/Modified
- `crates/mesh-rt/src/db/json.rs` - JSON field extraction runtime (mesh_json_get, mesh_json_get_nested)
- `crates/mesh-rt/src/db/mod.rs` - Module registration (pub mod json)
- `crates/mesh-rt/src/lib.rs` - Re-exports (mesh_json_get, mesh_json_get_nested)
- `crates/mesh-codegen/src/codegen/intrinsics.rs` - LLVM function declarations (ptr, ptr -> ptr signatures)
- `crates/mesh-codegen/src/mir/lower.rs` - Known functions + map_builtin_name entries
- `crates/mesh-typeck/src/infer.rs` - Json module method type signatures (String, String -> String)
- `crates/mesh-repl/src/jit.rs` - JIT symbol mappings for REPL
- `mesher/ingestion/routes.mpl` - handle_assign_issue: Json.get replaces Pool.query JSONB
- `mesher/ingestion/ws_handler.mpl` - handle_subscribe_update: Json.get_nested replaces Pool.query nested JSONB
- `mesher/ingestion/pipeline.mpl` - extract_condition_field: Json.get replaces Pool.query JSONB
- `mesher/api/alerts.mpl` - handle_toggle_alert_rule: Json.get with default replaces Pool.query JSONB
- `mesher/api/team.mpl` - extract_json_field: Json.get replaces Pool.query JSONB

## Decisions Made
- Json.get returns empty string on missing/invalid JSON (matches COALESCE($1::jsonb->>$2, '') pattern from all 5 call sites)
- Json.get_nested traverses exactly two levels (sufficient for ws_handler.mpl nested extraction; deeper nesting not needed)
- Pure Rust helpers separated from extern C wrappers for unit testability (follows orm.rs, migration.rs convention)
- "Json" module already existed in STDLIB_MODULE_NAMES/STDLIB_MODULES -- added get/get_nested methods to existing module (no new module needed)

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered

None.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness
- Json.get/Json.get_nested are fully operational as Mesh stdlib functions
- All 5 JSONB-parsing roundtrips eliminated
- Ready for Plan 02 (Query builder extensions) which addresses remaining raw SQL in queries.mpl

## Self-Check: PASSED

All created files verified to exist. All commits verified in git log.

---
*Phase: 103-refactor-orm-to-eliminate-raw-sql-and-rewrite-mesher*
*Completed: 2026-02-17*
