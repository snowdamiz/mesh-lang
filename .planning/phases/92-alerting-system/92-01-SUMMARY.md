---
phase: 92-alerting-system
plan: 01
subsystem: database
tags: [postgres, alerts, schema, ddl, query-helpers, jsonb]

# Dependency graph
requires:
  - phase: 87-core-storage
    provides: "Pool.query/Pool.execute patterns, PoolHandle type, base schema with alert_rules table"
  - phase: 91-search-dashboard-team
    provides: "queries.mpl with existing query helper patterns"
provides:
  - "alerts table DDL with 9 columns and 3 indexes"
  - "alert_rules cooldown_minutes and last_fired_at columns"
  - "Alert fired record struct with deriving(Json, Row)"
  - "13 alert query helpers: CRUD, evaluation, firing, state transitions, listing"
affects: [92-02-PLAN, 92-03-PLAN]

# Tech tracking
tech-stack:
  added: []
  patterns: ["JSONB extraction for rule creation from request body", "combined threshold+cooldown evaluation in single SQL query", "atomic fire_alert with last_fired_at update"]

key-files:
  created: []
  modified:
    - "mesher/storage/schema.mpl"
    - "mesher/types/alert.mpl"
    - "mesher/storage/queries.mpl"

key-decisions:
  - "8 pre-existing compilation errors (not 7 as plan estimated) -- no new errors introduced"

patterns-established:
  - "Alert query helpers follow existing Pool.query/Pool.execute patterns with ? error propagation"
  - "evaluate_threshold_rule combines event counting and cooldown check in single SQL for atomicity"
  - "fire_alert updates last_fired_at after insert for cooldown tracking"

# Metrics
duration: 2min
completed: 2026-02-15
---

# Phase 92 Plan 01: Alerting Data Foundation Summary

**PostgreSQL alerts table, Alert type, cooldown columns on alert_rules, and 13 query helpers for CRUD, threshold evaluation, alert firing, cooldown management, and state transitions**

## Performance

- **Duration:** 2 min
- **Started:** 2026-02-15T16:53:08Z
- **Completed:** 2026-02-15T16:55:33Z
- **Tasks:** 2
- **Files modified:** 3

## Accomplishments
- alerts table with 9 columns (id, rule_id, project_id, status, message, condition_snapshot, triggered_at, acknowledged_at, resolved_at) and 3 indexes
- alert_rules extended with cooldown_minutes (default 60) and last_fired_at columns via ALTER TABLE
- Alert fired record struct in types/alert.mpl with deriving(Json, Row)
- 13 query helpers covering: rule CRUD (create, list, toggle, delete), threshold evaluation with cooldown, alert firing with last_fired_at update, new issue detection, event-based rule lookup, cooldown-only check, state transitions (acknowledge, resolve), alert listing with status filter, threshold rule loading

## Task Commits

Each task was committed atomically:

1. **Task 1: Extend schema and types for alerting** - `89329640` (feat)
2. **Task 2: Add alert query helpers to queries.mpl** - `4c7eca0b` (feat)

**Plan metadata:** (pending) (docs: complete plan)

## Files Created/Modified
- `mesher/storage/schema.mpl` - Added alerts table DDL, cooldown columns on alert_rules, 3 alerts indexes
- `mesher/types/alert.mpl` - Added Alert fired record struct with 9 String fields
- `mesher/storage/queries.mpl` - Added 13 alert query functions (99 new lines), updated Alert import

## Decisions Made
- 8 pre-existing compilation errors baseline (plan estimated 7) -- no new errors introduced by this plan

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered
None

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- All alert query helpers are ready for import by 92-02 (evaluation engine, HTTP handlers)
- All schema DDL is idempotent (IF NOT EXISTS) and will apply cleanly on first startup
- Alert struct can be imported via `from Types.Alert import Alert` for typed usage

---
*Phase: 92-alerting-system*
*Completed: 2026-02-15*
