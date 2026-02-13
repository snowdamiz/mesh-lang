---
phase: 66-remote-links-monitors-failure-handling
plan: 01
subsystem: actor
tags: [monitors, down-signals, process-monitoring, fault-tolerance]

# Dependency graph
requires:
  - phase: 65-remote-send-distribution-router
    provides: "Remote PID routing, dist_send, node sessions"
provides:
  - "Process monitors/monitored_by fields on Process struct"
  - "ExitReason::Noconnection variant with encode/decode tag 6"
  - "DOWN_SIGNAL_TAG constant and encode_down_signal helper"
  - "next_monitor_ref() unique reference generator"
  - "snow_process_monitor/snow_process_demonitor extern C APIs"
  - "DOWN message delivery on process exit via handle_process_exit"
  - "pub(crate) encode_reason/decode_reason for wire message reuse"
affects: [66-02-remote-monitors, 66-03-remote-links]

# Tech tracking
tech-stack:
  added: []
  patterns: [bidirectional-monitor-registration, down-signal-encoding]

key-files:
  created: []
  modified:
    - crates/snow-rt/src/actor/process.rs
    - crates/snow-rt/src/actor/link.rs
    - crates/snow-rt/src/actor/mod.rs
    - crates/snow-rt/src/actor/scheduler.rs
    - crates/snow-rt/src/lib.rs

key-decisions:
  - "FxHashMap<u64, ProcessId> for monitors/monitored_by (O(1) lookup by monitor ref)"
  - "DOWN(noproc) delivered immediately when monitoring dead/nonexistent process"
  - "Remote monitor path records locally only (DIST_MONITOR deferred to Plan 02)"
  - "encode_reason/decode_reason promoted to pub(crate) for wire message reuse"

patterns-established:
  - "Monitor ref generation: global AtomicU64 counter starting at 1"
  - "DOWN signal encoding: [u64 monitor_ref][u64 monitored_pid][reason bytes]"
  - "Bidirectional monitor cleanup: monitors removed from both sides on exit/demonitor"

# Metrics
duration: 3min
completed: 2026-02-13
---

# Phase 66 Plan 01: Local Process Monitor Infrastructure Summary

**Process monitor infrastructure with FxHashMap-backed monitors/monitored_by, Noconnection exit reason, DOWN signal encoding, and snow_process_monitor/demonitor APIs delivering DOWN messages on exit**

## Performance

- **Duration:** 3 min
- **Started:** 2026-02-13T06:01:15Z
- **Completed:** 2026-02-13T06:04:47Z
- **Tasks:** 2
- **Files modified:** 5

## Accomplishments
- Process struct extended with monitors/monitored_by FxHashMap fields for bidirectional monitor tracking
- ExitReason::Noconnection variant added with encode/decode tag 6 for remote node disconnect signaling
- DOWN_SIGNAL_TAG, next_monitor_ref, encode_down_signal, and pub(crate) encode_reason/decode_reason in link.rs
- snow_process_monitor/snow_process_demonitor extern C APIs with immediate DOWN(noproc) for dead processes
- handle_process_exit delivers DOWN messages to all monitoring processes on exit

## Task Commits

Each task was committed atomically:

1. **Task 1: Add monitor fields, Noconnection variant, and DOWN signal helpers** - `10a5e80` (feat)
2. **Task 2: Add snow_process_monitor/demonitor APIs and DOWN delivery on exit** - `0059238` (feat)

**Plan metadata:** (pending final commit)

## Files Created/Modified
- `crates/snow-rt/src/actor/process.rs` - Added FxHashMap import, Noconnection variant, monitors/monitored_by fields
- `crates/snow-rt/src/actor/link.rs` - Added DOWN_SIGNAL_TAG, next_monitor_ref, encode_down_signal, Noconnection encode/decode, pub(crate) visibility
- `crates/snow-rt/src/actor/mod.rs` - Added snow_process_monitor, snow_process_demonitor, deliver_down_immediately helper
- `crates/snow-rt/src/actor/scheduler.rs` - Updated handle_process_exit for monitored_by extraction and DOWN delivery, Noconnection tag in invoke_terminate_callback
- `crates/snow-rt/src/lib.rs` - Re-exported snow_process_monitor, snow_process_demonitor

## Decisions Made
- Used FxHashMap<u64, ProcessId> for monitors/monitored_by maps -- O(1) lookup by monitor ref, consistent with existing process table patterns
- Monitoring a dead/nonexistent process delivers DOWN(noproc) immediately -- matches Erlang semantics
- Remote monitor path just records locally for now -- DIST_MONITOR wire message deferred to Plan 02
- Made encode_reason/decode_reason pub(crate) -- Plans 02 and 03 need them for wire message encoding

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered
None

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Local monitor infrastructure complete and verified (393 tests pass, zero regressions)
- Plan 02 (remote monitors) can build on: monitors/monitored_by fields, encode_down_signal, DOWN_SIGNAL_TAG, pub(crate) encode_reason/decode_reason
- Plan 03 (remote links) can use: Noconnection exit reason, encode_reason for wire exit propagation

## Self-Check: PASSED

All 5 modified files verified present. Both task commits (10a5e80, 0059238) verified in git log.

---
*Phase: 66-remote-links-monitors-failure-handling*
*Completed: 2026-02-13*
