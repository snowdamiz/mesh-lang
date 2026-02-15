---
phase: 93-data-retention-and-cleanup
plan: 02
subsystem: api
tags: [http, retention, sampling, ingestion, settings, storage]

# Dependency graph
requires:
  - phase: 93-data-retention-and-cleanup
    plan: 01
    provides: "Schema columns, 8 query functions (settings CRUD, storage, sampling, cleanup), retention_cleaner actor"
  - phase: 92-alerting-system
    provides: "Alert routes pattern in main.mpl and api/ module structure"
  - phase: 91.1-refactor-mesher-to-use-pipe-operators-and-idiomatic-mesh-features
    provides: "Pipe-chained HTTP router pattern, shared Api.Helpers module"
provides:
  - "HTTP API for project settings CRUD (GET/POST retention_days, sample_rate)"
  - "HTTP API for storage visibility (GET event_count, estimated_bytes)"
  - "Ingestion-time event sampling before rate limiting"
  - "Retention cleaner actor spawned at pipeline startup and restart"
affects: [pipeline-startup, ingestion-flow]

# Tech tracking
tech-stack:
  added: []
  patterns: ["sampling-before-rate-limiting for transparent event dropping", "bulk-specific sampling path preserving size limits"]

key-files:
  created: ["mesher/api/settings.mpl"]
  modified: ["mesher/ingestion/routes.mpl", "mesher/ingestion/pipeline.mpl", "mesher/main.mpl"]

key-decisions:
  - "[93-02] Actors cannot be imported across modules in Mesh -- retention_cleaner duplicated in pipeline.mpl (consistent with stream_drain_ticker, health_checker, spike_checker, alert_evaluator)"
  - "[93-02] Separate bulk sampling path (handle_bulk_sampled) preserves 5MB size limit vs 1MB single-event limit"

patterns-established:
  - "Sampling helpers follow define-before-use: authed -> sample_decision -> sampled -> pub handler"
  - "Settings API follows alerts.mpl pattern: registry lookup, pool access, query delegation, case match response"

# Metrics
duration: 5min
completed: 2026-02-15
---

# Phase 93 Plan 02: Retention API Routes and Ingestion Sampling Summary

**Settings CRUD and storage visibility HTTP API with ingestion-time event sampling before rate limiting, completing all four RETAIN requirements**

## Performance

- **Duration:** 5 min
- **Started:** 2026-02-15T17:33:23Z
- **Completed:** 2026-02-15T17:38:10Z
- **Tasks:** 2
- **Files modified:** 4

## Accomplishments
- Created settings API module with 3 handlers: GET/POST project settings (retention_days, sample_rate) and GET project storage (event_count, estimated_bytes)
- Added event sampling to ingestion pipeline (both single and bulk) that checks sample_rate BEFORE rate limiting, returning 202 Accepted for sampled-out events
- Spawned retention_cleaner actor in pipeline startup and restart, completing the daily cleanup automation
- Registered 3 new HTTP routes in the main router for settings and storage endpoints

## Task Commits

Each task was committed atomically:

1. **Task 1: Create settings API handlers and add sampling to ingestion** - `9c26a75b` (feat)
2. **Task 2: Spawn retention cleaner and register settings routes** - `b2ae7608` (feat)

## Files Created/Modified
- `mesher/api/settings.mpl` - New file: 3 pub HTTP handlers for project settings CRUD and storage visibility
- `mesher/ingestion/routes.mpl` - Added check_sample_rate import, event/bulk sampling helpers with define-before-use ordering
- `mesher/ingestion/pipeline.mpl` - Added retention_cleaner actor + helpers (duplicated from services/retention.mpl due to cross-module actor limitation), spawn in start_pipeline and restart_all_services
- `mesher/main.mpl` - Added Api.Settings import and 3 new HTTP routes for settings and storage

