---
phase: 03-type-system
plan: 04
subsystem: typeck
tags: [traits, interfaces, impl-blocks, where-clauses, operator-dispatch, hindley-milner]

# Dependency graph
requires:
  - phase: 03-01
    provides: "Parser with interface/impl/where-clause syntax, type representation (Ty), unification engine"
  - phase: 03-02
    provides: "Algorithm J inference engine, TypeckResult, expression/item walking"
provides:
  - "TraitRegistry for interface definitions and impl registrations"
  - "Compiler-known traits (Add, Sub, Mul, Div, Mod, Eq, Ord, Not) with built-in impls"
  - "Trait-based operator dispatch for arithmetic and comparison"
  - "Where-clause constraint enforcement at call sites"
  - "Trait method dispatch (call trait methods as regular functions)"
affects: ["03-05", "04-pattern-matching", "05-codegen"]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "TraitRegistry pattern: separate registry for trait defs and impl lookup alongside TypeEnv"
    - "FnConstraints pattern: per-function metadata for where-clause enforcement"
    - "Trait dispatch for binary ops: resolve operator to trait, check impl exists"

key-files:
  created:
    - "crates/snow-typeck/src/traits.rs"
    - "crates/snow-typeck/tests/traits.rs"
  modified:
    - "crates/snow-typeck/src/infer.rs"
    - "crates/snow-typeck/src/builtins.rs"
    - "crates/snow-typeck/src/error.rs"
    - "crates/snow-typeck/src/lib.rs"

key-decisions:
  - "Trait methods callable as regular functions with self-type dispatch (to_string(42) not 42.to_string())"
  - "Where-clause constraints stored per-function in FnConstraints map, checked at call site"
  - "Compiler-known traits registered in builtins alongside existing operator schemes for backward compat"
  - "Snow uses :: for param type annotations, : for trait bounds in where clauses"
  - "type_to_key() string-based impl lookup for exact type matching"

patterns-established:
  - "TraitRegistry: FxHashMap<(trait_name, type_key), ImplDef> for O(1) impl lookup"
  - "FnConstraints: HashMap<fn_name, FnConstraints> threaded through inference for where-clause checking"
  - "infer_trait_binary_op: unify operands, resolve type, check trait impl, return result type"

# Metrics
duration: ~15min
completed: 2026-02-06
---

# Phase 3 Plan 4: Trait System Summary

**TraitRegistry with interface/impl validation, where-clause enforcement, and compiler-known operator traits (Add, Eq, Ord) for arithmetic/comparison dispatch**

## Performance

- **Duration:** ~15 min (split across parallel execution with 03-03)
- **Started:** 2026-02-06T19:00:00Z (approximate, continued from prior session)
- **Completed:** 2026-02-06T19:28:02Z
- **Tasks:** 2 (TDD: RED then GREEN)
- **Files modified:** 6 files created/modified

## Accomplishments
- TraitRegistry with trait definitions, impl registrations, and impl validation (missing methods, signature mismatch)
- Where-clause constraint enforcement at function call sites with type parameter resolution
- Compiler-known traits (Add, Sub, Mul, Div, Mod, Eq, Ord, Not) with built-in impls for Int, Float, Bool, String
- Trait-based binary operator dispatch replacing hardcoded operator inference
- 13 integration tests covering interface defs, impl blocks, where clauses, operator traits, and method dispatch

## Task Commits

Each task was committed atomically:

1. **Task 1: Write failing tests for traits, impls, and where clauses (RED)** - `a0f4793` (test)
2. **Task 2: Implement trait registry, resolution, and compiler-known traits (GREEN)** - `208e7ee` (feat, merged with 03-03 parallel commit)

_Note: Task 2 was committed together with plan 03-03's struct implementation because both plans modified shared files (infer.rs, builtins.rs) in parallel. The 03-03 commit explicitly notes "Includes additive trait infrastructure from parallel plan 03-04"._

