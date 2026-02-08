# Project State

## Project Reference

See: .planning/PROJECT.md (updated 2026-02-07)

**Core value:** Expressive, readable concurrency -- writing concurrent programs should feel as natural as sequential code, with supervision and fault tolerance built in.
**Current focus:** Milestone v1.3 Traits & Protocols -- Phase 20 in progress

## Current Position

Phase: 20 of 22 (Essential Stdlib Protocols)
Plan: 4 of 5 in current phase
Status: In progress
Last activity: 2026-02-08 -- Completed 20-04-PLAN.md (Eq/Ord for structs)

Progress: ██████░░░░ 65% (11/17 plans)

## Performance Metrics

**v1.0 Totals:**
- Plans completed: 55
- Average duration: 9min
- Total execution time: 505min
- Commits: 213
- Lines of Rust: 52,611

**v1.1 Totals:**
- Plans completed: 10
- Phases: 5 (11-15)
- Average duration: 8min
- Commits: 45
- Lines of Rust: 56,539 (+3,928)

**v1.2 Totals:**
- Plans completed: 6
- Phases: 2 (16, 17)
- Commits: 22
- Lines of Rust: 57,657 (+1,118)

## Accumulated Context

### Decisions

Decisions logged in PROJECT.md Key Decisions table.
Full decision history archived in milestones/v1.0-ROADMAP.md, milestones/v1.1-ROADMAP.md, and milestones/v1.2-ROADMAP.md.

Recent decisions for v1.3:
- Static dispatch via monomorphization (no vtables, no trait objects)
- MIR lowering as the critical integration point (not codegen)
- Name mangling: Trait__Method__Type with double-underscore separators
- Zero new Rust crate dependencies
- FNV-1a for Hash protocol (~30 lines in snow-rt)
- (18-01) TraitRegistry storage: FxHashMap<String, Vec<ImplDef>> keyed by trait name
- (18-01) Type param freshening: single uppercase ASCII letter heuristic (A-Z)
- (18-01) Structural matching: throwaway InferCtx per match attempt
- (18-02) Duplicate detection: structural overlap check in register_impl before insert
- (18-02) find_method_traits as separate helper (ambiguity check at call site)
- (18-02) Push impl even on duplicate (for error recovery in downstream lookups)
- (18-03) TraitRegistry re-exported at crate root, owned in TypeckResult, borrowed in Lowerer
- (18-03) Unified dispatch: built-in and user types share identical TraitRegistry resolution path
- (19-01) extract_impl_names() as free function reused by pre-registration and lowering
- (19-01) Self parameter detected via SELF_KW, type from Ty::Fun zip (not manual construction)
- (19-01) typeck stores impl method Ty::Fun in types map for lowerer consumption
- (19-02) mir_type_to_ty as separate function in types.rs for MirType-to-Ty reverse mapping
- (19-02) First-match for ambiguous traits (typeck already reports ambiguity)
- ~~(19-02) Operator dispatch for Add/Sub/Mul/Eq/Lt only~~ -- EXTENDED in 20-04 (all 6 comparison operators)
- (19-03) Warning (not panic) for unresolvable trait methods (error recovery via LLVM codegen)
- (19-03) Mono depth limit of 64, tracked in both lower_fn_def and lower_impl_method
- (19-04) Smoke test reveals typeck gap: "expected Point, found Point" on self parameter in trait method calls (MIR lowering correct, typeck type identity issue)
- (20-01) Con(c) unifies with App(Con(c), []) bidirectionally for non-generic struct types
- (20-01) Display trait registered as compiler-known with to_string(self) -> String signature
- (20-02) Primitive Display mangled names redirected to runtime functions at MIR lowering (not codegen)
- (20-02) Display__to_string__String handled as identity via short-circuit in lower_call_expr
- (20-03) Debug impls auto-registered only for non-generic struct/sum types
- (20-03) Primitive Debug inspect reuses Display runtime functions (Int/Float/Bool); String wraps in quotes
- (20-03) Sum type Debug inspect returns variant name only (no payload details for v1.3)
- (20-04) Ord trait method renamed from "cmp" to "lt" for consistency with operator dispatch
- (20-04) String Ord impl registered (runtime lt helper not yet implemented)
- (20-04) NotEq/Gt/LtEq/GtEq expressed as negate/swap transformations of eq/lt
- (20-04) Negation in operator dispatch uses BinOp::Eq(x, false) not UnaryOp::Not

### Pending Todos

None.

### Blockers/Concerns

- (Research) Self type representation needed for Default protocol (`default() -> Self`) -- resolve during Phase 21 planning
- (Research) Higher-order constrained functions drop constraints when captured as values -- document as known limitation for v1.3
- ~~(19-04) Typeck type identity issue~~ -- RESOLVED in 20-01 (Con vs App(Con, []) unification case added)

## Session Continuity

Last session: 2026-02-08
Stopped at: Completed 20-04-PLAN.md
Resume file: None
Next action: Execute 20-05-PLAN.md (Eq/Ord for sum types)
