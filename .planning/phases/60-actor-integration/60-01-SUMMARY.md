---
phase: "60"
plan: "01"
subsystem: "ws-server"
tags: [websocket, actor, server, reader-thread-bridge, lifecycle-callbacks]
dependency-graph:
  requires: ["59-01 frame codec", "59-02 handshake and close", "actor system"]
  provides: ["snow_ws_serve", "snow_ws_send", "snow_ws_send_binary", "WS reserved type tags"]
  affects: ["ws/mod.rs", "ws/handshake.rs"]
tech-stack:
  added: []
  patterns: ["reader-thread-bridge", "reserved-type-tags", "actor-per-connection"]
key-files:
  created:
    - crates/snow-rt/src/ws/server.rs
  modified:
    - crates/snow-rt/src/ws/handshake.rs
    - crates/snow-rt/src/ws/mod.rs
key-decisions:
  - "Modified perform_upgrade in-place to return (path, headers) instead of adding new function"
  - "Used reserved type tags u64::MAX-1 through u64::MAX-4 for WS mailbox messages"
  - "Reader thread uses 5-second read timeout for periodic shutdown check"
  - "Both reader thread and actor share Arc<Mutex<TcpStream>> for writes to prevent frame interleaving"
  - "WsConnection stored on Rust heap via Box::into_raw, not GC heap"
metrics:
  duration: "216s"
  completed: "2026-02-12T22:58:41Z"
---

# Phase 60 Plan 01: WS Server + Actor Bridge Summary

Reader thread bridge pattern connecting Phase 59 WebSocket protocol layer to Snow actor system with actor-per-connection crash isolation and lifecycle callbacks.

## What Was Built

### server.rs (585 lines)
Complete WebSocket server runtime module implementing:

1. **Accept loop** (`snow_ws_serve`): Binds TCP listener, spawns one actor per accepted connection via `sched.spawn()`. Follows HTTP server pattern exactly.

2. **Actor entry** (`ws_connection_entry`): Performs upgrade handshake, splits TcpStream (read clone to reader thread, write to Arc<Mutex>), calls on_connect callback (with rejection support via close 1008), spawns reader thread, runs message loop, handles cleanup. Wrapped in `catch_unwind` for crash isolation (sends close 1011 on panic).

3. **Reader thread bridge** (`reader_thread_loop`): Dedicated OS thread per connection reads frames via `read_frame`, dispatches via `process_frame` (handles ping/pong/close), pushes data frames to actor mailbox with reserved type tags, wakes actor if Waiting. Uses 5-second read timeout for periodic shutdown check.

4. **Send functions** (`snow_ws_send`, `snow_ws_send_binary`): Lock shared write stream, call `write_frame` with Text/Binary opcode.

5. **Lifecycle callbacks** (`call_on_connect`, `call_on_message`, `call_on_close`): Invoke Snow closure pairs (fn_ptr + env_ptr) with appropriate Snow-level arguments (SnowString, map for headers).

### handshake.rs modification
Changed `perform_upgrade` return type from `Result<(), String>` to `Result<(String, Vec<(String, String)>), String>` -- now returns the request path and parsed headers on success.

### Reserved type tags
| Tag | Value | Purpose |
|-----|-------|---------|
| WS_TEXT_TAG | u64::MAX - 1 | Text frame from client |
| WS_BINARY_TAG | u64::MAX - 2 | Binary frame from client |
| WS_DISCONNECT_TAG | u64::MAX - 3 | Client disconnect or error |
| WS_CONNECT_TAG | u64::MAX - 4 | Connect notification (reserved) |

These extend the existing EXIT_SIGNAL_TAG (u64::MAX) reservation pattern from link.rs.

## Deviations from Plan

### Task Consolidation

**1. [Rule 3 - Blocking] Tasks 1 and 2 implemented as single compilation unit**
- **Found during:** Task 1
- **Issue:** The plan specified Task 1 as "server.rs skeleton with snow_ws_serve, snow_ws_send, snow_ws_send_binary" and Task 2 as "reader thread bridge, message loop, callbacks". However, server.rs must compile as a whole module -- the entry function, reader thread, message loop, and callbacks are all interdependent (they call each other, share types, and reference the same structs). Writing a "skeleton" that compiles without the implementation would require extensive stub code that would be immediately replaced.
- **Fix:** Implemented the complete server.rs in Task 1, verified Task 2 requirements against the implementation in Task 2 verification pass.
- **Files modified:** crates/snow-rt/src/ws/server.rs
- **Commit:** 58a5b53

## Verification Results

- `cargo check -p snow-rt`: Compiles successfully (2 expected warnings: dead_code on shutdown field, unused import suggestion)
- `cargo test -p snow-rt ws::`: 30 tests passed, 0 failed
- `cargo test -p snow-rt actor::`: 123 tests passed, 0 failed
- perform_upgrade updated test verifies path and headers extraction

## Commits

| Task | Commit | Description |
|------|--------|-------------|
| 1+2 | 58a5b53 | feat(60-01): create WS server skeleton with accept loop, send functions, and modified perform_upgrade |

## Self-Check: PASSED

- All 3 files exist (server.rs created, handshake.rs and mod.rs modified)
- Commit 58a5b53 verified in git log
- All 8 key functions present in server.rs
- 30 WS tests + 123 actor tests pass with no regressions
