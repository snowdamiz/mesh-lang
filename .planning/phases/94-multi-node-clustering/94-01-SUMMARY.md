---
phase: 94-multi-node-clustering
plan: 01
subsystem: infra
tags: [distributed, clustering, node, global-registry, env-config]

# Dependency graph
requires:
  - phase: 68-global-process-registry
    provides: "Global.register and Global.whereis runtime primitives"
  - phase: 64-node-start-connect
    provides: "Node.start, Node.connect, mesh formation"
  - phase: 65-remote-send
    provides: "Node.self, Node.list, cross-node messaging"
provides:
  - "Env-based node startup (MESHER_NODE_NAME, MESHER_COOKIE, MESHER_PEERS)"
  - "Global service registration for PipelineRegistry (CLUSTER-02)"
  - "Configurable HTTP and WebSocket ports (MESHER_HTTP_PORT, MESHER_WS_PORT)"
  - "Standalone mode fallback when env vars absent"
affects: [94-02, 94-03]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Env-based node configuration with standalone fallback"
    - "Global.register with node-specific and well-known names"
    - "Helper function extraction for case arm single-expression constraint"
    - "parse_port helper for String.to_int with default fallback"

key-files:
  created: []
  modified:
    - "mesher/main.mpl"
    - "mesher/ingestion/pipeline.mpl"
    - "crates/mesh-typeck/src/infer.rs"

key-decisions:
  - "Global.register/whereis type signatures fixed to use Pid<()> matching Process module"
  - "Helper functions for each case arm level to satisfy single-expression parser constraint"
  - "Node startup called after PG pool open but before schema creation and services"
  - "StreamManager kept node-local (connection handles are local pointers, per research pitfall 2)"

patterns-established:
  - "Env-based clustering: MESHER_NODE_NAME + MESHER_COOKIE for distributed mode, absent = standalone"
  - "Global service naming: mesher_registry@<node_name> for targeted, mesher_registry for default"
  - "register_global_services() called from both start_pipeline and restart_all_services"

# Metrics
duration: 7min
completed: 2026-02-15
---

# Phase 94 Plan 01: Node Startup and Global Service Registration Summary

**Env-based distributed node startup with Global.register for cross-node PipelineRegistry discovery**

## Performance

- **Duration:** 7 min
- **Started:** 2026-02-15T19:50:19Z
- **Completed:** 2026-02-15T19:57:41Z
- **Tasks:** 2
- **Files modified:** 3

## Accomplishments
- Mesher node starts with unique name and cookie from MESHER_NODE_NAME/MESHER_COOKIE env vars
- Node connects to seed peer via MESHER_PEERS for mesh formation (auto-gossip discovers additional peers)
- PipelineRegistry registered globally with both node-specific and well-known default names
- HTTP and WS ports configurable via MESHER_HTTP_PORT and MESHER_WS_PORT (defaults 8080/8081)
- Standalone mode is default when env vars are absent (fully backward compatible)

## Task Commits

Each task was committed atomically:

1. **Task 1: Add node startup with env-based configuration to main.mpl** - `8c180c81` (feat)
2. **Task 2: Add global service registration to pipeline.mpl** - `e5c67770` (feat)

## Files Created/Modified
- `mesher/main.mpl` - Added get_env_or_default, parse_port, start_node (with helpers), configurable ports
- `mesher/ingestion/pipeline.mpl` - Added register_global_services with Node.self check, called from start_pipeline and restart_all_services
- `crates/mesh-typeck/src/infer.rs` - Fixed Global.register/whereis type signatures to use Pid<()> matching Process module

## Decisions Made
- [94-01] Global.register/whereis type signatures fixed to Pid<()> for consistency with Process.register/whereis (runtime treats both as u64)
- [94-01] Helper functions (connect_to_peer, try_connect_peers, start_node_with, try_start_with_cookie) extracted for Mesh parser single-expression case arm constraint
- [94-01] Node startup placed after PG pool open but before schema creation (per research pitfall 4: HTTP.serve blocks)
- [94-01] StreamManager kept node-local with Process.register only (per research pitfall 2: connection handles are local pointers)
- [94-01] Global.register first-writer-wins for well-known "mesher_registry" name; node-specific names for targeted lookup

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Fixed Global.register/whereis type signatures**
- **Found during:** Task 2 (global service registration)
- **Issue:** Global.register accepted Int as second arg, but PipelineRegistry.start returns Pid<()>. Process.register already accepted Pid<()> -- inconsistency in the type checker.
- **Fix:** Changed Global.register signature from fn(String, Int) -> Int to fn(String, Pid<()>) -> Int; changed Global.whereis from fn(String) -> Int to fn(String) -> Pid<()>. Runtime representation unchanged (both are i64).
- **Files modified:** crates/mesh-typeck/src/infer.rs
- **Verification:** Full test suite passes, mesher/ compiles cleanly
- **Committed in:** e5c67770 (Task 2 commit)

**2. [Rule 1 - Bug] Restructured start_node to use helper functions**
- **Found during:** Task 1 (node startup)
- **Issue:** Nested case expressions with multi-line arms caused parse errors (Mesh parser single-expression case arm constraint, decision [88-02])
- **Fix:** Extracted connect_to_peer, try_connect_peers, start_node_with, try_start_with_cookie helpers so each case arm is a single function call
- **Files modified:** mesher/main.mpl
- **Verification:** mesher/ compiles cleanly
- **Committed in:** 8c180c81 (Task 1 commit)

---

**Total deviations:** 2 auto-fixed (2 bugs)
**Impact on plan:** Both fixes necessary for compilation. No scope creep.

## Issues Encountered
None beyond the deviations documented above.

## User Setup Required
None - no external service configuration required. The env vars (MESHER_NODE_NAME, MESHER_COOKIE, MESHER_PEERS, MESHER_HTTP_PORT, MESHER_WS_PORT) are optional; the application runs in standalone mode by default.

## Next Phase Readiness
- Node startup and global registration foundation is in place
- Plan 02 can now replace Process.whereis with Global.whereis in route handlers for cross-node service discovery
- Plan 03 can build load-based remote processor spawning on top of the mesh formed by Node.connect

---
*Phase: 94-multi-node-clustering*
*Completed: 2026-02-15*
