---
phase: 09-concurrency-standard-library
plan: 02
subsystem: type-checker
tags: [service, type-inference, module-resolution, job, polymorphic-schemes, state-unification]

# Dependency graph
requires:
  - phase: 09-01
    provides: "ServiceDef/CallHandler/CastHandler AST nodes, SERVICE_DEF/CALL_HANDLER/CAST_HANDLER SyntaxKinds"
  - phase: 03-type-system
    provides: "Type inference engine, unification, Scheme, TypeEnv"
  - phase: 06-actor-runtime
    provides: "Pid<M> type, actor type inference patterns"
  - phase: 08-standard-library
    provides: "Module-qualified access pattern, stdlib_modules(), STDLIB_MODULE_NAMES"
provides:
  - "infer_service_def() for type checking service definitions"
  - "Per-variant typed helper functions registered as ServiceName.method_name"
  - "State type unification across init/call/cast handlers"
  - "User-defined service module resolution in infer_field_access"
  - "Job module with polymorphic async/await/await_timeout/map"
  - "to_snake_case helper for PascalCase -> snake_case conversion"
affects:
  - 09-03 (MIR lowering for service definitions)
  - 09-04 (runtime implementation for service/job)
  - 09-05 (E2E testing of service type checking)

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Synthetic TyVars (u32::MAX - N) for polymorphic stdlib module schemes"
    - "Service module functions registered as qualified env entries (ServiceName.method_name)"
    - "User-defined module resolution via env.lookup in infer_field_access"

key-files:
  created: []
  modified:
    - "crates/snow-typeck/src/infer.rs"

key-decisions:
  - "Service helper functions registered directly in TypeEnv as 'ServiceName.method_name' entries -- avoids threading a user_modules map through all inference functions"
  - "Service Pid type is Pid<Unit> for callers -- internal message dispatching uses type_tags at runtime, not type-level message types"
  - "Job module uses synthetic TyVars (u32::MAX - 10..12) for proper polymorphic Schemes -- ctx.instantiate() replaces them with fresh vars"
  - "User-defined service module resolution checks env before sum type variant lookup in infer_field_access"

patterns-established:
  - "Service type checking: create state_ty fresh var, unify init return + all handler state params against it"
  - "Handler body type enforcement: call handlers return (state, reply) tuple, cast handlers return state"
  - "Synthetic TyVars for polymorphic stdlib modules (avoids needing InferCtx in stdlib_modules())"

# Metrics
duration: 6min
completed: 2026-02-07
---

# Phase 9 Plan 2: Service Type Checking Summary

**Service type inference with state unification, per-variant typed helpers (ServiceName.method_name), and polymorphic Job module (async/await/map)**

## Performance

- **Duration:** 6 min
- **Started:** 2026-02-07T08:02:33Z
- **Completed:** 2026-02-07T08:08:34Z
- **Tasks:** 2
- **Files modified:** 1

## Accomplishments
- infer_service_def() processes ServiceDef AST nodes: infers init, call handlers (with reply type), and cast handlers with state type unification
- Module-qualified access (Counter.get_count, Counter.increment) resolves through TypeEnv with per-variant typed returns
- Job module registered with 4 polymorphic functions using synthetic TyVar-based Schemes
- All 204 existing tests pass with zero regressions

## Task Commits

Each task was committed atomically:

1. **Task 1: Implement infer_service_def with state type unification and module registration** - `ed4f523` (feat)
2. **Task 2: Add Job module type registration** - `b2294a4` (feat)

**Plan metadata:** TBD (docs: complete plan)

## Files Created/Modified
- `crates/snow-typeck/src/infer.rs` - Added infer_service_def(), register service module helpers, extended infer_field_access for user-defined services, to_snake_case helper, Job module in stdlib_modules(), "Job" in STDLIB_MODULE_NAMES, TyVar import

## Decisions Made
- **Service functions in TypeEnv:** Registered as "ServiceName.method_name" entries in the existing TypeEnv rather than threading a separate user_modules map through 20+ functions. Cleaner architecture using the same qualified-name pattern as sum type constructors.
- **Pid<Unit> for service callers:** Service helper functions expose Pid<Unit> to callers (not Pid<ServiceMsg>). Internal message dispatching uses type_tags at runtime. This matches the design decision from Plan 01.
- **Synthetic TyVars for polymorphism:** Job module functions use TyVar(u32::MAX - N) as quantified variables in Scheme. These never enter the unification table directly -- ctx.instantiate() replaces them with real fresh vars. This enables proper polymorphic type checking for Job.async/await without modifying stdlib_modules() to require InferCtx.
- **User module resolution order:** In infer_field_access, user-defined service modules are checked after stdlib modules but before sum type variants. This ensures ServiceName.method resolves correctly without conflicting with variant constructors.

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered

- **Pre-existing compilation error:** The Item::ServiceDef variant was added to the AST in Plan 01 but had no match arm in infer_item, causing a non-exhaustive pattern error. This was the expected starting state for this plan -- wiring in the ServiceDef case was the first step.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness
- Service type checking infrastructure complete -- ServiceDef AST nodes are fully type-checked
- Job module types registered -- ready for MIR lowering and runtime implementation
- infer_field_access extended for user-defined service modules -- future plans can add more service-like constructs
- Ready for Plan 03 (MIR lowering) and Plan 04 (runtime)

## Self-Check: PASSED

---
*Phase: 09-concurrency-standard-library*
*Completed: 2026-02-07*
