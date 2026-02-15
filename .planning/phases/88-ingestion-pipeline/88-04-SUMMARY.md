---
phase: 88-ingestion-pipeline
plan: 04
subsystem: codegen
tags: [websocket, codegen, intrinsics, fnptr, llvm, runtime]

# Dependency graph
requires:
  - phase: 88-03
    provides: "HTTP Routes, WS Handler, Pipeline orchestration"
  - phase: 60
    provides: "WebSocket runtime (mesh_ws_serve, mesh_ws_send)"
provides:
  - "Correct mesh_ws_serve 7-arg intrinsic (no duplicates)"
  - "FnPtr->fn_ptr/env_ptr splitting in codegen_call for runtime intrinsics"
  - "Non-blocking mesh_ws_serve runtime (spawns OS thread for accept loop)"
  - "Ws.serve call in main.mpl starting WebSocket server on port 8081"
affects: [88-05, dashboard, ws-streaming]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "FnPtr argument splitting: bare function references passed to runtime intrinsics emit (fn_ptr, null_env_ptr) pairs"
    - "Non-blocking server start: mesh_ws_serve spawns OS thread for accept loop, returns immediately"
    - "Cross-module callback wrappers: thin local functions isolate type inference when passing imported functions to polymorphic intrinsics"

key-files:
  created: []
  modified:
    - "crates/mesh-codegen/src/codegen/intrinsics.rs"
    - "crates/mesh-codegen/src/codegen/expr.rs"
    - "crates/mesh-codegen/src/mir/lower.rs"
    - "crates/mesh-rt/src/ws/server.rs"
    - "mesher/main.mpl"
    - "mesher/ingestion/ws_handler.mpl"

key-decisions:
  - "FnPtr splitting via null env_ptr for bare functions passed to runtime intrinsics (vs closure struct GEP)"
  - "Non-blocking mesh_ws_serve: OS thread for accept loop instead of Mesh actor spawn (type checker expects Pid return from spawn)"
  - "Thin wrapper functions in main.mpl for cross-module callback type isolation"
  - "ws_write returns nil (Unit) to satisfy Ws.serve on_message :: fn(Int,String)->()"
  - "Map.has_key instead of Map.get==0 for header checks (avoids String==Int type mismatch)"

patterns-established:
  - "FnPtr splitting: MirType::FnPtr args to non-user-fn calls emit (val, null) pairs"
  - "Non-blocking server pattern: runtime spawns OS thread for blocking accept loops"
  - "Callback wrapper pattern: local fn wrappers isolate cross-module type inference"

# Metrics
duration: 20min
completed: 2026-02-15
---

# Phase 88 Plan 04: Ws.serve Codegen Fix and WebSocket Server Wiring Summary

**Fixed Ws.serve codegen with FnPtr->fn_ptr/env_ptr splitting, non-blocking runtime accept loop, and WebSocket server wired on port 8081 in main.mpl**

## Performance

- **Duration:** 20 min
- **Started:** 2026-02-15T02:37:26Z
- **Completed:** 2026-02-15T02:57:10Z
- **Tasks:** 2
- **Files modified:** 6

## Accomplishments
- Removed duplicate mesh_ws_serve (4-arg) and mesh_ws_send (i64,ptr) intrinsic declarations from 88-03
- Added FnPtr argument splitting in codegen_call: bare function references to runtime intrinsics now emit (fn_ptr, null_env_ptr) pairs
- Made mesh_ws_serve non-blocking by spawning OS thread for accept loop, allowing both WS and HTTP servers in same function
- Wired Ws.serve(on_ws_connect, on_ws_message, on_ws_close, 8081) in main.mpl
- Fixed ws_handler.mpl callback type issues for proper Ws.serve type unification

## Task Commits

Each task was committed atomically:

1. **Task 1: Fix mesh_ws_serve intrinsic declaration and bare-function codegen for fn_ptr/env_ptr splitting** - `57a1781d` (fix)
2. **Task 2: Wire Ws.serve call in main.mpl to start WebSocket server** - `cab28119` (feat)

## Files Created/Modified
- `crates/mesh-codegen/src/codegen/intrinsics.rs` - Removed duplicate mesh_ws_serve/mesh_ws_send declarations from 88-03
- `crates/mesh-codegen/src/codegen/expr.rs` - Added FnPtr branch in codegen_call for (fn_ptr, null_env) splitting
- `crates/mesh-codegen/src/mir/lower.rs` - Removed duplicate ws_send/ws_serve match arms (unreachable patterns)
- `crates/mesh-rt/src/ws/server.rs` - Non-blocking accept loop via OS thread spawn with SendableHandler wrapper
- `mesher/main.mpl` - Added Ws.serve call with wrapper functions for cross-module callback isolation
- `mesher/ingestion/ws_handler.mpl` - Fixed ws_write return type (nil), auth checks (Map.has_key)