## Files Created/Modified
- `crates/snow-typeck/src/traits.rs` - TraitRegistry, TraitDef, ImplDef, impl validation, where-clause checking, type_to_key helper
- `crates/snow-typeck/tests/traits.rs` - 13 integration tests for trait system
- `crates/snow-typeck/src/infer.rs` - Interface/impl processing, trait-based binary op dispatch, where-clause enforcement, FnConstraints
- `crates/snow-typeck/src/builtins.rs` - Compiler-known traits (Add, Sub, Mul, Div, Mod, Eq, Ord, Not) with built-in impls
- `crates/snow-typeck/src/error.rs` - TraitNotSatisfied, MissingTraitMethod, TraitMethodSignatureMismatch error variants
- `crates/snow-typeck/src/lib.rs` - Added `pub mod traits;` module declaration

## Decisions Made
- **Trait methods as regular functions:** `to_string(42)` dispatches to Int's impl (not method syntax `42.to_string()`). This keeps the inference simpler since method dispatch and dot notation are separate concerns.
- **FnConstraints stored separately:** Where-clause constraints are stored in a `FxHashMap<String, FnConstraints>` threaded through inference, not embedded in `Scheme`. This avoids changes to the core type representation.
- **String-based impl lookup:** `type_to_key()` converts types to string keys for `O(1)` HashMap lookup. Sufficient for exact-match concrete types; generic impl resolution deferred.
- **Snow param syntax :: vs :** Snow uses `::` for parameter type annotations (`x :: T`) and `:` for trait bounds in where clauses (`T: Printable`). Tests updated to use correct syntax.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Fixed param type annotation syntax in where-clause tests**
- **Found during:** Task 2 (GREEN phase verification)
- **Issue:** Test inputs used `x: T` (single colon) for param annotations, but Snow's parser expects `x :: T` (double colon). This caused the parser to misparse the fn_def, swallowing the subsequent `show(42)` call expression.
- **Fix:** Changed all where-clause test inputs from `(x: T)` to `(x :: T)`
- **Files modified:** `crates/snow-typeck/tests/traits.rs`
- **Verification:** All 13 trait tests pass after fix
- **Committed in:** `208e7ee` (part of merged commit)

**2. [Rule 3 - Blocking] Merged implementation with parallel 03-03 changes**
- **Found during:** Task 2 (GREEN phase)
- **Issue:** Plan 03-03 was running in parallel and had partially modified `infer.rs` with struct/type-alias infrastructure. The file had conflicting function signatures and missing stub implementations.
- **Fix:** Wrote a complete `infer.rs` that merged both plans' changes: 03-03's struct/field/type-alias infrastructure and 03-04's trait/interface/where-clause infrastructure. Added stub implementations for 03-03's missing functions.
- **Files modified:** `crates/snow-typeck/src/infer.rs`
- **Verification:** All 67 tests pass (23 unit + 16 inference + 15 struct + 13 trait)
- **Committed in:** `208e7ee` (joint commit with 03-03)

---

**Total deviations:** 2 auto-fixed (1 bug, 1 blocking)
**Impact on plan:** Both fixes were necessary for correct operation. The parallel merge was the main challenge -- resolved by combining both plans' changes into a coherent whole.

## Issues Encountered
- **Parallel plan conflict:** 03-03 and 03-04 both modified `infer.rs` simultaneously. 03-03's partial changes left broken function signatures and missing implementations. Resolved by writing a unified version that included both plans' contributions.
- **Parser syntax surprise:** Snow uses `::` (not `:`) for parameter type annotations. The plan's test examples used `:` which the parser did not recognize, causing the fn_def to misparse and consume trailing expressions.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness
- Trait system complete and tested, ready for use in pattern matching (03-05) and future phases
- Compiler-known traits provide clean operator dispatch; new operators can add new traits
- Generic impl resolution (e.g., `impl<T> Trait for List<T>`) deferred -- not needed until generic data structures ship
- Where-clause enforcement works at call sites; nested/higher-order constraint propagation may need future work

## Self-Check: PASSED

---
*Phase: 03-type-system*
*Completed: 2026-02-06*
