# Project State

## Project Reference

See: .planning/PROJECT.md (updated 2026-02-07)

**Core value:** Expressive, readable concurrency -- writing concurrent programs should feel as natural as sequential code, with supervision and fault tolerance built in.
**Current focus:** v1.1 Language Polish -- Phase 13 complete (String Pattern Matching)

## Current Position

Phase: 13 of 15 (String Pattern Matching)
Plan: 1/1 complete (Phase 13 fully done)
Status: Phase complete
Last activity: 2026-02-07 - Completed 13-01-PLAN.md (string pattern matching codegen + exhaustiveness + e2e tests)

Progress: ██████░░░░ 60% (3/5 v1.1 phases)

## Performance Metrics

**v1.0 Totals:**
- Plans completed: 55
- Average duration: 9min
- Total execution time: 505min
- Commits: 213
- Lines of Rust: 52,611

**v1.1:**
- Plans completed: 7
- Phases: 5 (11-15)
- Average duration: 7min

## Accumulated Context

### Decisions

Decisions logged in PROJECT.md Key Decisions table.
Full decision history archived in milestones/v1.0-ROADMAP.md.

| Decision | Phase | Rationale |
|----------|-------|-----------|
| Always use parse_fn_clause_param_list for all fn def param lists | 11-01 | Transparent backward compat -- handles both pattern and regular params |
| Guard clause parsed before body detection | 11-01 | Grammar reads: fn name(params) [when guard] [= expr \| do/end] |
| FN_EXPR_BODY node wraps body expression | 11-01 | Clean AST distinction between body forms via child node kind |
| Single = expr FnDef treated as 1-clause MultiClause group | 11-02 | Consistent desugaring path for all expression-body functions |
| Multi-clause guards bypass validate_guard_expr | 11-02 | User decision: arbitrary Bool expressions allowed in guards |
| Non-first clause annotations produce warnings not errors | 11-02 | Gentle UX -- non-first visibility/generics/return type ignored with warning |
| Exhaustiveness non-exhaustive is warning for multi-clause functions | 11-02 | Matches case expression behavior -- warn but don't block compilation |
| Single-param multi-clause uses Match; multi-param uses if-else chain | 11-03 | No MirExpr::Tuple exists, if-else chain avoids adding one |
| Guard variables bound via entry-block allocas before guard eval | 11-03 | Ensures proper LLVM domination and variable availability in guard expressions |
| Arrow body uses expr() wrapped in manual BLOCK for BAR detection | 12-01 | Enables multi-clause detection while preserving BLOCK node for downstream |
| Multi-clause first clause inline, subsequent in CLOSURE_CLAUSE | 12-01 | Avoids retroactive wrapping complexity in CST parser |
| fn IDENT at statement level remains named fn def | 12-01 | Bare closures work in all expression contexts via lhs() dispatch |
| fn do end is valid no-params closure | 12-01 | Natural extension of closure grammar; updated error_fn_missing_name test |
| Pipe arity check limitation documented, not fixed | 12-02 | Pre-existing: typeck checks arity before pipe desugaring. Fixing requires architectural change. |
| Multi-clause closure Match desugaring mirrors named fn pattern | 12-02 | Reuses Phase 11-03 approach: Match for single-param, if-else for multi-param |
| Pipe-aware inference in infer_pipe, not infer_call | 12-03 | CallExpr RHS handled directly: extract callee+args, prepend lhs_ty, unify as full function type |
| codegen_string_lit made pub(crate) for pattern.rs access | 13-01 | Both pattern.rs and expr.rs implement methods on CodeGen; cross-file visibility needed |
| DIAMOND (<>) mapped to BinOp::Concat alongside PLUS_PLUS (++) | 13-01 | Pre-existing gap: <> operator was falling through to BinOp::Add in MIR lowering |

### Pending Todos

None.

### Blockers/Concerns

None. Phase 13 complete with zero regressions across all 24 e2e tests and 60 unit tests.

## Session Continuity

Last session: 2026-02-07T23:53:12Z
Stopped at: Completed 13-01-PLAN.md (Phase 13 complete)
Resume file: None
Next action: Execute Phase 14 plans
