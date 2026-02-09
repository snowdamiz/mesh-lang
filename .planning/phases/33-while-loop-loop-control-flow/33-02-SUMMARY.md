---
phase: 33-while-loop-loop-control-flow
plan: 02
subsystem: codegen
tags: [while-loop, break, continue, llvm, mir, control-flow, reduction-check, formatter]

# Dependency graph
requires:
  - phase: 33-while-loop-loop-control-flow
    plan: 01
    provides: Parser (WHILE_EXPR, BREAK_EXPR, CONTINUE_EXPR CST nodes), type checker (loop-depth tracking, BRKC-04/05 errors)
provides:
  - MIR While/Break/Continue expression variants with lowering from AST
  - LLVM IR codegen with three-block loop structure (cond_check/body/merge)
  - Loop context stack (loop_stack) for nested break/continue target tracking
  - Reduction check emission at while back-edges and continue back-edges
  - Formatter support for while/break/continue syntax
  - E2E tests covering all acceptance criteria (WHILE-01/02/03, BRKC-01/04/05, RTIM-01)
affects: [future-loop-constructs, for-loop, loop-expression]

# Tech tracking
tech-stack:
  added: []
  patterns: [three-block-loop-codegen, loop-stack-context, back-edge-reduction-check]

key-files:
  created:
    - tests/e2e/while_loop.snow
    - tests/e2e/break_continue.snow
  modified:
    - crates/snow-codegen/src/mir/mod.rs
    - crates/snow-codegen/src/mir/lower.rs
    - crates/snow-codegen/src/codegen/expr.rs
    - crates/snow-codegen/src/codegen/mod.rs
    - crates/snow-codegen/src/mir/mono.rs
    - crates/snow-codegen/src/pattern/compile.rs
    - crates/snow-fmt/src/walker.rs
    - crates/snowc/tests/e2e.rs

key-decisions:
  - "While loops use alloca-free Unit return (no result merge needed since while always returns Unit)"
  - "Break/continue use loop_stack Vec<(cond_bb, merge_bb)> on CodeGen for O(1) target lookup"
  - "Reduction check emitted at both while back-edge AND continue back-edge to prevent scheduler starvation"
  - "E2E tests adapted to use break-based patterns since Snow has no mutable assignment"

patterns-established:
  - "Three-block loop codegen: cond_check/body/merge with back-edge reduction check"
  - "Loop context stack pattern for nested loop break/continue resolution"
  - "Terminator guard pattern: check bb.get_terminator().is_none() before emit in blocks and if-branches"

# Metrics
duration: 12min
completed: 2026-02-09
---

# Phase 33 Plan 02: While Loop Codegen Summary

**MIR While/Break/Continue with three-block LLVM codegen, loop_stack context, back-edge reduction checks, and formatter support**

## Performance

- **Duration:** 12 min
- **Started:** 2026-02-09T07:16:58Z
- **Completed:** 2026-02-09T07:29:12Z
- **Tasks:** 3 (implementation + tests + verification)
- **Files modified:** 8 (+ 2 created)

## Accomplishments
- MIR representation for While/Break/Continue with complete lowering from AST
- LLVM IR codegen using three-block structure (cond_check/body/merge) with snow_reduction_check() at back-edges
- Loop context stack (loop_stack) on CodeGen for nested break/continue target tracking
- Formatter walk functions for while/break/continue with proper indentation
- 6 new e2e tests covering all acceptance criteria: WHILE-01/02/03, BRKC-01/04/05
- 4 new codegen unit tests verifying basic block structure and reduction check emission (RTIM-01)
- 5 new formatter tests including idempotency check
- 3 new MIR lowering unit tests for while/break/continue
- All 1273 workspace tests pass with zero regressions

## Task Commits

Each task was committed atomically:

1. **Task 1: MIR + codegen + formatter** - `03d4e6e` (feat)
2. **Task 2: E2E and unit tests** - `ed98eb7` (test)

**Plan metadata:** (pending)

