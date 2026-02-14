---
phase: 87-foundation
plan: 01
subsystem: database
tags: [postgresql, deriving-json, deriving-row, mesh, data-model, schema, partitioning, pgcrypto, uuidv7]

# Dependency graph
requires: []
provides:
  - "16 Mesher data type structs (Event, Issue, Project, Organization, User, AlertRule, etc.)"
  - "Complete PostgreSQL DDL schema (9 tables, 15 indexes, daily event partitioning)"
  - "18 reusable CRUD query helper functions for all entity types"
  - "Daily event partition management (create_partition, create_partitions_ahead)"
affects: [87-02, 88-ingestion, 89-api, 90-dashboard, 91-search, 92-alerting]

# Tech tracking
tech-stack:
  added: [postgresql-18, pgcrypto, uuidv7, partition-by-range, jsonb, gin-index]
  patterns: [deriving-json-row, pool-query-as, recursive-loop, string-fields-for-row-structs, pg-side-uuid-generation, pg-side-password-hashing]

key-files:
  created:
    - mesher/types/event.mpl
    - mesher/types/issue.mpl
    - mesher/types/project.mpl
    - mesher/types/user.mpl
    - mesher/types/alert.mpl
    - mesher/storage/schema.mpl
    - mesher/storage/queries.mpl
  modified: []

key-decisions:
  - "Row structs use all-String fields for DB text protocol; JSONB parsed with from_json() in separate step"
  - "Recursive helper function instead of while loop for create_partitions_ahead (Mesh has no mutable assignment)"
  - "UUID columns cast to text (::text) in SELECT queries for deriving(Row) compatibility"
  - "User struct excludes password_hash field -- never exposed to application code"
  - "Flat org -> project hierarchy (no teams layer); org_memberships for roles"

patterns-established:
  - "Pool.query_as with Type.from_row for typed database reads"
  - "Pool.query with Map.get for INSERT RETURNING queries"
  - "Pool.execute for DDL and non-returning DML"
  - "case results do [item] -> Ok(item) | _ -> Err('not found') end for single-row queries"
  - "Recursive helper functions for iteration (no mutable variables in Mesh)"
  - "String.from(int) for integer-to-string conversion"
  - "UUID::text and TIMESTAMPTZ::text casts in SELECT for Row struct compatibility"

# Metrics
duration: 5min
completed: 2026-02-14
---

# Phase 87 Plan 01: Data Types & PostgreSQL Schema Summary

**16 Mesh data type structs with deriving(Json, Row) and full PostgreSQL schema (9 tables, 15 indexes, daily event partitioning) with 18 reusable CRUD query functions**

## Performance

- **Duration:** 5 min
- **Started:** 2026-02-14T21:25:18Z
- **Completed:** 2026-02-14T21:30:49Z
- **Tasks:** 2
- **Files created:** 7

## Accomplishments
- Defined all 16 Mesher data types across 5 modules with deriving(Json) and deriving(Json, Row) annotations
- Created idempotent PostgreSQL schema with 9 tables in FK dependency order, PARTITION BY RANGE for events, and 15 indexes including GIN(tags jsonb_path_ops)
- Built 18 reusable query helper functions covering organizations, projects, API keys, users, sessions, and org memberships
- Implemented partition management with recursive create_partitions_ahead that delegates date computation to PostgreSQL

## Task Commits

Each task was committed atomically:

1. **Task 1: Define all Mesher data type structs** - `aeb182c3` (feat)
2. **Task 2: Create PostgreSQL schema DDL and query helpers** - `a4d6b567` (feat)

## Files Created/Modified
- `mesher/types/event.mpl` - Event, EventPayload, Severity, StackFrame, ExceptionInfo, Breadcrumb types
- `mesher/types/issue.mpl` - Issue, IssueStatus types
- `mesher/types/project.mpl` - Organization, Project, ApiKey types
- `mesher/types/user.mpl` - User, OrgMembership, Session types
- `mesher/types/alert.mpl` - AlertRule, AlertCondition types
- `mesher/storage/schema.mpl` - create_schema (all DDL), create_partition, create_partitions_ahead
- `mesher/storage/queries.mpl` - 18 CRUD functions with imports from all type modules

## Decisions Made
- **Row struct String fields:** All Row structs use String fields because deriving(Row) maps through Map<String, String> text protocol. JSONB columns arrive as JSON strings that must be parsed with from_json() separately.
- **Recursive iteration:** Used recursive helper function for create_partitions_ahead instead of while loop because Mesh has no mutable variable assignment.
- **UUID text casting:** All UUID columns use `::text` cast in SELECT queries so deriving(Row) can map them to String fields.
- **User struct security:** User struct intentionally excludes password_hash -- the hash lives only in PostgreSQL and is never loaded into application memory.
- **Flat hierarchy:** Org -> Project without intermediate teams layer, using org_memberships for role-based access (owner/admin/member).

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Fixed create_partitions_ahead to use recursion instead of while loop**
- **Found during:** Task 2 (Create PostgreSQL schema DDL and query helpers)
- **Issue:** Plan specified `while i < days do ... i = i + 1 end` pattern, but Mesh has no mutable variable reassignment (verified via while_loop.mpl test: "Mesh has no mutable assignment")
- **Fix:** Replaced while loop with recursive helper function `create_partitions_loop(pool, days, i)` that increments i via parameter passing
- **Files modified:** mesher/storage/schema.mpl
- **Verification:** Function structure follows Mesh's immutable variable semantics
- **Committed in:** a4d6b567 (Task 2 commit)

**2. [Rule 1 - Bug] Fixed String.from_int to String.from**
- **Found during:** Task 2 (Create PostgreSQL schema DDL and query helpers)
- **Issue:** Plan referenced `String.from_int(i)` but the actual Mesh API is `String.from(i)` using the From trait (verified via tests/e2e/from_string_from_int.mpl)
- **Fix:** Used `String.from(i)` instead of `String.from_int(i)`
- **Files modified:** mesher/storage/schema.mpl
- **Verification:** Matches the From<Int> for String impl in builtins.rs
- **Committed in:** a4d6b567 (Task 2 commit)

---

**Total deviations:** 2 auto-fixed (2 bugs -- plan specified incorrect Mesh syntax)
**Impact on plan:** Both auto-fixes necessary for correctness. The plan's intent was correct but used non-existent Mesh language features. No scope creep.

## Issues Encountered
None -- both deviations were caught during code authoring (not runtime errors).

## User Setup Required

**External services require manual configuration.** The plan specifies PostgreSQL 18+ is needed:
- Install PostgreSQL 18+ locally
- Create database `mesher` and enable pgcrypto extension
- Set `DATABASE_URL` environment variable (e.g., `postgres://mesh:mesh@localhost:5432/mesher`)

## Next Phase Readiness
- All data types defined and ready for import by downstream modules
- Schema DDL ready to execute against a PostgreSQL 18+ database
- Query helpers ready for use by service layer (Plan 02: StorageWriter service)
- Partitioning infrastructure ready for PartitionManager service

## Self-Check: PASSED

All 7 created files verified on disk. Both task commits (aeb182c3, a4d6b567) verified in git log.

---
*Phase: 87-foundation*
*Completed: 2026-02-14*
