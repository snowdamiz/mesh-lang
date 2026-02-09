---
phase: 37-module-graph-foundation
plan: 02
subsystem: compiler
tags: [module-graph, topological-sort, cycle-detection, kahns-algorithm, import-extraction]

# Dependency graph
requires:
  - phase: 37-01
    provides: "ModuleId, ModuleInfo, ModuleGraph, CycleError types, discover_snow_files, path_to_module_name"
provides:
  - "topological_sort function with Kahn's algorithm and cycle detection"
  - "extract_imports function for both import and from-import declarations"
  - "build_module_graph pipeline: discover -> register -> parse -> edges -> toposort"
affects: [38-import-resolution, 39-cross-module-typeck, 40-multi-module-codegen]

# Tech tracking
tech-stack:
  added: []
  patterns: [BFS Kahn's algorithm with alphabetical tie-breaking, in-degree tracking for dependency ordering, two-phase graph construction (register then link)]

key-files:
  created: []
  modified:
    - crates/snow-common/src/module_graph.rs
    - crates/snowc/src/discovery.rs

key-decisions:
  - "Alphabetical tie-breaking in toposort for deterministic compilation order across platforms"
  - "Silent skip for unknown imports (stdlib, typos) -- Phase 39 handles error reporting"
  - "Self-import detected as a distinct error before toposort runs"
  - "Two-phase graph construction: register all modules first, then parse and build edges"

patterns-established:
  - "Kahn's BFS with in_degree = dependency count (reversed direction vs standard graph Kahn's)"
  - "extract_cycle_path follows unprocessed dependency edges to reconstruct cycle for error messages"
  - "build_module_graph as complete pipeline: file discovery -> module registration -> parsing -> edge construction -> topological sort"

# Metrics
duration: 6min
completed: 2026-02-09
---

# Phase 37 Plan 02: Topological Sort and Module Graph Pipeline Summary

**Kahn's algorithm toposort with cycle detection, import extraction from parsed ASTs, and full build_module_graph pipeline from file discovery through compilation ordering**

## Performance

- **Duration:** 6 min
- **Started:** 2026-02-09T18:39:41Z
- **Completed:** 2026-02-09T18:46:21Z
- **Tasks:** 2
- **Files modified:** 2

## Accomplishments
- Kahn's BFS topological sort with alphabetical tie-breaking for deterministic compilation order
- Cycle detection that extracts the full cycle path for error messages (e.g., "A -> B -> C -> A")
- Import extraction supporting both `import Foo.Bar` and `from Foo.Bar import { ... }` declarations
- Complete build_module_graph pipeline wiring file discovery, parsing, and toposort into one call
- 12 new tests (6 unit + 6 integration), all passing with zero regressions across entire workspace

## Task Commits

Each task was committed atomically:

1. **Task 1: Kahn's topological sort with cycle detection** - `863c9a2` (feat)
2. **Task 2: Import extraction and build_module_graph pipeline** - `f5b6913` (feat)

## Files Created/Modified
- `crates/snow-common/src/module_graph.rs` - Added topological_sort, extract_cycle_path functions, Debug derive on ModuleGraph, 6 unit tests
- `crates/snowc/src/discovery.rs` - Added extract_imports, build_module_graph functions, 6 integration tests with tempdir

## Decisions Made
- Alphabetical tie-breaking in Kahn's algorithm ensures same compilation order on all platforms
- Unknown imports (stdlib or typos) silently skipped during graph construction -- Phase 39 will handle unresolved import errors
- Self-import detected as a separate error ("Module 'X' cannot import itself") before toposort runs
- Two-phase graph construction: first register all modules (assigning IDs), then parse and build edges (so all modules are resolvable)
- Added Debug derive to ModuleGraph to support test assertions with unwrap_err()

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Added Debug derive to ModuleGraph**
- **Found during:** Task 2 (build_module_graph tests)
- **Issue:** Tests using `unwrap_err()` require the Ok type to implement Debug, but ModuleGraph lacked the derive
- **Fix:** Added `#[derive(Debug)]` to ModuleGraph struct
- **Files modified:** crates/snow-common/src/module_graph.rs
- **Verification:** All tests compile and pass
- **Committed in:** f5b6913 (Task 2 commit)

---

**Total deviations:** 1 auto-fixed (1 bug fix)
**Impact on plan:** Trivial derive addition needed for test ergonomics. No scope creep.

## Issues Encountered
None.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Complete module graph foundation ready for Phase 38 (import resolution integration into build pipeline)
- `build_module_graph` provides the single entry point that Phase 38 will call to get compilation order
- All success criteria from Phase 37 verified:
  - SC1 (file discovery): tested via test_discover_snow_files, test_build_module_graph_simple
  - SC2 (path naming): tested via test_path_to_module_name_nested, test_path_to_module_name_snake_case
  - SC3 (main.snow entry): tested via test_path_to_module_name_main, test_toposort_entry_last
  - SC4 (deterministic order): tested via test_toposort_diamond, test_toposort_independent
  - SC5 (cycle detection): tested via test_toposort_cycle, test_build_module_graph_cycle

## Self-Check: PASSED

- FOUND: crates/snow-common/src/module_graph.rs
- FOUND: crates/snowc/src/discovery.rs
- FOUND: .planning/phases/37-module-graph-foundation/37-02-SUMMARY.md
- FOUND: commit 863c9a2 (Task 1)
- FOUND: commit f5b6913 (Task 2)

---
*Phase: 37-module-graph-foundation*
*Completed: 2026-02-09*
