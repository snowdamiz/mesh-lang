---
phase: 68-global-registry
plan: 01
subsystem: dist
tags: [global-registry, wire-protocol, process-registration, cleanup-hooks]

# Dependency graph
requires:
  - phase: 67-remote-spawn-llvm-integration
    provides: "DIST_* wire protocol infrastructure, reader loop, ProcessId bit-packing"
  - phase: 64-cluster-foundation
    provides: "NodeState, sessions, TLS connections, write_msg/read_dist_msg"
  - phase: 66-distributed-fault-tolerance
    provides: "handle_node_disconnect, handle_process_exit cleanup hooks"
provides:
  - "GlobalRegistry struct with register/whereis/unregister/cleanup_node/cleanup_process/snapshot/merge_snapshot"
  - "DIST_GLOBAL_REGISTER (0x1B), DIST_GLOBAL_UNREGISTER (0x1C), DIST_GLOBAL_SYNC (0x1D) wire tags"
  - "Reader loop handlers for all three global registry wire messages"
  - "broadcast_global_register and broadcast_global_unregister functions"
  - "snow_global_register, snow_global_whereis, snow_global_unregister extern C runtime APIs"
  - "Process exit and node disconnect cleanup hooks for global registrations"
affects: [68-02-compiler-integration, 68-03-sync-on-connect]

# Tech tracking
tech-stack:
  added: []
  patterns: ["Single RwLock<Inner> for multi-map consistency", "PID reconstruction in reader loop via session.node_id"]

key-files:
  created:
    - "crates/snow-rt/src/dist/global.rs"
  modified:
    - "crates/snow-rt/src/dist/mod.rs"
    - "crates/snow-rt/src/dist/node.rs"
    - "crates/snow-rt/src/actor/mod.rs"
    - "crates/snow-rt/src/actor/scheduler.rs"
    - "crates/snow-rt/src/lib.rs"

key-decisions:
  - "Single RwLock<GlobalRegistryInner> wrapping all three maps (names, pid_names, node_names) to avoid deadlocks and ensure consistency"
  - "PID reconstruction on receive: reader loop replaces node_id=0 PIDs with session.node_id for correct remote routing"
  - "Broadcast pattern: collect session Arc refs, drop sessions lock, then iterate and write (follows send_peer_list)"
  - "nonode@nohost as node_name when Node.start not called (allows pre-distribution global registration)"

patterns-established:
  - "GlobalRegistry single-lock pattern: all three index maps under one RwLock for deadlock-free consistency"
  - "PID reconstruction in wire handlers: from_remote(session.node_id, session.creation, local_id) for incoming node_id=0 PIDs"

# Metrics
duration: 4min
completed: 2026-02-13
---

# Phase 68 Plan 01: Global Registry Runtime Summary

**GlobalRegistry with single-lock three-map design, three wire tags with reader-loop handlers, three extern C APIs, and process/node disconnect cleanup hooks**

## Performance

- **Duration:** 4 min
- **Started:** 2026-02-13T07:50:36Z
- **Completed:** 2026-02-13T07:54:51Z
- **Tasks:** 2
- **Files modified:** 6

## Accomplishments
- Built GlobalRegistry data structure with 8 methods (register, whereis, unregister, cleanup_node, cleanup_process, snapshot, merge_snapshot, new) using single RwLock design
- Added three wire protocol tags (0x1B, 0x1C, 0x1D) with reader loop dispatch including PID reconstruction for incoming remote PIDs
- Implemented three extern C runtime APIs (snow_global_register, snow_global_whereis, snow_global_unregister) callable from LLVM-generated code
- Wired cleanup hooks into both handle_process_exit and handle_node_disconnect with broadcast of unregister messages
- Included 10 unit tests for GlobalRegistry covering register, whereis, unregister, cleanup, snapshot/merge, concurrency

## Task Commits

Each task was committed atomically:

1. **Task 1: Create GlobalRegistry data structure and wire protocol constants** - `a995907` (feat)
2. **Task 2: Add extern "C" runtime APIs and cleanup hooks** - `45614ae` (feat)

## Files Created/Modified
- `crates/snow-rt/src/dist/global.rs` - GlobalRegistry struct, singleton, broadcast functions, 10 unit tests
- `crates/snow-rt/src/dist/mod.rs` - Added `pub mod global` declaration
- `crates/snow-rt/src/dist/node.rs` - Three wire tag constants, three reader loop handlers, node disconnect cleanup hook
- `crates/snow-rt/src/actor/mod.rs` - Three extern C functions (snow_global_register/whereis/unregister)
- `crates/snow-rt/src/actor/scheduler.rs` - Process exit cleanup hook for global registrations
- `crates/snow-rt/src/lib.rs` - Re-exported three new extern C functions

## Decisions Made
- **Single RwLock design:** Used `RwLock<GlobalRegistryInner>` wrapping all three maps instead of separate locks per map. This avoids lock ordering complexity and deadlock risk at negligible contention cost for Snow's target scale (3-20 nodes).
- **PID reconstruction on receive:** When a DIST_GLOBAL_REGISTER arrives with a PID that has node_id=0 (local to the sender), the reader loop reconstructs it using `ProcessId::from_remote(session.node_id, session.creation, pid.local_id())` so the PID routes correctly on the receiving node.
- **Broadcast safety:** Followed the established `send_peer_list` pattern of collecting session Arc references under sessions read lock, dropping the lock, then iterating and writing to each stream individually. This prevents deadlocks between sessions lock and stream lock.
- **Pre-distribution registration:** When `Node.start()` has not been called, `snow_global_register` uses "nonode@nohost" as the owning node name. The registration works locally and will be synced to peers when connections are established (via Plan 03's sync-on-connect).

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered

None.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness
- GlobalRegistry runtime is fully operational at the Rust level
- Plan 02 (compiler integration) can now add Global module type signatures, intrinsics, and codegen to expose these APIs to Snow programs
- Plan 03 (sync-on-connect) can add DIST_GLOBAL_SYNC send logic at connection establishment time

## Self-Check: PASSED

All created files exist. All commit hashes verified in git log.

---
*Phase: 68-global-registry*
*Completed: 2026-02-13*
