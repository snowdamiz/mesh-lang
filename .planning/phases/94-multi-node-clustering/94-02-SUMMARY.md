---
phase: 94-multi-node-clustering
plan: 02
subsystem: infra
tags: [distributed, clustering, service-discovery, global-registry, cross-node]

# Dependency graph
requires:
  - phase: 94-multi-node-clustering-01
    provides: "Global.register for PipelineRegistry, env-based node startup"
  - phase: 68-global-process-registry
    provides: "Global.register and Global.whereis runtime primitives"
provides:
  - "Cluster-aware get_registry() helper in Api.Helpers"
  - "All HTTP/WS handler files use get_registry() for cross-node service discovery (CLUSTER-03)"
  - "StreamManager lookups remain node-local (connection handles are local pointers)"
affects: [94-03]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Cluster-aware service discovery: Node.self() check to select Global.whereis vs Process.whereis"
    - "Shared helper module pattern for cross-cutting cluster concerns"

key-files:
  created: []
  modified:
    - "mesher/api/helpers.mpl"
    - "mesher/ingestion/routes.mpl"
    - "mesher/ingestion/ws_handler.mpl"
    - "mesher/api/search.mpl"
    - "mesher/api/dashboard.mpl"
    - "mesher/api/detail.mpl"
    - "mesher/api/team.mpl"
    - "mesher/api/alerts.mpl"
    - "mesher/api/settings.mpl"

key-decisions:
  - "Node.self() check for cluster/standalone mode instead of Pid-to-Int comparison (Pid type cannot be compared to 0)"
  - "Global.whereis in cluster mode, Process.whereis in standalone mode"
  - "StreamManager kept node-local (Process.whereis only) per research pitfall 2"

patterns-established:
  - "get_registry() as single entry point for PipelineRegistry lookup across all handlers"
  - "Node.self() == '' as standalone mode detection (empty string when Node.start not called)"

# Metrics
duration: 5min
completed: 2026-02-15
---

# Phase 94 Plan 02: Cross-Node Service Discovery Summary

**Cluster-aware get_registry() helper replacing 41 hardcoded Process.whereis calls across all HTTP/WS handlers with Node.self-based Global.whereis fallback**

## Performance

- **Duration:** 5 min
- **Started:** 2026-02-15T20:00:12Z
- **Completed:** 2026-02-15T20:05:32Z
- **Tasks:** 2
- **Files modified:** 9

## Accomplishments
- Added cluster-aware `get_registry()` function to `Api.Helpers` that uses `Global.whereis` in cluster mode and `Process.whereis` in standalone mode
- Replaced all 41 `Process.whereis("mesher_registry")` calls in handler files with `get_registry()`
- All 8 handler files import `get_registry` from `Api.Helpers`
- StreamManager lookups remain node-local (4 occurrences in ws_handler.mpl untouched)
- Pipeline health_checker retains its local-only `Process.whereis` check (intentional local health probe)

## Task Commits

Each task was committed atomically:

1. **Task 1: Add cluster-aware get_registry() helper to api/helpers.mpl** - `b21fa5f1` (feat)
2. **Task 2: Replace Process.whereis with get_registry() across all handlers** - `386f9dee` (feat)
3. **Fix: Pid type comparison constraint in get_registry()** - `755afa03` (fix)

## Files Created/Modified
- `mesher/api/helpers.mpl` - Added get_registry() with Node.self-based cluster/standalone routing
- `mesher/ingestion/routes.mpl` - 11 replacements + import + updated header comment
- `mesher/ingestion/ws_handler.mpl` - 2 replacements + new import line
- `mesher/api/search.mpl` - 4 replacements + import
- `mesher/api/dashboard.mpl` - 6 replacements + import
- `mesher/api/detail.mpl` - 1 replacement + import
- `mesher/api/team.mpl` - 7 replacements + import
- `mesher/api/alerts.mpl` - 7 replacements + import
- `mesher/api/settings.mpl` - 3 replacements + import

## Decisions Made
- [94-02] Node.self() check for cluster/standalone mode instead of Pid-to-Int comparison (Pid<()> type cannot be compared with Int literal 0, decision [88-05] deferred Pid.to_int)
- [94-02] Global.whereis used in cluster mode (cross-node discovery), Process.whereis in standalone mode (zero overhead)
- [94-02] StreamManager kept node-local with Process.whereis only (connection handles are raw local pointers, per research pitfall 2)

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Pid type cannot be compared to Int literal 0**
- **Found during:** Task 2 verification (compilation)
- **Issue:** Plan specified `if local != 0` to check Process.whereis result, but Process.whereis returns `Pid<()>` not `Int`. The Mesh type checker rejects `Pid != Int` comparisons. This was a known limitation (decision [88-05]: "Pid > 0 comparison needs future Pid.to_int support").
- **Fix:** Replaced the local-first/global-fallback pattern with a Node.self() string check: if Node.self() returns non-empty, use Global.whereis (cluster mode); otherwise use Process.whereis (standalone mode). Both return valid Pids in their respective modes.
- **Files modified:** mesher/api/helpers.mpl
- **Verification:** `meshc build mesher/` compiles with no new errors (3 pre-existing errors from plan 03 staging in pipeline.mpl)
- **Committed in:** 755afa03

---

**Total deviations:** 1 auto-fixed (1 bug)
**Impact on plan:** Necessary type system workaround. The semantic intent (cluster-aware discovery) is preserved -- the implementation just uses a different branching mechanism (Node.self check vs Pid comparison).

## Issues Encountered
None beyond the deviation documented above.

## User Setup Required
None - no external service configuration required. Cluster mode activates automatically when MESHER_NODE_NAME/MESHER_COOKIE env vars are set (configured in plan 01).

## Next Phase Readiness
- All handlers now use cluster-aware service discovery
- Plan 03 can build load-based remote processor spawning -- the load_monitor actor in pipeline.mpl (pre-staged) already uses Global.whereis for cross-node registry lookup
- Application works identically in standalone mode (backward compatible)

## Self-Check: PASSED

All 9 modified files verified on disk. All 3 task commits verified in git log. SUMMARY.md exists.

---
*Phase: 94-multi-node-clustering*
*Completed: 2026-02-15*
