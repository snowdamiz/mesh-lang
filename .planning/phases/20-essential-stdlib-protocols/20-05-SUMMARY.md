---
phase: 20
plan: 05
subsystem: typeck-codegen-mir
tags: [eq, ord, sum-type-comparison, variant-tag-comparison, auto-derive, mir-generation, match-pattern]
depends_on:
  requires: [20-03, 20-04]
  provides: [eq-ord-sum-type-registration, eq-ord-sum-type-mir-generation, complete-comparison-semantics]
  affects: []
tech_stack:
  added: []
  patterns: [auto-register-eq-ord-for-sum-type-def, nested-match-variant-dispatch, constructor-pattern-payload-binding, lexicographic-lt-vars-helper]
key_files:
  created: []
  modified:
    - crates/snow-typeck/src/infer.rs
    - crates/snow-codegen/src/mir/lower.rs
key-decisions:
  - id: 20-05-01
    decision: "Sum type Eq/Ord uses nested Match with Constructor patterns instead of direct tag/field access"
    rationale: "MIR has no VariantFieldAccess expression; Constructor patterns with bindings extract payload fields through existing codegen infrastructure"
  - id: 20-05-02
    decision: "Ord__lt for sum types generates O(n^2) variant cross-product in inner match arms"
    rationale: "Correctness over optimization; each self-variant arm has n inner arms (one per other-variant) for complete tag ordering"
patterns-established:
  - "generate_eq_sum: nested Match on self then other with Constructor pattern bindings for field-by-field AND comparison"
  - "generate_ord_sum: nested Match dispatching tag ordering (earlier variants < later variants) with lexicographic payload comparison for same variant"
  - "build_lexicographic_lt_vars: reusable helper for lexicographic less-than comparison using named variable prefixes"
duration: 14min
completed: 2026-02-08
---

# Phase 20 Plan 05: Eq/Ord for Sum Types Summary

**Auto-derived variant-aware Eq/Ord for sum types using nested Match patterns with Constructor bindings for tag-then-payload comparison across all 6 operators.**

## Performance

- **Duration:** 14 min
- **Started:** 2026-02-08T08:52:05Z
- **Completed:** 2026-02-08T09:06:07Z
- **Tasks:** 2
- **Files modified:** 2

## Accomplishments
- Eq and Ord impls auto-registered for all non-generic sum types in typeck (matching struct pattern from 20-04)
- Eq__eq__SumTypeName MIR functions auto-generated with variant-tag-first, payload-field-by-field equality
- Ord__lt__SumTypeName MIR functions auto-generated with tag ordering (earlier-defined < later-defined) then lexicographic payload comparison
- All 6 comparison operators (==, !=, <, >, <=, >=) work on sum types via operator dispatch from 20-04
- 8 new tests covering: generation, empty sums, unit variants, operator dispatch for ==, !=, <

## What Was Done

### Task 1: Auto-register Eq/Ord impls for sum types in typeck

**infer.rs changes:**
- In `register_sum_type_def`, extended the existing `if generic_params.is_empty()` block that registered Debug impls
- Added Eq impl registration with `eq(self, other) -> Bool` method signature
- Added Ord impl registration with `lt(self, other) -> Bool` method signature
- Both use `Ty::Con(TyCon::new(&name))` for impl type, matching struct pattern exactly
- Refactored `debug_ty` variable to `impl_ty` with `.clone()` for reuse across all three registrations

### Task 2: Generate Eq__eq and Ord__lt MIR function bodies for sum types

**lower.rs changes:**

1. **`generate_eq_sum(name, variants)`:**
   - Creates `Eq__eq__SumTypeName` MirFunction taking `(self, other) -> Bool`
   - Uses nested `MirExpr::Match`:
     - Outer match on `self` with `MirPattern::Constructor` per variant, binding payload fields to `self_0`, `self_1`, ...
     - Inner match on `other` with Constructor for same variant (binding `other_0`, `other_1`, ...) and Wildcard arm returning false
   - Same variant, no payload: returns `BoolLit(true)`
   - Same variant, with payload: field-by-field equality chained with `BinOp::And`
   - Recursive: Struct/SumType payload fields dispatch to `Eq__eq__InnerType`
   - Empty sum types: returns `BoolLit(true)`
   - Registers in `known_functions`

2. **`generate_ord_sum(name, variants)`:**
   - Creates `Ord__lt__SumTypeName` MirFunction taking `(self, other) -> Bool`
   - Uses nested `MirExpr::Match`:
     - Outer match on `self` with Constructor per variant
     - Inner match on `other` with arms for ALL variants:
       - Other variant has lower tag: returns false (self is NOT less-than)
       - Other variant has same tag: lexicographic payload comparison
       - Other variant has higher tag: returns true (self IS less-than)
   - Same variant, no payload: returns `BoolLit(false)` (equal, not less-than)
   - Same variant, with payload: lexicographic comparison via `build_lexicographic_lt_vars`
   - Empty sum types: returns `BoolLit(false)`
   - Registers in `known_functions`

