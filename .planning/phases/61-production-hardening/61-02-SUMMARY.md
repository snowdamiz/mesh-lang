---
phase: 61-production-hardening
plan: 02
subsystem: codegen
tags: [llvm, intrinsics, mir, websocket, tls, codegen-wiring]

# Dependency graph
requires:
  - phase: 60-actor-integration
    provides: "snow_ws_serve LLVM declaration pattern and MIR wiring conventions"
  - phase: 61-production-hardening plan 01
    provides: "snow_ws_serve_tls runtime function in snow-rt"
provides:
  - "LLVM external function declaration for snow_ws_serve_tls (9-arg, void return)"
  - "MIR known_functions entry with correct type signature"
  - "Builtin name mapping ws_serve_tls -> snow_ws_serve_tls"
  - "Ws.serve_tls callable from Snow source code"
affects: [62-rooms]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "WS TLS codegen wiring follows same Ptr convention as WS plain-text functions"

key-files:
  created: []
  modified:
    - "crates/snow-codegen/src/codegen/intrinsics.rs"
    - "crates/snow-codegen/src/mir/lower.rs"

key-decisions:
  - "Used MirType::Ptr (not MirType::String) for cert/key SnowString pointer args, consistent with WS function family convention from Phase 60-02"

patterns-established:
  - "WS function codegen uses MirType::Ptr for all SnowString pointer arguments"

# Metrics
duration: 6min
completed: 2026-02-12
---

# Phase 61 Plan 02: Codegen Wiring for ws_serve_tls Summary

**LLVM intrinsic declaration and MIR wiring for snow_ws_serve_tls enabling Ws.serve_tls() calls from Snow source**

## Performance

- **Duration:** 6 min
- **Started:** 2026-02-12T23:58:41Z
- **Completed:** 2026-02-13T00:04:57Z
- **Tasks:** 2
- **Files modified:** 2

## Accomplishments
- Added 9-argument LLVM external function declaration (6 ptr + 1 i64 + 2 ptr, void return) matching the runtime extern "C" signature exactly
- Added MIR known_functions entry with MirType::Ptr for SnowString pointer arguments, following WS function family convention
- Added map_builtin_name mapping so Snow source `Ws.serve_tls(...)` compiles to `snow_ws_serve_tls(...)` in LLVM IR
- Full workspace builds and all 1515 tests pass with zero regressions

## Task Commits

Each task was committed atomically:

1. **Task 1: Add LLVM intrinsic declaration and MIR wiring for snow_ws_serve_tls** - `aea608b` (feat)
2. **Task 2: Verify full workspace build and run complete test suite** - verification only, no code changes

## Files Created/Modified
- `crates/snow-codegen/src/codegen/intrinsics.rs` - Added snow_ws_serve_tls LLVM declaration + test assertion
- `crates/snow-codegen/src/mir/lower.rs` - Added known_functions entry + map_builtin_name mapping

## Decisions Made
- Used MirType::Ptr (not MirType::String) for cert_path and key_path arguments, consistent with the WS function family convention established in Phase 60-02. The HTTP TLS function uses MirType::String, but WS functions use Ptr for SnowString pointer arguments.

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered
None.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Phase 61 (Production Hardening) is complete: TLS, heartbeat, fragmentation (plan 01) and codegen wiring (plan 02) are done
- WebSocket TLS is fully wired from Snow source through codegen to runtime
- Ready for Phase 62 (Rooms/Channels) which builds on the WebSocket foundation

## Self-Check: PASSED

- FOUND: crates/snow-codegen/src/codegen/intrinsics.rs
- FOUND: crates/snow-codegen/src/mir/lower.rs
- FOUND: .planning/phases/61-production-hardening/61-02-SUMMARY.md
- FOUND: aea608b (Task 1 commit)

---
*Phase: 61-production-hardening*
*Completed: 2026-02-12*
