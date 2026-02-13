---
phase: 69-cross-node-integration
plan: 02
subsystem: actor
tags: [supervisor, remote-spawn, distribution, child-spec, fault-tolerance]

# Dependency graph
requires:
  - phase: 67-remote-spawn
    provides: "snow_node_spawn with link_flag, remote PID construction, DIST_SPAWN wire protocol"
  - phase: 66-fault-propagation
    provides: "send_dist_exit, DIST_EXIT wire protocol, remote link/monitor propagation"
provides:
  - "ChildSpec with target_node/start_fn_name for remote child spawning"
  - "Remote-aware start_single_child routing to snow_node_spawn"
  - "Remote-aware terminate_single_child using send_dist_exit"
  - "Backward-compatible supervisor config wire format with optional target_node"
affects: [69-cross-node-integration]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Remote child routing via target_node Option in ChildSpec"
    - "Backward-compatible wire format extension (has_target_node byte)"

key-files:
  created: []
  modified:
    - "crates/snow-rt/src/actor/child_spec.rs"
    - "crates/snow-rt/src/actor/supervisor.rs"
    - "crates/snow-rt/src/actor/mod.rs"

key-decisions:
  - "Clone target_node/start_fn_name before mutable borrow to satisfy Rust borrow checker"
  - "Backward compat: missing has_target_node byte after child_type treated as local (pos < data.len() check)"
  - "Remote terminate is asynchronous: mark not-running immediately, supervisor receive loop handles exit signal"

patterns-established:
  - "Remote child lifecycle: spawn via snow_node_spawn(link_flag=1), terminate via send_dist_exit(Shutdown)"
  - "target_node persists in ChildSpec across restarts (restart routes to same remote node)"

# Metrics
duration: 4min
completed: 2026-02-13
---

# Phase 69 Plan 02: Remote Supervisor Children Summary

**Remote-aware supervisor that spawns, monitors, and restarts children on remote nodes via snow_node_spawn/send_dist_exit, with backward-compatible wire format**

## Performance

- **Duration:** 4 min
- **Started:** 2026-02-13T16:30:21Z
- **Completed:** 2026-02-13T16:34:38Z
- **Tasks:** 2
- **Files modified:** 3

## Accomplishments
- ChildSpec extended with `target_node` and `start_fn_name` Option<String> fields for remote spawning
- start_single_child routes to start_single_child_remote when target_node is set, calling snow_node_spawn with link_flag=1
- terminate_single_child sends DIST_EXIT for remote children (bypasses local process table)
- parse_supervisor_config reads optional target_node from wire format with full backward compatibility
- 10 new tests: remote spec fields, find_child_index with remote PIDs, remote termination, config parsing (with/without target_node)
- All 421 tests pass with zero regressions

## Task Commits

Each task was committed atomically:

1. **Task 1: Extend ChildSpec and add remote spawn/terminate to supervisor** - `b9da30b` (feat)
2. **Task 2: Update supervisor config parser and add remote supervisor tests** - `ca34a31` (feat)

## Files Created/Modified
- `crates/snow-rt/src/actor/child_spec.rs` - Added target_node and start_fn_name fields to ChildSpec
- `crates/snow-rt/src/actor/supervisor.rs` - Added start_single_child_remote, remote routing in start_single_child, remote path in terminate_single_child, 3 remote tests
- `crates/snow-rt/src/actor/mod.rs` - Extended parse_supervisor_config with backward-compatible target_node wire format, 3 config parser tests

## Decisions Made
- Clone target_node/start_fn_name from child.spec before calling start_single_child_remote (Rust borrow checker requires it since child is mutably borrowed)
- Backward compatibility: if data ends right after child_type byte (no has_target_node byte), treat as local -- existing compiled programs work without recompilation
- Remote termination is asynchronous: mark child as not-running immediately and let the supervisor's receive loop handle the actual DIST_EXIT signal back
- sup_pid is unused in start_single_child_remote since snow_node_spawn reads the current PID from stack::get_current_pid() for the link

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Fixed borrow checker conflict in start_single_child**
- **Found during:** Task 1 (start_single_child routing)
- **Issue:** Immutable borrow of child.spec.target_node conflicted with mutable borrow of child passed to start_single_child_remote
- **Fix:** Clone target_node and start_fn_name before the mutable call
- **Files modified:** crates/snow-rt/src/actor/supervisor.rs
- **Verification:** cargo build succeeds, all tests pass
- **Committed in:** b9da30b (Task 1 commit)

---

**Total deviations:** 1 auto-fixed (1 bug)
**Impact on plan:** Minor Rust ownership fix. No scope creep.

## Issues Encountered
None

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Remote supervisor children fully functional via existing distribution primitives
- Supervision trees can now span multiple nodes (CLUST-05 satisfied)
- Remote child restarts correctly route to the same remote node (target_node persists)

## Self-Check: PASSED

All files verified present. All commits verified in git log.

---
*Phase: 69-cross-node-integration*
*Completed: 2026-02-13*
