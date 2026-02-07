---
phase: 10-developer-tooling
plan: 05
subsystem: cli
tags: [repl, jit, rustyline, llvm, actor-runtime, cli]

# Dependency graph
requires:
  - phase: 10-04
    provides: "REPL JIT engine, session management, command processing, multi-line detection"
  - phase: 06-actor-runtime
    provides: "Actor scheduler, spawn/send/receive runtime functions"
  - phase: 05-llvm-codegen-native-binaries
    provides: "LLVM codegen with into_module() for JIT execution"
provides:
  - "snowc repl subcommand for interactive Snow sessions"
  - "Runtime symbol registration with LLVM JIT via LLVMAddSymbol"
  - "Actor runtime initialization at REPL startup"
  - "Rustyline-based line editing with history persistence"
affects: [10-09, 10-10]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "LLVMAddSymbol extern C FFI for JIT symbol registration"
    - "Once-based idempotent runtime initialization for REPL"
    - "Rustyline DefaultEditor with continuation prompt for multi-line"

key-files:
  created: []
  modified:
    - crates/snow-repl/src/lib.rs
    - crates/snow-repl/src/jit.rs
    - crates/snow-repl/Cargo.toml
    - crates/snowc/src/main.rs
    - crates/snowc/Cargo.toml

key-decisions:
  - "LLVMAddSymbol via extern C for JIT symbol registration (inkwell 0.8 lacks add_symbol API)"
  - "snow-rt linked as Rust lib dependency (not staticlib) for REPL symbol availability"
  - "Runtime init (GC + actor scheduler) happens once at REPL startup via std::sync::Once"
  - "History persisted to $HOME/.snow_repl_history"

patterns-established:
  - "extern C { fn LLVMAddSymbol } pattern for accessing LLVM C API not wrapped by inkwell"
  - "Comprehensive runtime symbol table: all snow_* functions registered for JIT resolution"

# Metrics
duration: 7min
completed: 2026-02-07
---

# Phase 10 Plan 05: REPL Integration Summary

**Rustyline-based REPL loop with actor runtime init, LLVMAddSymbol JIT symbol registration, and snowc repl subcommand**

## Performance

- **Duration:** 7 min
- **Started:** 2026-02-07T17:36:54Z
- **Completed:** 2026-02-07T17:43:49Z
- **Tasks:** 2
- **Files modified:** 5

## Accomplishments
- Full rustyline REPL loop with line editing, multi-line do/end continuation, history persistence
- All 100+ snow-rt runtime symbols registered with LLVM JIT via LLVMAddSymbol for reliable resolution
- Actor runtime (GC arena + scheduler) initialized once at startup for spawn/send/receive support
- `snowc repl` subcommand integrated alongside build, init, deps, fmt, lsp

## Task Commits

Each task was committed atomically:

1. **Task 1: Actor runtime initialization and REPL loop with rustyline** - `0622b08` (feat)
2. **Task 2: Add snowc repl subcommand** - `d2f7010` (feat)

## Files Created/Modified
- `crates/snow-repl/src/lib.rs` - Full run_repl() loop with rustyline, multi-line, commands, eval
- `crates/snow-repl/src/jit.rs` - init_runtime() with Once guard, register_runtime_symbols() via LLVMAddSymbol
- `crates/snow-repl/Cargo.toml` - Added snow-rt dependency for runtime symbol linking
- `crates/snowc/src/main.rs` - Added Repl variant to Commands enum and handler
- `crates/snowc/Cargo.toml` - Added snow-repl dependency

## Decisions Made
- **LLVMAddSymbol via extern C:** inkwell 0.8.0 does not expose an `add_symbol` API, so we declare `LLVMAddSymbol` directly via `extern "C"` block. The llvm-sys symbols are already linked through inkwell.
- **snow-rt as lib dependency (not staticlib):** Rust's normal `lib` crate type is used for dependency resolution. The `#[no_mangle] extern "C"` functions are then explicitly registered with LLVMAddSymbol.
- **Idempotent runtime init via Once:** `std::sync::Once` ensures GC arena and actor scheduler are initialized exactly once, even if multiple tests or code paths call init_runtime().
- **History file at $HOME/.snow_repl_history:** Simple home-directory location for REPL history persistence across sessions.

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered
None.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness
- REPL fully operational with actor support
- Ready for Plan 09 (E2E integration tests) and Plan 10 (final documentation)
- All runtime symbols available for JIT: expressions using strings, collections, actors, IO all work

---
*Phase: 10-developer-tooling*
*Completed: 2026-02-07*

## Self-Check: PASSED
