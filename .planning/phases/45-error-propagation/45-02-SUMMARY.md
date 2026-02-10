---
phase: 45-error-propagation
plan: "02"
subsystem: codegen
tags: [mir-lowering, try-operator, result, option, pattern-matching, early-return, llvm]

# Dependency graph
requires:
  - phase: 45-error-propagation
    provides: "45-01 parser TRY_EXPR node + typeck TryIncompatibleReturn validation"
provides:
  - "MIR lowering desugars expr? to Match + Return for Result<T,E> and Option<T>"
  - "fn_return_type_stack properly tracks return type including generic args"
  - "codegen handles early-return in match arms without LLVM terminator errors"
  - "5 e2e tests covering Ok/Err/Some/None paths and chained ? usage"
affects: [error-propagation, result-types, option-types, closures]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Match + Return + ConstructVariant desugaring for try operator"
    - "current_fn_return_type save/restore pattern for nested functions and closures"
    - "Terminator-aware match arm codegen (skip store after ret)"

key-files:
  created:
    - tests/e2e/try_result_ok_path.snow
    - tests/e2e/try_result_err_path.snow
    - tests/e2e/try_option_some_path.snow
    - tests/e2e/try_option_none_path.snow
    - tests/e2e/try_chained_result.snow
  modified:
    - crates/snow-codegen/src/mir/lower.rs
    - crates/snow-typeck/src/infer.rs
    - crates/snow-codegen/src/codegen/pattern.rs
    - crates/snowc/tests/e2e.rs

key-decisions:
  - "Desugar entirely to existing MIR primitives (Match + Return + ConstructVariant) with zero new MIR nodes or codegen changes"
  - "Use generic sum type base names (Result, Option) for ConstructVariant type_name, not monomorphized names"
  - "Fix fn return type annotation parsing to use resolve_type_annotation for proper generic/sugar type support"

patterns-established:
  - "current_fn_return_type tracking: save/restore around fn_def, impl_method, and closure bodies"
  - "Unique __try_val_N / __try_err_N bindings via try_counter to avoid shadowing in nested ? expressions"

# Metrics
duration: 25min
completed: 2026-02-09
---

# Phase 45 Plan 02: MIR Lowering & Codegen Summary

**Desugar `expr?` to Match + Return + ConstructVariant in MIR, with bugfixes for return type annotation parsing and match-arm early-return codegen**

## Performance

- **Duration:** ~25 min
- **Started:** 2026-02-09T15:50:00Z
- **Completed:** 2026-02-09T16:15:00Z
- **Tasks:** 3 (Task 3 was already done in a prior phase)
- **Files modified:** 9

## Accomplishments
- Implemented MIR lowering that desugars `expr?` into a Match expression with two arms: unwrap on success (Ok/Some), early-return on failure (Err/None)
- Tracks enclosing function return type through save/restore pattern for correct early-return variant construction in nested functions and closures
- Fixed three blocking bugs in typeck and codegen that prevented the ? operator from working end-to-end
- Added 5 comprehensive e2e tests covering all ? operator paths (Result Ok, Result Err, Option Some, Option None, chained ?)

## Task Commits

Each task was committed atomically:

1. **Task 1: MIR lowering for TryExpr** - `ae28dba` (feat)
2. **Task 1b: Return type annotation + codegen bugfixes** - `a9baddf` (fix)
3. **Task 2: E2e tests** - `5caa8b7` (test)
4. **Task 3: Formatter** - already handled by prior phase (walk_tokens_inline)

