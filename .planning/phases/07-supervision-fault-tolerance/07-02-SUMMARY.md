---
phase: 07-supervision-fault-tolerance
plan: 02
subsystem: compiler
tags: [parser, typeck, mir, llvm-codegen, supervisor, actor, intrinsics, e2e]

# Dependency graph
requires:
  - phase: 07-01
    provides: "Supervisor runtime (snow_supervisor_start, snow_supervisor_start_child, etc.)"
  - phase: 06-actor-runtime
    provides: "Actor spawn/send/receive infrastructure and LLVM codegen patterns"
  - phase: 05-llvm-codegen-native-binaries
    provides: "LLVM codegen framework, intrinsic declarations, linking pipeline"
provides:
  - "SUPERVISOR_DEF, CHILD_SPEC_DEF, STRATEGY_CLAUSE SyntaxKind variants and parser"
  - "SupervisorDef AST wrapper with typed accessors"
  - "infer_supervisor_def type checker registering supervisors as () -> Pid<Unit>"
  - "MirExpr::SupervisorStart and MirChildSpec for MIR representation"
  - "LLVM codegen emitting calls to snow_supervisor_start with binary config serialization"
  - "All 6 supervisor runtime intrinsic declarations in LLVM module"
  - "E2E test proving supervisor blocks compile to working native binaries"
affects: ["07-03", "08-channels-select"]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Supervisor config serialized to binary buffer with fn_ptr patching at codegen time"
    - "Supervisor definition lowers to MIR function returning SupervisorStart expression"

key-files:
  created:
    - "crates/snowc/tests/e2e_supervisors.rs"
    - "tests/e2e/supervisor_basic.snow"
  modified:
    - "crates/snow-parser/src/syntax_kind.rs"
    - "crates/snow-parser/src/parser/mod.rs"
    - "crates/snow-parser/src/parser/items.rs"
    - "crates/snow-parser/src/ast/item.rs"
    - "crates/snow-typeck/src/infer.rs"
    - "crates/snow-codegen/src/mir/mod.rs"
    - "crates/snow-codegen/src/mir/lower.rs"
    - "crates/snow-codegen/src/mir/mono.rs"
    - "crates/snow-codegen/src/codegen/expr.rs"
    - "crates/snow-codegen/src/codegen/intrinsics.rs"
    - "crates/snow-codegen/src/pattern/compile.rs"

key-decisions:
  - "Supervisors registered as zero-arg functions returning Pid<Unit> -- they don't receive user messages"
  - "Binary config buffer format: strategy(u8) + max_restarts(u32) + max_seconds(u64) + child_count(u32) + per-child specs with fn_ptr patching"
  - "Function pointers in supervisor config are patched at codegen time via stack-copied buffer with GEP stores"
  - "Child spec start closures resolved by walking CST tokens to find SPAWN_KW and extracting actor name"

patterns-established:
  - "Supervisor block parsing: supervisor Name do ... end with key-value interior (strategy, max_restarts, max_seconds, child blocks)"
  - "Child spec parsing: child Name do ... end with start/restart/shutdown key-value pairs"
  - "MIR SupervisorStart node carrying all config as data, lowered to single runtime call"

# Metrics
duration: 12min
completed: 2026-02-06
---

# Phase 7 Plan 2: Supervisor Compiler Pipeline Summary

**Full compiler pipeline for supervisor blocks: parser, type checker, MIR SupervisorStart node, LLVM codegen emitting snow_supervisor_start calls, 6 supervisor intrinsic declarations, and E2E test compiling a Snow supervisor program to a working native binary**

## Performance

- **Duration:** 12 min
- **Started:** 2026-02-06T22:45:15Z
- **Completed:** 2026-02-06T22:57:15Z
- **Tasks:** 2
- **Files modified:** 13 (11 modified, 2 created)

## Accomplishments

- `supervisor Name do ... end` syntax parses correctly into SUPERVISOR_DEF CST nodes with strategy, limits, and child spec sub-nodes
- Type checker validates supervisor definitions, registering them as zero-arg functions returning Pid<Unit>
- MIR SupervisorStart node carries all configuration (strategy, max_restarts, max_seconds, child specs with function references)
- LLVM codegen serializes supervisor config to binary buffer, patches function pointers, and calls snow_supervisor_start
- All 6 supervisor runtime intrinsics declared: snow_supervisor_start, snow_supervisor_start_child, snow_supervisor_terminate_child, snow_supervisor_count_children, snow_actor_trap_exit, snow_actor_exit
- E2E test: supervisor_basic.snow compiles to a native binary that runs and prints "supervisor started"

## Task Commits

Each task was committed atomically:

1. **Task 1: Parser and AST for supervisor blocks** - `2a2b12f` (feat)
2. **Task 2: Type checker, MIR, LLVM codegen, intrinsics, and E2E test** - `a243dff` (feat)

## Files Created/Modified

- `crates/snow-parser/src/syntax_kind.rs` - Added SUPERVISOR_DEF, CHILD_SPEC_DEF, STRATEGY_CLAUSE, RESTART_LIMIT, SECONDS_LIMIT node kinds
- `crates/snow-parser/src/parser/mod.rs` - Dispatch SUPERVISOR_KW to parse_supervisor_def for bare and pub visibility
- `crates/snow-parser/src/parser/items.rs` - Implemented parse_supervisor_def, parse_supervisor_body, parse_strategy_clause, parse_restart_limit, parse_seconds_limit, parse_child_spec, parse_child_spec_body
- `crates/snow-parser/src/ast/item.rs` - Added SupervisorDef AST wrapper with name(), strategy(), max_restarts(), max_seconds(), child_specs() accessors
- `crates/snow-typeck/src/infer.rs` - Added infer_supervisor_def registering supervisor as () -> Pid<Unit>
- `crates/snow-codegen/src/mir/mod.rs` - Added MirExpr::SupervisorStart variant and MirChildSpec struct
- `crates/snow-codegen/src/mir/lower.rs` - Implemented lower_supervisor_def extracting config from CST, generating MIR functions
- `crates/snow-codegen/src/mir/mono.rs` - Added SupervisorStart handling for function reference collection
- `crates/snow-codegen/src/pattern/compile.rs` - Added SupervisorStart arm (no sub-patterns)
- `crates/snow-codegen/src/codegen/expr.rs` - Implemented codegen_supervisor_start with binary config serialization and fn_ptr patching
- `crates/snow-codegen/src/codegen/intrinsics.rs` - Declared 6 supervisor runtime function intrinsics
- `crates/snowc/tests/e2e_supervisors.rs` - E2E test: supervisor_basic compilation and execution
- `tests/e2e/supervisor_basic.snow` - Test fixture: supervisor with one_for_one strategy and worker child

## Decisions Made

- Supervisors are registered as zero-arg functions returning Pid<Unit> -- they don't accept user messages, only manage children
- Binary config format matches 07-01 runtime expectations: strategy(u8) + max_restarts(u32 LE) + max_seconds(u64 LE) + child_count(u32 LE) + per-child specs
- Function pointers in the config are patched at codegen time by creating a stack copy of the global config and using GEP + store to write fn_ptrs
- Child spec start closures are resolved by walking CST tokens to find the actor name referenced by SPAWN_KW

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered

None.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness

- Supervisor compiler pipeline is complete -- users can write `supervisor Name do ... end` blocks and compile them
- Ready for Plan 07-03 which will add supervision tree tests, deeper type checking, and multi-level supervision
- The E2E test verifies the compilation and execution pipeline works end-to-end

## Self-Check: PASSED

---
*Phase: 07-supervision-fault-tolerance*
*Completed: 2026-02-06*
