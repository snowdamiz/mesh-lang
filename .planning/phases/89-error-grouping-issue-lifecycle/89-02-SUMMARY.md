---
phase: 89-error-grouping-issue-lifecycle
plan: 02
subsystem: ingestion
tags: [issue-lifecycle, state-machine, spike-detection, resolve, archive, assign, discard, delete, http-api]

requires:
  - phase: 89-error-grouping-issue-lifecycle
    plan: 01
    provides: Fingerprint computation, issue upsert with regression detection, enriched EventProcessor pipeline, discard check query
provides:
  - Issue state transition queries (resolve, archive, unresolve, assign, discard, delete)
  - Issue listing query with status filter and manual Issue struct construction
  - Volume spike detection query (archived issue auto-escalation)
  - 7 HTTP route handlers for issue management API
  - spike_checker actor with 5-minute periodic check
  - Full issue lifecycle state machine (unresolved <-> resolved, any -> archived, archived -> unresolved via spike, resolved -> unresolved via regression, any -> discarded)
affects: [90-alerting-notifications, dashboard-ui, issue-detail-api]

tech-stack:
  added: []
  patterns:
    - "POST routes with action-in-path for state transitions (POST /issues/:id/resolve instead of PUT)"
    - "PostgreSQL jsonb parsing in route handler for request body extraction (avoiding Mesh-level JSON parsing)"
    - "Recursive list-to-JSON serialization with accumulator pattern (issues_to_json_loop)"
    - "parse_event_count helper for Int field construction from DB text protocol"
    - "Timer.sleep + recursive actor pattern for periodic spike detection (5 min interval)"

key-files:
  created: []
  modified:
    - mesher/storage/queries.mpl
    - mesher/ingestion/routes.mpl
    - mesher/ingestion/pipeline.mpl
    - mesher/main.mpl

key-decisions:
  - "POST routes for all state transitions (resolve, archive, unresolve, assign, discard, delete) instead of PUT/DELETE -- avoids untested HTTP method support"
  - "Default to 'unresolved' status filter for list endpoint -- Mesh lacks query string parsing"
  - "PostgreSQL jsonb extraction for assign request body parsing -- avoids cross-module from_json limitation"
  - "Extracted log_spike_result helper for single-expression case arm constraint in spike_checker actor"

patterns-established:
  - "Issue management route handler pattern: registry lookup -> pool -> query -> JSON response"
  - "Recursive JSON array builder with accumulator for List<Struct> serialization"
  - "parse_event_count: Option<Int> -> Int fallback pattern for DB text protocol Int fields"

duration: 3min
completed: 2026-02-15
---

# Phase 89 Plan 02: Issue Lifecycle Management Summary

**Issue lifecycle API with state transitions (resolve/archive/unresolve/assign/discard/delete), listing endpoint, and automated 5-minute spike detection actor for archived issue escalation**

## Performance

- **Duration:** 3 min
- **Started:** 2026-02-15T04:12:38Z
- **Completed:** 2026-02-15T04:16:17Z
- **Tasks:** 2
- **Files modified:** 4

## Accomplishments
- 8 new query functions covering the full issue lifecycle: resolve, archive, unresolve, assign, discard, delete, list_by_status, check_volume_spikes
- 7 HTTP route handlers for issue management API (1 GET for listing, 6 POST for state transitions)
- spike_checker actor with 5-minute interval for automatic archived issue escalation when volume spikes detected
- Complete state machine: unresolved <-> resolved (manual), any -> archived (manual), archived -> unresolved (automatic spike), resolved -> unresolved (automatic regression from Plan 01), any -> discarded (manual)

## Task Commits

Each task was committed atomically:

1. **Task 1: Issue management queries and HTTP route handlers** - `1f5bd279` (feat)
2. **Task 2: Spike detection actor and main.mpl route registration** - `06149a7a` (feat)

## Files Created/Modified
- `mesher/storage/queries.mpl` - 8 new pub functions: resolve_issue, archive_issue, unresolve_issue, assign_issue, discard_issue, delete_issue, list_issues_by_status, check_volume_spikes + parse_event_count helper
- `mesher/ingestion/routes.mpl` - 7 new pub handler functions + issue_to_json_str, issues_to_json_loop, issues_to_json, assign_from_rows helpers
- `mesher/ingestion/pipeline.mpl` - spike_checker actor, log_spike_result helper, spawn in start_pipeline, import check_volume_spikes
- `mesher/main.mpl` - Extended imports for 7 new handlers, registered 7 new HTTP routes (1 GET + 6 POST)

## Decisions Made
- **POST for all state transitions:** Used POST with action-in-path (e.g., `/api/v1/issues/:id/resolve`) instead of PUT/DELETE HTTP methods. This avoids depending on untested HTTP.on_put/on_delete runtime functions (research Pitfall 7) and is consistent with the existing POST /api/v1/events pattern.
- **Default unresolved listing:** The list endpoint defaults to `status = 'unresolved'` since Mesh lacks query string parsing for the status filter parameter. Future enhancement can add query string support.
- **PostgreSQL jsonb for body parsing:** The assign handler uses `SELECT $1::jsonb->>'user_id'` to extract the user_id from the JSON request body, consistent with the SQL-based field extraction approach established in Plan 01 (decision [89-01]).
- **Extracted log_spike_result helper:** The spike_checker actor's case arm for `Ok(n)` was extracted into a helper function to satisfy Mesh's single-expression case arm constraint (decision [88-02]).

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Extracted log_spike_result helper for spike_checker actor**
- **Found during:** Task 2 (spike_checker implementation)
- **Issue:** The Ok(n) case arm in spike_checker contained an if/else expression with println, which could violate the single-expression case arm constraint (decision [88-02])
- **Fix:** Extracted `log_spike_result(n)` helper function so the case arm is a single function call
- **Files modified:** mesher/ingestion/pipeline.mpl
- **Verification:** Cargo build passes with no errors
- **Committed in:** 06149a7a (Task 2 commit)

---

**Total deviations:** 1 auto-fixed (1 bug -- Mesh parser constraint)
**Impact on plan:** Minor preventive fix. No scope creep.

## Issues Encountered
None -- both tasks executed cleanly. Compilation passed on first attempt for both tasks.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Phase 89 is now fully complete: error grouping pipeline (Plan 01) + issue lifecycle management (Plan 02)
- All ISSUE requirements satisfied: ISSUE-01 (state transitions), ISSUE-02 (regression detection), ISSUE-03 (spike escalation), ISSUE-04 (assignment), ISSUE-05 (delete/discard)
- Ready for Phase 90: Alerting & Notifications (alert rules, notification channels, alert evaluation)
- The issue management API provides the foundation for dashboard UI issue management features

## Self-Check: PASSED

All files verified present. All commits verified in git log. 29 pub query functions in queries.mpl (8 new), 9 pub route handlers in routes.mpl (7 new), 1 spike_checker actor, 9 HTTP routes registered in main.mpl (7 new).

---
*Phase: 89-error-grouping-issue-lifecycle*
*Completed: 2026-02-15*