## Decisions Made
- [93-02] Actors cannot be imported across modules in Mesh -- retention_cleaner and all its helper functions (cleanup_projects_loop, drop_partitions_loop, run_retention_cleanup, log helpers) duplicated in pipeline.mpl, consistent with how stream_drain_ticker, health_checker, spike_checker, and alert_evaluator are all defined locally in pipeline.mpl
- [93-02] Separate bulk sampling path (handle_bulk_sampled -> handle_bulk_sample_decision -> handle_bulk_authed) preserves the 5MB size limit for bulk ingestion vs the 1MB limit for single events. Using a shared handle_event_sampled for both (as plan suggested) would have incorrectly applied the 1MB limit to bulk payloads.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Fixed define-before-use ordering for sampling helpers**
- **Found during:** Task 1 (Create settings API handlers and add sampling)
- **Issue:** Plan specified placing sampling helpers BEFORE handle_event_authed/handle_bulk_authed, but Mesh requires callees to be defined before callers (define-before-use)
- **Fix:** Reversed ordering so authed functions come first, then sample_decision, then sampled helpers
- **Files modified:** mesher/ingestion/routes.mpl
- **Verification:** Compilation returns 7 pre-existing errors (no new errors)
- **Committed in:** 9c26a75b (Task 1 commit)

**2. [Rule 1 - Bug] Created separate bulk sampling path to preserve 5MB size limit**
- **Found during:** Task 1 (Create settings API handlers and add sampling)
- **Issue:** Plan specified calling handle_event_sampled from handle_bulk, but handle_event_sampled routes to handle_event_authed which uses 1MB size limit via process_event_body. Bulk ingestion requires the 5MB limit in handle_bulk_authed.
- **Fix:** Created handle_bulk_sampled and handle_bulk_sample_decision that route to handle_bulk_authed instead, preserving the correct size validation
- **Files modified:** mesher/ingestion/routes.mpl
- **Verification:** Bulk path correctly routes through handle_bulk_authed with 5MB validation
- **Committed in:** 9c26a75b (Task 1 commit)

**3. [Rule 1 - Bug] Moved retention_cleaner actor to pipeline.mpl instead of cross-module import**
- **Found during:** Task 2 (Spawn retention cleaner and register settings routes)
- **Issue:** Plan specified `from Services.Retention import retention_cleaner` but actors cannot be imported across modules in Mesh (known limitation, decision [90-02])
- **Fix:** Duplicated retention_cleaner actor and all helper functions into pipeline.mpl, following the established pattern for all other pipeline actors
- **Files modified:** mesher/ingestion/pipeline.mpl
- **Verification:** Compilation returns 7 pre-existing errors (no new errors)
- **Committed in:** b2ae7608 (Task 2 commit)

---

**Total deviations:** 3 auto-fixed (3 bugs in plan specification)
**Impact on plan:** All fixes required for Mesh language constraints (define-before-use, no cross-module actor import) or correctness (bulk size limit). No scope change.

## Issues Encountered
None beyond the plan specification issues documented above.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- All four RETAIN requirements are now wired end-to-end:
  - RETAIN-01: Settings CRUD API + daily retention_cleaner actor
  - RETAIN-02: Issue summaries preserved by schema design (no FK from events to issues)
  - RETAIN-03: Storage visibility API endpoint
  - RETAIN-04: Ingestion-time sampling before rate limiting
- Phase 93 (Data Retention & Cleanup) is fully complete
- 7 pre-existing compilation errors unchanged -- no regression

## Self-Check: PASSED

- [x] mesher/api/settings.mpl exists
- [x] mesher/ingestion/routes.mpl exists
- [x] mesher/ingestion/pipeline.mpl exists
- [x] mesher/main.mpl exists
- [x] 93-02-SUMMARY.md exists
- [x] Commit 9c26a75b found
- [x] Commit b2ae7608 found

---
*Phase: 93-data-retention-and-cleanup*
*Completed: 2026-02-15*
