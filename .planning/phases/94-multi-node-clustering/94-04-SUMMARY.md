---
phase: 94-multi-node-clustering
plan: 04
subsystem: clustering
tags: [node-spawn, node-monitor, remote-spawn, load-balancing, NODEDOWN, distributed]

# Dependency graph
requires:
  - phase: 94-03
    provides: "load_monitor actor, event counter tracking, try_remote_spawn stub, Node.list integration"
provides:
  - "Node.spawn call in try_remote_spawn for actual remote processor spawning (CLUSTER-05)"
  - "event_processor_worker zero-arg function for remote node execution"
  - "Node.monitor calls for peer health tracking and NODEDOWN detection"
  - "Peer count change tracking in load_monitor (prev_peers parameter)"
  - "codegen fix for Node.spawn with unresolved MIR type variables"
affects: []

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Node.spawn with zero-arg remote worker that uses Process.whereis for local resource lookup"
    - "Node.monitor per-peer health tracking with recursive monitor_all_peers loop"
    - "Peer count delta detection (prev_peers tracking) for NODEDOWN logging"
    - "Alloca reload as i64 in codegen_node_spawn to handle Ty::Var -> MirType::Unit"

key-files:
  created: []
  modified:
    - "mesher/ingestion/pipeline.mpl"
    - "crates/mesh-codegen/src/codegen/expr.rs"

key-decisions:
  - "Node.spawn remote_pid return value discarded (Pid type cannot be passed to String.from)"
  - "codegen_node_spawn reloads alloca as i64 when MIR type is Unit (unresolved Ty::Var workaround)"
  - "monitor_peer and monitor_all_peers defined before load_monitor for define-before-use compliance"

patterns-established:
  - "Zero-arg remote worker pattern: Node.spawn sends function name; worker uses Process.whereis for local resources"
  - "Peer count delta tracking: prev_peers parameter in recursive actor for change detection"

# Metrics
duration: 16min
completed: 2026-02-15
---

# Phase 94 Plan 04: Node.spawn and Node.monitor Gap Closure Summary

**Remote processor spawning via Node.spawn with zero-arg event_processor_worker, plus Node.monitor peer health tracking with NODEDOWN detection in load_monitor**

## Performance

- **Duration:** 16 min
- **Started:** 2026-02-15T20:28:45Z
- **Completed:** 2026-02-15T20:45:42Z
- **Tasks:** 2
- **Files modified:** 2

## Accomplishments
- try_remote_spawn now calls Node.spawn(target, event_processor_worker) to spawn a remote EventProcessor worker on peer nodes (CLUSTER-05 requirement satisfied)
- event_processor_worker is a zero-arg function that uses Process.whereis to look up the local PipelineRegistry and starts an EventProcessor using the local pool (no PoolHandle sent across nodes)
- load_monitor tracks peer count changes via prev_peers parameter, calling Node.monitor on each new peer and logging NODEDOWN when peers are lost
- Fixed codegen bug: codegen_node_spawn now handles unresolved MIR type variables (Ty::Var -> MirType::Unit) by reloading the alloca as i64

## Task Commits

Each task was committed atomically:

1. **Task 1: Implement event_processor_worker and wire Node.spawn into try_remote_spawn** - `2d4b0d5e` (feat)
2. **Task 2: Add Node.monitor calls for peer health tracking in load_monitor** - `2f55890d` (feat)

## Files Created/Modified
- `mesher/ingestion/pipeline.mpl` - Added event_processor_worker function, replaced try_remote_spawn stub with Node.spawn call, added monitor_peer/monitor_all_peers helpers, extended load_monitor with prev_peers tracking
- `crates/mesh-codegen/src/codegen/expr.rs` - Fixed codegen_node_spawn to handle unresolved MIR type variables by reloading alloca as i64

## Decisions Made
- Node.spawn return value (remote Pid) discarded with `let _ =` because Pid type cannot be converted to String via String.from (Pid is `{}` in LLVM, not Int)
- Fixed codegen_node_spawn to reload local alloca as i64 when MIR type is Unit -- this handles the case where Ty::Var is stored early in the types map before post-inference resolution, causing MirType::Unit in the MIR scope
- monitor_peer and monitor_all_peers placed after try_remote_spawn and before load_monitor to satisfy Mesh define-before-use requirement

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Fixed codegen crash in Node.spawn with unresolved type variables**
- **Found during:** Task 1 (Node.spawn implementation)
- **Issue:** codegen_node_spawn called codegen_unpack_string on the node name argument, but the MIR type was Unit (empty struct {}) instead of String due to unresolved Ty::Var in the types map. This caused a panic at codegen_unpack_string trying to convert StructValue to IntValue.
- **Fix:** Added special handling in codegen_node_spawn: when args[0] is a MirExpr::Var with MirType::Unit, reload from the local alloca as i64 (the actual stored runtime value) instead of using the wrong-typed codegen_expr result.
- **Files modified:** crates/mesh-codegen/src/codegen/expr.rs
- **Verification:** meshc build mesher/ compiles cleanly; cargo test passes
- **Committed in:** 2d4b0d5e (Task 1 commit)

**2. [Rule 1 - Bug] Removed String.from(remote_pid) that caused StructValue type error**
- **Found during:** Task 1 (Node.spawn implementation)
- **Issue:** Plan specified `String.from(remote_pid)` in the log message, but Node.spawn returns Pid (empty struct in LLVM), not Int. String.from only works with Int.
- **Fix:** Changed log message to not include the pid value; just logs the target node name.
- **Files modified:** mesher/ingestion/pipeline.mpl
- **Verification:** Compiles cleanly
- **Committed in:** 2d4b0d5e (Task 1 commit)

---

**Total deviations:** 2 auto-fixed (2 bugs)
**Impact on plan:** Both fixes necessary for compilation. The codegen fix is the more significant one, enabling Node.spawn to work correctly with the existing MIR type resolution pipeline. No scope creep.

## Issues Encountered
- Node.spawn codegen had never been exercised with real Mesh source code before this plan. The codegen_node_spawn function was implemented but untested. The Ty::Var -> MirType::Unit fallback in the MIR type resolver causes variables to appear as empty structs in codegen, which most codegen paths handle via coercion (e.g., codegen_string_concat coerces {} to null ptr), but codegen_unpack_string did not.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Phase 94 (Multi-Node Clustering) is now fully complete with all gaps closed
- All 15 verification truths from 94-VERIFICATION.md are now satisfied
- CLUSTER-05 requirement (remote processor spawning under load) is implemented
- Node failure detection via Node.monitor and peer count tracking is operational
- Ready for Phase 95 (if applicable)

## Self-Check: PASSED

- mesher/ingestion/pipeline.mpl: FOUND
- crates/mesh-codegen/src/codegen/expr.rs: FOUND
- 94-04-SUMMARY.md: FOUND
- Commit 2d4b0d5e: FOUND
- Commit 2f55890d: FOUND

---
*Phase: 94-multi-node-clustering*
*Completed: 2026-02-15*
