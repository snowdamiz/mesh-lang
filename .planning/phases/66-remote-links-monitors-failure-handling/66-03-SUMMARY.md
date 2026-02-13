---
phase: 66-remote-links-monitors-failure-handling
plan: 03
subsystem: actor
tags: [remote-links, dist-exit, dist-link, fault-propagation, distributed-actors]

# Dependency graph
requires:
  - phase: 66-remote-links-monitors-failure-handling
    plan: 01
    provides: "Local monitor infrastructure, ExitReason::Noconnection, encode/decode_reason"
  - phase: 66-remote-links-monitors-failure-handling
    plan: 02
    provides: "DIST_MONITOR/DEMONITOR/MONITOR_EXIT wire handlers, handle_node_disconnect, send_dist_monitor_exit"
provides:
  - "DIST_LINK/DIST_UNLINK/DIST_EXIT wire message constants (0x13, 0x14, 0x15)"
  - "send_dist_link/send_dist_unlink/send_dist_exit pub(crate) helpers"
  - "send_dist_monitor_exit_by_pid for PID-based session lookup"
  - "DIST_LINK/UNLINK/EXIT reader loop handlers in reader_loop_session"
  - "snow_actor_link extended for remote PIDs via node_id check + DIST_LINK"
  - "handle_process_exit partitions local/remote links and monitors"
  - "Remote exit signal propagation via DIST_EXIT on local process death"
  - "Remote monitor notification via DIST_MONITOR_EXIT on local process death"
affects: [66-04-integration-tests]

# Tech tracking
tech-stack:
  added: []
  patterns: [local-remote-partitioning, pid-based-session-lookup]

key-files:
  created: []
  modified:
    - crates/snow-rt/src/dist/node.rs
    - crates/snow-rt/src/actor/mod.rs
    - crates/snow-rt/src/actor/scheduler.rs

key-decisions:
  - "Partition links/monitors by node_id in handle_process_exit rather than checking at propagation time -- cleaner separation"
  - "Push all trap_exit signals before waking in handle_node_disconnect -- avoids re-acquire borrow issues"
  - "send_dist_unlink marked #[allow(dead_code)] since snow_actor_unlink not yet exposed as extern C API"

patterns-established:
  - "Local/remote partitioning: .partition(|pid| pid.node_id() == 0) before dispatch"
  - "PID-based session lookup: node_id -> node_name -> session arc for send helpers"
  - "Wire message convention: DIST_LINK/UNLINK use [tag][u64 from][u64 to], DIST_EXIT adds [reason_bytes]"

# Metrics
duration: 7min
completed: 2026-02-13
---

# Phase 66 Plan 03: Remote Links and Exit Signal Propagation Summary

**DIST_LINK/UNLINK/EXIT wire messages with remote snow_actor_link, local/remote link partitioning in handle_process_exit, and DIST_MONITOR_EXIT for remote monitor notifications**

## Performance

- **Duration:** 7 min
- **Started:** 2026-02-13T06:07:37Z
- **Completed:** 2026-02-13T06:14:43Z
- **Tasks:** 2
- **Files modified:** 3

## Accomplishments
- DIST_LINK (0x13), DIST_UNLINK (0x14), DIST_EXIT (0x15) wire constants and send helpers for remote link management
- Reader loop handles DIST_LINK (add to local links set), DIST_UNLINK (remove), and DIST_EXIT (decode reason + apply exit semantics with trap_exit support)
- snow_actor_link extended to detect remote PIDs by node_id and send DIST_LINK instead of direct link set mutation
- handle_process_exit partitions linked_pids and monitored_by into local/remote, dispatching DIST_EXIT and DIST_MONITOR_EXIT for remote entries
- send_dist_monitor_exit_by_pid helper for PID-based session lookup used in scheduler exit path

## Task Commits

Each task was committed atomically:

1. **Task 1: Add DIST_LINK/UNLINK/EXIT wire messages and extend snow_actor_link** - `316cbb0` (feat) -- included in Plan 02 combined commit
2. **Task 2: Modify handle_process_exit for remote link/monitor exit propagation** - `693c887` (feat)

**Plan metadata:** (pending final commit)

## Files Created/Modified
- `crates/snow-rt/src/dist/node.rs` - DIST_LINK/UNLINK/EXIT constants, send_dist_link/unlink/exit helpers, send_dist_monitor_exit_by_pid, reader loop handlers for all three message types
- `crates/snow-rt/src/actor/mod.rs` - snow_actor_link extended with node_id() == 0 check for local vs remote dispatch
- `crates/snow-rt/src/actor/scheduler.rs` - handle_process_exit partitions linked_pids and monitored_by into local/remote for separate dispatch paths

## Decisions Made
- Partitioning links/monitors by node_id at the top of the exit path rather than checking inside propagate_exit -- keeps propagate_exit pure-local and avoids mixing concerns
- DIST_EXIT reader handler applies full Erlang semantics: trap_exit delivers as message, non-trap_exit crashes the linked process via Linked variant
- send_dist_unlink added but marked dead_code since there's no snow_actor_unlink extern C API yet (matches Erlang where unlink is rarely needed explicitly)

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Fixed borrow-after-drop in handle_node_disconnect**
- **Found during:** Task 1 (pre-existing from Plan 02 commit 316cbb0)
- **Issue:** Code dropped proc MutexGuard, called wake_process, then tried to re-acquire from a shorter-lived `pa` binding, causing E0597 borrow error
- **Fix:** Changed to batch all trap_exit messages first, then wake once after the loop using a `need_wake` flag
- **Files modified:** crates/snow-rt/src/dist/node.rs
- **Verification:** cargo build succeeds, all 393 tests pass
- **Committed in:** 316cbb0 (already fixed in combined 66-02 commit)

---

**Total deviations:** 1 auto-fixed (1 blocking)
**Impact on plan:** Essential build fix for Plan 02 code. No scope creep.

## Issues Encountered
- Task 1 work was already committed as part of 66-02's combined commit (316cbb0), so no separate commit was needed for Task 1. The previous executor front-loaded DIST_LINK/UNLINK/EXIT work into the Plan 02 commit.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Remote links fully functional: snow_actor_link works for both local and remote PIDs
- Exit signal propagation complete: local process death sends DIST_EXIT to remote linked processes
- Remote monitor notification complete: local process death sends DIST_MONITOR_EXIT to remote monitoring processes
- All 393 tests pass, zero regressions

## Self-Check: PASSED

All 3 modified files verified present. Both task commits (316cbb0, 693c887) verified in git log.

---
*Phase: 66-remote-links-monitors-failure-handling*
*Completed: 2026-02-13*
