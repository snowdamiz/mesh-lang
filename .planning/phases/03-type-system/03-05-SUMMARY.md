---
phase: 03-type-system
plan: 05
subsystem: typeck
tags: [ariadne, diagnostics, error-reporting, snapshots, end-to-end, phase-verification]

# Dependency graph
requires:
  - phase: 03-01
    provides: "Parser with type annotations, type representation (Ty), unification engine"
  - phase: 03-02
    provides: "Algorithm J inference engine, TypeckResult, expression/item walking"
  - phase: 03-03
    provides: "Struct definitions, Option/Result constructors, type aliases"
  - phase: 03-04
    provides: "TraitRegistry, interface/impl validation, where-clause enforcement, operator traits"
provides:
  - "ariadne-based diagnostic renderer with dual-span labels and fix suggestions"
  - "TypeckResult::render_errors() for batch diagnostic rendering"
  - "8 insta snapshot tests for error message formatting"
  - "15 integration tests verifying all 5 phase success criteria"
  - "Error codes E0001-E0009 for all TypeError variants"
affects: ["04-pattern-matching", "05-codegen", "06-actor-runtime"]

# Tech tracking
tech-stack:
  added: ["ariadne 0.6"]
  patterns:
    - "render_diagnostic(): TypeError -> formatted string via ariadne Report with colorless Config"
    - "Snapshot-tested diagnostics for regression-proof error messages"
    - "param_type_param_names: positional mapping from function params to type param names for call-site constraint resolution"

key-files:
  created:
    - "crates/snow-typeck/src/diagnostics.rs"
    - "crates/snow-typeck/tests/diagnostics.rs"
    - "crates/snow-typeck/tests/integration.rs"
    - "crates/snow-typeck/tests/snapshots/diagnostics__diag_type_mismatch.snap"
    - "crates/snow-typeck/tests/snapshots/diagnostics__diag_if_branch_mismatch.snap"
    - "crates/snow-typeck/tests/snapshots/diagnostics__diag_arity_mismatch.snap"
    - "crates/snow-typeck/tests/snapshots/diagnostics__diag_unbound_variable.snap"
    - "crates/snow-typeck/tests/snapshots/diagnostics__diag_not_a_function.snap"
    - "crates/snow-typeck/tests/snapshots/diagnostics__diag_trait_not_satisfied.snap"
    - "crates/snow-typeck/tests/snapshots/diagnostics__diag_missing_field.snap"
    - "crates/snow-typeck/tests/snapshots/diagnostics__diag_unknown_field.snap"
  modified:
    - "crates/snow-typeck/Cargo.toml"
    - "crates/snow-typeck/src/lib.rs"
    - "crates/snow-typeck/src/infer.rs"
    - "Cargo.toml"

key-decisions:
  - "ariadne 0.6 with colorless Config for deterministic test output"
  - "render_diagnostic() returns String (not printed directly) for test capture"
  - "Error codes E0001-E0009 assigned to each TypeError variant"
  - "Fix suggestions based on type pair analysis (Option wrapping, Result wrapping, numeric conversion, to_string)"
  - "param_type_param_names tracks which function params correspond to type params for accurate call-site where-clause checking"

patterns-established:
  - "Diagnostic rendering: TypeError -> ariadne Report -> String buffer with Config::default().with_color(false)"
  - "Snapshot testing: insta::assert_snapshot! on rendered diagnostic output for regression protection"
  - "Call-site type param resolution: use positional arg types (not definition-time type vars) for where-clause constraint checking"

# Metrics
duration: ~6min
completed: 2026-02-06
---

# Phase 3 Plan 5: Type Error Diagnostics and Phase Verification Summary

**ariadne-based error rendering with dual-span labels, fix suggestions, error codes E0001-E0009, and all 5 phase success criteria verified end-to-end**

## Performance

- **Duration:** ~6 min
- **Started:** 2026-02-06T19:32:38Z
- **Completed:** 2026-02-06T19:38:32Z
- **Tasks:** 2/2
- **Files modified:** 15 files created/modified

