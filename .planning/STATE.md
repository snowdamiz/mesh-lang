# Project State

## Project Reference

See: .planning/PROJECT.md (updated 2026-02-07)

**Core value:** Expressive, readable concurrency -- writing concurrent programs should feel as natural as sequential code, with supervision and fault tolerance built in.
**Current focus:** v1.1 Language Polish -- Phase 15 complete (HTTP Actor Model)

## Current Position

Phase: 15 of 15 (HTTP Actor Model)
Plan: 1/1 complete (Phase 15 fully done)
Status: Phase complete -- v1.1 complete
Last activity: 2026-02-08 - Completed 15-01-PLAN.md (actor-per-connection HTTP server)

Progress: ██████████ 100% (5/5 v1.1 phases)

## Performance Metrics

**v1.0 Totals:**
- Plans completed: 55
- Average duration: 9min
- Total execution time: 505min
- Commits: 213
- Lines of Rust: 52,611

**v1.1:**
- Plans completed: 10
- Phases: 5 (11-15)
- Average duration: 8min

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
| Lazy key_type tagging at Map.put instead of Map.new | 14-01 | HM let-generalization prevents type resolution at Map.new(); detect string keys at put/get sites |
| Bidirectional ptr/int coercion in codegen_call | 14-01 | General-purpose: ptr->i64 for args, i64->ptr for returns when runtime uses uniform u64 values |
| Map literal parsed in lhs() on PERCENT + L_BRACE | 14-02 | No lexer changes needed; modulo is infix op handled separately |
| MIR desugaring reuses infer_map_key_type for key_type_tag | 14-02 | Consistent with Map.new() path; resolves Ty::App(Map, [K,V]) for tag |
| snow_panic changed from abort() to panic!() | 15-01 | catch_unwind crash isolation requires catchable panics, not process aborts |
| Actor scheduler init idempotent in snow_http_serve | 15-01 | Ensures scheduler ready even if Snow program doesn't use actors directly |
| Multi-clause function for crash test trigger | 15-01 | case expr non-exhaustive is compile error; multi-clause non-exhaustive is warning only |

### Pending Todos

None.

### Blockers/Concerns

None. Phase 15 complete with zero regressions across all 32 e2e tests and full test suite. v1.1 milestone complete.

## Session Continuity

Last session: 2026-02-08T02:30:46Z
Stopped at: Completed 15-01-PLAN.md (Phase 15 fully complete: actor-per-connection HTTP server with crash isolation)
Resume file: None
Next action: v1.1 complete -- tag milestone