## Files Created/Modified
- `crates/snow-codegen/src/mir/lower.rs` - Added lower_try_expr, lower_try_result, lower_try_option with current_fn_return_type tracking
- `crates/snow-typeck/src/infer.rs` - Fixed return type annotation parsing to handle generic/sugar types (Result<T,E>, Int!String, Int?)
- `crates/snow-codegen/src/codegen/pattern.rs` - Fixed codegen_leaf to handle terminator-producing match arm bodies
- `crates/snowc/tests/e2e.rs` - Added 5 e2e test functions for ? operator
- `tests/e2e/try_result_ok_path.snow` - Result ? Ok path fixture
- `tests/e2e/try_result_err_path.snow` - Result ? Err propagation fixture
- `tests/e2e/try_option_some_path.snow` - Option ? Some path fixture
- `tests/e2e/try_option_none_path.snow` - Option ? None propagation fixture
- `tests/e2e/try_chained_result.snow` - Chained ? pipeline fixture

## Decisions Made
- Used generic sum type base names ("Result", "Option") as type_name in ConstructVariant, matching the existing lowering pattern for variant constructors
- Desugared entirely to existing MIR primitives with zero new MIR nodes -- codegen required only a single 3-line fix for terminator handling
- Fixed return type annotation parsing at the source (resolve_type_annotation with ARROW skip) rather than adding a separate resolution path

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Return type annotation parsing loses generic type arguments**
- **Found during:** Task 2 (e2e test development)
- **Issue:** `resolve_type_name_str` only extracts the base type name from TYPE_ANNOTATION nodes, so `-> Result<Int, String>` was parsed as just `Result` and `-> Int!String` as just `Int`. This caused the fn_return_type_stack to have wrong types, making ? operator validation fail with "enclosing function returns Int"
- **Fix:** Changed return type annotation parsing in both single-clause and multi-clause infer_fn_def to use `resolve_type_annotation` which handles generic args and sugar types correctly
- **Files modified:** `crates/snow-typeck/src/infer.rs`
- **Verification:** All existing 111 e2e tests pass, plus 5 new ? operator tests
- **Committed in:** `a9baddf`

**2. [Rule 1 - Bug] resolve_type_annotation fails on return type annotations due to leading ARROW token**
- **Found during:** Task 2 (e2e test development)
- **Issue:** TYPE_ANNOTATION nodes for return types include the `->` (ARROW) token as a child. `collect_annotation_tokens` collects ARROW tokens (needed for `Fun(A) -> B` syntax), but `parse_type_tokens` fails when the first token is ARROW instead of IDENT, returning `Ty::Never`
- **Fix:** Added ARROW skip at the start of `resolve_type_annotation` before calling `parse_type_tokens`
- **Files modified:** `crates/snow-typeck/src/infer.rs`
- **Verification:** Return type `Int!String` and `Result<Int, String>` both resolve correctly
- **Committed in:** `a9baddf`

**3. [Rule 1 - Bug] LLVM "terminator in middle of basic block" from match arm with early return**
- **Found during:** Task 2 (e2e test development)
- **Issue:** `codegen_leaf` unconditionally stores the arm body result and branches to merge, but when the arm body is `MirExpr::Return(...)`, the codegen emits a `ret` instruction that terminates the block. The subsequent `build_store` adds an instruction after the terminator, causing LLVM verification failure
- **Fix:** Moved the `build_store` inside the existing terminator check, so store+branch are both skipped when the block already has a terminator
- **Files modified:** `crates/snow-codegen/src/codegen/pattern.rs`
- **Verification:** ? operator compiles and runs correctly, LLVM verification passes
- **Committed in:** `a9baddf`

---

**Total deviations:** 3 auto-fixed (3 bugs)
**Impact on plan:** All three bugs were blocking issues preventing the ? operator from working end-to-end. Two were pre-existing (return type annotation parsing was always simplistic, match arm early-return was never tested before). One was a direct consequence of the ? desugaring pattern. No scope creep.

## Issues Encountered
None beyond the auto-fixed bugs documented above.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- The ? operator works end-to-end for both Result<T,E> and Option<T>
- Phase 45 is complete: parser (45-01), typeck (45-01), MIR lowering (45-02), codegen (45-02), e2e tests (45-02)
- No known blockers for future phases

## Self-Check: PASSED

All 6 created files verified present. All 3 task commits verified in git log.

---
*Phase: 45-error-propagation*
*Completed: 2026-02-09*
