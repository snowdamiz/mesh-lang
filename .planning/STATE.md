# Project State

## Project Reference

See: .planning/PROJECT.md (updated 2026-02-09)

**Core value:** Expressive, readable concurrency -- writing concurrent programs should feel as natural as sequential code, with supervision and fault tolerance built in.
**Current focus:** v1.9 Stdlib & Ergonomics -- Phase 47 (Extended Collection Operations)

## Current Position

Phase: 47 of 48 (Extended Collection Operations)
Plan: 2 of 3 complete
Status: Executing Phase 47
Last activity: 2026-02-10 -- Phase 47 Plan 02 complete (Map/Set conversion operations)

Progress: [████████░░] 83% (5/6 v1.9 phases)

## Performance Metrics

**All-time Totals:**
- Plans completed: 138
- Phases completed: 46
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
- [46-01] Extracted SnowOption to shared crate::option module (used by env.rs, http/server.rs, collections/list.rs)
- [46-01] List.contains uses raw u64 equality -- String content equality requires List.any with == predicate
- [46-01] List.find returns Option but pattern matching on FFI Option has pre-existing codegen gap (documented for future phase)
- [46-02] Used Ptr MIR types (not String) for split/join/to_int/to_float since they interact with List/Option opaque pointers
- [46-02] Unique variable names required in case arms to avoid pre-existing LLVM alloca naming collision
- [47-01] alloc_pair helper uses GC heap layout {len=2, elem0, elem1} matching Snow tuple convention
- [47-01] Tuple Con unification escape hatch: Ty::Con("Tuple") unifies with any Ty::Tuple(...) for Tuple.first/second compat
- [47-01] MIR call type fix: use Ptr when typeck resolves Tuple but known_functions returns Ptr
- [47-01] known_functions type priority over typeck-resolved type for stdlib module Var nodes in lower_field_access
- [47-02] Map.from_list defaults to KEY_TYPE_INT since runtime cannot detect key type from list of tuples
- [47-02] No bare name mappings for to_list/from_list to avoid Map/Set ambiguity; merge and difference get bare mappings

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
Stopped at: Completed 47-02-PLAN.md
Resume file: None
Next action: Execute 47-03-PLAN.md or complete Phase 47
