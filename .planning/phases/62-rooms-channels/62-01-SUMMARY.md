---
phase: 62-rooms-channels
plan: 01
subsystem: ws
tags: [websocket, rooms, pubsub, broadcast, concurrent-registry]

# Dependency graph
requires:
  - phase: 60-actor-integration
    provides: "WsConnection struct, ws_connection_entry actor lifecycle, Arc<Mutex<WsStream>>"
  - phase: 61-production-hardening
    provides: "Unified WsStream enum (plain + TLS), shutdown flag"
provides:
  - "RoomRegistry struct with join/leave/cleanup/members methods"
  - "snow_ws_join extern C function"
  - "snow_ws_leave extern C function"
  - "snow_ws_broadcast extern C function"
  - "snow_ws_broadcast_except extern C function"
  - "global_room_registry() singleton accessor"
  - "Automatic room cleanup on WebSocket disconnect"
affects: [62-02-codegen-wiring]

# Tech tracking
tech-stack:
  added: []
  patterns: ["RoomRegistry modeled on ProcessRegistry with dual RwLock maps", "Connection pointer as usize room key", "Snapshot-then-iterate broadcast pattern"]

key-files:
  created: [crates/snow-rt/src/ws/rooms.rs]
  modified: [crates/snow-rt/src/ws/server.rs, crates/snow-rt/src/ws/mod.rs]

key-decisions:
  - "Nested lock ordering (rooms first, conn_rooms second) for deadlock prevention"
  - "WsStream made pub(crate) alongside WsConnection for cross-module access"
  - "Room cleanup inserted before shutdown.store to prevent UAF in concurrent broadcasts"

patterns-established:
  - "RoomRegistry: dual-map concurrent registry (forward + reverse) with consistent lock ordering"
  - "Broadcast: snapshot members via read lock, release lock, iterate with shutdown flag check"

# Metrics
duration: 2min
completed: 2026-02-13
---

# Phase 62 Plan 01: Room Registry Summary

**RoomRegistry with RwLock-based concurrent maps and four extern C runtime functions (join, leave, broadcast, broadcast_except) plus automatic disconnect cleanup**

## Performance

- **Duration:** 2 min
- **Started:** 2026-02-13T01:45:28Z
- **Completed:** 2026-02-13T01:47:42Z
- **Tasks:** 2
- **Files modified:** 3

## Accomplishments
- RoomRegistry struct with dual RwLock<FxHashMap> maps (rooms + conn_rooms reverse index)
- Four extern "C" runtime functions: snow_ws_join, snow_ws_leave, snow_ws_broadcast, snow_ws_broadcast_except
- Automatic room cleanup on disconnect via cleanup_connection before Box::from_raw
- 9 new unit tests covering join/leave/cleanup/concurrency/null args
- All 332 tests pass with zero regressions

## Task Commits

Each task was committed atomically:

1. **Task 1: Create rooms.rs with RoomRegistry and four runtime functions** - `54ab7de` (feat)
2. **Task 2: Hook room cleanup into ws_connection_entry disconnect path** - `3164bd3` (feat)

## Files Created/Modified
- `crates/snow-rt/src/ws/rooms.rs` - RoomRegistry struct, global instance, 4 extern C functions, 9 unit tests
- `crates/snow-rt/src/ws/server.rs` - WsConnection/WsStream made pub(crate), cleanup_connection hook in ws_connection_entry
- `crates/snow-rt/src/ws/mod.rs` - Added pub mod rooms and re-export of global_room_registry

## Decisions Made
- **Nested lock ordering:** Always acquire rooms write lock first, then conn_rooms write lock. Consistent across join, leave, and cleanup_connection to prevent deadlock.
- **WsStream pub(crate):** Required alongside WsConnection pub(crate) because write_stream field exposes Arc<Mutex<WsStream>>. Without this, rooms.rs cannot lock the stream for broadcast writes.
- **Cleanup before shutdown:** Room cleanup inserted as the FIRST action in the ws_connection_entry cleanup path, before shutdown.store(true, ...), to prevent use-after-free when concurrent broadcasts dereference connection pointers.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Made WsStream enum pub(crate)**
- **Found during:** Task 1 (cargo check)
- **Issue:** Plan specified making WsConnection and its fields pub(crate), but WsStream (the type inside Arc<Mutex<WsStream>>) was still private. Rust's private_interfaces error prevented rooms.rs from locking write_stream.
- **Fix:** Changed `enum WsStream` to `pub(crate) enum WsStream` in server.rs
- **Files modified:** crates/snow-rt/src/ws/server.rs
- **Verification:** cargo check passes with no errors
- **Committed in:** 54ab7de (Task 1 commit)

---

**Total deviations:** 1 auto-fixed (1 blocking)
**Impact on plan:** Necessary visibility change for cross-module access. No scope creep.

## Issues Encountered
None

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Room registry runtime functions are complete and ready for codegen wiring (Plan 02)
- Plan 02 will add intrinsic declarations, known_functions entries, and map_builtin_name mappings for Ws.join/leave/broadcast/broadcast_except
- All existing infrastructure (ProcessRegistry pattern, write_frame, WsConnection) is proven and stable

---
*Phase: 62-rooms-channels*
*Completed: 2026-02-13*
