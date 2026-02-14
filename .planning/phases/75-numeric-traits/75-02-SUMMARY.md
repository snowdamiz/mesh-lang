---
phase: 75-numeric-traits
plan: 02
subsystem: codegen
tags: [mir, operator-dispatch, neg, arithmetic, e2e-tests, parser, self-access]

# Dependency graph
requires:
  - phase: 75-01
    provides: "Output associated type on Add/Sub/Mul/Div/Mod, Neg trait, infer_trait_binary_op Output resolution, infer_unary Neg dispatch"
provides:
  - "MIR binary dispatch uses correct Output type for arithmetic (not Bool)"
  - "Div and Mod in MIR user-type binary dispatch table"
  - "MIR Neg unary dispatch emits Call to Neg__neg__TypeName for user types"
  - "Parser supports self.field access in impl method bodies"
  - "Typeck includes all params in impl method fn_ty (not just self)"
  - "E2E tests for custom Add/Sub/Mul and custom Neg with Output"
affects: [numeric-types, operator-overloading, iterator-traits, custom-types]

# Tech tracking
tech-stack:
  added: []
  patterns: ["self.field access in impl method bodies for struct field arithmetic"]

key-files:
  created:
    - "tests/e2e/numeric_traits.mpl"
    - "tests/e2e/numeric_neg.mpl"
  modified:
    - "crates/mesh-codegen/src/mir/lower.rs"
    - "crates/meshc/tests/e2e.rs"
    - "crates/mesh-parser/src/parser/expressions.rs"
    - "crates/mesh-parser/src/ast/expr.rs"
    - "crates/mesh-typeck/src/infer.rs"

key-decisions:
  - "Parser disambiguates self keyword: self() is actor self-call, self.x is method receiver field access"
  - "Typeck fn_ty for impl methods includes all params (self + non-self) for correct MIR lowering"
  - "Comparison operators (Eq/Ord) keep Bool return type; arithmetic uses Output type from resolve_range"

patterns-established:
  - "User impl method self.field access: parser produces NAME_REF for self when not followed by (), field access postfix applies normally"
  - "Result type split in binary dispatch: comparison ops -> Bool, arithmetic ops -> typeck Output type"

# Metrics
duration: 14min
completed: 2026-02-14
---

# Phase 75 Plan 02: MIR Dispatch Fix + Neg Dispatch + E2E Tests Summary

**Fixed MIR binary operator return type (Output not Bool), added Div/Mod and Neg dispatch, with E2E tests proving custom Vec2 arithmetic and Point negation work end-to-end**

## Performance

- **Duration:** 14 min
- **Started:** 2026-02-14T01:13:37Z
- **Completed:** 2026-02-14T01:28:16Z
- **Tasks:** 2
- **Files modified:** 7

## Accomplishments
- Binary operator dispatch for user types now uses the correct Output type from typeck (not hardcoded MirType::Bool) for arithmetic operations
- Div and Mod added to the user-type binary dispatch table alongside Add/Sub/Mul
- Neg unary dispatch emits Call to Neg__neg__TypeName for user-defined types (primitives use hardware path)
- E2E test proves Vec2 with Add/Sub/Mul works including operator chaining (v1 + v2 + v3)
- E2E test proves Point with Neg works including unary minus dispatch
- Primitive arithmetic and negation verified backward-compatible in both tests
- Zero regressions across full workspace test suite

## Task Commits

Each task was committed atomically:

1. **Task 1: Fix MIR binary dispatch and add Neg unary dispatch** - `a1ff4537` (feat)
2. **Task 2: Write E2E tests for numeric traits and Neg** - `02423eac` (feat)

## Files Created/Modified
- `crates/mesh-codegen/src/mir/lower.rs` - Added Div/Mod to dispatch table, fixed return type (Output not Bool), added Neg dispatch in lower_unary_expr
- `tests/e2e/numeric_traits.mpl` - E2E test: Vec2 struct with Add/Sub/Mul, operator chaining, primitive backward compat
- `tests/e2e/numeric_neg.mpl` - E2E test: Point struct with Neg, primitive neg backward compat
- `crates/meshc/tests/e2e.rs` - Test harness entries for numeric_traits and numeric_neg
- `crates/mesh-parser/src/parser/expressions.rs` - Parser disambiguates self keyword: self() vs self.field
- `crates/mesh-parser/src/ast/expr.rs` - NameRef::text() handles SELF_KW token
- `crates/mesh-typeck/src/infer.rs` - impl method fn_ty includes all params (not just self)

## Decisions Made
- Parser disambiguates `self` keyword by looking ahead: `self(` -> actor self-call (SELF_EXPR), `self.` or bare `self` -> method receiver (NAME_REF). This enables field access on self in impl method bodies.
- Typeck fn_ty for impl methods now includes all params (self + non-self like `other`) so the MIR lowerer can bind them correctly.
- Comparison operators (Eq/Ord) keep MirType::Bool return type; arithmetic operators (Add/Sub/Mul/Div/Mod) use the Output type from typeck resolve_range.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Parser did not support self.field access in impl method bodies**
- **Found during:** Task 2 (E2E test creation)
- **Issue:** The parser treated `self` as keyword requiring `self()` syntax (actor self-call). `self.x` in impl methods caused parse errors (expected L_PAREN).
- **Fix:** Modified parser to check lookahead after `self`: if followed by `(` -> parse as SELF_EXPR, otherwise -> parse as NAME_REF. Also updated NameRef::text() to handle SELF_KW token.
- **Files modified:** crates/mesh-parser/src/parser/expressions.rs, crates/mesh-parser/src/ast/expr.rs
- **Verification:** All existing tests pass (self() actor calls unaffected), new E2E tests work with self.x
- **Committed in:** 02423eac (Task 2 commit)

**2. [Rule 1 - Bug] Typeck impl method fn_ty only included self parameter**
- **Found during:** Task 2 (E2E test creation)
- **Issue:** `infer_impl_def` built fn_ty with `params = vec![impl_type.clone()]` -- missing non-self params like `other`. The MIR lowerer's `lower_impl_method` uses fn_ty param count to zip params, leaving `other` unbound.
- **Fix:** Collected all param types (self + non-self) during body inference and used them in fn_ty construction.
- **Files modified:** crates/mesh-typeck/src/infer.rs
- **Verification:** All existing tests pass, `other` parameter correctly bound in E2E test impl methods
- **Committed in:** 02423eac (Task 2 commit)

---

**Total deviations:** 2 auto-fixed (1 blocking, 1 bug)
**Impact on plan:** Both fixes were essential prerequisites for user-written impl methods with field access and multiple parameters. No scope creep -- these enable the exact functionality the plan tests.

## Issues Encountered
None beyond the auto-fixed deviations above.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- All three numeric trait requirements satisfied: NUM-01 (user impl Add/Sub/Mul/Div with Output), NUM-02 (Output for result type), NUM-03 (Neg trait for unary minus)
- Parser now supports self.field access in impl method bodies, enabling more complex trait implementations in future phases
- Typeck correctly types multi-param impl methods, enabling Iterator and other trait patterns

---
*Phase: 75-numeric-traits*
*Completed: 2026-02-14*

## Self-Check: PASSED

All files verified present, all commit hashes found in git log.
