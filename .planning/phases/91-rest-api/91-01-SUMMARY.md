---
phase: 91-rest-api
plan: 01
subsystem: api
tags: [search, filtering, pagination, keyset, tsvector, jsonb, rest-api, postgresql]

# Dependency graph
requires:
  - phase: 89-error-grouping-issue-lifecycle
    provides: Issue lifecycle queries, event storage, upsert_issue, list_issues_by_status
  - phase: 88-ingestion-pipeline
    provides: PipelineRegistry service pattern, HTTP handler conventions, JSON serialization helpers
provides:
  - Filtered issue listing endpoint with status/level/assignment filters and keyset pagination
  - Full-text event search via inline tsvector/plainto_tsquery
  - Tag-based event filtering via JSONB containment operator
  - Per-issue event listing with keyset pagination
  - Reusable query_or_default and get_limit helpers for query parameter extraction
  - Reusable JSON array builder and paginated response builder
affects: [91-02-dashboard, 91-03-detail, frontend, sdk]

# Tech tracking
tech-stack:
  added: []
  patterns: [keyset-pagination, inline-tsvector-search, jsonb-containment-filter, query-param-extraction]

key-files:
  created:
    - mesher/api/search.mpl
  modified:
    - mesher/storage/queries.mpl
    - mesher/main.mpl

key-decisions:
  - "Return raw Map rows from search queries (not Issue structs) for flexible JSON serialization without cross-module struct issues"
  - "Use inline to_tsvector in WHERE clause (not stored tsvector column) to avoid partition complications"
  - "Build tag_json in handler from key/value params rather than passing raw user JSON to JSONB containment"

patterns-established:
  - "Query param extraction: query_or_default(request, param, default) with Request.query Option matching"
  - "Limit handling: get_limit with parse/cap/default pattern returning String for SQL param"
  - "Paginated response: build_paginated_response wrapping data array with cursor metadata"
  - "Api module convention: mesher/api/*.mpl for REST endpoint handlers separate from ingestion"

# Metrics
duration: 3min
completed: 2026-02-15
---

# Phase 91 Plan 01: Search, Filtering, and Pagination Summary

**Filtered issue listing, full-text event search, tag filtering, and keyset pagination via 4 new GET endpoints using parameterized PostgreSQL queries**

## Performance

- **Duration:** 3 min
- **Started:** 2026-02-15T06:59:30Z
- **Completed:** 2026-02-15T07:02:58Z
- **Tasks:** 2
- **Files modified:** 3

## Accomplishments
- 4 parameterized search/filter query functions in queries.mpl with keyset pagination and SQL-injection-safe conditional filters
- 4 GET endpoint handlers in new api/search.mpl with full query parameter extraction, JSON serialization, and pagination metadata
- Replaced hardcoded handle_list_issues with new handle_search_issues supporting optional status/level/assignment filters
- All event queries include 24-hour default time range for partition pruning

## Task Commits

Each task was committed atomically:

1. **Task 1: Add search and filter query functions to queries.mpl** - `3edabfab` (feat)
2. **Task 2: Create search route handlers and register in main.mpl** - `66b8b970` (feat)

**Plan metadata:** (pending final docs commit)

## Files Created/Modified
- `mesher/storage/queries.mpl` - Extended with list_issues_filtered, search_events_fulltext, filter_events_by_tag, list_events_for_issue
- `mesher/api/search.mpl` - New file with 4 pub handler functions, 15+ helper functions for query extraction, JSON serialization, and pagination
- `mesher/main.mpl` - Updated imports (removed handle_list_issues, added Api.Search), registered 4 new GET routes

## Decisions Made
- Return raw `List<Map<String, String>>` from search queries instead of typed Issue structs -- avoids cross-module struct serialization issues and provides more flexibility for JSON construction
- Use inline `to_tsvector('english', message)` in SQL WHERE clause rather than a stored tsvector column -- avoids schema migration complications with partitioned events table (per research Pitfall 3)
- Construct tag_json from separate key/value query params in the handler (`{"key":"value"}`) rather than accepting raw JSON -- prevents JSONB injection
- Cap pagination limit at 100, default 25, minimum 1 -- prevents unbounded result sets

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered

None.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness
- Search and pagination infrastructure established for Plans 02 (dashboard) and 03 (detail)
- Api module convention (mesher/api/*.mpl) ready for dashboard.mpl and detail.mpl
- Query parameter extraction helpers (query_or_default, get_limit) reusable across new endpoints
- JSON array builder and pagination response builder reusable for all list endpoints

## Self-Check: PASSED

- All 3 artifact files exist on disk
- Both task commits (3edabfab, 66b8b970) verified in git history
- cargo build succeeds with 0 errors

---
*Phase: 91-rest-api*
*Completed: 2026-02-15*
