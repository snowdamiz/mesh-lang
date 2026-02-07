---
phase: 06-actor-runtime
plan: 04
subsystem: typeck
tags: [pid, actor, type-inference, message-passing, generics, unification, diagnostics]

# Dependency graph
requires:
  - phase: 06-02
    provides: "Actor AST nodes (ActorDef, SpawnExpr, SendExpr, ReceiveExpr, SelfExpr, LinkExpr)"
  - phase: 02-typeck
    provides: "Type inference engine (Algorithm J), unification, type environment"
provides:
  - "Pid<M> typed actor identity in the type system"
  - "Compile-time message type validation for send()"
  - "Inference rules for spawn, send, receive, self(), link"
  - "Actor-specific error types E0014-E0017 with diagnostics"
  - "Untyped Pid escape hatch via unification special case"
affects: ["06-05", "06-06", "06-07", "07-stdlib"]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Pid<M> represented as Ty::App(Ty::Con('Pid'), [M])"
    - "ACTOR_MSG_TYPE_KEY env binding for actor context tracking"
    - "Unification escape hatch for untyped/typed Pid compatibility"
    - "Let binding with annotation uses annotation type for scheme"

key-files:
  created:
    - "crates/snow-typeck/tests/actors.rs"
    - "crates/snow-typeck/tests/snapshots/diagnostics__diag_send_type_mismatch.snap"
    - "crates/snow-typeck/tests/snapshots/diagnostics__diag_self_outside_actor.snap"
    - "crates/snow-typeck/tests/snapshots/diagnostics__diag_spawn_non_function.snap"
    - "crates/snow-typeck/tests/snapshots/diagnostics__diag_receive_outside_actor.snap"
  modified:
    - "crates/snow-typeck/src/ty.rs"
    - "crates/snow-typeck/src/builtins.rs"
    - "crates/snow-typeck/src/infer.rs"
    - "crates/snow-typeck/src/unify.rs"
    - "crates/snow-typeck/src/error.rs"
    - "crates/snow-typeck/src/diagnostics.rs"
    - "crates/snow-typeck/tests/diagnostics.rs"

key-decisions:
  - "Pid<M> uses existing Ty::App/Ty::Con rather than a new Ty variant"
  - "Actor message type tracked via ACTOR_MSG_TYPE_KEY in TypeEnv, not a separate context struct"
  - "Let binding with annotation uses annotation type for the scheme (enables Pid escape hatch)"
  - "Untyped Pid unifies with typed Pid<M> in both directions without constraining M"

patterns-established:
  - "Actor context tracking: bind well-known key in env to track actor message type"
  - "Escape hatch unification: special case in unify for subtype-like coercion"

# Metrics
duration: 9min
completed: 2026-02-07
---

# Phase 6 Plan 4: Pid<M> Typed Actor Identity Summary

**Pid<M> type with compile-time message type validation via send(), spawn returning Pid<M>, receive inferring message type, self() inside actors, and untyped Pid escape hatch**

## Performance

- **Duration:** 9 min
- **Started:** 2026-02-07T01:48:01Z
- **Completed:** 2026-02-07T01:57:07Z
- **Tasks:** 2
- **Files modified:** 12

## Accomplishments
- Pid<M> is a first-class type: `Ty::pid(msg_type)` creates typed Pid, `Ty::untyped_pid()` creates untyped
- Compile-time send validation: `send(typed_pid, wrong_type)` produces E0014 SendTypeMismatch error
- Full actor inference: ActorDef registers as function, spawn returns Pid<M>, receive constrains message type, self() returns actor's Pid<M>
- Escape hatch: untyped Pid accepts any message, typed Pid assignable to untyped Pid
- 4 new error types (E0014-E0017) with ariadne diagnostics, source spans, and help text
- 22 new tests (17 actor + 5 diagnostic) all passing

## Task Commits

Each task was committed atomically:

1. **Task 1: Register Pid type and add inference rules for actor expressions** - `86abfe5` (feat)
2. **Task 2: Actor type errors and diagnostic messages** - `188df4b` (test)

