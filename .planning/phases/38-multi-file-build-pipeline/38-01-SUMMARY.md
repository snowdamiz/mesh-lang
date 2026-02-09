---
phase: 38-multi-file-build-pipeline
plan: 01
subsystem: compiler
tags: [module-graph, parsing, pipeline, project-data]

# Dependency graph
requires:
  - phase: 37-module-graph-foundation
    provides: "ModuleGraph, ModuleId, topological_sort, discover_snow_files, extract_imports, build_module_graph"
provides:
  - "ProjectData struct with graph, compilation_order, module_sources, module_parses"
  - "build_project function as main multi-file pipeline entry point"
  - "build_module_graph backward-compatible wrapper"
affects: [38-02, 39-cross-module-type-checking, 40-name-mangling, 41-mir-merging]

# Tech tracking
tech-stack:
  added: []
  patterns: ["ProjectData struct with parallel Vec indexing by ModuleId.0", "Pipeline function retaining parse results for downstream phases"]

key-files:
  created: []
  modified:
    - "crates/snowc/src/discovery.rs"

key-decisions:
  - "build_project returns ProjectData with all intermediate results, eliminating double-parsing"
  - "build_module_graph preserved as thin wrapper for Phase 37 backward compatibility"
  - "Parse errors retained in ProjectData without failing build_project (build() handles error reporting)"

patterns-established:
  - "ProjectData as single return type for multi-file pipeline data"
  - "Parallel Vec indexing by ModuleId.0 for sources, parses, and graph modules"

# Metrics
duration: 3min
completed: 2026-02-09
---

# Phase 38 Plan 01: ProjectData and build_project Pipeline Summary

**ProjectData struct and build_project function retaining per-file parse results and sources alongside module graph for downstream compilation**

## Performance

- **Duration:** 3 min
- **Started:** 2026-02-09T19:09:32Z
- **Completed:** 2026-02-09T19:12:22Z
- **Tasks:** 2
- **Files modified:** 1

## Accomplishments
- Created `ProjectData` struct with `graph`, `compilation_order`, `module_sources`, `module_parses` fields, all indexed by `ModuleId.0`
- Implemented `build_project` function with three-phase pipeline: discover+parse, build edges, toposort
- Refactored `build_module_graph` as thin wrapper that delegates to `build_project` and returns only `(graph, compilation_order)`
- All 13 existing Phase 37 tests pass unmodified, validating backward compatibility
- 3 new `build_project` tests validate parse retention, single-file projects, and parse error preservation

## Task Commits

Each task was committed atomically:

1. **Task 1: Create ProjectData struct and build_project function** - `3e88057` (feat)
2. **Task 2: Add build_project unit tests** - `16a6bb7` (test)

## Files Created/Modified
- `crates/snowc/src/discovery.rs` - Added `ProjectData` struct, `build_project` function, refactored `build_module_graph` as wrapper, added 3 new tests

## Decisions Made
- `build_project` returns `ProjectData` owning all data (graph, sources, parses) -- avoids lifetime complexity with references
- Parse errors are retained in `ProjectData` without failing `build_project` -- the caller (`build()`) decides how to handle them
- `build_module_graph` kept as a wrapper (not removed) to preserve all Phase 37 test compatibility with zero changes

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered

None.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness
- `ProjectData` struct ready for Plan 02 to integrate into `build()` function in `main.rs`
- Plan 02 will use `build_project` to replace the single-file parse path and process all modules
- All 171 existing tests pass (16 unit + 155 integration/e2e), providing a solid regression safety net

## Self-Check: PASSED

- FOUND: crates/snowc/src/discovery.rs
- FOUND: 38-01-SUMMARY.md
- FOUND: 3e88057 (Task 1 commit)
- FOUND: 16a6bb7 (Task 2 commit)

---
*Phase: 38-multi-file-build-pipeline*
*Completed: 2026-02-09*
