---
phase: 33-while-loop-loop-control-flow
plan: 01
subsystem: compiler
tags: [while, break, continue, lexer, parser, ast, typeck, loop-control-flow, hindley-milner]

# Dependency graph
requires:
  - phase: 32-collections-map-set
    provides: "Existing compiler pipeline (lexer, parser, typeck, codegen) infrastructure"
provides:
  - "While, Break, Continue TokenKind variants and keyword_from_str recognition"
  - "WHILE_KW, BREAK_KW, CONTINUE_KW SyntaxKind keyword nodes"
  - "WHILE_EXPR, BREAK_EXPR, CONTINUE_EXPR composite CST node kinds"
  - "WhileExpr (condition/body accessors), BreakExpr, ContinueExpr AST wrappers"
  - "parse_while_expr, parse_break_expr, parse_continue_expr parser functions"
  - "infer_while (Unit return), infer_break (Never), infer_continue (Never) type inference"
  - "loop_depth tracking on InferCtx with enter_loop/exit_loop/enter_closure/exit_closure"
  - "BreakOutsideLoop (E0032), ContinueOutsideLoop (E0033) TypeError variants with ariadne diagnostics"
affects: [33-02-codegen-while-break-continue]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "InferCtx.loop_depth for loop-scoped semantic checks (avoids 55+ function signature changes)"
    - "enter_closure/exit_closure saves and resets loop_depth to enforce closure boundary rule"

key-files:
  created: []
  modified:
    - "crates/snow-common/src/token.rs"
    - "crates/snow-parser/src/syntax_kind.rs"
    - "crates/snow-parser/src/ast/expr.rs"
    - "crates/snow-parser/src/parser/expressions.rs"
    - "crates/snow-typeck/src/unify.rs"
    - "crates/snow-typeck/src/error.rs"
    - "crates/snow-typeck/src/diagnostics.rs"
    - "crates/snow-typeck/src/infer.rs"
    - "crates/snow-codegen/src/mir/lower.rs"
    - "crates/snow-lsp/src/analysis.rs"

key-decisions:
  - "Used InferCtx.loop_depth field instead of threading loop_depth through 55+ function signatures"
  - "Reset loop_depth to 0 in closure bodies for BRKC-05 (simpler than BreakInClosure sentinel)"
  - "Error codes E0032/E0033 for BreakOutsideLoop/ContinueOutsideLoop (E0030/E0031 were taken)"

patterns-established:
  - "InferCtx state fields for scope-sensitive semantic checks: add field to ctx, use enter/exit methods"

# Metrics
duration: 10min
completed: 2026-02-09
---

# Phase 33 Plan 01: While/Break/Continue Front-Half Summary

**while/break/continue keywords through lexer, parser, AST, and type checker with loop-depth tracking and closure boundary enforcement**

## Performance

- **Duration:** 10 min
- **Started:** 2026-02-09T07:03:17Z
- **Completed:** 2026-02-09T07:13:18Z
- **Tasks:** 2
- **Files modified:** 10

## Accomplishments
- while/break/continue recognized as keywords (not identifiers) with full lexer, parser, CST, and AST support
- Type checker infers while as Unit, break/continue as Never with loop_depth tracking via InferCtx
- break/continue outside loops produce compile-time errors (E0032/E0033) with ariadne diagnostics
- Closure bodies reset loop_depth to 0, enforcing the "cannot cross closure boundary" rule (BRKC-05)
- All 1,255 existing tests pass without regression

## Task Commits

Each task was committed atomically:

1. **Task 1: Add while/break/continue keywords, CST nodes, AST wrappers, and parser** - `ca6a812` (feat)
2. **Task 2: Add type checker inference for while/break/continue with loop-depth tracking** - `113b207` (feat)