## Files Created/Modified
- `crates/snow-typeck/src/ty.rs` - Added Ty::pid() and Ty::untyped_pid() constructors
- `crates/snow-typeck/src/builtins.rs` - Registered Pid as builtin type constructor
- `crates/snow-typeck/src/infer.rs` - Actor inference: ActorDef, SpawnExpr, SendExpr, ReceiveExpr, SelfExpr, LinkExpr
- `crates/snow-typeck/src/unify.rs` - Pid escape hatch: untyped Pid unifies with Pid<M>
- `crates/snow-typeck/src/error.rs` - E0014 SendTypeMismatch, E0015 SelfOutsideActor, E0016 SpawnNonFunction, E0017 ReceiveOutsideActor
- `crates/snow-typeck/src/diagnostics.rs` - Ariadne rendering for all 4 new error types
- `crates/snow-typeck/tests/actors.rs` - 17 actor inference tests
- `crates/snow-typeck/tests/diagnostics.rs` - 5 new diagnostic tests
- `crates/snow-typeck/tests/snapshots/diagnostics__diag_*.snap` - 4 diagnostic snapshots

## Decisions Made
- **Pid<M> as Ty::App, not new variant**: Keeps the type system uniform. Pid is just another generic type constructor like Option or Result, using `Ty::App(Ty::Con("Pid"), [M])`.
- **ACTOR_MSG_TYPE_KEY environment binding**: Rather than adding a context field to every inference function signature, actor message type is tracked via a well-known key (`__actor_msg_type__`) in the TypeEnv. Simple, non-invasive, works with existing scoping.
- **Let binding uses annotation type for scheme**: When a let binding has a type annotation and unification succeeds, the annotation type is used for the binding's type scheme (not the inferred type). This correctly enables `let pid :: Pid = spawn(counter, 0)` to store `Pid` rather than `Pid<Int>`.
- **Bidirectional Pid escape hatch**: Both directions work: `Pid<M>` to `Pid` and `Pid` to `Pid<M>`, implemented as a special case in unification that returns Ok without constraining either side.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Added diagnostics in Task 1 to allow compilation**
- **Found during:** Task 1 (implementing error types)
- **Issue:** Adding error variants to TypeError required updating the exhaustive match in diagnostics.rs, which was planned for Task 2
- **Fix:** Added error codes E0014-E0017 and full ariadne rendering in Task 1
- **Files modified:** crates/snow-typeck/src/diagnostics.rs
- **Verification:** cargo check -p snow-typeck compiles
- **Committed in:** 86abfe5 (Task 1 commit)

**2. [Rule 1 - Bug] Fixed let binding annotation semantics for Pid escape hatch**
- **Found during:** Task 1 (test_send_untyped_pid_any_type failing)
- **Issue:** Let binding with annotation `:: Pid` was storing the inferred type `Pid<Int>` instead of the annotation type, making the escape hatch ineffective
- **Fix:** Changed infer_let_binding to use annotation type for the binding scheme when annotation is present
- **Files modified:** crates/snow-typeck/src/infer.rs
- **Verification:** test_send_untyped_pid_any_type passes; all existing tests still pass
- **Committed in:** 86abfe5 (Task 1 commit)

---

**Total deviations:** 2 auto-fixed (1 blocking, 1 bug)
**Impact on plan:** Both fixes were necessary for correct operation. The diagnostics move was purely organizational (Task 2 focused on snapshot tests instead). The let binding fix ensures the Pid escape hatch works correctly per the language design.

## Issues Encountered
- Pattern annotations (`n :: Int`) in receive arms are not valid Snow syntax -- patterns don't have type annotations. Tests were rewritten to use bare patterns with type inference from usage context (e.g., `n -> counter(state + n)` infers n as Int from the addition).

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Pid<M> type system is complete, ready for runtime implementation (06-05, 06-06)
- All 192 typeck tests pass (up from 170)
- Full workspace test suite passes with 0 failures
- Actor spawning, message sending, and receive blocks have correct type inference
- Error diagnostics follow established patterns (ariadne, error codes, help text)

---
*Phase: 06-actor-runtime*
*Completed: 2026-02-07*

## Self-Check: PASSED
