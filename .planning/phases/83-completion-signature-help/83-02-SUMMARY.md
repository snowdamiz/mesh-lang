---
phase: 83-completion-signature-help
plan: 02
subsystem: lsp
tags: [signature-help, parameter-info, active-parameter, call-detection, cst, tower-lsp]

# Dependency graph
requires:
  - phase: 83-completion-signature-help
    provides: "Completion engine with scope-aware CST walk patterns"
  - phase: 81-lsp-core
    provides: "LSP server with hover, diagnostics, go-to-definition, document symbols, CST traversal patterns"
provides:
  - "Signature help engine with CALL_EXPR detection, comma counting, callee type resolution, and parameter name extraction"
  - "compute_signature_help public API for LSP signature help handler"
  - "signature_help_provider capability with ( and , trigger characters"
affects: []

# Tech tracking
tech-stack:
  added: []
  patterns: ["CALL_EXPR/ARG_LIST upward walk for call detection", "Multi-strategy callee type resolution from TypeckResult", "Parameter name extraction from FN_DEF CST nodes"]

key-files:
  created:
    - "crates/mesh-lsp/src/signature_help.rs"
  modified:
    - "crates/mesh-lsp/src/server.rs"
    - "crates/mesh-lsp/src/lib.rs"

key-decisions:
  - "Module declaration moved to Task 1 since signature_help.rs tests require it to compile"
  - "Multi-strategy type resolution: direct range lookup, NAME_REF children, then Ty::Fun containment scan"
  - "Parameter names extracted from CST FN_DEF nodes; type-only labels for builtins without CST definitions"

patterns-established:
  - "Call detection by walking upward to ARG_LIST then checking parent is CALL_EXPR"
  - "Active parameter tracking via comma token counting before cursor offset"

# Metrics
duration: 4min
completed: 2026-02-14
---

# Phase 83 Plan 02: Signature Help Summary

**LSP signature help with CALL_EXPR detection, comma-based active parameter tracking, callee type resolution from TypeckResult, and parameter name extraction from CST**

## Performance

- **Duration:** 4 min
- **Started:** 2026-02-14T16:19:50Z
- **Completed:** 2026-02-14T16:23:47Z
- **Tasks:** 2
- **Files modified:** 3

## Accomplishments
- Implemented signature_help.rs with full call detection, active parameter tracking, and parameter info building
- Multi-strategy callee type resolution handles direct range lookup, NAME_REF/FIELD_ACCESS children, and Ty::Fun containment scan
- Parameter names extracted from user-defined FN_DEF CST nodes; type-only labels for built-in functions
- Server advertises signature_help_provider capability with `(` and `,` trigger characters
- 5 new tests covering simple call, active parameter after comma, no-call, first parameter, and parameter names

## Task Commits

Each task was committed atomically:

1. **Task 1: Create signature_help.rs with call detection and parameter extraction** - `005b6073` (feat)
2. **Task 2: Wire signature help handler into server.rs and register module** - `29692087` (feat)

## Files Created/Modified
- `crates/mesh-lsp/src/signature_help.rs` - Signature help engine with compute_signature_help entry point and 5 tests
- `crates/mesh-lsp/src/server.rs` - Signature help handler method, capability advertisement, updated test
- `crates/mesh-lsp/src/lib.rs` - Module declaration and doc comment update

## Decisions Made
- Multi-strategy callee type resolution: tries direct callee range lookup (Strategy A), then NAME_REF/FIELD_ACCESS sub-node lookup (Strategy B), then Ty::Fun containment scan (Strategy C) -- handles the various ways the type checker may store function types
- Parameter names come from CST FN_DEF nodes when available; built-in functions without CST definitions get type-only labels
- Module declaration moved to Task 1 (same deviation as Plan 01 -- tests require module in lib.rs to compile)

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Module declaration moved to Task 1**
- **Found during:** Task 1 (test compilation)
- **Issue:** Task 1 tests require `pub mod signature_help;` in lib.rs to compile, but the plan assigned this to Task 2
- **Fix:** Added module declaration in Task 1 alongside the signature_help.rs file
- **Files modified:** crates/mesh-lsp/src/lib.rs
- **Verification:** `cargo test -p mesh-lsp` compiles and runs all tests
- **Committed in:** 005b6073 (Task 1 commit)

---

**Total deviations:** 1 auto-fixed (1 blocking)
**Impact on plan:** Necessary for compilation. No scope creep.

## Issues Encountered
None.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Phase 83 is now complete: both completion engine (Plan 01) and signature help (Plan 02) are fully operational
- The Mesh LSP server now supports all 6 features: diagnostics, hover, go-to-definition, document symbols, completion, and signature help
- Ready to proceed to the next phase in the v8.0 Developer Tooling milestone

## Self-Check: PASSED

- All 3 created/modified files verified on disk
- Both task commits (005b6073, 29692087) verified in git log
- All 43 tests passing (38 existing + 5 new)

---
*Phase: 83-completion-signature-help*
*Completed: 2026-02-14*
