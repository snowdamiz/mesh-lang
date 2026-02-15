---
phase: 88-ingestion-pipeline
plan: 02
subsystem: api
tags: [mesh, services, rate-limiter, event-processor, auth, validation, ingestion]

# Dependency graph
requires:
  - phase: 87.2-refactor-phase-87-code-to-use-cross-module-services
    provides: "Cross-module service pattern: services/X.mpl with from Services.X import"
  - phase: 88-01
    provides: "MeshHttpResponse with headers field for 429 Retry-After responses"
provides:
  - "RateLimiter service with per-project rate counting (CheckLimit call, ResetWindow cast, rate_window_ticker actor)"
  - "EventProcessor service with ProcessEvent call that routes pre-validated events to StorageWriter"
  - "Auth functions (extract_api_key, authenticate_request) for DSN-style API key extraction"
  - "Validation functions (validate_event, validate_payload_size, validate_bulk_count) for event payload checking"
affects: [88-03, ingestion-pipeline, http-routes, websocket-handler]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Helper function extraction for single-expression case arm limitation in Mesh parser"
    - "Let-binding for function call conditions in if/do/else/end blocks (parser limitation workaround)"
    - "Inferred return type for service call handlers returning Result (explicit :: Result annotation not supported)"
    - "Caller-side validation pattern when cross-module from_json is unavailable"

key-files:
  created:
    - "mesher/services/rate_limiter.mpl"
    - "mesher/services/event_processor.mpl"
    - "mesher/ingestion/auth.mpl"
    - "mesher/ingestion/validation.mpl"
  modified: []

key-decisions:
  - "Map.get returns 0 for missing keys -- no need for Map.get_or helper when tracking Int counters"
  - "EventProcessor delegates validation to caller (HTTP handler) due to cross-module from_json not resolving"
  - "Service call handlers use inferred return types instead of explicit :: Result annotation"
  - "Nested if/else avoided by extracting to helper functions (parser limitation: else keyword terminates block body)"

patterns-established:
  - "Single-expression case arms: extract multi-line logic into helper functions"
  - "If-condition with function calls: bind result to variable first, then use variable in if condition"
  - "Service call handlers returning Result: omit :: Type annotation, let type inference handle it"

# Metrics
duration: 9min
completed: 2026-02-15
---

# Phase 88 Plan 02: Ingestion Services Summary

**RateLimiter service with per-project rate counting, EventProcessor service routing to StorageWriter, auth helpers for API key extraction, and event validation functions -- all in Mesh**

## Performance

- **Duration:** 9 min
- **Started:** 2026-02-15T01:24:35Z
- **Completed:** 2026-02-15T01:33:51Z
- **Tasks:** 2
- **Files created:** 4

## Accomplishments
- RateLimiter service with CheckLimit (sync call), ResetWindow (async cast), and rate_window_ticker actor for periodic window reset
- EventProcessor service with ProcessEvent call handler that routes pre-validated events to StorageWriter
- Auth module with extract_api_key (X-Sentry-Auth / Authorization header fallback) and authenticate_request (DB lookup via get_project_by_api_key)
- Validation module with validate_event (message + severity level checks), validate_payload_size, validate_bulk_count
- New mesher/ingestion/ directory namespace established (Ingestion.Auth, Ingestion.Validation)
- All 4 files compile as part of `meshc build mesher/`

## Task Commits

Each task was committed atomically:

1. **Task 1: Create RateLimiter service and auth/validation modules** - `e313a8b0` (feat)
2. **Task 2: Create EventProcessor service** - `4bd89334` (feat)

## Files Created/Modified
- `mesher/services/rate_limiter.mpl` - RateLimiter service with per-project rate counting, CheckLimit call, ResetWindow cast, rate_window_ticker actor
- `mesher/services/event_processor.mpl` - EventProcessor service with ProcessEvent call handler routing to StorageWriter
- `mesher/ingestion/auth.mpl` - API key extraction from request headers and project authentication via DB lookup
- `mesher/ingestion/validation.mpl` - Event payload validation: required fields, severity levels, payload size, bulk count

