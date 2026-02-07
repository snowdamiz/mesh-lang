---
phase: "06-actor-runtime"
plan: "06"
subsystem: "actor-runtime"
tags: ["linking", "exit-propagation", "registry", "terminate-callback", "trap-exit"]

dependency-graph:
  requires: ["06-01", "06-03", "06-05"]
  provides: ["bidirectional-process-linking", "exit-signal-propagation", "named-process-registry", "terminate-callback-invocation"]
  affects: ["06-07"]

tech-stack:
  added: []
  patterns: ["exit-signal-propagation", "bidirectional-linking", "named-registry-with-auto-cleanup"]

key-files:
  created:
    - "crates/snow-rt/src/actor/link.rs"
    - "crates/snow-rt/src/actor/registry.rs"
  modified:
    - "crates/snow-rt/src/actor/process.rs"
    - "crates/snow-rt/src/actor/scheduler.rs"
    - "crates/snow-rt/src/actor/mod.rs"
    - "crates/snow-rt/src/lib.rs"

decisions:
  - id: "EXIT_SIGNAL_TAG"
    decision: "u64::MAX reserved as exit signal type_tag sentinel in messages"
    reason: "No regular message should use u64::MAX, provides unambiguous exit signal identification"
  - id: "HASHSET_LINKS"
    decision: "Process.links changed from Vec<ProcessId> to HashSet<ProcessId>"
    reason: "O(1) insert/remove/contains for bidirectional link management"
  - id: "REGISTRY_REVERSE_INDEX"
    decision: "ProcessRegistry maintains PID-to-names reverse index"
    reason: "Efficient cleanup_process on exit without scanning entire name map"
  - id: "NORMAL_EXIT_DELIVERS_MESSAGE"
    decision: "Normal exit delivers exit signal as message but does NOT crash linked processes"
    reason: "Per Erlang/BEAM semantics: normal exit is informational, not a crash propagation"
  - id: "TERMINATE_BEFORE_PROPAGATION"
    decision: "Terminate callback invoked BEFORE exit signal propagation to links"
    reason: "Per user locked decision from CONTEXT.md and must_haves"

metrics:
  duration: "6min"
  completed: "2026-02-07"
---

# Phase 6 Plan 6: Process Linking, Registry, and Terminate Callbacks Summary

Bidirectional process linking with exit signal propagation, named process registry with auto-cleanup, and terminate callback invocation before exit.

## Task Commits

| Task | Name | Commit | Key Changes |
|------|------|--------|-------------|
| 1 | Bidirectional process linking, exit propagation, terminate callback | 6363998 | link.rs, process.rs (HashSet links + trap_exit), scheduler.rs (handle_process_exit) |
| 2 | Named process registry | 5ce39af | registry.rs, mod.rs (register/whereis extern C), scheduler.rs (cleanup_process), lib.rs (re-exports) |

## What Was Built

### Task 1: Bidirectional Process Linking and Exit Propagation

**link.rs** -- New module providing:
- `link(proc_a, proc_b, pid_a, pid_b)`: Create bidirectional link (idempotent via HashSet)
- `unlink(proc_a, proc_b, pid_a, pid_b)`: Remove bidirectional link
- `propagate_exit(exiting_pid, reason, linked_pids, get_process)`: Propagate exit signals
- `EXIT_SIGNAL_TAG = u64::MAX`: Sentinel for exit signal messages
- `encode_exit_signal()`: Serialize exit reason as `[u64 pid, u8 tag, ...data]`

**Exit propagation rules:**
- Normal exit: deliver `{:exit, pid, :normal}` as message -- linked process does NOT crash
- Error/Killed: deliver `{:exit, pid, reason}` -- linked process crashes with `Linked(pid, reason)`
- If target has `trap_exit = true`: deliver as message instead of crashing (supervisor pattern)
- Exited processes are skipped during propagation
- Reverse links are cleaned up during propagation

**process.rs changes:**
- `links: Vec<ProcessId>` changed to `links: HashSet<ProcessId>` for O(1) operations
- Added `trap_exit: bool` field (default false) for future supervisor support

**scheduler.rs changes:**
- `mark_exited` replaced by `handle_process_exit` with full exit lifecycle:
  1. Extract terminate callback and links under lock
  2. Invoke terminate callback (catch_unwind for panic safety)
  3. Propagate exit signals to linked processes
  4. Clean up registry registrations
  5. Mark process as Exited

### Task 2: Named Process Registry

**registry.rs** -- New module providing:
- `ProcessRegistry` with `RwLock<FxHashMap<String, ProcessId>>` for name-to-PID mapping
- `register(name, pid)` -- returns error if name already taken
- `whereis(name)` -- returns `Option<ProcessId>`
- `unregister(name)` -- explicit name removal
- `cleanup_process(pid)` -- remove all names for a PID on exit
- PID-to-names reverse index for efficient cleanup
- `global_registry()` via OnceLock for singleton access

**extern "C" functions added:**
- `snow_actor_register(name_ptr, name_len) -> u64` -- register current actor by name
- `snow_actor_whereis(name_ptr, name_len) -> u64` -- lookup by name, returns 0 if not found

**lib.rs** updated to re-export: `snow_actor_link`, `snow_actor_register`, `snow_actor_set_terminate`, `snow_actor_whereis`

## Test Coverage

77 total tests passing (was 55 before this plan):
- 12 new link/exit propagation tests in link.rs
- 8 new registry tests in registry.rs
- 7 new integration tests in mod.rs (linking, exit propagation, trap_exit, terminate callback, registry)

## Deviations from Plan

None -- plan executed exactly as written.

## Decisions Made

1. **EXIT_SIGNAL_TAG = u64::MAX** -- Reserved sentinel for exit signal messages. No regular message uses this tag.
2. **HashSet for links** -- Changed from Vec to HashSet for O(1) bidirectional link operations.
3. **Reverse index in registry** -- PID-to-names map enables O(1) cleanup without full name scan.
4. **Normal exit is informational** -- Delivers message to linked processes but does not crash them.
5. **Terminate callback before propagation** -- Per locked decision, callback runs first, then exit signals propagate.

## Next Phase Readiness

Plan 06-07 (the final plan in Phase 6) can proceed. This plan provides:
- Bidirectional linking for supervision trees
- Named registry for service discovery
- Exit propagation infrastructure for fault tolerance
- All extern "C" functions needed by the compiler codegen

No blockers identified.

## Self-Check: PASSED
