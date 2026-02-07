---
phase: 09-concurrency-standard-library
plan: 03
subsystem: codegen
tags: [service, actor, genserver, llvm, mir, codegen, runtime, call-reply, cast]

# Dependency graph
requires:
  - phase: 09-01
    provides: "ServiceDef AST with call/cast handlers, service/call/cast keywords"
  - phase: 09-02
    provides: "Type inference for services: state unification, module function types"
  - phase: 06-03
    provides: "MessageBuffer with type_tag, heap message layout"
  - phase: 06-05
    provides: "Pid -> i64 mapping, actor spawn/send/receive primitives"
provides:
  - "snow_service_call and snow_service_reply runtime functions"
  - "lower_service_def() MIR lowering that desugars service to actor primitives"
  - "LLVM codegen for service loop dispatch, call helpers, cast helpers"
  - "Service module field access resolution (Counter.start -> __service_counter_start)"
  - "Job module in STDLIB_MODULES for Plan 04 prep"
affects:
  - 09-04 (Job async/await uses service infrastructure)
  - 09-05 (integration testing of full concurrency stack)

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Service desugars to actor primitives at MIR level"
    - "Service loop uses switch dispatch on integer type_tag"
    - "Call helpers pack args into payload buffer, call snow_service_call"
    - "Cast helpers pack [tag|0|args] message, call snow_actor_send"
    - "service_dispatch table passes dispatch metadata from MIR to codegen"

key-files:
  created:
    - "crates/snow-rt/src/actor/service.rs"
  modified:
    - "crates/snow-rt/src/actor/mod.rs"
    - "crates/snow-rt/src/lib.rs"
    - "crates/snow-codegen/src/mir/lower.rs"
    - "crates/snow-codegen/src/mir/mod.rs"
    - "crates/snow-codegen/src/mir/mono.rs"
    - "crates/snow-codegen/src/codegen/intrinsics.rs"
    - "crates/snow-codegen/src/codegen/expr.rs"
    - "crates/snow-codegen/src/codegen/mod.rs"

key-decisions:
  - "Service loop function body is MirExpr::Unit; codegen generates dispatch loop specially by detecting __service_*_loop naming pattern"
  - "Service dispatch uses switch on integer type_tag (sequential tags: calls 0,1,2..., casts N,N+1,...)"
  - "Call handler returns tuple (new_state, reply) at Snow level; codegen extracts via snow_tuple_first/second"
  - "Cast message format includes [tag][0 caller][args] for uniform dispatch in service loop"
  - "Service modules tracked dynamically in lowerer for field access resolution"
  - "Job module added to STDLIB_MODULES with map_builtin_name entries for Plan 04"

patterns-established:
  - "Service-to-actor desugaring: each service generates init, loop, start, call/cast helpers, handler functions"
  - "Codegen interception for service call/cast: detected by function name pattern in codegen_call"
  - "service_dispatch HashMap on MirModule passes structured dispatch metadata to codegen"

# Metrics
duration: 19min
completed: 2026-02-07
---

# Phase 9 Plan 3: Service Runtime and Codegen Summary

**Service definitions compile through full pipeline: runtime call/reply functions, MIR lowering to actor primitives, LLVM codegen with switch-dispatch service loops**

## Performance

- **Duration:** 19 min
- **Started:** 2026-02-07T08:13:00Z
- **Completed:** 2026-02-07T08:31:43Z
- **Tasks:** 2/2
- **Files modified:** 9

## Accomplishments

- Created snow_service_call (synchronous send+block-for-reply) and snow_service_reply (send reply back to caller) runtime functions
- Implemented lower_service_def() that generates 7+ MIR functions per service: init, loop, start, call/cast helpers, handler body functions
- Built LLVM codegen for service loop with receive + switch dispatch on type_tag, handler invocation, tuple extraction for call replies
- Wired service module field access so Counter.start/Counter.get_count resolve to generated __service_* functions
- Added Job module stubs to STDLIB_MODULES for Plan 04 preparation

## Task Commits

Each task was committed atomically:

1. **Task 1: Add Service runtime functions** - `f35674c` (feat)
2. **Task 2: Add MIR lowering and LLVM codegen for Service** - `95eb92f` (feat)

## Files Created/Modified

- `crates/snow-rt/src/actor/service.rs` - Runtime snow_service_call and snow_service_reply extern "C" functions
- `crates/snow-rt/src/actor/mod.rs` - Added pub mod service
- `crates/snow-rt/src/lib.rs` - Re-exported service functions
- `crates/snow-codegen/src/mir/lower.rs` - lower_service_def(), to_snake_case, service_modules tracking, Job STDLIB_MODULES
- `crates/snow-codegen/src/mir/mod.rs` - service_dispatch field on MirModule
- `crates/snow-codegen/src/mir/mono.rs` - Updated MirModule construction for new field
- `crates/snow-codegen/src/codegen/intrinsics.rs` - snow_service_call and snow_service_reply LLVM declarations
- `crates/snow-codegen/src/codegen/expr.rs` - codegen_service_loop, codegen_service_call_helper, codegen_service_cast_helper
- `crates/snow-codegen/src/codegen/mod.rs` - service_dispatch field on CodeGen, compile_function interception for service loops

## Decisions Made

1. **Service loop as codegen-level construct:** The MIR service loop function has a Unit body placeholder; the actual receive+dispatch+handler logic is generated entirely at LLVM codegen level. This avoids the need for raw pointer operations in MIR (which has no pointer arithmetic primitives).

2. **Dispatch via switch on integer type_tag:** Service handlers are assigned sequential integer tags (call handlers: 0,1,2...; cast handlers: N,N+1,...). The loop uses an LLVM switch instruction for efficient dispatch.

3. **Call reply extraction via tuple functions:** Call handlers return (new_state, reply) as a Snow tuple. Codegen extracts values using snow_tuple_first/snow_tuple_second runtime functions, then sends reply via snow_service_reply.

4. **Cast message includes dummy caller_pid:** Cast messages use format [tag][0][args] with caller_pid=0 for uniform message layout, even though no reply is sent.

5. **Service call/cast interception in codegen_call:** Rather than adding new MIR expression variants, the codegen detects calls to snow_service_call/snow_actor_send from service helper functions and generates the appropriate payload buffer construction inline.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Fixed service syntax in MIR lowering tests**
- **Found during:** Task 2 (writing tests)
- **Issue:** Test service source used `call GetCount() :: Int |count| do` but parser expects `call GetCount() :: Int do |count|` (state param after `do`)
- **Fix:** Updated all test sources to use correct parser syntax
- **Files modified:** crates/snow-codegen/src/mir/lower.rs (test section)
- **Committed in:** 95eb92f

---

**Total deviations:** 1 auto-fixed (1 bug)
**Impact on plan:** Syntax mismatch was test-only. No impact on generated code.

## Issues Encountered

None - plan executed smoothly after the syntax fix.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness

- Service definitions compile through MIR lowering and have LLVM codegen wiring
- Runtime call/reply functions are linked and exported
- Job module stubs are registered in STDLIB_MODULES for Plan 04
- Full E2E testing (compile Counter service to native binary and run) requires runtime concurrency setup; service loop needs actor scheduler initialization in the compiled program

---
*Phase: 09-concurrency-standard-library*
*Plan: 03*
*Completed: 2026-02-07*

## Self-Check: PASSED
