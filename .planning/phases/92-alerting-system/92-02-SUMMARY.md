---
phase: 92-alerting-system
plan: 02
subsystem: api
tags: [alerts, websocket, actors, threshold, evaluation, cooldown, timer]

# Dependency graph
requires:
  - phase: 92-alerting-system-01
    provides: "alert query helpers (get_threshold_rules, evaluate_threshold_rule, fire_alert, check_new_issue, get_event_alert_rules, should_fire_by_cooldown)"
provides:
  - "Timer-driven alert_evaluator actor (30s interval) for threshold-based alert evaluation"
  - "Event-based alert triggers (new issue detection) in ingestion routes"
  - "WebSocket alert broadcast via Ws.broadcast for both evaluation paths"
  - "Cooldown enforcement on both timer-based and event-based alert firing"
affects: [92-03-PLAN]

# Tech tracking
tech-stack:
  added: []
  patterns: ["PostgreSQL JSONB field extraction via extract_condition_field for condition_json parsing", "Recursive rules loop with index-based iteration for alert evaluation", "fire_and_broadcast pattern combining fire_alert + Ws.broadcast"]

key-files:
  created: []
  modified:
    - "mesher/ingestion/pipeline.mpl"
    - "mesher/ingestion/routes.mpl"

key-decisions:
  - "Moved restart_all_services after alert_evaluator actor for define-before-use compliance"

patterns-established:
  - "Alert evaluation chain: extract_condition_field -> evaluate_threshold -> fire_and_broadcast for threshold rules"
  - "Event-based alert pattern: check_event_alerts -> handle_new_issue_alert -> fire_matching_event_alerts for inline triggers"

# Metrics
duration: 4min
completed: 2026-02-15
---

# Phase 92 Plan 02: Alert Evaluation Engine Summary

**Timer-driven threshold evaluator actor (30s interval) and inline event-based new-issue alert triggers with cooldown enforcement and WebSocket broadcast**

## Performance

- **Duration:** 4 min
- **Started:** 2026-02-15T16:57:56Z
- **Completed:** 2026-02-15T17:02:11Z
- **Tasks:** 2
- **Files modified:** 2

## Accomplishments
- Timer-driven alert_evaluator actor runs every 30 seconds, loads all enabled threshold rules, evaluates each against event counts with cooldown, fires and broadcasts alerts when conditions met
- Event-based alert checking runs inline after every event processing in broadcast_event, detecting new issues and firing matching new_issue alert rules with cooldown enforcement
- Both evaluation paths broadcast alert notifications via WebSocket to project rooms
- Spawned alert_evaluator in both start_pipeline and restart_all_services for fault tolerance

## Task Commits

Each task was committed atomically:

1. **Task 1: Alert evaluator actor in pipeline.mpl** - `f15f6fb4` (feat)
2. **Task 2: Event-based alert triggers in routes.mpl** - `fd1ae4d0` (feat)

**Plan metadata:** (pending) (docs: complete plan)

## Files Created/Modified
- `mesher/ingestion/pipeline.mpl` - Added alert_evaluator actor, threshold evaluation helpers (broadcast_alert, fire_and_broadcast, extract_condition_field, evaluate chain), spawned in start_pipeline and restart_all_services, moved restart_all_services for define-before-use
- `mesher/ingestion/routes.mpl` - Added event-based alert helpers (broadcast_alert_notification, fire_if_cooldown_ok, fire_event_alert, fire_event_alerts_loop, fire_matching_event_alerts, handle_new_issue_alert, check_event_alerts), modified broadcast_event to call check_event_alerts

## Decisions Made
- Moved restart_all_services after alert_evaluator actor definition to satisfy define-before-use constraint (decision [90-03])

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Moved restart_all_services for define-before-use compliance**
- **Found during:** Task 1 (alert evaluator actor in pipeline.mpl)
- **Issue:** restart_all_services was defined before alert_evaluator, but spawning alert_evaluator requires it to be defined first (Mesh define-before-use constraint)
- **Fix:** Relocated restart_all_services from before health_checker to after alert_evaluator actor definition
- **Files modified:** mesher/ingestion/pipeline.mpl
- **Verification:** Build passes with 8 pre-existing errors (no new errors)
- **Committed in:** f15f6fb4 (Task 1 commit)

---

**Total deviations:** 1 auto-fixed (1 blocking)
**Impact on plan:** Necessary reordering for language constraint compliance. No scope creep.

## Issues Encountered
None

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Alert evaluation engine is fully operational for both timer-based and event-based triggers
- Ready for 92-03 (HTTP API endpoints for alert rule and alert management)
- ALERT-02 (threshold), ALERT-03 (new issue), ALERT-04 (WebSocket broadcast), and ALERT-05 (cooldown) are all implemented

## Self-Check: PASSED

- FOUND: mesher/ingestion/pipeline.mpl
- FOUND: mesher/ingestion/routes.mpl
- FOUND: 92-02-SUMMARY.md
- FOUND: f15f6fb4 (Task 1 commit)
- FOUND: fd1ae4d0 (Task 2 commit)

---
*Phase: 92-alerting-system*
*Completed: 2026-02-15*
