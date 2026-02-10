---
phase: 45-error-propagation
plan: 01
subsystem: compiler
tags: [parser, typeck, error-propagation, try-operator, pratt-parser, hindley-milner]

# Dependency graph
requires:
  - phase: 44-timer
    provides: "Stable parser/typeck infrastructure with Pratt parser and HM inference"
provides:
  - "TRY_EXPR SyntaxKind and Pratt parser postfix ? at BP 25"
  - "TryExpr AST node with operand() accessor"
  - "fn_return_type_stack on InferCtx for tracking enclosing function return types"
  - "infer_try_expr with Result<T,E> and Option<T> type extraction"
  - "TryIncompatibleReturn (E0036) and TryOnNonResultOption (E0037) error variants"
  - "Ariadne diagnostic rendering for E0036 and E0037"
affects: [45-02-codegen, 45-03-stdlib, future-async-phases]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "fn_return_type_stack push/pop discipline mirroring loop_depth enter/exit pattern"
    - "Postfix operator parsing via QUESTION token check in Pratt expr_bp loop"
    - "Type variable unification approach for ? operand when types not yet resolved"

key-files:
  created: []
  modified:
    - "crates/snow-parser/src/syntax_kind.rs"
    - "crates/snow-parser/src/parser/expressions.rs"
    - "crates/snow-parser/src/ast/expr.rs"
    - "crates/snow-typeck/src/unify.rs"
    - "crates/snow-typeck/src/infer.rs"
    - "crates/snow-typeck/src/error.rs"
    - "crates/snow-typeck/src/diagnostics.rs"
    - "crates/snow-fmt/src/walker.rs"
    - "crates/snow-codegen/src/mir/lower.rs"
    - "crates/snow-lsp/src/analysis.rs"

key-decisions:
  - "Used fn_return_type_stack as Vec<Option<Ty>> following the same push/pop discipline as loop_depth"
  - "Closures push None (inferred return type) -- ? inside closures validates against closure's own return"
  - "For unresolved type variables as ? operand, attempt Result<T,E> unification first, then Option<T>"
  - "Added TryOnNonResultOption (E0037) as separate error from TryIncompatibleReturn (E0036) for clarity"

patterns-established:
  - "fn_return_type_stack: push when entering function/closure body, pop when leaving. Closures always push None."
  - "Postfix operator pattern: check token + POSTFIX_BP in expr_bp loop, open_before + close with node kind"

# Metrics
duration: 5min
completed: 2025-02-09
---

# Phase 45 Plan 01: Parser and Type Checker for Postfix ? Operator Summary

**Postfix `?` operator parsing at POSTFIX_BP with HM type inference extracting T from Result<T,E> and Option<T>, fn_return_type_stack tracking, and E0036/E0037 diagnostics**

## Performance

- **Duration:** ~5 min
- **Started:** 2025-02-09T14:22:00Z
- **Completed:** 2025-02-09T14:27:00Z
- **Tasks:** 2
- **Files modified:** 10

## Accomplishments
- Pratt parser produces TRY_EXPR nodes for `expr?` syntax at same precedence as field access, calls, and indexing
- Type checker validates ? operand is Result<T,E> or Option<T> and extracts the success type T
- fn_return_type_stack correctly tracks return types through nested functions and closures
- E0036 (incompatible function return type) and E0037 (non-Result/Option operand) diagnostics with actionable messages
- All 1,419+ existing workspace tests pass with zero regressions

## Task Commits

Each task was committed atomically:

1. **Task 1: Add TRY_EXPR to parser and AST** - `869f943` (feat)
2. **Task 2: Add type checking for ? operator with fn_return_type_stack** - `1349c41` (feat)

## Files Created/Modified
- `crates/snow-parser/src/syntax_kind.rs` - Added TRY_EXPR variant and updated variant count test
- `crates/snow-parser/src/parser/expressions.rs` - Added postfix ? parsing in Pratt expr_bp loop
- `crates/snow-parser/src/ast/expr.rs` - Added TryExpr AST node with operand() accessor
- `crates/snow-fmt/src/walker.rs` - Added TRY_EXPR handling via walk_tokens_inline
- `crates/snow-typeck/src/unify.rs` - Added fn_return_type_stack with push/pop/current helpers
- `crates/snow-typeck/src/infer.rs` - Added infer_try_expr, push/pop in fn_def/closure/multi-clause
- `crates/snow-typeck/src/error.rs` - Added TryIncompatibleReturn and TryOnNonResultOption variants
- `crates/snow-typeck/src/diagnostics.rs` - Added E0036/E0037 error codes and ariadne rendering
- `crates/snow-codegen/src/mir/lower.rs` - Added TryExpr stub (MirExpr::Unit placeholder)
- `crates/snow-lsp/src/analysis.rs` - Added span extraction for new TypeError variants

## Decisions Made
- Used `Vec<Option<Ty>>` for fn_return_type_stack: `Some(ty)` for annotated return types, `None` for inferred. This mirrors the existing loop_depth push/pop pattern.
- Closures always push `None` since their return type is inferred. This ensures ? inside a closure validates against the closure, not the outer function.
- When ? operand is an unresolved type variable, attempt unification with `Result<fresh_t, fresh_e>` first (most common case), then fall back to `Option<fresh_t>`.
- Created two separate error variants: `TryIncompatibleReturn` (E0036) for when the function return type is wrong, and `TryOnNonResultOption` (E0037) for when the operand type is wrong. This provides more specific diagnostics.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Fixed exhaustive match in snow-codegen and snow-lsp**
- **Found during:** Task 2 (workspace build verification)
- **Issue:** Adding Expr::TryExpr variant caused non-exhaustive pattern match errors in snow-codegen (mir/lower.rs) and snow-lsp (analysis.rs)
- **Fix:** Added TryExpr arm in codegen (stub returning MirExpr::Unit) and TypeError span extraction in LSP
- **Files modified:** crates/snow-codegen/src/mir/lower.rs, crates/snow-lsp/src/analysis.rs
- **Verification:** Full workspace builds and all tests pass
- **Committed in:** 1349c41 (Task 2 commit)

---

**Total deviations:** 1 auto-fixed (1 blocking)
**Impact on plan:** Essential fix for downstream crates. Codegen stub is intentional -- actual MIR lowering for ? will be in a later plan.

## Issues Encountered
None - plan executed smoothly.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Parser and type checker for ? are complete and verified
- Codegen needs actual MIR lowering for ? (currently stubs to Unit) in plan 02
- E2E tests for ? operator should be added in a subsequent plan
- The fn_return_type_stack infrastructure is ready for use by any future features needing return type context

---
*Phase: 45-error-propagation*
*Completed: 2025-02-09*
