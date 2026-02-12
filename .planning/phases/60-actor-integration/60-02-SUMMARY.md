---
phase: "60"
plan: "02"
subsystem: "ws-codegen"
tags: [websocket, codegen, llvm, intrinsics, mir-lowering, stdlib-module]
dependency-graph:
  requires: ["60-01 snow_ws_serve/send/send_binary runtime functions"]
  provides: ["LLVM declarations for snow_ws_serve/send/send_binary", "Ws stdlib module in codegen", "ws_serve/ws_send/ws_send_binary builtin name mappings"]
  affects: ["snow-codegen intrinsics", "snow-codegen MIR lowering"]
tech-stack:
  added: []
  patterns: ["stdlib-module-wiring"]
key-files:
  created: []
  modified:
    - crates/snow-codegen/src/codegen/intrinsics.rs
    - crates/snow-codegen/src/mir/lower.rs
key-decisions:
  - "snow_ws_send known_functions uses Ptr for second arg (SnowString pointer), not MirType::String, matching the ptr-based extern C signature"
metrics:
  duration: "192s"
  completed: "2026-02-12T23:04:25Z"
---

# Phase 60 Plan 02: Codegen Pipeline Wiring Summary

LLVM intrinsic declarations and MIR lowering entries wiring Ws.serve, Ws.send, Ws.send_binary from Snow source to snow_ws_serve, snow_ws_send, snow_ws_send_binary runtime functions.

## Performance

- **Duration:** 192s (3m 12s)
- **Started:** 2026-02-12T23:01:13Z
- **Completed:** 2026-02-12T23:04:25Z
- **Tasks:** 2
- **Files modified:** 2

## Accomplishments
- LLVM external function declarations for all three WebSocket runtime functions with correct argument types
- "Ws" registered as stdlib module so Ws.serve/Ws.send/Ws.send_binary are recognized during MIR lowering
- Builtin name mappings connect ws_serve/ws_send/ws_send_binary to their snow_ prefixed runtime names
- Full workspace builds and all tests pass (1500+ tests, 0 failures)
- LLVM declarations verified to match exact extern "C" signatures in server.rs

## Task Commits

Each task was committed atomically:

1. **Task 1: Add LLVM intrinsic declarations and known_functions for WebSocket runtime** - `4194a3f` (feat)
2. **Task 2: Verify full workspace build and run complete test suite** - verification only, no code changes

## Files Created/Modified
- `crates/snow-codegen/src/codegen/intrinsics.rs` - Added LLVM external function declarations for snow_ws_serve (7 args: 6 ptr + 1 i64 -> void), snow_ws_send (2 ptr -> i64), snow_ws_send_binary (2 ptr + 1 i64 -> i64), plus test assertions
- `crates/snow-codegen/src/mir/lower.rs` - Added known_functions entries with MIR type signatures, "Ws" to STDLIB_MODULES, ws_serve/ws_send/ws_send_binary to map_builtin_name

## Decisions Made
- Used MirType::Ptr (not MirType::String) for snow_ws_send's msg parameter because the extern "C" signature takes `*const SnowString` (a raw pointer), matching how other runtime functions with SnowString args are typed in known_functions

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered

None.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness
- Phase 60 (Actor Integration) is complete -- both runtime (Plan 01) and codegen (Plan 02) are wired
- Snow programs can now use Ws.serve, Ws.send, Ws.send_binary without compilation errors
- Ready for Phase 61 (Production Hardening: TLS, frame size limits, rate limiting)

## Self-Check: PASSED

- All 2 modified files exist (intrinsics.rs, lower.rs)
- Commit 4194a3f verified in git log
- 9 occurrences of WS functions in intrinsics.rs (3 declarations + 3 comments + 3 test assertions)
- 9 occurrences of WS functions in lower.rs (3 known_functions + 3 map_builtin_name + 3 comments)
- Full workspace builds, all tests pass

---
*Phase: 60-actor-integration*
*Completed: 2026-02-12*