## Accomplishments
- ariadne diagnostic renderer covering all 11 TypeError variants with terse messages, labeled source spans, and fix suggestions
- 8 insta snapshot tests for error message formatting (mismatch, if-branch, arity, unbound, not-a-function, missing-field, unknown-field, trait-not-satisfied)
- 15 integration tests verifying all 5 phase success criteria (let-polymorphism, occurs check, structs/Option/Result, traits, error locations)
- All 258 workspace tests passing

## Task Commits

Each task was committed atomically:

1. **Task 1: Implement ariadne diagnostic renderer with fix suggestions** - `b9a8e4e` (feat)
2. **Task 2: Snapshot tests for diagnostics and end-to-end phase verification** - `e39b6e0` (test)

## Files Created/Modified
- `crates/snow-typeck/src/diagnostics.rs` - ariadne-based error rendering with fix suggestions for all TypeError variants
- `crates/snow-typeck/tests/diagnostics.rs` - 10 tests for diagnostic snapshot and error code verification
- `crates/snow-typeck/tests/integration.rs` - 15 end-to-end tests for all 5 phase success criteria
- `crates/snow-typeck/tests/snapshots/*.snap` - 8 insta snapshots for error message formatting
- `crates/snow-typeck/src/lib.rs` - Added `pub mod diagnostics;` and `TypeckResult::render_errors()`
- `crates/snow-typeck/src/infer.rs` - Fixed where-clause constraint resolution at call sites
- `crates/snow-typeck/Cargo.toml` - Added ariadne dependency
- `Cargo.toml` - Added ariadne to workspace dependencies

## Decisions Made
- **ariadne 0.6 with colorless output:** Config::default().with_color(false) produces deterministic output for snapshot tests. Colors can be enabled for user-facing output later.
- **String return (not print):** render_diagnostic() returns a String rather than printing directly, enabling test capture and flexible output routing.
- **Error codes E0001-E0009:** Each TypeError variant gets a unique error code, following the E-prefix convention.
- **Fix suggestions by type pair:** When expected/found types match known patterns (Option<T>/T, Result<T,E>/T, Int/Float, String/Int), a contextual fix is suggested.
- **param_type_param_names for call-site resolution:** Tracks which function params correspond to type params so where-clause checking uses actual call-site arg types (not definition-time type vars that become stale after generalization).

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Fixed where-clause constraint resolution at call sites**
- **Found during:** Task 2 (integration test `test_success_criterion_4_where_clause_satisfied`)
- **Issue:** Where-clause checking at call sites resolved type params from definition-time type variables (FnConstraints.type_params), but after generalization + instantiation, those variables are stale (unbound). The constraint checker saw `?7` (unresolved var) instead of `Int` (the actual argument type), causing a false TraitNotSatisfied error.
- **Fix:** Added `param_type_param_names` to FnConstraints to record which function params are annotated with type param names. At call sites, resolved type params from the actual argument types (arg_types[i]) after unification, falling back to definition-time vars for non-generic cases.
- **Files modified:** `crates/snow-typeck/src/infer.rs`
- **Verification:** `test_success_criterion_4_where_clause_satisfied` passes, all 13 existing trait tests still pass
- **Committed in:** `e39b6e0`

---

**Total deviations:** 1 auto-fixed (1 bug)
**Impact on plan:** Essential bug fix for correct where-clause enforcement. Without this fix, any generic function with where-clause constraints would produce false positive errors at call sites.

## Issues Encountered
- **Where-clause type param stale after generalization:** The core issue was that generalize() quantifies type variables, making them unreachable through the unification table. When instantiate() creates fresh replacements, the FnConstraints still pointed to the original (now-stale) variables. Fixed by tracking the param-to-type-param mapping and resolving from call-site argument types instead.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness
- Phase 3 (Type System) is complete: all 5 success criteria verified end-to-end
- Diagnostic infrastructure ready for pattern matching errors in Phase 4
- ariadne rendering can be extended with colored output for user-facing compiler messages
- 258 tests provide comprehensive regression protection for future phases
- Trait system, struct system, and inference engine are stable foundations for codegen (Phase 5)

## Self-Check: PASSED
