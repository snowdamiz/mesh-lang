---
phase: 31-extended-method-support
plan: 02
subsystem: compiler
tags: [codegen, mir, method-resolution, dot-syntax, stdlib-modules, e2e-tests]

# Dependency graph
requires:
  - phase: 31-extended-method-support
    plan: 01
    provides: "Non-struct NoSuchField error triggers retry; stdlib module method fallback in typeck; Display for collections"
  - phase: 30-core-method-resolution
    provides: "Retry-based method resolution, resolve_trait_callee, method interception in lower_call_expr"
provides:
  - "Stdlib module method fallback in MIR resolve_trait_callee (String.length via dot-syntax at MIR level)"
  - "E2e tests proving primitive, generic, chaining, and mixed field/method dot-syntax patterns"
affects: [32-method-polish, future-method-extensions]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "MIR stdlib module method fallback: type-to-prefix mapping through map_builtin_name"
    - "True dot-syntax chaining works end-to-end: p.to_string().length(), p.name.length()"

key-files:
  created: []
  modified:
    - "crates/snow-codegen/src/mir/lower.rs"
    - "crates/snowc/tests/e2e.rs"

key-decisions:
  - "Stdlib module fallback maps MirType::String to string_ prefix, MirType::Ptr to list_ prefix"
  - "True chaining (p.to_string().length()) works -- no need for intermediate variable workaround"
  - "Mixed field+method (p.name.length()) works -- no need for module-qualified fallback"

patterns-established:
  - "MIR stdlib method fallback: construct prefixed name, map through map_builtin_name, check known_functions or prefix match"
  - "E2e test naming: e2e_method_dot_syntax_{variant} for all method dot-syntax tests"

# Metrics
duration: 5min
completed: 2026-02-09
---

# Phase 31 Plan 02: MIR Stdlib Module Method Fallback + E2E Tests Summary

**Stdlib module method fallback in MIR lowering enables "hello".length() at codegen level; 6 new e2e tests prove all Phase 31 criteria including true chaining**

## Performance

- **Duration:** 5 min
- **Started:** 2026-02-09T04:30:49Z
- **Completed:** 2026-02-09T04:36:00Z
- **Tasks:** 2
- **Files modified:** 2

## Accomplishments
- Added stdlib module method fallback in `resolve_trait_callee` so String module functions (length, trim, contains, etc.) resolve at MIR level when called via dot syntax
- Added 6 new compile-and-run e2e tests covering all four Phase 31 success criteria: primitive method calls (Int, Bool, Float), generic type method calls (List), true chaining (p.to_string().length()), and mixed field access + method calls (p.name.length())
- Confirmed true dot-syntax chaining works end-to-end without intermediate variables -- both `p.to_string().length()` and `p.name.length()` compile and produce correct output
- Full workspace: 1,248 tests pass with 0 regressions

## Task Commits

Each task was committed atomically:

1. **Task 1: Add stdlib module method fallback in resolve_trait_callee** - `2ec0437` (feat)
2. **Task 2: Add e2e tests for all Phase 31 success criteria** - `60bc797` (test)

## Files Created/Modified
- `crates/snow-codegen/src/mir/lower.rs` - Added stdlib module method fallback in resolve_trait_callee: maps MirType::String to string_ prefix, MirType::Ptr to list_ prefix, routes through map_builtin_name
- `crates/snowc/tests/e2e.rs` - Added 6 new e2e tests: primitive Int/Bool/Float to_string, generic List to_string, true chaining to_string().length(), mixed field+method p.name.length()

## Decisions Made
- Stdlib module fallback maps `MirType::String` to `string_` prefix and `MirType::Ptr` to `list_` prefix, matching the existing `map_builtin_name` convention
- True chaining form `p.to_string().length()` works end-to-end, so tests use the direct chaining form rather than the module-qualified workaround `String.length(p.to_string())`
- Mixed field+method `p.name.length()` works end-to-end, so tests use the direct dot-syntax form rather than `String.length(p.name)`

## Deviations from Plan

None - plan executed exactly as written. The plan anticipated that true chaining might not work and suggested fallbacks, but both chaining forms work correctly.

## Issues Encountered

None.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness
- All four Phase 31 success criteria are proven by passing e2e tests
- Phase 31 (Extended Method Support) is complete -- primitive, generic, chaining, and mixed patterns all work
- Phase 32 (method polish/cleanup) can proceed if needed
- 1,248 tests pass across the full workspace

## Self-Check: PASSED

- All modified files exist on disk
- All task commits verified in git history (2ec0437, 60bc797)
- SUMMARY.md created at expected path

---
*Phase: 31-extended-method-support*
*Completed: 2026-02-09*
