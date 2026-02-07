---
phase: 06-actor-runtime
plan: 05
subsystem: codegen
tags: [actor, codegen, llvm, mir, lowering, reduction-check, pid]

dependency-graph:
  requires: ["06-01", "06-02", "06-03", "06-04"]
  provides:
    - "Complete actor code generation pipeline: AST -> MIR -> LLVM IR"
    - "Reduction check instrumentation at all call sites"
    - "Runtime stubs for snow_actor_link and snow_actor_set_terminate"
  affects: ["06-06", "06-07"]

tech-stack:
  added: []
  patterns:
    - "Actor MIR lowering: ActorDef -> MirFunction with receive loop"
    - "Terminate callback: separate __terminate_<name> MIR functions"
    - "Pid as i64 at LLVM level, type-safety compile-time only"
    - "Reduction check insertion after every user function/closure call"
    - "Message serialization: stack-allocated i64 array for spawn args"

key-files:
  created: []
  modified:
    - crates/snow-codegen/src/mir/lower.rs
    - crates/snow-codegen/src/mir/types.rs
    - crates/snow-codegen/src/codegen/intrinsics.rs
    - crates/snow-codegen/src/codegen/expr.rs
    - crates/snow-codegen/src/codegen/types.rs
    - crates/snow-codegen/src/codegen/mod.rs
    - crates/snow-rt/src/actor/mod.rs

decisions:
  - id: "pid-i64-llvm"
    description: "Pid maps to i64 at LLVM level (was previously ptr)"
    rationale: "Runtime uses u64 process IDs, type safety is compile-time only"
  - id: "reduction-check-user-calls-only"
    description: "Reduction checks inserted after user function calls, not runtime intrinsic calls"
    rationale: "Runtime functions are short and shouldn't count as reductions"
  - id: "actor-link-set-terminate-stubs"
    description: "Added snow_actor_link and snow_actor_set_terminate to runtime"
    rationale: "Codegen declares these as extern functions, they must exist at link time"

metrics:
  duration: "7m 0s"
  completed: "2026-02-07"
---

# Phase 06 Plan 05: Actor Codegen Pipeline Summary

Complete actor code generation from AST through MIR to LLVM IR, with reduction check instrumentation and terminate callback registration.

## What Was Done

### Task 1: AST-to-MIR Lowering (43f123f)

**MIR Lowering (lower.rs):**
- `ActorDef` lowers to a `MirFunction` containing the actor body with receive loop
- `SpawnExpr` -> `MirExpr::ActorSpawn` with function reference, state args, priority, and optional terminate callback
- `SendExpr` -> `MirExpr::ActorSend` with target pid and message
- `ReceiveExpr` -> `MirExpr::ActorReceive` with match arms and optional timeout/timeout_body
- `SelfExpr` -> `MirExpr::ActorSelf` with Pid type
- `LinkExpr` -> `MirExpr::ActorLink` with target pid
- Terminate clauses lower to separate `__terminate_<actor_name>` callback functions with signature `(Ptr, Ptr) -> Unit`
- Actor names registered in first-pass scan for call dispatch resolution
- Spawn automatically detects matching terminate callback by naming convention

**Type Resolution (types.rs):**
- `Ty::Con("Pid")` -> `MirType::Pid(None)` (untyped Pid)
- `Ty::App(Ty::Con("Pid"), [M])` -> `MirType::Pid(Some(resolve(M)))` (typed Pid)

### Task 2: LLVM Codegen (3866ee7)

**Intrinsics (intrinsics.rs):**
Declared 8 actor runtime functions as LLVM extern declarations:
- `snow_rt_init_actor(i32) -> void`
- `snow_actor_spawn(ptr, ptr, i64, i8) -> i64`
- `snow_actor_send(i64, ptr, i64) -> void`
- `snow_actor_receive(i64) -> ptr`
- `snow_actor_self() -> i64`
- `snow_actor_link(i64) -> void`
- `snow_reduction_check() -> void`
- `snow_actor_set_terminate(i64, ptr) -> void`

