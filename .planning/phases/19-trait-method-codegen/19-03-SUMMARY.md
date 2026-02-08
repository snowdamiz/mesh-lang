---
phase: 19-trait-method-codegen
plan: 03
subsystem: codegen
tags: [mir, monomorphization, defense-in-depth, where-clause, depth-limit]

# Dependency graph
requires:
  - phase: 19-02
    provides: "Call-site trait method rewriting with find_method_traits"
provides:
  - "Defense-in-depth warning for unresolvable trait method calls (CODEGEN-04)"
  - "Monomorphization depth limit preventing compiler stack overflow (CODEGEN-05)"
affects: [19-04, 20-protocol-defaults, 21-protocol-sugar]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "mono_depth increment/decrement around function body lowering"
    - "Non-fatal eprintln warning for typeck safety net in MIR lowerer"

key-files:
  created: []
  modified:
    - "crates/snow-codegen/src/mir/lower.rs"

key-decisions:
  - "Warning instead of panic for unresolvable trait methods (error recovery: let LLVM fail with clearer error)"
  - "Depth limit of 64 (configurable via max_mono_depth field)"
  - "Depth tracking in both lower_fn_def and lower_impl_method for completeness"

patterns-established:
  - "Defense-in-depth pattern: trust typeck, add non-fatal warnings in MIR lowerer for safety"
  - "Depth counter pattern: u32 increment/decrement around body lowering, MirExpr::Panic on exceed"

# Metrics
duration: 3min
completed: 2026-02-08
---

# Phase 19 Plan 03: Defense-in-Depth and Mono Depth Limit Summary

**Non-fatal warning on unresolvable trait method calls (CODEGEN-04) and configurable monomorphization depth limit of 64 (CODEGEN-05) in MIR lowerer**

## Performance

- **Duration:** 3 min
- **Started:** 2026-02-08T06:53:56Z
- **Completed:** 2026-02-08T06:56:31Z
- **Tasks:** 2
- **Files modified:** 1

## Accomplishments
- Defense-in-depth warning when a callee is not in known_functions, not a local variable, and find_method_traits returns empty -- catches where-clause violations that bypass typeck
- Monomorphization depth limit (default 64) with MirExpr::Panic on exceed instead of stack overflow
- Depth tracking in both lower_fn_def and lower_impl_method
- Two new unit tests: mono_depth_limit_prevents_overflow and mono_depth_fields_initialized

## Task Commits

Each task was committed atomically:

1. **Task 1: Add where-clause defense-in-depth assertion** - `974c253` (feat)
2. **Task 2: Add monomorphization depth limit** - `d5adb60` (feat)

## Files Created/Modified
- `crates/snow-codegen/src/mir/lower.rs` - Added mono_depth/max_mono_depth fields, depth tracking in lower_fn_def and lower_impl_method, defense-in-depth warning in trait method resolution, two new tests

## Decisions Made
- Warning (eprintln) instead of panic for unresolvable trait methods: non-fatal error recovery lets LLVM codegen produce a clearer "undefined function" error
- Default depth limit of 64: high enough for any realistic program, low enough to catch infinite recursion early
- Depth tracking in both lower_fn_def and lower_impl_method for completeness, since regular functions can also trigger deep recursion through trait method calls

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Fixed MirExpr field names in test**
- **Found during:** Task 2 (unit test writing)
- **Issue:** Test used `left`/`right` and `then_`/`else_` field names but actual MirExpr::BinOp uses `lhs`/`rhs` and MirExpr::If uses `then_body`/`else_body`
- **Fix:** Corrected field names in the has_panic() helper
- **Files modified:** crates/snow-codegen/src/mir/lower.rs (test module)
- **Verification:** cargo test passes
- **Committed in:** d5adb60 (Task 2 commit)

---

**Total deviations:** 1 auto-fixed (1 bug)
**Impact on plan:** Minor test code fix. No scope creep.

## Issues Encountered
None

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- CODEGEN-04 (where-clause enforcement) and CODEGEN-05 (mono depth limit) are complete
- Ready for Plan 19-04 (integration tests / end-to-end trait method codegen verification)
- All 102 tests pass (100 existing + 2 new)

---
*Phase: 19-trait-method-codegen*
*Completed: 2026-02-08*

## Self-Check: PASSED
