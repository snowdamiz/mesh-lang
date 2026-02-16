---
status: resolved
trigger: "mesher-sigbus-persistent: SIGBUS crash on dashboard HTTP requests, 3 previous fix attempts failed"
created: 2026-02-15T00:00:00Z
updated: 2026-02-15T00:03:00Z
---

## Current Focus

hypothesis: CONFIRMED - Root cause was frontend sending "default" slug as project_id, backend passing directly to SQL with ::uuid cast
test: Rebuilt compiler+mesher, started mesher, curled all 4 dashboard endpoints
expecting: Valid JSON responses, no crash
next_action: Archive and commit

## Symptoms

expected: Mesher handles HTTP requests to dashboard endpoints and returns JSON responses
actual: Mesher crashes with "Bus error: 10" (exit code 138) when dashboard page is loaded
errors: "sh: line 1: 43515 Bus error: 10           ./mesher"
reproduction: Run mesher, curl http://localhost:8080/api/v1/projects/default/dashboard/health
started: Persistent across 3 sessions. Two codegen fixes applied but crash continues.

## Eliminated

- hypothesis: SIGBUS crash on dashboard endpoints with current build
  evidence: Built fresh (cargo build --release + meshc build mesher/), started mesher, curled all 4 dashboard endpoints with "default" slug. All returned JSON error responses (SQL uuid parse error), mesher stayed running. No SIGBUS.
  timestamp: 2026-02-15T00:01:00Z

## Evidence

- timestamp: 2026-02-15T00:01:00Z
  checked: All 4 dashboard endpoints with "default" slug (pre-fix)
  found: Mesher returns {"error":"invalid input syntax for type uuid: \"default\""} for all endpoints. Mesher stays running (PID alive after all curls).
  implication: Previous codegen fixes DID resolve the SIGBUS. The remaining bug is functional.

- timestamp: 2026-02-15T00:02:00Z
  checked: Frontend project-store.ts and database schema
  found: Frontend hardcodes activeProjectId="default". Projects table has no slug column. All SQL queries use $1::uuid cast. DB has one project with UUID 00000000-0000-0000-0000-000000000002.
  implication: Need to add slug column + resolver so "default" maps to actual UUID.

- timestamp: 2026-02-15T00:03:00Z
  checked: All 4 dashboard endpoints after fix
  found: Health returns {"unresolved_count":0,"events_24h":0,"new_today":0}. Levels, Volume, Top Issues return []. UUID direct access also works. Mesher stays running.
  implication: Fix verified. Both slug and UUID access paths work.

## Resolution

root_cause: Two-part issue. (1) Previous codegen fixes already resolved the SIGBUS crash. (2) The remaining functional bug was that the frontend project-store.ts hardcodes activeProjectId="default" (a slug, not a UUID), but all backend API handlers pass this directly to SQL queries using $1::uuid cast, causing PostgreSQL error "invalid input syntax for type uuid".

fix: Added slug-to-UUID resolution:
  1. Schema migration: added `slug TEXT` column to projects table with unique index
  2. Seed migration: sets slug='default' on the first project
  3. New query: `get_project_id_by_slug(pool, slug)` in storage/queries.mpl
  4. New helper: `resolve_project_id(pool, raw_id)` in api/helpers.mpl - if input is 36 chars (UUID), pass through; otherwise resolve as slug
  5. Updated all 16 handlers across dashboard.mpl, search.mpl, settings.mpl, team.mpl, alerts.mpl to use resolve_project_id

verification: Rebuilt compiler and mesher. Started mesher. All 4 dashboard endpoints with "default" slug return valid JSON. UUID direct access also works. Mesher stays running after all requests.

files_changed:
  - mesher/storage/schema.mpl (add slug column + index + seed)
  - mesher/storage/queries.mpl (add get_project_id_by_slug)
  - mesher/api/helpers.mpl (add resolve_project_id)
  - mesher/api/dashboard.mpl (use resolve_project_id in 5 handlers)
  - mesher/api/search.mpl (use resolve_project_id in 3 handlers)
  - mesher/api/settings.mpl (use resolve_project_id in 3 handlers)
  - mesher/api/team.mpl (use resolve_project_id in 2 handlers)
  - mesher/api/alerts.mpl (use resolve_project_id in 3 handlers)