**Type Mapping (codegen/types.rs):**
- `MirType::Pid(_)` -> `i64` (was previously ptr, changed to match runtime's u64 PID representation)

**Expression Codegen (expr.rs):**
- `ActorSpawn`: Serialize args to stack-allocated `[N x i64]` array, call `snow_actor_spawn`, then `snow_actor_set_terminate` if callback exists
- `ActorSend`: Store message value on stack, pass pointer + size to `snow_actor_send`
- `ActorReceive`: Call `snow_actor_receive(timeout_ms)` with -1 for infinite wait
- `ActorSelf`: Call `snow_actor_self()` returning i64
- `ActorLink`: Call `snow_actor_link(target_pid)`

**Reduction Check Instrumentation:**
- `snow_reduction_check()` inserted after every user function call (direct, indirect, closure)
- Not inserted after runtime intrinsic calls (they're short and shouldn't count)
- Only emits if current basic block is not yet terminated

**Main Wrapper (mod.rs):**
- `snow_rt_init_actor(0)` called after `snow_rt_init()` to initialize actor scheduler with default thread count

**Runtime Stubs (snow-rt/actor/mod.rs):**
- `snow_actor_link(u64)`: Bidirectional process linking (adds to both processes' link lists)
- `snow_actor_set_terminate(u64, *const u8)`: Registers terminate callback on process via transmute to `TerminateCallback` type

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Added runtime stubs for snow_actor_link and snow_actor_set_terminate**
- **Found during:** Task 2
- **Issue:** Codegen declares these as LLVM extern functions, but they didn't exist in the runtime. Link would fail.
- **Fix:** Implemented `snow_actor_link` (bidirectional linking) and `snow_actor_set_terminate` (callback registration) in `crates/snow-rt/src/actor/mod.rs`
- **Files modified:** `crates/snow-rt/src/actor/mod.rs`
- **Commit:** 3866ee7

**2. [Rule 1 - Bug] Changed Pid LLVM type from ptr to i64**
- **Found during:** Task 2
- **Issue:** Plan 06-02 had set `MirType::Pid` to map to `ptr` in LLVM, but the plan requires "Pid is represented as i64 at the LLVM level" to match runtime's u64 ProcessId
- **Fix:** Changed `MirType::Pid(_) -> i64` in codegen/types.rs
- **Commit:** 3866ee7

## Task Commits

| Task | Name | Commit | Key Changes |
|------|------|--------|-------------|
| 1 | AST-to-MIR lowering for actor constructs | 43f123f | lower.rs: 6 actor expression lowerers + actor def handler; types.rs: Pid type resolution |
| 2 | LLVM codegen for actor primitives | 3866ee7 | intrinsics.rs: 8 runtime declarations; expr.rs: 5 actor codegen methods + reduction check; mod.rs: init_actor in main; runtime: link + set_terminate stubs |

## Test Results

- `cargo test -p snow-codegen`: 82 tests passed (was 73 before)
- `cargo test --workspace`: All tests green across all crates
- New tests added:
  - `test_actor_spawn_codegen` - spawn generates snow_actor_spawn call
  - `test_actor_send_codegen` - send generates snow_actor_send call
  - `test_actor_receive_codegen` - receive generates snow_actor_receive call
  - `test_actor_link_codegen` - link generates snow_actor_link call
  - `test_main_wrapper_init_actor` - main calls snow_rt_init_actor
  - `test_reduction_check_after_call` - reduction check inserted after calls
  - `test_pid_is_i64_in_ir` - Pid maps to i64 in LLVM IR
  - `test_actor_spawn_with_terminate_callback` - snow_actor_set_terminate called
  - `test_pid_type_is_i64` - Pid LLVM type is 64-bit integer
  - `resolve_untyped_pid` - Ty::Con("Pid") -> MirType::Pid(None)
  - `resolve_typed_pid` - Ty::App("Pid", [Int]) -> MirType::Pid(Some(Int))
  - `lower_self_expr` - SelfExpr lowers correctly
  - `lower_spawn_produces_actor_spawn` - SpawnExpr produces ActorSpawn
  - `lower_pid_type_resolves` - Pid type resolution in lowering

## Success Criteria Verification

1. Actor blocks lower to MIR functions with ActorReceive -- VERIFIED
2. Terminate clauses lower to callback functions referenced by ActorSpawn -- VERIFIED
3. spawn/send/receive/self/link generate correct LLVM IR -- VERIFIED (8 tests)
4. snow_actor_set_terminate called after spawn for actors with terminate clauses -- VERIFIED
5. snow_reduction_check inserted at every function call site -- VERIFIED
6. Main wraps with snow_rt_init_actor -- VERIFIED
7. Pid maps to i64 -- VERIFIED
8. Existing sequential programs still compile -- VERIFIED (all e2e tests pass)

## Next Phase Readiness

Plan 06-05 completes Wave 3. The full compiler pipeline now handles actor constructs end-to-end:
- Parser (06-02) -> Type Checker (06-04) -> MIR Lowering (06-05) -> LLVM Codegen (06-05) -> Runtime (06-01, 06-03)

Plans 06-06 and 06-07 can now build on this foundation for supervisor trees and e2e integration tests.

## Self-Check: PASSED
