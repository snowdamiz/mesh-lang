---
phase: 91-rest-api
plan: 02
subsystem: api
tags: [dashboard, aggregation, event-detail, navigation, rest-api, postgresql, date_trunc, jsonb]

# Dependency graph
requires:
  - phase: 91-rest-api
    plan: 01
    provides: Query parameter extraction helpers (query_or_default, get_limit), JSON array builder, Api module convention
  - phase: 89-error-grouping-issue-lifecycle
    provides: Issue and event tables, upsert_issue, issue management queries
  - phase: 88-ingestion-pipeline
    provides: PipelineRegistry service pattern, HTTP handler conventions
provides:
  - Dashboard event volume endpoint with hourly/daily bucketing
  - Dashboard error breakdown by severity level
  - Dashboard top issues ranked by event frequency
  - Dashboard tag breakdown by key (environment, release)
  - Per-issue event timeline endpoint
  - Project health summary (unresolved count, 24h events, new today)
  - Full event detail endpoint with all JSONB fields (exception, stacktrace, breadcrumbs, tags, extra, user_context)
  - Event navigation (next/prev) within an issue
affects: [91-03-detail, frontend, sdk]

# Tech tracking
tech-stack:
  added: []
  patterns: [date_trunc-bucketing, jsonb-raw-embedding, two-query-handler, null-safe-json-formatting]

key-files:
  created:
    - mesher/api/dashboard.mpl
    - mesher/api/detail.mpl
  modified:
    - mesher/storage/queries.mpl
    - mesher/main.mpl

key-decisions:
  - "JSONB fields (exception, stacktrace, breadcrumbs, tags, extra, user_context) embedded raw in JSON response without double-quoting"
  - "Two-query pattern in event detail handler: detail query then neighbors query, combined into single response"
  - "Null neighbor IDs formatted as JSON null (not empty string) for clean API contract"
  - "Health summary returns numeric values without string quoting for direct consumption"

patterns-established:
  - "JSONB raw embedding: PostgreSQL JSONB fields arrive as valid JSON strings, concatenated directly into response without additional quoting"
  - "Two-query handler: sequential queries combined via helper functions to satisfy single-expression case arm constraint"
  - "Null-safe JSON formatting: format_neighbor_id checks empty string and emits null or quoted string"
  - "Dashboard handler pattern: registry -> pool -> params -> query -> serialize helper -> respond"

# Metrics
duration: 3min
completed: 2026-02-15
---

# Phase 91 Plan 02: Dashboard Aggregation and Event Detail Summary

**6 dashboard aggregation endpoints (volume, levels, top issues, tags, timeline, health) and full event detail endpoint with raw JSONB payload and next/prev navigation**

## Performance

- **Duration:** 3 min
- **Started:** 2026-02-15T07:05:35Z
- **Completed:** 2026-02-15T07:08:55Z
- **Tasks:** 3
- **Files modified:** 4

## Accomplishments
- 8 parameterized query functions in queries.mpl: 6 dashboard aggregation (date_trunc bucketing, GROUP BY, COUNT, subquery aggregates) and 2 event detail (full JSONB payload, tuple-based neighbor navigation)
- 6 dashboard GET endpoint handlers in new api/dashboard.mpl with JSON serialization (numeric counts without quoting, null-safe tag values)
- 1 event detail GET handler in new api/detail.mpl with two-query pattern (detail + neighbors) and raw JSONB field embedding
- All 7 routes registered in main.mpl router

## Task Commits

Each task was committed atomically:

1. **Task 1: Add dashboard and event detail query functions to queries.mpl** - `478fc923` (feat)
2. **Task 2: Create dashboard route handlers (api/dashboard.mpl)** - `ca1912ae` (feat)
3. **Task 3: Create event detail route handlers (api/detail.mpl)** - `afa12bb1` (feat)

**Plan metadata:** (pending final docs commit)

## Files Created/Modified
- `mesher/storage/queries.mpl` - Extended with 8 new query functions: event_volume_hourly, error_breakdown_by_level, top_issues_by_frequency, event_breakdown_by_tag, issue_event_timeline, project_health_summary, get_event_detail, get_event_neighbors
- `mesher/api/dashboard.mpl` - New file with 6 pub handler functions and JSON serialization helpers for dashboard endpoints
- `mesher/api/detail.mpl` - New file with 1 pub handler function, event_detail_to_json with raw JSONB embedding, and neighbor navigation helpers
- `mesher/main.mpl` - Updated imports (Api.Dashboard, Api.Detail), registered 7 new GET routes

## Decisions Made
- JSONB fields embedded raw in JSON response -- PostgreSQL returns valid JSON strings for JSONB columns, so they can be concatenated directly into the response without additional `\"` quoting (per research Pitfall 5)
- Two-query pattern for event detail handler -- fetches event detail first, extracts issue_id and received_at, then queries neighbors; uses helper functions (add_navigation, build_nav_response, build_event_response_from_rows) to satisfy single-expression case arm constraint
- Null neighbor IDs formatted as JSON `null` (not empty string) -- format_neighbor_id checks String.length and emits either `null` or `"quoted-id"` for clean API contract
- Health summary returns numeric values without string quoting -- unresolved_count, events_24h, new_today concatenated as bare numbers in JSON

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered

None.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness
- All dashboard and detail endpoints operational and compiling
- Event detail with JSONB raw embedding pattern established for any future endpoints needing JSONB payloads
- Ready for Plan 03 (remaining endpoints or integration)

## Self-Check: PASSED

- All 4 artifact files exist on disk
- All 3 task commits (478fc923, ca1912ae, afa12bb1) verified in git history
- cargo build succeeds with 0 errors

---
*Phase: 91-rest-api*
*Completed: 2026-02-15*