## Decisions Made
- **Map.get returns 0 for missing keys:** The Mesh runtime's mesh_map_get returns 0 for missing keys. Since rate limiter tracks Int counters, this is the correct default -- no Map.get_or helper needed.
- **Caller-side validation:** EventProcessor does not call from_json or validate_event internally. Cross-module EventPayload.from_json resolution fails (type checker reports "no trait impl providing from_json" for cross-module types). The HTTP handler (Plan 03) will parse JSON, validate, and pass pre-validated event_json to EventProcessor.
- **Inferred call return type:** Service call handlers with `:: String!String` return type annotation fail type checking ("expected Result, found String"). Using inferred types (no `::` annotation) works correctly, matching the UserService pattern from Phase 87.2.
- **Helper function extraction:** Mesh parser only supports single-expression case arms. Multi-line logic in case arms must be extracted into separate helper functions.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug workaround] Function call in if-condition causes parse failure**
- **Found during:** Task 1 (validation.mpl)
- **Issue:** `if List.contains(valid_levels, level) do ... else ... end` fails to parse. The parser mishandles the closing `)` of the function call before `do`.
- **Fix:** Bind function call result to variable first: `let is_valid = List.contains(valid_levels, level)` then `if is_valid do`
- **Files modified:** mesher/ingestion/validation.mpl
- **Verification:** meshc build mesher/ succeeds
- **Committed in:** e313a8b0 (Task 1 commit)

**2. [Rule 1 - Bug workaround] Single-expression case arm limitation**
- **Found during:** Task 1 (auth.mpl)
- **Issue:** Case arms only support single expressions. `None ->` followed by `let` + `case` block fails with "expected expression".
- **Fix:** Extracted fallback logic to `try_authorization_header` helper function called from the case arm.
- **Files modified:** mesher/ingestion/auth.mpl
- **Verification:** meshc build mesher/ succeeds
- **Committed in:** e313a8b0 (Task 1 commit)

**3. [Rule 1 - Bug workaround] Nested if/else fails inside else block**
- **Found during:** Task 1 (validation.mpl)
- **Issue:** parse_block_body stops at ELSE_KW, so inner `if/else` inside outer `else` block is consumed by outer block's terminator.
- **Fix:** Extracted inner level validation to `validate_level` helper function.
- **Files modified:** mesher/ingestion/validation.mpl
- **Verification:** meshc build mesher/ succeeds
- **Committed in:** e313a8b0 (Task 1 commit)

**4. [Rule 3 - Blocking] Cross-module from_json not resolving**
- **Found during:** Task 2 (event_processor.mpl)
- **Issue:** `EventPayload.from_json(event_json)` fails with "type Event.EventPayload has no trait impl providing from_json" when called cross-module.
- **Fix:** Restructured EventProcessor to accept pre-validated events. Validation responsibility moved to caller (HTTP handler in Plan 03).
- **Files modified:** mesher/services/event_processor.mpl
- **Verification:** meshc build mesher/ succeeds
- **Committed in:** 4bd89334 (Task 2 commit)

**5. [Rule 3 - Blocking] Explicit Result type annotation on service call handler fails**
- **Found during:** Task 2 (event_processor.mpl)
- **Issue:** `call ProcessEvent(...) :: String!String do |state|` fails type checking. Service call handlers don't support explicit Result type annotations.
- **Fix:** Removed `:: String!String` annotation; used inferred type (matching UserService pattern).
- **Files modified:** mesher/services/event_processor.mpl
- **Verification:** meshc build mesher/ succeeds
- **Committed in:** 4bd89334 (Task 2 commit)

---

**Total deviations:** 5 auto-fixed (3 bug workarounds, 2 blocking)
**Impact on plan:** All workarounds necessary for compilation. Parser limitations require specific coding patterns. Cross-module from_json limitation shifts validation to caller but preserves all functionality. No scope creep.

## Issues Encountered
- Mesh parser limitations required significant restructuring: single-expression case arms, function-call-in-if-condition parse failure, nested if/else block termination. All resolved by extracting logic into helper functions.
- Cross-module from_json resolution failure is a known limitation of the Mesh type checker for imported types. Worked around by moving validation to the call site.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness
- All 4 ingestion building blocks ready for Plan 03 (HTTP routes and WebSocket handler)
- Auth functions can be composed into middleware or called inline by route handlers
- Validation functions available for pre-validation before EventProcessor.ProcessEvent
- RateLimiter service ready to be started and queried per-request
- EventProcessor service ready to receive pre-validated events and route to StorageWriter
- Pattern note for Plan 03: use inferred return types for call handlers, bind function calls to variables before if-conditions, extract multi-line case arm logic to helpers

## Self-Check: PASSED

- [x] mesher/services/rate_limiter.mpl exists and contains service RateLimiter
- [x] mesher/services/event_processor.mpl exists and contains service EventProcessor
- [x] mesher/ingestion/auth.mpl exists and contains pub fn extract_api_key
- [x] mesher/ingestion/validation.mpl exists and contains pub fn validate_event
- [x] Commit e313a8b0 exists (Task 1)
- [x] Commit 4bd89334 exists (Task 2)
- [x] meshc build mesher/ compiles successfully

---
*Phase: 88-ingestion-pipeline*
*Completed: 2026-02-15*
