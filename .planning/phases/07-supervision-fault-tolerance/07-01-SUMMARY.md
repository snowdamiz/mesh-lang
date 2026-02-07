---
phase: 07-supervision-fault-tolerance
plan: 01
subsystem: actor-runtime
tags: [supervisor, otp, fault-tolerance, restart-strategy, child-spec, exit-reason]

# Dependency graph
requires:
  - phase: 06-actor-runtime
    provides: "Scheduler, Process, ProcessId, link/unlink, exit propagation, mailbox, coroutine-based actors"
provides:
  - "SupervisorState with four OTP restart strategies (one_for_one, one_for_all, rest_for_one, simple_one_for_one)"
  - "ChildSpec, RestartType, ShutdownType, Strategy, ChildType types"
  - "Restart limit tracking via sliding window (max_restarts/max_seconds)"
  - "Ordered shutdown with timeout/brutal_kill in reverse start order"
  - "ExitReason expanded with Shutdown and Custom(String) variants"
  - "decode_exit_signal for round-trip parsing of exit signal messages"
  - "6 extern C ABI functions for supervisor operations"
affects:
  - 07-02 (compiler integration for supervisor start/init callbacks)
  - 07-03 (supervision tree E2E tests)

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Global supervisor state registry via OnceLock<Mutex<FxHashMap<ProcessId, Arc<Mutex<SupervisorState>>>>>"
    - "Sliding window restart limit tracking with VecDeque<Instant>"
    - "Binary config deserialization for supervisor config from compiled Snow programs"
    - "Strategy dispatch pattern: match on Strategy enum to select restart behavior"

key-files:
  created:
    - "crates/snow-rt/src/actor/supervisor.rs"
    - "crates/snow-rt/src/actor/child_spec.rs"
  modified:
    - "crates/snow-rt/src/actor/process.rs"
    - "crates/snow-rt/src/actor/link.rs"
    - "crates/snow-rt/src/actor/scheduler.rs"
    - "crates/snow-rt/src/actor/mod.rs"

key-decisions:
  - "Used OnceLock + Mutex<FxHashMap> for global supervisor state registry instead of lazy_static (no new dependency needed)"
  - "Supervisor state stored in global registry keyed by PID because coroutine entry functions only receive *const u8"
  - "Shutdown treated as non-crashing for exit propagation (same as Normal) -- Transient children do NOT restart on Shutdown"
  - "Custom(String) treated as crashing for exit propagation (same as Error)"
  - "Binary config format for snow_supervisor_start: strategy(u8) + max_restarts(u32) + max_seconds(u64) + child specs"

patterns-established:
  - "Supervisor state machine: handle_child_exit -> check restart policy -> check_restart_limit -> apply_strategy"
  - "Reverse-order shutdown: children terminated in reverse of start order for dependency-safe shutdown"
  - "Exit signal encode/decode round-trip: encode_exit_signal -> mailbox -> decode_exit_signal"

# Metrics
duration: 8min
completed: 2026-02-07
---

# Phase 7 Plan 01: Supervisor Runtime Summary

**OTP-style supervisor with four restart strategies, sliding-window restart limits, ordered shutdown, ExitReason expansion (Shutdown/Custom), and 6 extern C ABI functions -- 34 new tests, 111 total passing**

## Performance

- **Duration:** 8 min
- **Started:** 2026-02-07T03:38:30Z
- **Completed:** 2026-02-07T03:46:14Z
- **Tasks:** 2/2
- **Files modified:** 6

## Accomplishments

- Complete supervisor state machine with all four OTP strategies (one_for_one, one_for_all, rest_for_one, simple_one_for_one) -- each tested independently
- Restart limit enforcement via sliding window prevents infinite crash loops (configurable max_restarts/max_seconds)
- All three restart types (permanent/transient/temporary) follow Erlang semantics: permanent always restarts, transient only on abnormal exit, temporary never restarts and is removed
- ExitReason expanded with Shutdown (non-crashing, like Normal) and Custom(String) (crashing, like Error) variants with full encode/decode round-trip
- Six extern "C" ABI functions ready for compiler integration: snow_supervisor_start, snow_supervisor_start_child, snow_supervisor_terminate_child, snow_supervisor_count_children, snow_actor_trap_exit, snow_actor_exit

## Task Commits

Each task was committed atomically:

1. **Task 1: ExitReason expansion, exit signal encode/decode, child spec types** - `41179f5` (feat)
2. **Task 2: Supervisor state machine, strategy dispatch, restart limits, ordered shutdown, extern C ABI** - `0bcef5e` (feat)

## Files Created/Modified

- `crates/snow-rt/src/actor/supervisor.rs` - Complete supervisor runtime: SupervisorState, strategy dispatch, restart limits, child lifecycle, ordered shutdown, global state registry, 18 unit tests
- `crates/snow-rt/src/actor/child_spec.rs` - Supervision types: Strategy, RestartType, ShutdownType, ChildType, ChildSpec, ChildState, 6 unit tests
- `crates/snow-rt/src/actor/process.rs` - ExitReason expanded with Shutdown and Custom(String) variants
- `crates/snow-rt/src/actor/link.rs` - encode_exit_signal made public, decode_exit_signal and decode_reason added, Shutdown treated as non-crashing in propagate_exit, 10 new tests
- `crates/snow-rt/src/actor/scheduler.rs` - invoke_terminate_callback updated for Shutdown(4) and Custom(5) reason tags
- `crates/snow-rt/src/actor/mod.rs` - 6 extern "C" ABI functions, parse_supervisor_config, child_spec/supervisor module registration, re-exports

## Decisions Made

1. **OnceLock instead of lazy_static** -- Used `std::sync::OnceLock<Mutex<FxHashMap>>` for the global supervisor state registry to avoid adding a new dependency. The existing codebase already uses OnceLock for GLOBAL_SCHEDULER.

2. **Global registry for supervisor state** -- Since coroutine entry functions only receive `*const u8`, supervisor state must be stored externally and looked up by PID. An `Arc<Mutex<SupervisorState>>` per supervisor allows the ABI functions and the supervisor's receive loop to share state safely.

3. **Shutdown as non-crashing** -- ExitReason::Shutdown is treated identically to Normal for exit propagation: linked processes receive an exit signal message but are NOT crashed. This matches Erlang/OTP semantics where `shutdown` is a controlled termination reason.

4. **Binary config format** -- Defined a compact binary format for `snow_supervisor_start` config deserialization (strategy byte + max_restarts u32 + max_seconds u64 + child specs), enabling the compiler to emit supervisor configs directly.

## Deviations from Plan

None -- plan executed exactly as written.

## Issues Encountered

1. **Compile error with `_` in format string** -- Used `_` (wildcard) in an `assert!` format string which is not valid in expressions. Fixed by using a named loop variable `i` instead.
2. **lazy_static not in dependencies** -- Plan implied using `lazy_static::lazy_static!` but the crate was not a dependency. Replaced with `std::sync::OnceLock` which requires no additional dependency and matches the existing pattern used by `GLOBAL_SCHEDULER`.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness

- All supervisor runtime functions are ready for compiler integration in Plan 07-02
- The 6 extern "C" ABI functions provide the complete interface that compiled Snow programs will call
- Plan 07-03 (E2E tests) can exercise supervision trees through the runtime
- No blockers or concerns

## Self-Check: PASSED

---
*Phase: 07-supervision-fault-tolerance*
*Completed: 2026-02-07*
