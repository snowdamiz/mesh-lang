---
phase: 102-mesher-rewrite
plan: 01
subsystem: database
tags: [schema, deriving, migration, orm, postgresql]

# Dependency graph
requires:
  - phase: 96-compiler-additions
    provides: "deriving(Schema) codegen infrastructure, atoms, keyword args"
  - phase: 97-schema-metadata-sql-generation
    provides: "Schema options (table, primary_key, timestamps), SQL type mapping"
  - phase: 100-relationships-preloading
    provides: "Relationship declarations (belongs_to, has_many, has_one) and __relationship_meta__()"
  - phase: 101-migration-system
    provides: "Migration DSL (create_table, drop_table, create_index) and runner CLI"
provides:
  - "11 Schema-annotated type structs with table names, primary keys, and relationships"
  - "RetentionSettings virtual projection struct (table 'projects')"
  - "Initial migration file creating all 10 tables, 19 indexes, and pgcrypto extension"
  - "Schema DDL removed from storage/schema.mpl (partition functions retained)"
  - "main.mpl no longer runs imperative DDL on startup"
affects: [102-02, 102-03, mesher-rewrite]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "deriving(Schema, Json, Row) triple-derive for ORM-backed database structs"
    - "Virtual projection structs mapping to existing tables for subset queries"
    - "Migration files using mix of Migration DSL and raw Pool.execute for complex DDL"

key-files:
  created:
    - mesher/types/retention.mpl
    - mesher/migrations/20260216120000_create_initial_schema.mpl
  modified:
    - mesher/types/project.mpl
    - mesher/types/user.mpl
    - mesher/types/event.mpl
    - mesher/types/issue.mpl
    - mesher/types/alert.mpl
    - mesher/storage/schema.mpl
    - mesher/main.mpl

key-decisions:
  - "102-01: Raw Pool.execute used for tables with composite UNIQUE constraints (org_memberships, issues) since Migration.create_table column spec format does not support table-level constraints"
  - "102-01: All indexes use raw Pool.execute for exact name control and support of partial/GIN/DESC indexes"
  - "102-01: Blank lines between consecutive Pool.execute calls in migration file to prevent Mesh parser multi-line continuation interference"
  - "102-01: Migration.create_index takes 4 args (pool, table, columns, options) not 5 -- index name is auto-generated"

patterns-established:
  - "Migration files can mix Migration DSL calls with raw Pool.execute for unsupported SQL features"
  - "Blank line separation required between consecutive expression statements in Mesh to prevent multi-line continuation"

# Metrics
duration: 6min
completed: 2026-02-16
---

# Phase 102 Plan 01: Schema Struct Conversion + Initial Migration Summary

**11 Mesher type structs annotated with deriving(Schema, Json, Row) plus initial migration file replacing imperative DDL with versioned schema management**

## Performance

- **Duration:** 6 min
- **Started:** 2026-02-16T23:02:28Z
- **Completed:** 2026-02-16T23:09:09Z
- **Tasks:** 2
- **Files modified:** 9

## Accomplishments
- All 11 type structs (Organization, Project, ApiKey, User, OrgMembership, Session, Event, Issue, AlertRule, Alert, RetentionSettings) now have deriving(Schema, Json, Row) with correct schema metadata
- Session struct correctly uses primary_key :token (non-standard PK)
- New RetentionSettings struct created as virtual projection of projects table
- Complete initial migration file with 10 tables, pgcrypto extension, and 19 indexes in FK dependency order
- Removed imperative create_schema from storage/schema.mpl and main.mpl startup flow
- Mesher project compiles without errors

## Task Commits

Each task was committed atomically:

1. **Task 1: Add deriving(Schema) to all 11 type structs** - `c523a0db` (feat)
2. **Task 2: Create initial migration file and update main.mpl** - `1c18a36e` (feat)

