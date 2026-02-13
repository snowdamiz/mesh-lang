---
phase: 67-remote-spawn-llvm-integration
plan: 01
subsystem: runtime, codegen
tags: [llvm, intrinsics, function-registry, remote-spawn, distributed]

# Dependency graph
requires:
  - phase: 66-remote-links-monitors
    provides: "NodeSession struct, DIST wire constants, node distribution infrastructure"
provides:
  - "FUNCTION_REGISTRY static with register/lookup APIs"
  - "LLVM intrinsic declarations for all Phase 67 runtime functions"
  - "Function registration loop in codegen main wrapper"
  - "DIST_SPAWN/DIST_SPAWN_REPLY wire format constants"
  - "pending_spawns field on NodeSession"
  - "SPAWN_REQUEST_ID atomic counter"
affects: [67-02-remote-spawn-wire-protocol, 67-03-node-spawn-api]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "FnPtr newtype wrapper for Send+Sync function pointer storage"
    - "Codegen registration loop pattern in generate_main_wrapper"

key-files:
  created: []
  modified:
    - "crates/snow-rt/src/dist/node.rs"
    - "crates/snow-rt/src/lib.rs"
    - "crates/snow-codegen/src/codegen/intrinsics.rs"
    - "crates/snow-codegen/src/codegen/mod.rs"

key-decisions:
  - "FnPtr newtype for Send+Sync: raw *const u8 cannot be stored in static; wrapping in FnPtr with unsafe impl Send+Sync is correct since function pointers are text-segment-lifetime"
  - "snow_node_start LLVM declaration has 4 params (port embedded in name string), matching runtime signature exactly"
  - "Registration loop skips closures (is_closure_fn) and internal functions (__ prefix) since they cannot be remotely spawned"

patterns-established:
  - "Function registry pattern: codegen emits snow_register_function calls at startup, runtime stores in OnceLock<RwLock<HashMap>>"

# Metrics
duration: 7min
completed: 2026-02-13
---

# Phase 67 Plan 01: Function Registry & LLVM Intrinsic Declarations Summary

**Function name registry in snow-rt with FnPtr wrapper, 10 new LLVM intrinsics for Phase 67, and codegen registration loop in main wrapper**

## Performance

- **Duration:** 7 min
- **Started:** 2026-02-13T06:43:07Z
- **Completed:** 2026-02-13T06:50:19Z
- **Tasks:** 2
- **Files modified:** 4

## Accomplishments
- Function name registry operational in snow-rt (register + lookup) with thread-safe FnPtr wrapper
- All 10 Phase 67 LLVM intrinsics declared (snow_node_start, snow_node_connect, snow_node_self, snow_node_list, snow_node_monitor, snow_node_spawn, snow_register_function, snow_process_monitor, snow_process_demonitor, snow_actor_send_named)
- Codegen emits registration calls for all top-level functions at program startup
- Wire format constants (DIST_SPAWN 0x19, DIST_SPAWN_REPLY 0x1A, SPAWN_REPLY_TAG) ready for Plan 02
- pending_spawns tracking field added to NodeSession for spawn request/reply correlation
- Zero test regressions (1585 tests passing)

## Task Commits

Each task was committed atomically:

1. **Task 1: Add function name registry and LLVM intrinsic declarations** - `d2dc395` (feat)
2. **Task 2: Emit function registration calls in codegen main wrapper** - `c779a66` (feat)

## Files Created/Modified
- `crates/snow-rt/src/dist/node.rs` - FUNCTION_REGISTRY static, FnPtr wrapper, snow_register_function/lookup_function, DIST_SPAWN constants, SPAWN_REQUEST_ID, pending_spawns on NodeSession
- `crates/snow-rt/src/lib.rs` - Re-export snow_register_function
- `crates/snow-codegen/src/codegen/intrinsics.rs` - 10 new LLVM intrinsic declarations + test assertions
- `crates/snow-codegen/src/codegen/mod.rs` - Registration loop in generate_main_wrapper

## Decisions Made
- **FnPtr newtype for Send+Sync:** Raw `*const u8` cannot be stored in `OnceLock<RwLock<FxHashMap<String, *const u8>>>` because `*const u8` does not implement `Send`/`Sync`. Wrapped in a `FnPtr` newtype with `unsafe impl Send + Sync` since function pointers point to the text segment and are valid for the entire program lifetime.
- **snow_node_start has 4 LLVM params:** The runtime signature takes `(name_ptr, name_len, cookie_ptr, cookie_len)` with port embedded in the name string. The plan originally suggested 5 params but the actual runtime has 4.
- **Skip closures and __-prefixed functions in registration:** Closures capture environment pointers and cannot be spawned remotely. Internal compiler-generated functions (prefixed with `__`) are not meaningful spawn targets.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] FnPtr wrapper for Send+Sync compliance**
- **Found during:** Task 1
- **Issue:** `*const u8` does not implement `Send`/`Sync`, so `OnceLock<RwLock<FxHashMap<String, *const u8>>>` cannot be used in a static
- **Fix:** Created `FnPtr(*const u8)` newtype with `unsafe impl Send for FnPtr` and `unsafe impl Sync for FnPtr`
- **Files modified:** `crates/snow-rt/src/dist/node.rs`
- **Verification:** `cargo build -p snow-rt` compiles cleanly
- **Committed in:** d2dc395

---

**Total deviations:** 1 auto-fixed (1 bug)
**Impact on plan:** Necessary for correctness. No scope creep.

## Issues Encountered
None.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Function registry is populated at startup, ready for Plan 02's DIST_SPAWN handler to use `lookup_function`
- All LLVM intrinsics declared for Plans 02 and 03 to emit calls to
- Wire format constants defined for Plan 02's spawn request/reply protocol
- pending_spawns field ready for Plan 02's request correlation logic

---
*Phase: 67-remote-spawn-llvm-integration*
*Completed: 2026-02-13*
