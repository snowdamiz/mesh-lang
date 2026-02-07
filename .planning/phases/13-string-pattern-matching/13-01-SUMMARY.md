---
phase: 13-string-pattern-matching
plan: 01
subsystem: codegen
tags: [string, pattern-matching, exhaustiveness, llvm, snow_string_eq, case-expression]

# Dependency graph
requires:
  - phase: 04-pattern-matching-adts
    provides: "Pattern compilation pipeline, DecisionTree::Test, MirLiteral::String"
  - phase: 08-standard-library
    provides: "snow_string_eq runtime function and intrinsic declaration"
  - phase: 11-multi-clause-functions
    provides: "Multi-clause function pattern matching codegen path"
  - phase: 12-pipe-operator-closures
    provides: "Multi-clause closure pattern matching codegen path"
provides:
  - "Working string literal pattern matching in case expressions via snow_string_eq"
  - "Working string equality/inequality binary comparison (== and !=)"
  - "Correct exhaustiveness checking for string patterns (distinguishes content)"
  - "DIAMOND (<>) operator correctly mapped to BinOp::Concat in MIR lowering"
affects: [14-generic-map-types, 15-http-actor-model]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "String pattern codegen: codegen_string_lit + snow_string_eq + i8-to-i1 comparison"
    - "String exhaustiveness: Walk LITERAL_PAT children for STRING_CONTENT tokens"

key-files:
  created: []
  modified:
    - "crates/snow-codegen/src/codegen/pattern.rs"
    - "crates/snow-codegen/src/codegen/expr.rs"
    - "crates/snow-typeck/src/infer.rs"
    - "crates/snow-codegen/src/mir/lower.rs"
    - "crates/snowc/tests/e2e.rs"

key-decisions:
  - "Made codegen_string_lit pub(crate) for cross-file access from pattern.rs"
  - "Auto-fixed DIAMOND operator mapping to BinOp::Concat (was falling through to BinOp::Add)"

patterns-established:
  - "String intrinsic call pattern: get_intrinsic -> build_call -> extract i8 -> compare NE zero -> i1"

# Metrics
duration: 4min
completed: 2026-02-07
---

# Phase 13 Plan 01: String Pattern Matching Summary

**String pattern matching via snow_string_eq in case expressions, binary == / != comparison, and correct exhaustiveness content extraction**

## Performance

- **Duration:** 4 min
- **Started:** 2026-02-07T23:49:00Z
- **Completed:** 2026-02-07T23:53:12Z
- **Tasks:** 2
- **Files modified:** 5

## Accomplishments
- String literal patterns in case expressions now compile and match correctly at runtime using snow_string_eq
- Binary string == and != operators produce correct boolean results instead of always false/true
- Exhaustiveness checker correctly distinguishes different string patterns ("alice" vs "bob") and requires wildcard
- String patterns work in multi-clause functions and closures (same codegen path)
- Three e2e tests prove end-to-end string pattern matching, equality comparison, and mixed pattern/variable bindings

## Task Commits

Each task was committed atomically:

1. **Task 1: Fix string codegen placeholders and exhaustiveness bug** - `fcdbf96` (feat)
2. **Task 2: Add e2e tests for string pattern matching and comparison** - `c757b56` (test)

**Plan metadata:** pending (docs: complete plan)

## Files Created/Modified
- `crates/snow-codegen/src/codegen/pattern.rs` - String pattern test codegen via snow_string_eq (was always-false placeholder)
- `crates/snow-codegen/src/codegen/expr.rs` - String binary comparison via snow_string_eq (was always-false/true placeholder); made codegen_string_lit pub(crate)
- `crates/snow-typeck/src/infer.rs` - Extract STRING_CONTENT from LITERAL_PAT children (was using quote char from STRING_START)
- `crates/snow-codegen/src/mir/lower.rs` - Map DIAMOND (<>) to BinOp::Concat alongside PLUS_PLUS (++)
- `crates/snowc/tests/e2e.rs` - Three new e2e tests: string pattern matching, string equality comparison, string patterns mixed with variable bindings

## Decisions Made
- Made `codegen_string_lit` pub(crate) visibility so pattern.rs can call it (was private to expr.rs impl block, but both files implement methods on the same CodeGen struct)
- No string interning for pattern comparisons -- each test creates a new snow_string_new allocation; simple and correct, optimization deferred

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] DIAMOND operator not mapped to BinOp::Concat in MIR lowering**
- **Found during:** Task 2 (e2e_string_pattern_mixed_with_variable test)
- **Issue:** The `<>` string concat operator (DIAMOND syntax kind) was not mapped in the MIR lowering binop dispatch. Only `++` (PLUS_PLUS) was mapped to BinOp::Concat. DIAMOND fell through to the `_ => BinOp::Add` fallback, causing "Unsupported binop type: String" error when `<>` was used with strings.
- **Fix:** Added `SyntaxKind::DIAMOND` to the PLUS_PLUS arm: `SyntaxKind::PLUS_PLUS | SyntaxKind::DIAMOND => BinOp::Concat`
- **Files modified:** `crates/snow-codegen/src/mir/lower.rs`
- **Verification:** All e2e tests pass including test with `<>` in case arm body
- **Committed in:** c757b56 (Task 2 commit)

**2. [Rule 3 - Blocking] codegen_string_lit visibility insufficient for cross-file access**
- **Found during:** Task 1 (cargo build)
- **Issue:** `codegen_string_lit` was `fn` (private) in expr.rs, but pattern.rs needed to call it. Both files are in the `codegen` module implementing methods on `CodeGen`.
- **Fix:** Changed visibility to `pub(crate) fn codegen_string_lit`
- **Files modified:** `crates/snow-codegen/src/codegen/expr.rs`
- **Verification:** Cargo build succeeds, all tests pass
- **Committed in:** fcdbf96 (Task 1 commit)

---

**Total deviations:** 2 auto-fixed (2 blocking)
**Impact on plan:** Both auto-fixes were necessary for the implementation to compile and for the e2e tests to pass. No scope creep.

## Issues Encountered
None beyond the auto-fixed deviations above.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Phase 13 complete (single plan phase). String pattern matching fully operational.
- Ready for Phase 14 (Generic Map Types) or Phase 15 (HTTP Actor Model).
- All 24 e2e tests pass, all 60 unit tests pass. Zero regressions.

## Self-Check: PASSED

---
*Phase: 13-string-pattern-matching*
*Completed: 2026-02-07*