## Files Created/Modified
- `mesher/types/project.mpl` - Organization, Project, ApiKey with Schema annotations and relationships
- `mesher/types/user.mpl` - User, OrgMembership, Session with Schema annotations (Session uses primary_key :token)
- `mesher/types/event.mpl` - Event with Schema annotations and belongs_to relationships
- `mesher/types/issue.mpl` - Issue with Schema annotations and belongs_to/has_many relationships
- `mesher/types/alert.mpl` - AlertRule, Alert with Schema annotations and relationships
- `mesher/types/retention.mpl` - New RetentionSettings struct with table "projects"
- `mesher/migrations/20260216120000_create_initial_schema.mpl` - Initial migration with up/down for all tables and indexes
- `mesher/storage/schema.mpl` - Removed create_schema, retained partition functions
- `mesher/main.mpl` - Removed create_schema import and call

## Decisions Made
- Used raw Pool.execute for org_memberships and issues tables due to composite UNIQUE constraints not being expressible in Migration.create_table column spec format
- All indexes use raw Pool.execute for exact name control and to support partial indexes, GIN indexes, and DESC ordering
- Discovered Migration.create_index takes 4 arguments (pool, table, columns, options) with auto-generated index names, not 5 arguments as assumed in the plan
- Required blank line separation between consecutive expression statements in migration file to prevent Mesh parser multi-line continuation parsing

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Fixed Migration.create_index argument count**
- **Found during:** Task 2 (migration file creation)
- **Issue:** Plan specified Migration.create_index(pool, table, name, columns, options) with 5 args, but actual signature is (pool, table, columns, options) with 4 args (name auto-generated)
- **Fix:** Switched all index creation to raw Pool.execute for exact index name control
- **Files modified:** mesher/migrations/20260216120000_create_initial_schema.mpl
- **Verification:** Mesher project compiles without errors
- **Committed in:** 1c18a36e (Task 2 commit)

**2. [Rule 3 - Blocking] Fixed multi-line continuation parsing in migration file**
- **Found during:** Task 2 (migration file creation)
- **Issue:** Consecutive Pool.execute calls without blank lines caused Mesh parser to treat them as multi-line continuations, producing "expected 4 argument(s), found 5" errors
- **Fix:** Added blank lines between every consecutive Pool.execute call
- **Files modified:** mesher/migrations/20260216120000_create_initial_schema.mpl
- **Verification:** Mesher project compiles without errors
- **Committed in:** 1c18a36e (Task 2 commit)

**3. [Rule 1 - Bug] Fixed nullable column spec format in Migration.create_table**
- **Found during:** Task 2 (migration file creation)
- **Issue:** Nullable columns with trailing colon (e.g., "platform:TEXT:") produced malformed SQL with trailing space
- **Fix:** Used 2-part format without trailing colon for nullable columns (e.g., "platform:TEXT")
- **Files modified:** mesher/migrations/20260216120000_create_initial_schema.mpl
- **Verification:** Mesher project compiles without errors
- **Committed in:** 1c18a36e (Task 2 commit)

---

**Total deviations:** 3 auto-fixed (2 bugs, 1 blocking)
**Impact on plan:** All auto-fixes necessary for correct compilation. No scope creep.

## Issues Encountered
- Migration.create_table column spec format does not support table-level constraints like UNIQUE(col1, col2). Used raw Pool.execute as fallback for affected tables.
- Mesh parser multi-line continuation can span consecutive expression statements. Blank line separation is the solution.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- All 11 Schema-annotated structs are ready for ORM query conversion (Plan 102-02/03)
- Schema metadata functions (__table__, __fields__, __primary_key__, __relationships__) available for Repo/Query calls
- Migration file ready for `meshc migrate up` execution
- Partition functions retained in storage/schema.mpl for runtime use

## Self-Check: PASSED

All 9 files verified present. Both task commits (c523a0db, 1c18a36e) verified in git log.

---
*Phase: 102-mesher-rewrite*
*Completed: 2026-02-16*
