---
phase: 93-data-retention-and-cleanup
plan: 01
subsystem: database
tags: [postgres, retention, partitions, cleanup, sampling, actor]

# Dependency graph
requires:
  - phase: 92-alerting-system
    provides: "Schema foundation with alert_rules and alerts tables"
  - phase: 87-core-data-foundation
    provides: "projects table, Pool.query/execute patterns, query module structure"
provides:
  - "retention_days and sample_rate columns on projects table"
  - "8 query functions for retention cleanup, storage estimation, settings CRUD, and sampling"
  - "retention_cleaner actor with daily Timer.sleep cycle"
affects: [93-02-retention-api-and-sampling, pipeline-startup]

# Tech tracking
tech-stack:
  added: []
  patterns: ["pg_inherits partition enumeration for cleanup", "hybrid per-project DELETE + global partition DROP strategy"]

key-files:
  created: ["mesher/services/retention.mpl"]
  modified: ["mesher/storage/schema.mpl", "mesher/storage/queries.mpl"]

key-decisions:
  - "[93-01] Use 'actor' not 'pub actor' -- Mesh grammar doesn't support pub before actor keyword"

patterns-established:
  - "Partition cleanup via pg_inherits/pg_class query to enumerate expired events_YYYYMMDD tables"
  - "SQL-side random() sampling check (check_sample_rate) for ingestion-time event filtering"

# Metrics
duration: 2min
completed: 2026-02-15
---

# Phase 93 Plan 01: Retention Data Foundation Summary

**Retention schema columns, 8 cleanup/storage/settings/sampling queries, and daily retention_cleaner actor with per-project DELETE + partition DROP strategy**

## Performance

- **Duration:** 2 min
- **Started:** 2026-02-15T17:27:55Z
- **Completed:** 2026-02-15T17:30:45Z
- **Tasks:** 2
- **Files modified:** 3

## Accomplishments
- Added retention_days (INT, default 90) and sample_rate (REAL, default 1.0) columns to projects table via idempotent ALTER TABLE
- Added 8 new query functions covering expired event deletion, partition enumeration/drop, storage estimation, project settings CRUD, and probabilistic sampling
- Created retention_cleaner actor with 24-hour Timer.sleep cycle that iterates all projects for per-retention cleanup then drops partitions older than 90 days

## Task Commits

Each task was committed atomically:

1. **Task 1: Schema extension and retention/storage/settings/sampling queries** - `014fa5ea` (feat)
2. **Task 2: Create retention cleaner actor** - `1bfff54a` (feat)

## Files Created/Modified
- `mesher/storage/schema.mpl` - Added 2 ALTER TABLE statements for retention_days and sample_rate on projects
- `mesher/storage/queries.mpl` - Added 8 new pub functions: delete_expired_events, get_expired_partitions, drop_partition, get_all_project_retention, get_project_storage, update_project_settings, get_project_settings, check_sample_rate
- `mesher/services/retention.mpl` - New file with retention_cleaner actor, cleanup_projects_loop, drop_partitions_loop, run_retention_cleanup orchestration

## Decisions Made
- [93-01] Use `actor` not `pub actor` -- Mesh grammar doesn't support `pub` before `actor` keyword (parser expects fn/module/struct/interface/type/supervisor after pub)

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Fixed pub actor to actor in retention_cleaner**
- **Found during:** Task 2 (Create retention cleaner actor)
- **Issue:** Plan specified `pub actor retention_cleaner` but Mesh parser only accepts `actor` without `pub` prefix
- **Fix:** Changed `pub actor` to `actor` to match all existing actor definitions in the codebase
- **Files modified:** mesher/services/retention.mpl
- **Verification:** Compilation returns 7 pre-existing errors (no new errors), matching baseline from [92-03]
- **Committed in:** 1bfff54a (Task 2 commit, amended)

---

**Total deviations:** 1 auto-fixed (1 bug in plan specification)
**Impact on plan:** Minimal -- syntax correction required by Mesh grammar. No scope change.

## Issues Encountered
None beyond the pub actor syntax issue documented above.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Schema columns and all query functions ready for Plan 02 to wire into HTTP API routes and ingestion pipeline
- retention_cleaner actor ready to be spawned from pipeline.mpl start_pipeline function (Plan 02 integration)
- check_sample_rate query ready for ingestion-time sampling in event processing pipeline (Plan 02)
- 7 pre-existing compilation errors unchanged -- no regression

## Self-Check: PASSED

- [x] mesher/services/retention.mpl exists
- [x] mesher/storage/schema.mpl exists
- [x] mesher/storage/queries.mpl exists
- [x] 93-01-SUMMARY.md exists
- [x] Commit 014fa5ea found
- [x] Commit 1bfff54a found

---
*Phase: 93-data-retention-and-cleanup*
*Completed: 2026-02-15*
