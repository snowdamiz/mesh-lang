---
phase: 48-tail-call-elimination
plan: 01
subsystem: compiler
tags: [mir, tail-call-elimination, tce, codegen, self-recursion]

# Dependency graph
requires: []
provides:
  - "MirExpr::TailCall variant for representing self-recursive tail calls in MIR"
  - "MirFunction.has_tail_calls flag for codegen loop wrapping decision"
  - "rewrite_tail_calls post-lowering pass detecting tail position through 7 expression contexts"
affects: [48-02-codegen-loop-wrapping]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Post-lowering MIR rewrite pass (structural tree walk, no CFG needed)"
    - "TailCall as MirType::Never (same as Break/Continue/Return -- control flow, not value)"

key-files:
  modified:
    - "crates/snow-codegen/src/mir/mod.rs"
    - "crates/snow-codegen/src/mir/lower.rs"
    - "crates/snow-codegen/src/mir/mono.rs"
    - "crates/snow-codegen/src/codegen/expr.rs"
    - "crates/snow-codegen/src/codegen/mod.rs"
    - "crates/snow-codegen/src/pattern/compile.rs"

key-decisions:
  - "TailCall ty() returns MirType::Never (branches, never produces a value)"
  - "Rewrite pass integrated into 6 lowering paths: fn_def, impl_method, default_method, multi_clause_fn (x2), actor_def"
  - "Non-user functions (closures, callbacks, trait impls) get has_tail_calls: false by default"

patterns-established:
  - "Post-lowering rewrite pass pattern: standalone fn operating on &mut MirExpr tree"
  - "Tail position propagation: Block(last), Let(body), If(both), Match(all arms), ActorReceive(all arms+timeout), Return(inner)"

# Metrics
duration: 8min
completed: 2026-02-10
---

# Phase 48 Plan 01: MIR TCE Infrastructure Summary

**MirExpr::TailCall variant with post-lowering rewrite pass detecting self-recursive calls in 7 tail position contexts across all function lowering paths**

## Performance

- **Duration:** 8 min
- **Started:** 2026-02-10T17:18:02Z
- **Completed:** 2026-02-10T17:26:31Z
- **Tasks:** 2
- **Files modified:** 6

## Accomplishments
- Added MirExpr::TailCall variant with args and ty fields, returning MirType::Never
- Added has_tail_calls: bool to MirFunction (40 constructor sites updated)
- Implemented rewrite_tail_calls handling Block, Let, If, Match, ActorReceive, Return tail contexts
- Integrated TCE rewrite into all 6 function lowering paths (fn_def, impl_method, default_method, multi_clause_fn single/multi, actor_def)
- All 175 unit tests and 118 e2e tests pass unchanged

## Task Commits

Each task was committed atomically:

1. **Task 1: Add MirExpr::TailCall variant and MirFunction.has_tail_calls field** - `8c7cec5` (feat)
2. **Task 2: Implement rewrite_tail_calls pass and integrate into all function lowering paths** - `be6854b` (feat)

## Files Created/Modified
- `crates/snow-codegen/src/mir/mod.rs` - TailCall variant, has_tail_calls field, ty() arm
- `crates/snow-codegen/src/mir/lower.rs` - rewrite_tail_calls function, integration into 6 lowering paths, has_tail_calls on all 40 constructors
- `crates/snow-codegen/src/mir/mono.rs` - TailCall arm in collect_function_refs, has_tail_calls in test constructors
- `crates/snow-codegen/src/codegen/expr.rs` - Placeholder TailCall arm (codegen in Plan 02)
- `crates/snow-codegen/src/codegen/mod.rs` - has_tail_calls in test MirFunction constructors
- `crates/snow-codegen/src/pattern/compile.rs` - TailCall arm in compile_expr_patterns

## Decisions Made
- TailCall returns MirType::Never (consistent with Break/Continue/Return -- control flow nodes that never produce a value)
- Post-lowering rewrite pass chosen over threading `is_tail_position` through every lower_* method (cleaner, less churn)
- Integrated TCE into multi-clause functions and default methods (not just the 3 paths specified in plan) since these are user-defined functions that can be self-recursive
- TailCall codegen in expr.rs returns error placeholder -- actual implementation deferred to Plan 02

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Added TailCall arms to exhaustive pattern matches**
- **Found during:** Task 1 (adding TailCall variant)
- **Issue:** Rust enforces exhaustive matching; new MirExpr variant caused compile errors in expr.rs, mono.rs, lower.rs, pattern/compile.rs
- **Fix:** Added TailCall handling arms: placeholder error in expr.rs (Plan 02), function ref collection in mono.rs, free var collection in lower.rs, pattern compilation in compile.rs
- **Files modified:** codegen/expr.rs, mir/mono.rs, mir/lower.rs, pattern/compile.rs
- **Verification:** `cargo build -p snow-codegen` succeeds
- **Committed in:** 8c7cec5 (Task 1 commit)

**2. [Rule 2 - Missing Critical] Added TCE to multi-clause functions and default methods**
- **Found during:** Task 2 (integrating rewrite pass)
- **Issue:** Plan specified 3 integration sites (fn_def, impl_method, actor_def), but multi-clause functions and default methods also produce user-callable MirFunctions that can be self-recursive
- **Fix:** Added rewrite_tail_calls calls in lower_multi_clause_fn (both branches) and lower_default_method
- **Files modified:** mir/lower.rs
- **Verification:** All tests pass, grep shows 15 rewrite_tail_calls occurrences (6 call sites + function def + recursive calls)
- **Committed in:** be6854b (Task 2 commit)

---

**Total deviations:** 2 auto-fixed (1 blocking, 1 missing critical)
**Impact on plan:** Both deviations necessary for correctness. No scope creep -- exhaustive matches are required by Rust, and multi-clause TCE ensures complete coverage.

## Issues Encountered
None.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- MIR infrastructure complete: TailCall nodes are created during lowering for any self-recursive function
- Plan 02 can now implement codegen loop wrapping: check has_tail_calls flag, create loop header, handle TailCall as param reassignment + branch
- The placeholder TailCall arm in expr.rs is ready to be replaced with actual codegen

## Self-Check: PASSED

All files exist, both commits verified in git log.

---
*Phase: 48-tail-call-elimination*
*Completed: 2026-02-10*