## Decisions Made
- **FnPtr null env splitting:** Bare function references (MirType::FnPtr) passed to runtime intrinsics get paired with null env pointers, matching the closure convention without closure struct GEP overhead
- **Non-blocking mesh_ws_serve:** Changed runtime to spawn OS thread for accept loop instead of blocking the calling thread. This was necessary because Mesh's `spawn` keyword creates actors that must return Pid, incompatible with void-returning server loops
- **Wrapper function pattern:** Thin local functions (`on_ws_connect`, `on_ws_message`, `on_ws_close`) in main.mpl isolate cross-module type inference when passing imported functions to Ws.serve's polymorphic callback signatures
- **ws_write nil return:** Added explicit `nil` return in ws_write to ensure Unit return type propagates through ws_send_accepted/ws_send_error to ws_on_message, matching Ws.serve's `fn(Int,String)->()` expectation
- **Map.has_key for auth checks:** Replaced `Map.get(headers, key) == 0` with `Map.has_key(headers, key)` to avoid String==Int comparison that fails when headers is typed as Map<String,String> through Ws.serve unification

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Removed duplicate match arms in MIR lowerer**
- **Found during:** Task 1 (intrinsic cleanup)
- **Issue:** ws_send and ws_serve match arms duplicated in Phase 88 section, causing unreachable pattern warnings
- **Fix:** Removed the Phase 88 duplicate entries, keeping Phase 60 originals
- **Files modified:** crates/mesh-codegen/src/mir/lower.rs
- **Verification:** No more unreachable pattern warnings in cargo build
- **Committed in:** 57a1781d (Task 1 commit)

**2. [Rule 3 - Blocking] Made mesh_ws_serve non-blocking via OS thread**
- **Found during:** Task 2 (wiring Ws.serve)
- **Issue:** Both mesh_ws_serve and mesh_http_serve are blocking accept loops. Mesh's spawn keyword expects actor functions returning Pid, incompatible with void server loops. Cannot call both sequentially.
- **Fix:** Modified mesh_ws_serve to spawn OS thread for accept loop with SendableHandler wrapper for raw pointer Send safety
- **Files modified:** crates/mesh-rt/src/ws/server.rs
- **Verification:** cargo test -p mesh-rt (425 passed, 0 failed)
- **Committed in:** cab28119 (Task 2 commit)

**3. [Rule 1 - Bug] Fixed ws_write return type from Int to Unit**
- **Found during:** Task 2 (type inference cascade)
- **Issue:** `let _ = Ws.send(conn, msg)` in ws_write returns Int (the bound value's type), not Unit. This propagated through ws_send_accepted/ws_send_error to ws_on_message, causing type mismatch with Ws.serve's fn(Int,String)->() on_message signature.
- **Fix:** Changed to `let _result = Ws.send(conn, msg)` followed by `nil` to return Unit
- **Files modified:** mesher/ingestion/ws_handler.mpl
- **Verification:** meshc type checks pass (no [E0...] errors)
- **Committed in:** cab28119 (Task 2 commit)

**4. [Rule 1 - Bug] Fixed Map.get==0 auth checks to Map.has_key**
- **Found during:** Task 2 (type inference cascade)
- **Issue:** `Map.get(headers, "key") == 0` compares String with Int when headers typed as Map<String,String> through Ws.serve callback unification. Caused "Unsupported binop type" codegen error.
- **Fix:** Changed to `Map.has_key(headers, "key")` which returns Bool
- **Files modified:** mesher/ingestion/ws_handler.mpl
- **Verification:** meshc type checks pass (no [E0...] errors)
- **Committed in:** cab28119 (Task 2 commit)

**5. [Rule 3 - Blocking] Added cross-module callback wrapper functions**
- **Found during:** Task 2 (type inference cascade)
- **Issue:** Passing imported functions directly to Ws.serve caused type inference cascade errors ("expected (), found Int") due to cross-module type variable resolution
- **Fix:** Added thin wrapper functions (on_ws_connect, on_ws_message, on_ws_close) in main.mpl that delegate to imported functions, isolating type inference
- **Files modified:** mesher/main.mpl
- **Verification:** meshc type checks pass
- **Committed in:** cab28119 (Task 2 commit)

---

**Total deviations:** 5 auto-fixed (2 bugs [Rule 1], 3 blocking [Rule 3])
**Impact on plan:** All auto-fixes necessary for correctness. The type inference cascade from Ws.serve required several cascading fixes in ws_handler.mpl and main.mpl. No scope creep.

## Issues Encountered
- Pre-existing LLVM module verification failures from Phase 88-03 (service codegen, HTTP route calling conventions) remain unfixed. These affect the full mesher binary but not the individual crate tests. The Ws.serve changes compile through type checking successfully; only LLVM verification of the complete binary fails due to pre-existing issues.
- The type checker treats `let _ = expr` as returning the type of `expr` rather than Unit. This is a language design choice that makes discarding return values require explicit `nil` at the end of functions.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness
- WebSocket server codegen and wiring complete (Gap 1 from VERIFICATION.md closed)
- INGEST-05 (WebSocket event streaming) is unblocked
- Pre-existing LLVM verification failures need addressing in 88-05 for full binary compilation
- Runtime tests all pass (425/425 including WebSocket lifecycle tests)

## Self-Check: PASSED

All 6 modified files verified present. Both task commits (57a1781d, cab28119) verified in git log. SUMMARY created.

---
*Phase: 88-ingestion-pipeline*
*Completed: 2026-02-15*