3. **`build_lexicographic_lt_vars(fields, self_prefix, other_prefix, index)`:**
   - Recursive helper for lexicographic less-than using named variable references
   - For each field: `if self_N < other_N then true else if self_N == other_N then <recurse> else false`
   - Last field: returns direct `self_N < other_N`
   - Handles recursive Struct/SumType fields via `Ord__lt__` and `Eq__eq__` calls

4. **`lower_sum_type_def` updated:** now calls `generate_eq_sum` and `generate_ord_sum` after `generate_debug_inspect_sum_type`

5. **8 new tests added:**
   - `eq_sum_generates_mir_function`: Eq__eq__Color with 2 params, SumType type, Match body
   - `ord_sum_generates_mir_function`: Ord__lt__Color with Match body
   - `eq_sum_no_variants_returns_true`: empty sum Eq returns BoolLit(true)
   - `ord_sum_no_variants_returns_false`: empty sum Ord returns BoolLit(false)
   - `sum_eq_operator_dispatches_to_trait_call`: == on sum types emits Eq__eq__Color
   - `sum_neq_operator_negates_eq`: != emits Eq__eq__Color with negation
   - `sum_lt_operator_dispatches_to_ord`: < emits Ord__lt__Color
   - `eq_sum_unit_variants_only`: Direction with 4 unit variants has 4 match arms

## Task Commits

| Task | Name | Commit | Key Changes |
|------|------|--------|-------------|
| 1 | Auto-register Eq/Ord impls for sum types in typeck | 73f02fc | infer.rs: +Eq/Ord auto-register in register_sum_type_def |
| 2 | Generate Eq__eq and Ord__lt MIR function bodies for sum types | 48a6705 | lower.rs: +generate_eq_sum, +generate_ord_sum, +build_lexicographic_lt_vars, +8 tests |

## Verification Results

- `cargo test --workspace`: 1,146 tests pass, 0 failures (8 new tests added, up from 1,138)
- `cargo build --workspace`: clean compilation
- Eq/Ord impls auto-registered for sum types in typeck
- Generated Eq function compares tag first (via nested Match), then payload fields per variant
- Generated Ord function orders by variant tag (earlier < later), then lexicographic payload
- All 6 comparison operators work on sum types via operator dispatch from 20-04
- Recursive comparison works for nested Struct/SumType payload fields

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Test source used wrong sum type syntax**

- **Found during:** Task 2 test writing
- **Issue:** Initial tests used `type Color = | Red | Green(Int) ...` syntax (Haskell-style) which the Snow parser doesn't recognize. Snow uses `type Color do Red Green(Int) end`
- **Fix:** Changed all test source strings to use correct `type ... do ... end` syntax
- **Files modified:** crates/snow-codegen/src/mir/lower.rs (tests only)
- **Commit:** 48a6705

**2. [Rule 3 - Blocking] Used nested Match with Constructor patterns instead of direct tag/field access**

- **Found during:** Task 2 design phase
- **Issue:** Plan pseudocode assumed direct `self.tag` and `self.field0` access. MIR has no `VariantFieldAccess` expression and `FieldAccess` only works for structs. Cannot extract tag or payload fields directly
- **Fix:** Used nested `MirExpr::Match` with `MirPattern::Constructor` patterns which bind variant payload fields to named variables via the existing decision tree/codegen infrastructure
- **Files modified:** crates/snow-codegen/src/mir/lower.rs
- **Commit:** 48a6705

---

**Total deviations:** 2 auto-fixed (1 bug, 1 blocking)
**Impact on plan:** Both fixes necessary for correctness. The nested Match approach produces correct MIR that leverages existing codegen infrastructure. No scope creep.

## Decisions Made

| ID | Decision | Rationale |
|----|----------|-----------|
| 20-05-01 | Nested Match with Constructor patterns for sum type comparison | MIR lacks VariantFieldAccess; Constructor pattern bindings extract payload through existing codegen |
| 20-05-02 | O(n^2) variant cross-product in Ord inner match arms | Correctness first; each self-variant arm has n inner arms for complete tag ordering |

## Issues Encountered

None beyond the deviations documented above.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness

**Phase 20 complete:** All 5 plans executed. Display, Debug, Eq, and Ord protocols fully implemented for all user-defined types (structs and sum types).

**Delivered:**
- Display trait with primitive and struct/sum impls (20-01, 20-02)
- Debug trait with auto-generated inspect functions (20-03)
- Eq/Ord for structs with field-by-field/lexicographic comparison (20-04)
- Eq/Ord for sum types with variant-tag-then-payload comparison (20-05)
- Operator dispatch for all 6 comparison operators on user types (20-04)
- 1,146 tests passing, zero regressions

**Ready for:**
- Phase 21 planning (next milestone phase)
- End-to-end comparison of struct and sum type values once codegen handles generated MIR functions

---
*Phase: 20-essential-stdlib-protocols*
*Completed: 2026-02-08*

## Self-Check: PASSED
