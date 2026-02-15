---
phase: 91-rest-api
plan: 03
subsystem: api
tags: [team-management, api-tokens, org-membership, rest-api, postgresql, jsonb-extraction]

# Dependency graph
requires:
  - phase: 91-rest-api
    plan: 02
    provides: Dashboard and event detail endpoints, established Api module convention, handler patterns
  - phase: 87-core-schema
    provides: org_memberships table, api_keys table, users table with bcrypt auth
  - phase: 88-ingestion-pipeline
    provides: PipelineRegistry service pattern, HTTP handler conventions
provides:
  - Team membership listing with enriched user data (email, display_name)
  - Team member addition with role assignment (owner/admin/member)
  - Team member role update with SQL-side validation
  - Team member removal from organization
  - API token listing with revocation status
  - API token creation with auto-generated key
  - API token revocation
affects: [frontend, sdk, auth]

# Tech tracking
tech-stack:
  added: []
  patterns: [postgresql-jsonb-body-parsing, sql-side-role-validation, extract-json-field-helper]

key-files:
  created:
    - mesher/api/team.mpl
  modified:
    - mesher/storage/queries.mpl
    - mesher/main.mpl

key-decisions:
  - "PostgreSQL jsonb extraction for request body parsing (COALESCE($1::jsonb->>$2, '')) consistent with routes.mpl assign_issue pattern"
  - "SQL-side role validation (AND $2 IN ('owner','admin','member')) ensures only valid roles accepted at database level"
  - "API key listing returns revoked_at as null (not empty string) in JSON for clean API contract"
  - "extract_json_field helper reusable across team handlers for body field extraction"

patterns-established:
  - "Body field extraction: extract_json_field(pool, body, field) uses PostgreSQL jsonb to parse request body without Mesh-level JSON parsing"
  - "SQL-side enum validation: WHERE clause with IN(...) rejects invalid values at query level"
  - "Null-to-JSON formatting: revoked_at empty string maps to JSON null for nullable timestamps"

# Metrics
duration: 2min
completed: 2026-02-15
---

# Phase 91 Plan 03: Team Membership and API Token Management Summary

**Team membership CRUD with SQL-validated roles (owner/admin/member) and API token lifecycle (create/list/revoke) using PostgreSQL jsonb body parsing**

## Performance

- **Duration:** 2 min
- **Started:** 2026-02-15T07:11:24Z
- **Completed:** 2026-02-15T07:13:44Z
- **Tasks:** 2
- **Files modified:** 3

## Accomplishments
- 4 new query functions in queries.mpl: update_member_role with SQL-side validation, remove_member, get_members_with_users with user JOIN, list_api_keys with revoked_at handling
- 7 pub handler functions in new api/team.mpl: 4 team membership (list/add/update-role/remove) and 3 API token (list/create/revoke) endpoints
- All 7 routes registered in main.mpl (4 team ORG-04 + 3 token ORG-05), POST for all mutations per decision [89-02]
- PostgreSQL jsonb extraction pattern (extract_json_field) for request body parsing consistent with existing codebase

## Task Commits

Each task was committed atomically:

1. **Task 1: Add team and token management query functions to queries.mpl** - `d09af799` (feat)
2. **Task 2: Create team and token route handlers and register in main.mpl** - `59809084` (feat)

**Plan metadata:** (pending final docs commit)

## Files Created/Modified
- `mesher/storage/queries.mpl` - Extended with 4 new query functions: update_member_role, remove_member, get_members_with_users, list_api_keys
- `mesher/api/team.mpl` - New file with 7 pub handler functions, JSON serializers (member_to_json, api_key_to_json), extract_json_field helper, and case arm extraction helpers
- `mesher/main.mpl` - Updated imports (Api.Team), registered 7 new routes (4 team + 3 token)

## Decisions Made
- PostgreSQL jsonb extraction for body parsing -- `extract_json_field` uses `COALESCE($1::jsonb->>$2, '')` to parse JSON request bodies, consistent with the `handle_assign_issue` pattern in routes.mpl; avoids needing Mesh-level JSON parsing
- SQL-side role validation -- `AND $2 IN ('owner', 'admin', 'member')` in update_member_role query ensures only valid roles are accepted at the database level, returning 0 affected rows for invalid roles
- API key revoked_at formatted as JSON null -- empty string from COALESCE maps to `null` in JSON output (not empty string or missing field) for clean API contract, consistent with format_neighbor_id pattern from detail.mpl
- extract_json_field as reusable helper -- factored out PostgreSQL jsonb field extraction into a shared function used by add_member (user_id, role) and create_api_key (label) handlers

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered

None.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness
- All Phase 91 REST API endpoints complete (search/filter/pagination, dashboard aggregation, event detail, team management, API tokens)
- Complete Mesher REST API: 26+ HTTP endpoints covering event ingestion, issue lifecycle, search, dashboard, event detail, team management, and API token management
- Ready for Phase 92 or subsequent phases

## Self-Check: PASSED

- All 3 artifact files exist on disk (queries.mpl, team.mpl, main.mpl)
- All 2 task commits (d09af799, 59809084) verified in git history
- cargo build succeeds with 0 errors

---
*Phase: 91-rest-api*
*Completed: 2026-02-15*