## Files Created/Modified
- `crates/snow-codegen/src/mir/mod.rs` - Added MirExpr::While, Break, Continue variants + ty() implementations
- `crates/snow-codegen/src/mir/lower.rs` - lower_while_expr, break/continue lowering, collect_free_vars arms, unit tests
- `crates/snow-codegen/src/codegen/expr.rs` - codegen_while (three-block), codegen_break, codegen_continue, codegen_block terminator guard, codegen_if store guard
- `crates/snow-codegen/src/codegen/mod.rs` - loop_stack field, initialization, codegen unit tests
- `crates/snow-codegen/src/mir/mono.rs` - While/Break/Continue arms in collect_function_refs
- `crates/snow-codegen/src/pattern/compile.rs` - While/Break/Continue arms in compile_expr_patterns
- `crates/snow-fmt/src/walker.rs` - walk_while_expr, walk_break_expr, walk_continue_expr, formatter tests
- `crates/snowc/tests/e2e.rs` - 6 new e2e tests for while/break/continue
- `tests/e2e/while_loop.snow` - WHILE-01/02/03 fixture
- `tests/e2e/break_continue.snow` - BRKC-01 fixture

## Decisions Made
- While loops use alloca-free Unit return since while always returns Unit (no phi/alloca needed for result merging)
- Break/continue use `loop_stack: Vec<(cond_bb, merge_bb)>` on CodeGen for O(1) target lookup with natural nesting support
- Reduction check emitted at both while back-edge AND continue back-edge to prevent actor scheduler starvation in tight loops
- E2E tests adapted to use break-based patterns since Snow has no mutable variable assignment -- while loops are primarily useful for infinite loops with break, actor receive loops, and server loops

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Added exhaustive match arms in mono.rs, pattern/compile.rs, lower.rs**
- **Found during:** Task 1 (initial compilation)
- **Issue:** Adding While/Break/Continue to MirExpr enum broke exhaustive matches in collect_function_refs (mono.rs), compile_expr_patterns (pattern/compile.rs), and collect_free_vars (lower.rs)
- **Fix:** Added proper recursion arms for While (recurse into cond+body) and leaf arms for Break/Continue
- **Files modified:** mir/mono.rs, pattern/compile.rs, mir/lower.rs
- **Verification:** cargo check passes
- **Committed in:** 03d4e6e (Task 1 commit)

**2. [Rule 1 - Bug] Fixed codegen_block to skip expressions after terminated blocks**
- **Found during:** Task 1 (proactive analysis)
- **Issue:** codegen_block iterated all expressions without checking if the current LLVM block was already terminated. After break/continue/return, subsequent expressions would emit into a terminated block, causing LLVM verification failure.
- **Fix:** Added `bb.get_terminator().is_some()` check before each expression in codegen_block, breaking on terminated block
- **Files modified:** codegen/expr.rs
- **Verification:** break_continue.snow compiles and runs correctly
- **Committed in:** 03d4e6e (Task 1 commit)

**3. [Rule 1 - Bug] Fixed codegen_if to guard store+branch after terminated then/else blocks**
- **Found during:** Task 2 (break_continue.snow compilation)
- **Issue:** codegen_if always called build_store after codegen_expr(then_body) even if the body terminated the block via break. LLVM rejected "Terminator found in the middle of a basic block".
- **Fix:** Moved build_store inside the existing get_terminator().is_none() guard so both store and branch are skipped when the block is already terminated
- **Files modified:** codegen/expr.rs
- **Verification:** `while true do if true do break end end` compiles and runs correctly
- **Committed in:** ed98eb7 (Task 2 commit)

---

**Total deviations:** 3 auto-fixed (1 blocking, 2 bugs)
**Impact on plan:** All auto-fixes necessary for correctness. The codegen_if bug (deviation 3) was a pre-existing issue that only manifests with break/continue inside if-expressions within loops. No scope creep.

## Issues Encountered
- Snow has no mutable variable assignment, so the plan's suggested e2e tests (`while x > 0 do x = x - 1 end`) could not be implemented as written. Adapted tests to use break-based patterns and boolean conditions, which still cover all acceptance criteria.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- While loop + break/continue fully operational across the compiler pipeline
- All 1273 workspace tests pass with zero regressions
- Pattern established for future loop constructs (for-in, loop)
- No blockers for subsequent phases

---
*Phase: 33-while-loop-loop-control-flow*
*Plan: 02*
*Completed: 2026-02-09*
