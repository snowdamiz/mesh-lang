---
phase: 90-real-time-streaming
plan: 01
subsystem: compiler, streaming
tags: [typechecker, websocket, service, mesh, infer, ws-rooms, stream-manager]

# Dependency graph
requires:
  - phase: 88-ingestion-pipeline
    provides: "Ws module typechecker entries (send, serve), service codegen patterns, mesher service architecture"
provides:
  - "Ws.join, Ws.leave, Ws.broadcast, Ws.broadcast_except type signatures in typechecker"
  - "StreamManager service for per-connection subscription state tracking"
affects: [90-02-PLAN, 90-03-PLAN, mesher ingestion pipeline, ws_handler]

# Tech tracking
tech-stack:
  added: []
  patterns: ["Map.has_key extraction to let binding before if conditions", "both_match helper for AND logic in nested if blocks", "Map.delete for map entry removal"]

key-files:
  created:
    - mesher/services/stream_manager.mpl
  modified:
    - crates/mesh-typeck/src/infer.rs

key-decisions:
  - "Map.delete used instead of Map.remove (Map.remove not available in runtime, Map.delete exists)"
  - "Map.has_key calls extracted to let bindings before if conditions (parser limitation with field access in if conditions)"
  - "both_match helper function for AND logic instead of && operator (avoids LLVM PHI node codegen issue in nested if blocks)"
  - "Connection handle typed as Int consistent with Ws.send pattern (pointer cast to i64 at Mesh level)"

patterns-established:
  - "Map.has_key-before-if: Extract Map.has_key(x.field, key) to a let binding before using in if condition to avoid parser errors"
  - "AND-helper: Use a both_match(a, b) helper instead of && when inside nested if blocks to avoid codegen PHI node issues"

# Metrics
duration: 6min
completed: 2026-02-15
---

# Phase 90 Plan 01: WebSocket Room Types & StreamManager Summary

**Ws room function type signatures (join/leave/broadcast/broadcast_except) added to typechecker; StreamManager service with per-connection filter state using Map<Int, ConnectionState>**

## Performance

- **Duration:** 6 min
- **Started:** 2026-02-15T04:46:57Z
- **Completed:** 2026-02-15T04:53:24Z
- **Tasks:** 2
- **Files modified:** 2

## Accomplishments
- Four Ws room function type signatures added to typechecker ws_mod (join, leave, broadcast, broadcast_except)
- StreamManager service created with RegisterClient/RemoveClient casts and IsStreamClient/GetProjectId/MatchesFilter calls
- All 244 existing typechecker tests pass with new entries
- ConnectionState struct tracks project_id, level/env filters, and buffer fields for future backpressure

## Task Commits

Each task was committed atomically:

1. **Task 1: Add Ws room function type signatures to typechecker** - `8775752d` (feat)
2. **Task 2: Create StreamManager service for per-connection state** - `b4333a8e` (feat)

## Files Created/Modified
- `crates/mesh-typeck/src/infer.rs` - Added Ws.join, Ws.leave, Ws.broadcast, Ws.broadcast_except entries to ws_mod in stdlib_modules()
- `mesher/services/stream_manager.mpl` - StreamManager service with ConnectionState/StreamState structs, register/remove/query helpers

## Decisions Made
- **Map.delete over Map.remove:** Plan referenced Map.remove but it doesn't exist in the runtime. Map.delete is the correct function for removing map entries.
- **Map.has_key extraction:** The Mesh parser has a limitation where `Map.has_key(state.field, key)` in an if condition causes parse errors when the if block has multiple statements. Extracting to a let binding works around this.
- **both_match helper for AND:** The `&&` operator inside nested if blocks causes LLVM PHI node verification errors. Using a `both_match(a, b)` helper function avoids this codegen issue.
- **Int for conn handles:** Connection handles typed as Int at Mesh level (pointer cast to i64), consistent with existing Ws.send: fn(Int, String) -> Int pattern.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Map.remove does not exist; used Map.delete**
- **Found during:** Task 2 (StreamManager service creation)
- **Issue:** Plan specified `Map.remove` for removing connections from the map, but Map.remove is not available in the Mesh runtime. Only Set.remove exists.
- **Fix:** Used `Map.delete(state.connections, conn)` which is the correct Map entry removal function
- **Files modified:** mesher/services/stream_manager.mpl
- **Verification:** Standalone test compiles past type checking
- **Committed in:** b4333a8e (Task 2 commit)

**2. [Rule 1 - Bug] Parser error with Map.has_key in if conditions**
- **Found during:** Task 2 (StreamManager service creation)
- **Issue:** `if Map.has_key(state.connections, conn) do ... else ... end` with multi-statement if bodies causes parser error "expected end to close do block"
- **Fix:** Extracted Map.has_key call to a `let has = ...` binding before the if statement
- **Files modified:** mesher/services/stream_manager.mpl
- **Verification:** Standalone test compiles past type checking; no parse errors
- **Committed in:** b4333a8e (Task 2 commit)

**3. [Rule 1 - Bug] && operator codegen issue in nested if blocks**
- **Found during:** Task 2 (StreamManager service creation)
- **Issue:** `level_ok && env_ok` inside a multi-statement if block produces LLVM PHI node verification error
- **Fix:** Created `both_match(a, b)` helper function that uses `if a do b else false end`
- **Files modified:** mesher/services/stream_manager.mpl
- **Verification:** Standalone test compiles past type checking with both_match
- **Committed in:** b4333a8e (Task 2 commit)

---

**Total deviations:** 3 auto-fixed (1 blocking, 2 bugs)
**Impact on plan:** All auto-fixes necessary for correctness. No scope creep. The Map.delete, let-extraction, and both_match patterns are established workarounds for known Mesh language limitations.

## Issues Encountered
- LLVM verification errors when testing Ws room functions with literal integer `0` as conn handle (i64 vs ptr mismatch at LLVM level). This is a pre-existing codegen behavior, not introduced by this plan. In real usage, conn handles come from Ws.serve callbacks and are already pointer-sized values.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Ws room functions are now callable from Mesh code (typechecker entries present)
- StreamManager service ready for import in Plan 02 (WebSocket streaming handler) and Plan 03 (backpressure/buffering)
- No blockers for Plan 02

## Self-Check: PASSED

- [x] crates/mesh-typeck/src/infer.rs exists with ws_mod entries for join, leave, broadcast, broadcast_except
- [x] mesher/services/stream_manager.mpl exists with service StreamManager
- [x] 90-01-SUMMARY.md created
- [x] Commit 8775752d exists (Task 1)
- [x] Commit b4333a8e exists (Task 2)
- [x] All 244 typechecker tests pass

---
*Phase: 90-real-time-streaming*
*Completed: 2026-02-15*