## Files Created/Modified
- `crates/snow-common/src/token.rs` - Added While, Break, Continue keyword variants (45->48 keywords)
- `crates/snow-parser/src/syntax_kind.rs` - Added KW and EXPR SyntaxKind variants with From<TokenKind> mappings
- `crates/snow-parser/src/ast/expr.rs` - Added WhileExpr (condition/body), BreakExpr, ContinueExpr AST wrappers
- `crates/snow-parser/src/parser/expressions.rs` - Added parse_while_expr, parse_break_expr, parse_continue_expr
- `crates/snow-typeck/src/unify.rs` - Added loop_depth field and enter/exit methods to InferCtx
- `crates/snow-typeck/src/error.rs` - Added BreakOutsideLoop, ContinueOutsideLoop TypeError variants
- `crates/snow-typeck/src/diagnostics.rs` - Added E0032/E0033 error codes and ariadne rendering
- `crates/snow-typeck/src/infer.rs` - Added infer_while, infer_break, infer_continue with closure boundary handling
- `crates/snow-codegen/src/mir/lower.rs` - Added placeholder match arms for new Expr variants (codegen in Plan 02)
- `crates/snow-lsp/src/analysis.rs` - Added span extraction for new TypeError variants

## Decisions Made
- **InferCtx.loop_depth instead of function signature threading:** The plan suggested renaming infer_expr to infer_expr_inner with loop_depth parameter, requiring updates to 55+ function signatures and all helper functions. Instead, added loop_depth as a field on InferCtx (already passed everywhere as &mut), achieving identical behavior with zero signature changes. This is a well-established pattern in the codebase (errors/warnings are already tracked the same way).
- **Simple reset-to-0 for closure boundary:** Rather than tracking separate closure_depth_at_loop_entry for more specific error messages, reset loop_depth to 0 when entering closure bodies. This produces BreakOutsideLoop error (acceptable for v1.7 as noted in the plan).
- **Error codes E0032/E0033:** The plan suggested E0030/E0031, but E0030 was already used for NoSuchMethod. Used E0032/E0033 instead.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Fixed exhaustive match in codegen and LSP for new Expr/TypeError variants**
- **Found during:** Task 2 (cargo check on full workspace)
- **Issue:** Adding new Expr variants (WhileExpr, BreakExpr, ContinueExpr) and TypeError variants (BreakOutsideLoop, ContinueOutsideLoop) caused exhaustive match errors in snow-codegen and snow-lsp
- **Fix:** Added placeholder Unit return for loop expressions in codegen (real lowering in Plan 02), added span extraction for new TypeError variants in LSP
- **Files modified:** crates/snow-codegen/src/mir/lower.rs, crates/snow-lsp/src/analysis.rs
- **Verification:** cargo check succeeds, all 1,255 tests pass
- **Committed in:** 113b207 (Task 2 commit)

**2. [Rule 1 - Bug] Corrected error code assignment to avoid collision with E0030**
- **Found during:** Task 2 (adding error codes in diagnostics.rs)
- **Issue:** Plan suggested E0030/E0031 for BreakOutsideLoop/ContinueOutsideLoop, but E0030 was already assigned to NoSuchMethod
- **Fix:** Used E0032/E0033 instead
- **Files modified:** crates/snow-typeck/src/diagnostics.rs
- **Verification:** No duplicate error codes, all diagnostic tests pass
- **Committed in:** 113b207 (Task 2 commit)

---

**Total deviations:** 2 auto-fixed (1 blocking, 1 bug)
**Impact on plan:** Both fixes necessary for correctness. No scope creep.

## Issues Encountered
None

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Lexer/parser/AST/typeck complete for while/break/continue
- Codegen placeholder in place, ready for Plan 02 (MIR lowering + LLVM IR generation)
- loop_depth tracking pattern established, extensible for future loop constructs (for-in)

## Self-Check: PASSED

All 11 modified files verified present. Both task commits (ca6a812, 113b207) verified in git log.

---
*Phase: 33-while-loop-loop-control-flow*
*Completed: 2026-02-09*
