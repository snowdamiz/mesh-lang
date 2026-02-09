---
phase: 40-visibility-enforcement
plan: 01
subsystem: typeck
tags: [visibility, pub, modules, imports, diagnostics]

# Dependency graph
requires:
  - phase: 39-cross-module-type-checking
    provides: collect_exports, ModuleExports, ImportContext, import resolution in infer.rs
provides:
  - Visibility filtering in collect_exports (only pub items exported)
  - private_names field on ExportedSymbols and ModuleExports
  - PrivateItem TypeError variant with E0035 code and "add pub" diagnostic
  - Import resolution distinguishes private from nonexistent items
  - Parser support for `pub type` sum type definitions
affects: [40-visibility-enforcement, cross-module-tests, diagnostics]

# Tech tracking
tech-stack:
  added: []
  patterns: [visibility-gated-exports, private-vs-nonexistent-error-distinction]

key-files:
  created: []
  modified:
    - crates/snow-typeck/src/lib.rs
    - crates/snow-typeck/src/error.rs
    - crates/snow-typeck/src/diagnostics.rs
    - crates/snow-typeck/src/infer.rs
    - crates/snowc/src/main.rs
    - crates/snowc/tests/e2e.rs
    - crates/snow-parser/src/parser/mod.rs
    - crates/snow-lsp/src/analysis.rs

key-decisions:
  - "Trait impls remain unconditionally exported (XMOD-05) while trait defs are gated by pub"
  - "PrivateItem error only for from-import (selective) -- qualified access to private items produces unbound/no-such-field naturally"

patterns-established:
  - "Visibility check pattern: item.visibility().is_some() gates export, else insert into private_names"
  - "Error distinction: check private_names before falling through to ImportNameNotFound"

# Metrics
duration: 10min
completed: 2026-02-09
---

# Phase 40 Plan 01: Visibility Enforcement Summary

**Pub-gated exports in collect_exports with PrivateItem error (E0035) and "add pub" diagnostic for private import attempts**

## Performance

- **Duration:** 10 min
- **Started:** 2026-02-09T21:58:32Z
- **Completed:** 2026-02-09T22:08:48Z
- **Tasks:** 2
- **Files modified:** 8

## Accomplishments
- collect_exports now filters FnDef, StructDef, SumTypeDef, and InterfaceDef by visibility().is_some()
- PrivateItem TypeError variant with E0035 error code, "add pub" help text, and ariadne diagnostic rendering
- Import resolution in FromImportDecl distinguishes private items (PrivateItem error) from nonexistent ones (ImportNameNotFound)
- All 8 cross-module e2e tests updated with `pub` and passing
- Parser dispatch supports `pub type` for sum type definitions

## Task Commits

Each task was committed atomically:

1. **Task 1: Filter collect_exports by pub, plumb private_names through pipeline** - `10352cf` (feat)
2. **Task 2: PrivateItem TypeError, diagnostic, and import resolution check** - `63afda6` (feat)

## Files Created/Modified
- `crates/snow-typeck/src/lib.rs` - Added private_names to ExportedSymbols/ModuleExports, visibility filtering in collect_exports
- `crates/snow-typeck/src/error.rs` - Added PrivateItem TypeError variant with Display impl
- `crates/snow-typeck/src/diagnostics.rs` - Added E0035 error code, diagnostic rendering with "add pub" help
- `crates/snow-typeck/src/infer.rs` - Updated FromImportDecl to check private_names before ImportNameNotFound
- `crates/snowc/src/main.rs` - Passes private_names through build_import_context
- `crates/snowc/tests/e2e.rs` - Added `pub` to all cross-module test fixtures
- `crates/snow-parser/src/parser/mod.rs` - Added `pub type` dispatch for sum type definitions
- `crates/snow-lsp/src/analysis.rs` - Added PrivateItem match arm for LSP diagnostics

## Decisions Made
- Trait impls remain unconditionally exported (XMOD-05) while trait defs are gated by pub visibility
- PrivateItem error is only produced for `from Module import name` (selective imports) -- qualified access to private items through `Module.name` naturally produces "no such field" or "unbound variable" errors, which is acceptable per VIS-03

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Parser dispatch missing `pub type` support**
- **Found during:** Task 1 (e2e test for cross-module sum type)
- **Issue:** The parser's `parse_item_or_stmt` PUB_KW match arm did not include TYPE_KW, so `pub type Shape do...end` produced a parse error
- **Fix:** Added SyntaxKind::TYPE_KW case to PUB_KW dispatch with same lookahead logic as the non-pub TYPE_KW branch
- **Files modified:** crates/snow-parser/src/parser/mod.rs
- **Verification:** e2e_cross_module_sum_type test passes with `pub type Shape`
- **Committed in:** 10352cf (Task 1 commit)

**2. [Rule 3 - Blocking] Missing PrivateItem match arm in snow-lsp**
- **Found during:** Task 2 (compilation failed)
- **Issue:** snow-lsp's analysis.rs has an exhaustive match on TypeError for span extraction; new PrivateItem variant caused compile error
- **Fix:** Added `TypeError::PrivateItem { span, .. } => Some(*span)` to the match
- **Files modified:** crates/snow-lsp/src/analysis.rs
- **Verification:** Full workspace compiles and tests pass
- **Committed in:** 63afda6 (Task 2 commit)

---

**Total deviations:** 2 auto-fixed (2 blocking)
**Impact on plan:** Both auto-fixes were necessary for correctness. No scope creep.

## Issues Encountered
None beyond the auto-fixed deviations above.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Visibility enforcement infrastructure complete (collect_exports filtering, PrivateItem error, diagnostic)
- Plan 40-02 can add targeted e2e tests for private import errors and edge cases
- All existing tests green with zero regressions

---
## Self-Check: PASSED

All 8 modified files verified present. Both task commits (10352cf, 63afda6) verified in git log.

---
*Phase: 40-visibility-enforcement*
*Completed: 2026-02-09*
