---
phase: 42-diagnostics-integration
plan: 01
subsystem: diagnostics
tags: [ariadne, named-source, spans, error-messages, snapshot-tests]

# Dependency graph
requires:
  - phase: 10-diagnostics
    provides: "Ariadne-based diagnostic rendering with dual-span labels"
provides:
  - "Named-source ariadne spans showing actual file paths in diagnostics"
  - "17 updated snapshot tests verifying filename display"
affects: [42-02, compiler-integration, editor-tooling]

# Tech tracking
tech-stack:
  added: []
  patterns: ["Named-source ariadne cache via ariadne::sources()", "(String, Range<usize>) tuple spans for file-aware diagnostics"]

key-files:
  created: []
  modified:
    - crates/snow-typeck/src/diagnostics.rs
    - crates/snow-typeck/tests/snapshots/diagnostics__diag_type_mismatch.snap
    - crates/snow-typeck/tests/snapshots/diagnostics__diag_missing_field.snap
    - crates/snow-typeck/tests/snapshots/diagnostics__diag_unknown_field.snap
    - crates/snow-typeck/tests/snapshots/diagnostics__diag_unbound_variable.snap
    - crates/snow-typeck/tests/snapshots/diagnostics__diag_arity_mismatch.snap
    - crates/snow-typeck/tests/snapshots/diagnostics__diag_if_branch_mismatch.snap
    - crates/snow-typeck/tests/snapshots/diagnostics__diag_trait_not_satisfied.snap
    - crates/snow-typeck/tests/snapshots/diagnostics__diag_invalid_guard_expression.snap
    - crates/snow-typeck/tests/snapshots/diagnostics__diag_non_exhaustive_match.snap
    - crates/snow-typeck/tests/snapshots/diagnostics__diag_redundant_arm.snap
    - crates/snow-typeck/tests/snapshots/diagnostics__diag_receive_outside_actor.snap
    - crates/snow-typeck/tests/snapshots/diagnostics__diag_self_outside_actor.snap
    - crates/snow-typeck/tests/snapshots/diagnostics__diag_send_type_mismatch.snap
    - crates/snow-typeck/tests/snapshots/diagnostics__diag_spawn_non_function.snap
    - crates/snow-typeck/tests/snapshots/diagnostics__diag_not_a_function.snap
    - crates/snow-typeck/tests/snapshots/diagnostics__diag_ambiguous_method_deterministic_order.snap
    - crates/snow-typeck/tests/snapshots/diagnostics__diag_ambiguous_method_help_text.snap

key-decisions:
  - "ariadne::sources() named cache with (String, Range<usize>) spans replaces anonymous Source::from()"
  - "fname local variable avoids repetitive filename.to_string() allocation at each span site"

patterns-established:
  - "Named-source diagnostics: all ariadne Report::build and Label::new calls use (filename, range) tuple spans"

# Metrics
duration: 6min
completed: 2026-02-09
---

# Phase 42 Plan 01: Named-Source Diagnostics Summary

**Ariadne named-source spans wired into render_diagnostic so compile errors display actual file paths (test.snow) instead of `<unknown>`**

## Performance

- **Duration:** 6 min
- **Started:** 2026-02-09T23:26:22Z
- **Completed:** 2026-02-09T23:32:36Z
- **Tasks:** 2
- **Files modified:** 18

## Accomplishments
- Refactored render_diagnostic to use ariadne's named-source API with (String, Range<usize>) tuple spans
- Replaced anonymous Source::from(source) cache with ariadne::sources() named cache
- Updated all 17 diagnostic snapshot tests from `<unknown>` to `test.snow`
- All snow-typeck and snowc tests pass with zero regressions

## Task Commits

Each task was committed atomically:

1. **Task 1: Refactor render_diagnostic to use ariadne named-source spans** - `4b069f4` (feat)
2. **Task 2: Update all diagnostic snapshot tests** - `d1891fa` (test)

## Files Created/Modified
- `crates/snow-typeck/src/diagnostics.rs` - Named-source ariadne spans in render_diagnostic
- `crates/snow-typeck/tests/snapshots/*.snap` (17 files) - Updated snapshots showing test.snow filename

## Decisions Made
- Used a `fname` local variable (single `filename.to_string()` at function start) to avoid repetitive allocations at each of the ~80+ span construction sites
- Removed `Source` import entirely since the human-readable path no longer uses it (JSON mode never used it)

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered
None.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- DIAG-01 complete: diagnostics now show actual file paths
- Ready for 42-02 (remaining diagnostics/integration work)
- Multi-module compilation will automatically show correct module file paths since filename is already threaded through the call chain

## Self-Check: PASSED

All files exist. All commits verified. All 17 snapshots updated. Zero `<unknown>` in snapshots. Zero `Source::from` in diagnostics.rs.

---
*Phase: 42-diagnostics-integration*
*Completed: 2026-02-09*
