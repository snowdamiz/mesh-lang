---
phase: 32-diagnostics-integration
plan: 01
subsystem: diagnostics
tags: [ariadne, insta, typeck, lsp, error-reporting, ambiguity]

# Dependency graph
requires:
  - phase: 30-core-method-resolution
    provides: "AmbiguousMethod error variant and find_method_traits API"
provides:
  - "Deterministic alphabetical ordering of find_method_traits output"
  - "Precise span-based AmbiguousMethod error location"
  - "Per-trait qualified syntax help text in ambiguity diagnostics"
  - "Diagnostic snapshot tests for AmbiguousMethod rendering"
affects: [32-02, snow-lsp, snow-typeck diagnostics]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Pattern A (constructed TypeError) snapshot tests for trait-ambiguity diagnostics"
    - "Sorted trait name output from find_method_traits for deterministic error messages"

key-files:
  created:
    - "crates/snow-typeck/tests/snapshots/diagnostics__diag_ambiguous_method_deterministic_order.snap"
    - "crates/snow-typeck/tests/snapshots/diagnostics__diag_ambiguous_method_help_text.snap"
  modified:
    - "crates/snow-typeck/src/traits.rs"
    - "crates/snow-typeck/src/error.rs"
    - "crates/snow-typeck/src/diagnostics.rs"
    - "crates/snow-typeck/src/infer.rs"
    - "crates/snow-lsp/src/analysis.rs"
    - "crates/snow-typeck/tests/diagnostics.rs"

key-decisions:
  - "AmbiguousMethod span field uses TextRange (rowan), consistent with other span-bearing variants"
  - "Help text lists per-trait qualified syntax joined by 'or' separator"
  - "Display impl ignores span (span: _) following existing conventions"

patterns-established:
  - "Constructed TypeError snapshot tests for errors requiring multi-trait setup"

# Metrics
duration: 6min
completed: 2026-02-09
---

# Phase 32 Plan 01: Ambiguous Method Diagnostics Summary

**Deterministic sorted trait ordering, precise span-based error location, and per-trait qualified syntax suggestions for AmbiguousMethod diagnostics**

## Performance

- **Duration:** 6 min
- **Started:** 2026-02-09T05:09:09Z
- **Completed:** 2026-02-09T05:15:17Z
- **Tasks:** 2
- **Files modified:** 6

## Accomplishments
- find_method_traits now returns traits in deterministic alphabetical order (DIAG-03)
- AmbiguousMethod error points precisely at the method call site via TextRange span (DIAG-02)
- Help text lists each candidate trait's qualified syntax (e.g., "Display.to_string(value) or Printable.to_string(value)")
- Two insta snapshot tests verify deterministic ordering, precise span, and actionable suggestions
- All 282 tests pass (76 unit + 27 diagnostic + 31 LSP + 148 integration) with zero regressions

## Task Commits

Each task was committed atomically:

1. **Task 1: Sort find_method_traits, add span to AmbiguousMethod, improve help text** - `2c30d54` (feat)
2. **Task 2: Add diagnostic snapshot tests for AmbiguousMethod** - `3965e2d` (test)

## Files Created/Modified
- `crates/snow-typeck/src/traits.rs` - Added trait_names.sort() before return in find_method_traits
- `crates/snow-typeck/src/error.rs` - Added span: TextRange field to AmbiguousMethod variant
- `crates/snow-typeck/src/diagnostics.rs` - Updated rendering to use actual span, per-trait help text, JSON span group
- `crates/snow-typeck/src/infer.rs` - Both AmbiguousMethod construction sites now pass fa.syntax().text_range()
- `crates/snow-lsp/src/analysis.rs` - AmbiguousMethod now returns Some(*span) instead of None
- `crates/snow-typeck/tests/diagnostics.rs` - Two new AmbiguousMethod snapshot tests
- `crates/snow-typeck/tests/snapshots/diagnostics__diag_ambiguous_method_deterministic_order.snap` - Snapshot for ordering test
- `crates/snow-typeck/tests/snapshots/diagnostics__diag_ambiguous_method_help_text.snap` - Snapshot for help text test

## Decisions Made
- AmbiguousMethod span field uses TextRange (consistent with NoSuchMethod, NoSuchField, etc.)
- Display impl destructures span as `span: _` (follows existing convention of not including span in Display output)
- Help text format: "use qualified syntax: TraitA.method(value) or TraitB.method(value)" joined with "or"
- Updated find_method_traits_multiple test to verify deterministic order directly instead of sorting after call

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered

None.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness
- All DIAG-02 and DIAG-03 requirements satisfied
- Ready for 32-02 plan execution (remaining diagnostics polish)
- Snapshot test pattern established for future diagnostic tests

---
*Phase: 32-diagnostics-integration*
*Completed: 2026-02-09*
