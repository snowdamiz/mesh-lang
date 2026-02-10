# Project State

## Project Reference

See: .planning/PROJECT.md (updated 2026-02-09)

**Core value:** Expressive, readable concurrency -- writing concurrent programs should feel as natural as sequential code, with supervision and fault tolerance built in.
**Current focus:** v1.9 Stdlib & Ergonomics -- Phase 46 (Core Collection Operations)

## Current Position

Phase: 45 of 48 (Error Propagation) -- COMPLETE
Plan: 3 of 3 complete
Status: Phase Verified & Complete
Last activity: 2026-02-10 -- Phase 45 verified (5/5 must-haves passed)

Progress: [█████░░░░░] 50% (3/6 v1.9 phases)

## Performance Metrics

**All-time Totals:**
- Plans completed: 135
- Phases completed: 45
- Milestones shipped: 9 (v1.0-v1.8)
- Lines of Rust: 73,384
- Timeline: 5 days (2026-02-05 -> 2026-02-09)

## Accumulated Context

### Decisions

- [43-01] Used LLVM intrinsics (not runtime functions) for all math operations -- zero new dependencies
- [43-01] User-defined modules shadow stdlib modules in lower_field_access resolution order
- [43-01] TyVar(92000) for Math polymorphic type variable; Math.pi as compile-time constant in codegen_var
- [43-02] pow/sqrt are Float-only (not polymorphic) -- users convert with Int.to_float() if needed
- [43-02] floor/ceil/round return Int via fptosi after LLVM intrinsic -- matches "Float to Int" requirement purpose
- [44-01] Extracted codegen_recv_load_message/codegen_recv_process_arms helpers to share code between timeout and no-timeout paths
- [44-01] Used result_alloca + merge block pattern (same as codegen_if) for receive timeout branching
- [44-02] Timer.sleep uses yield loop with deadline (state stays Ready, not Waiting) to avoid scheduler skip
- [44-02] Timer.send_after spawns background OS thread with deep-copied message bytes
- [44-02] Timer.send_after typed as fn(Pid<T>, Int, T) -> Unit (polymorphic with TyVar(MAX-20))
- [45-01] fn_return_type_stack as Vec<Option<Ty>> following loop_depth push/pop pattern; closures always push None
- [45-01] Separate error variants: E0036 TryIncompatibleReturn (wrong fn return type) and E0037 TryOnNonResultOption (wrong operand type)
- [45-01] For unresolved type vars as ? operand: attempt Result<T,E> unification first, then Option<T> fallback
- [45-02] Desugar expr? entirely to Match + Return + ConstructVariant with zero new MIR nodes or codegen paths
- [45-02] Use generic sum type base names (Result, Option) not monomorphized names for ConstructVariant type_name
- [45-02] Fixed return type annotation parsing to use resolve_type_annotation for proper generic/sugar type support
- [45-03] Asserted on both error code (E0036/E0037) and message text with || for resilience in compile_expect_error tests

### Research Notes

- All 6 features require ZERO new Rust crate dependencies
- Receive `after` clause already parsed, type-checked, and MIR-lowered; codegen gap only (~20 lines)
- Result<T,E> and Option<T> fully implemented; ? operator desugars to match+return in MIR
- RECV must complete before TIMER (Timer.sleep uses receive-with-timeout internally)
- TCE uses MIR loop transformation (not LLVM musttail) for reliability
- Collection sort needs comparator callback synthesis; reuse existing Ord dispatch
- Missing snow_string_compare (tech debt at lower.rs:5799) needed for sort

### Pending Todos

None.

### Blockers/Concerns

None.

## Session Continuity

Last session: 2026-02-10
Stopped at: Phase 45 verified & complete
Resume file: None
Next action: /gsd:plan-phase 46
