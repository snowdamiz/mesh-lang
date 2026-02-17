# Phase 102: Mesher Rewrite - Research

**Researched:** 2026-02-16
**Domain:** Mesher application database layer rewrite using Mesh ORM (deriving(Schema), Query, Repo, Changeset, Migration DSL)
**Confidence:** HIGH

## Summary

Phase 102 rewrites Mesher's entire database layer -- three storage files (schema.mpl, queries.mpl, writer.mpl) totaling ~750 lines of raw SQL and imperative DDL -- to use the ORM infrastructure built in Phases 96-101. The rewrite touches 11 schema types across 4 type files, replaces 62 raw Pool.query/Pool.execute call sites across 8 modules, and converts 82 lines of imperative DDL into versioned migration files. Additionally, 5 non-storage modules (ingestion/pipeline.mpl, ingestion/routes.mpl, ingestion/ws_handler.mpl, api/team.mpl, api/alerts.mpl) contain direct Pool.query calls that must also be converted.

The rewrite decomposes into four distinct workstreams: (1) converting all 11 type structs to `deriving(Schema)` with correct schema options, relationships, and field types, (2) creating versioned migration files using the Migration DSL to replace storage/schema.mpl's 82 lines of imperative DDL, (3) replacing storage/queries.mpl's 627 lines of raw SQL with Repo/Query ORM calls plus Changeset validation where appropriate, and (4) updating all service and API modules that directly call Pool.query/Pool.execute to use the new ORM-backed query functions instead.

**Primary recommendation:** Implement in five plans: (1) Schema struct conversion for all 11 types, (2) migration file generation from existing DDL, (3) core CRUD query replacement in storage/queries.mpl, (4) complex query and analytics query replacement, (5) service/API module migration and end-to-end verification.

## Standard Stack

### Core

| Crate | Location | Purpose | Relevance |
|-------|----------|---------|-----------|
| mesh-rt | crates/mesh-rt/src/db/ | ORM runtime (query.rs, repo.rs, changeset.rs, migration.rs, orm.rs) | All ORM functions the rewrite depends on |
| mesh-codegen | crates/mesh-codegen | deriving(Schema) codegen, MIR lowering | Schema metadata generation for all 11 types |
| mesh-typeck | crates/mesh-typeck | Type inference | Schema/Query/Repo/Changeset module type checking |
| meshc | crates/meshc | Compiler + CLI | `meshc migrate` for migration execution |

### Mesher Application Files

| File | Lines | Purpose | Rewrite Scope |
|------|-------|---------|---------------|
| mesher/storage/schema.mpl | 82 | Imperative DDL (CREATE TABLE, CREATE INDEX, ALTER TABLE) | Replace entirely with migration files |
| mesher/storage/queries.mpl | 627 | All CRUD + analytics queries via raw SQL | Replace with Repo/Query ORM calls |
| mesher/storage/writer.mpl | 24 | Batch event writer actor | Replace Pool.execute with Repo.insert |
| mesher/types/project.mpl | 75 | Organization, User, OrgMembership, Session, Project, ApiKey structs | Add deriving(Schema), schema options, relationships |
| mesher/types/event.mpl | 52 | EventPayload, StackFrame, ExceptionInfo, Event structs | Add deriving(Schema) to Event; payload types stay as-is |
| mesher/types/issue.mpl | 22 | Issue struct | Add deriving(Schema) |
| mesher/types/alert.mpl | 23 | AlertRule, Alert structs | Add deriving(Schema) |
| mesher/services/org.mpl | 71 | OrgService actor | Replace direct query calls with ORM-backed functions |
| mesher/services/project.mpl | 83 | ProjectService actor | Replace direct query calls |
| mesher/services/user.mpl | 90 | UserService actor | Replace direct query calls |
| mesher/services/event_processor.mpl | 103 | EventProcessor actor | Replace direct query calls |
| mesher/services/retention.mpl | 75 | RetentionService actor | Replace direct query calls |
| mesher/services/writer.mpl | 72 | WriterService actor | Replace Pool.execute with Repo |
| mesher/ingestion/pipeline.mpl | 170 | PipelineRegistry actor + alert evaluation | Replace Pool.query with Repo |
| mesher/ingestion/routes.mpl | 416 | HTTP route handlers for REST API | Replace Pool.query with Repo |
| mesher/ingestion/ws_handler.mpl | 147 | WebSocket handlers | Replace Pool.query with Repo |
| mesher/api/team.mpl | 52 | Team management endpoints | Replace Pool.query with Repo |
| mesher/api/alerts.mpl | 98 | Alert management endpoints | Replace Pool.query with Repo |
| mesher/api/settings.mpl | 56 | Retention/sampling settings endpoints | Replace raw queries |
| mesher/api/dashboard.mpl | 57 | Dashboard analytics endpoints | Replace raw queries |
| mesher/api/detail.mpl | 65 | Issue detail + event detail endpoints | Replace raw queries |
| mesher/api/search.mpl | 60 | Search + tag filtering endpoints | Replace raw queries |

## Architecture Patterns

### Pattern 1: Current Mesher Type Struct Layout (Pre-Rewrite)

All 11 type structs currently use `deriving(Json, Row)`. They do NOT have `deriving(Schema)`. Here is the complete inventory:

**File: mesher/types/project.mpl (6 structs)**
```
struct Organization do
  id :: String
  name :: String
  slug :: String
  created_at :: String
end deriving(Json, Row)

struct User do
  id :: String
  email :: String
  password_hash :: String       # <-- sensitive, must NOT be in Schema fields for reads
  display_name :: String
  created_at :: String
end deriving(Json, Row)

struct OrgMembership do
  id :: String
  user_id :: String
  org_id :: String
  role :: String
  joined_at :: String
end deriving(Json, Row)

struct Session do
  token :: String               # <-- PK is token, not id
  user_id :: String
  created_at :: String
  expires_at :: String
end deriving(Json, Row)

struct Project do
  id :: String
  org_id :: String
  name :: String
  platform :: String
  created_at :: String
end deriving(Json, Row)

struct ApiKey do
  id :: String
  project_id :: String
  key_value :: String
  label :: String
  created_at :: String
  revoked_at :: String
end deriving(Json, Row)
```

**File: mesher/types/event.mpl (4 structs, only Event needs Schema)**
```
struct EventPayload do ... end deriving(Json)        # Input DTO, not persisted
struct StackFrame do ... end deriving(Json)           # Nested type, not a table
struct ExceptionInfo do ... end deriving(Json)        # Nested type, not a table

struct Event do
  id :: String
  project_id :: String
  issue_id :: String
  fingerprint :: String
  title :: String
  message :: String
  level :: String
  platform :: String
  received_at :: String
  payload :: String
end deriving(Json, Row)
```

**File: mesher/types/issue.mpl (1 struct)**
```
struct Issue do
  id :: String
  project_id :: String
  fingerprint :: String
  title :: String
  level :: String
  status :: String
  event_count :: String
  first_seen :: String
  last_seen :: String
  assigned_to :: String
end deriving(Json, Row)
```

**File: mesher/types/alert.mpl (2 structs)**
```
struct AlertRule do
  id :: String
  project_id :: String
  name :: String
  condition_json :: String
  action_json :: String
  enabled :: String
  created_at :: String
end deriving(Json, Row)

struct Alert do
  id :: String
  rule_id :: String
  project_id :: String
  status :: String
  message :: String
  condition_snapshot :: String
  triggered_at :: String
  acknowledged_at :: String
  resolved_at :: String
end deriving(Json, Row)
```

**Missing type: RetentionSettings**

RetentionSettings is listed in the success criteria but does NOT exist as a struct today. Retention data (retention_days, sample_rate) is stored as columns on the `projects` table, queried via `SELECT retention_days::text, sample_rate::text FROM projects WHERE id = $1`. The rewrite must create this as a new type:

```
struct RetentionSettings do
  retention_days :: String
  sample_rate :: String
end deriving(Json, Row, Schema)
```

However, RetentionSettings is NOT a separate database table -- it is a virtual/projection type representing a subset of Project columns. Two options exist:
- Option A: Create it as a Schema struct with `table "projects"` (pointing to the same table as Project)
- Option B: Keep it as a non-Schema value type (just Json, Row) and query via raw SQL or a custom Repo pattern
- **Recommendation:** Option A with `table "projects"` since the success criteria explicitly requires `deriving(Schema)` on all 11 types.

### Pattern 2: Current DDL Structure (storage/schema.mpl)

The current schema.mpl is a single `pub fn ensure_schema(pool)` function containing 44 Pool.execute calls organized as:

1. **Extension**: `CREATE EXTENSION IF NOT EXISTS pgcrypto` (1 call)
2. **Tables**: 9 CREATE TABLE IF NOT EXISTS statements (organizations, users, org_memberships, sessions, projects, api_keys, issues, events, alert_rules, alerts) -- 10 calls
3. **ALTER TABLE additions**: 5 calls adding columns to alert_rules and projects that were added after initial schema
4. **Data migration**: 1 UPDATE for default slug values
5. **Indexes**: 17 CREATE INDEX IF NOT EXISTS statements covering foreign keys, lookup patterns, and GIN indexes for JSONB
6. **Partitioning helper**: Function `ensure_daily_partition` (11 lines) generating date-based partition table names

**Key DDL characteristics that affect migration conversion:**
- All tables use `UUID PRIMARY KEY DEFAULT gen_random_uuid()` except sessions (which uses `token TEXT PRIMARY KEY`)
- Events table uses JSONB columns: `tags jsonb`, `context jsonb`
- Has GIN index: `CREATE INDEX ... ON events USING GIN(tags jsonb_path_ops)`
- Foreign keys with `ON DELETE CASCADE` on most relationships
- Composite unique constraints: `UNIQUE(user_id, org_id)` on org_memberships, `UNIQUE(project_id, fingerprint)` on issues
- Partial indexes: `WHERE revoked_at IS NULL`, `WHERE enabled = true`, `WHERE slug IS NOT NULL`
- ALTER TABLE for schema evolution (adding columns to existing tables)

### Pattern 3: Current Query Patterns in queries.mpl (627 lines, 62 functions)

The queries fall into these categories:

**Category A: Simple CRUD (directly replaceable with Repo)**
- `create_org(pool, name, slug)` -> `Repo.insert(pool, "organizations", %{"name" => name, "slug" => slug})`
- `get_org_by_id(pool, id)` -> `Repo.get(pool, "organizations", id)`
- `list_orgs(pool)` -> `Repo.all(pool, Query.from("organizations") |> Query.order_by(:name, :asc))`
- `create_project(pool, org_id, name, platform)` -> Repo.insert
- `get_project_by_id(pool, id)` -> Repo.get
- `list_projects_for_org(pool, org_id)` -> Query.where + Repo.all
- `create_api_key(pool, project_id, label)` -> Repo.insert (but uses PG functions for key generation)
- `create_user(pool, email, password, display_name)` -> Repo.insert (but uses crypt() for password)
- `get_user_by_id(pool, id)` -> Repo.get
- `create_session(pool, user_id)` -> Repo.insert (but uses encode/gen_random_bytes for token)
- `validate_session(pool, token)` -> Query.where + `expires_at > now()` condition
- `delete_session(pool, token)` -> Repo.delete
- `add_org_member(pool, user_id, org_id, role)` -> Repo.insert
- `list_org_members(pool, org_id)` -> Query.where + Repo.all
- `list_user_memberships(pool, user_id)` -> Query.where + Repo.all
- Simple status updates: `resolve_issue`, `archive_issue`, `reopen_issue`, `discard_issue` -> Repo.update
- `assign_issue(pool, issue_id, user_id)` -> Repo.update
- `delete_issue_with_events(pool, issue_id)` -> Repo.delete (needs transaction for multi-table)
- `update_member_role`, `remove_member` -> Repo.update/delete
- Alert status updates: `acknowledge_alert`, `resolve_alert` -> Repo.update

**Count: ~25 functions directly replaceable with simple Repo/Query calls.**

**Category B: Queries with PG-Specific Functions (need fragment or raw SQL)**
- `create_user` uses `crypt($2, gen_salt('bf', 12))` for password hashing
- `authenticate_user` uses `crypt($2, password_hash)` for password verification
- `create_api_key` uses `'mshr_' || encode(gen_random_bytes(24), 'hex')` for key generation
- `create_session` uses `encode(gen_random_bytes(32), 'hex')` for token generation
- `validate_session` uses `expires_at > now()` for session validity
- `delete_expired_events` uses `now() - ($2 || ' days')::interval` for date arithmetic
- `check_sample_rate` uses `random() < COALESCE(...)` for probabilistic sampling
- `get_retention_settings` / `update_retention_settings` use `::jsonb->>'key'` for JSON field extraction
- `ensure_daily_partition` generates dynamic DDL for table partitioning

**Count: ~10 functions requiring Query.fragment() or continued raw Pool.query.**

**Category C: Complex Analytical/Aggregation Queries (likely need fragment)**
- `upsert_issue` uses `INSERT ... ON CONFLICT (project_id, fingerprint) DO UPDATE SET event_count = issues.event_count + 1, last_seen = now()` -- UPSERT with increment
- `detect_regressions` uses complex subquery with `HAVING count(*) > GREATEST(10, ...)` and correlated subquery
- `insert_event_from_json` uses `$1::jsonb->>'field'` JSON extraction in INSERT ... SELECT
- `list_issues_paginated` uses keyset pagination: `WHERE (last_seen, id::text) < ($5::timestamptz, $6)` with multi-column ordering
- `search_issues` uses `title ILIKE '%' || $2 || '%'` for text search
- `search_issues_by_tag` uses `tags @> $2::jsonb` for JSONB containment
- `list_events_paginated` uses keyset pagination similar to issues
- `event_counts_by_bucket` uses `date_trunc($2, received_at)` for time bucketing with GROUP BY
- `level_distribution` uses `count(*)` with GROUP BY and percentage calculation
- `recent_events_summary` returns partial event data with truncated payload
- `tag_value_counts` uses `jsonb_each_text(tags)` with GROUP BY for tag analytics
- `event_timeline` uses `date_trunc('hour', received_at)` with GROUP BY
- `get_event_detail` returns full event row with JSONB columns
- `adjacent_events` uses `ROW_NUMBER() OVER (ORDER BY received_at)` for next/prev navigation
- Various alert rule queries using JSONB extraction and interval math

**Count: ~25 functions with complex SQL that may need fragment() or partial raw SQL.**

### Pattern 4: Direct Pool.query Calls Outside storage/ (5 modules)

These modules bypass the storage layer and call Pool.query directly:

**ingestion/pipeline.mpl** (1 call):
```
Pool.query(pool, "SELECT COALESCE($1::jsonb->>$2, '') AS val", [condition_json, field])
```
Used to extract JSONB fields from alert rule conditions. This is a JSONB utility, not a table query.

**ingestion/routes.mpl** (3 calls):
```
Pool.query(pool, "SELECT count(*)::text AS cnt FROM issues WHERE ...", [project_id])
Pool.query(pool, "SELECT project_id::text FROM issues WHERE id = $1::uuid", [issue_id])
Pool.query(pool, "SELECT COALESCE($1::jsonb->>'user_id', '') AS user_id", [body])
```
First two are simple issue lookups replaceable with Repo.count/Repo.get. Third is JSONB extraction.

**ingestion/ws_handler.mpl** (1 call):
```
Pool.query(pool, "SELECT COALESCE($1::jsonb->'filters'->>'level', '') AS level, ...", [message])
```
JSONB extraction from a WebSocket message. Not a table query.

**api/team.mpl** (1 call):
```
Pool.query(pool, "SELECT COALESCE($1::jsonb->>$2, '') AS val", [body, field])
```
JSONB field extraction from request body.

**api/alerts.mpl** (1 call):
```
Pool.query(pool, "SELECT COALESCE($1::jsonb->>'enabled', 'true') AS enabled", [body])
```
JSONB field extraction from request body.

**Key insight:** The non-storage Pool.query calls fall into two groups:
1. **Table queries** (2 calls in routes.mpl): Directly replaceable with Repo.count/Repo.get
2. **JSONB utility calls** (4 calls): These use PostgreSQL as a JSON parser (passing a string parameter and extracting fields). These are NOT table queries -- they use PG's `::jsonb->>'key'` on parameter values, not on table data. These should remain as raw `Pool.query` calls or be replaced with a Mesh-native JSON parsing function if available.

### Pattern 5: Service Module Architecture

All Mesher service modules follow the same actor pattern:
```
# Service module (e.g., OrgService)
pub fn start(pool) do
  Process.spawn(fn() do
    Process.register("service_name")
    loop(pool)
  end)
end

fn loop(pool) do
  receive do
    {:create, name, slug, reply_to} ->
      let result = Queries.create_org(pool, name, slug)
      Process.send(reply_to, result)
      loop(pool)
    ...
  end
end

pub fn create(pid, name, slug) do
  Process.call(pid, {:create, name, slug})
end
```

The services delegate ALL database operations to `Queries.*` functions. The services themselves do NOT contain SQL -- they are pure message-passing wrappers. This means the rewrite can focus on `storage/queries.mpl` and the services will automatically use the new ORM-backed functions.

**Exception: EventProcessor** (services/event_processor.mpl) calls several Queries functions in sequence for the event processing pipeline: `check_sample_rate -> check_discarded -> upsert_issue -> evaluate_alert_rules`. This sequential logic must be preserved.

**Exception: WriterService** (services/writer.mpl) directly calls `Pool.execute` in its loop, bypassing Queries. This must change to use Repo.insert or a Queries function.

### Pattern 6: Relationship Map Between All 11 Types

Based on analysis of foreign keys in schema.mpl and usage patterns in queries.mpl:

```
Organization
  has_many :projects, Project         (FK: projects.org_id)
  has_many :org_memberships, OrgMembership (FK: org_memberships.org_id)

User
  has_many :org_memberships, OrgMembership (FK: org_memberships.user_id)
  has_many :sessions, Session         (FK: sessions.user_id)

OrgMembership
  belongs_to :user, User              (FK: user_id)
  belongs_to :org, Organization       (FK: org_id)

Session
  belongs_to :user, User              (FK: user_id)

Project
  belongs_to :org, Organization       (FK: org_id)
  has_many :api_keys, ApiKey          (FK: api_keys.project_id)
  has_many :issues, Issue             (FK: issues.project_id)
  has_many :events, Event             (FK: events.project_id)
  has_many :alert_rules, AlertRule    (FK: alert_rules.project_id)
  has_many :alerts, Alert             (FK: alerts.project_id)

ApiKey
  belongs_to :project, Project        (FK: project_id)

Issue
  belongs_to :project, Project        (FK: project_id)
  has_many :events, Event             (FK: events.issue_id)

Event
  belongs_to :project, Project        (FK: project_id)
  belongs_to :issue, Issue            (FK: issue_id)

AlertRule
  belongs_to :project, Project        (FK: project_id)
  has_many :alerts, Alert             (FK: alerts.rule_id)

Alert
  belongs_to :rule, AlertRule         (FK: rule_id)
  belongs_to :project, Project        (FK: project_id)

RetentionSettings
  (virtual type -- maps to projects table; no independent relationships)
```

### Pattern 7: ORM API Available (from Phases 96-101)

**Schema (Phase 96-97):**
```
struct User do
  table "users"
  primary_key :id
  timestamps true

  id :: String
  name :: String
  belongs_to :org, Organization
  has_many :posts, Post
end deriving(Schema, Row, Json)

User.__table__()              -> "users"
User.__fields__()             -> ["id", "name", ...]
User.__field_types__()        -> ["id:TEXT", "name:TEXT", ...]
User.__primary_key__()        -> "id"
User.__relationships__()      -> ["belongs_to:org:Organization"]
User.__relationship_meta__()  -> ["belongs_to:org:Organization:org_id:organizations"]
User.__name_col__()           -> "name"
```

**Query Builder (Phase 98):**
```
Query.from("users")
  |> Query.where(:name, "Alice")
  |> Query.where_op(:age, :gt, "18")
  |> Query.where_in(:status, ["active", "pending"])
  |> Query.where_null(:deleted_at)
  |> Query.where_not_null(:email)
  |> Query.order_by(:created_at, :desc)
  |> Query.limit(10)
  |> Query.offset(0)
  |> Query.select(["id", "name"])
  |> Query.join(:inner, "posts", "posts.user_id = users.id")
  |> Query.group_by(:status)
  |> Query.having("count(*) >", "5")
  |> Query.fragment("date_trunc('day', ?)", ["created_at"])
```

**Repo (Phase 98):**
```
Repo.all(pool, query)         -> Result<List<Map<String,String>>, String>
Repo.one(pool, query)         -> Result<Map<String,String>, String>
Repo.get(pool, table, id)     -> Result<Map<String,String>, String>
Repo.get_by(pool, table, field, value) -> Result<Map<String,String>, String>
Repo.count(pool, query)       -> Result<Int, String>
Repo.exists(pool, query)      -> Result<Bool, String>
Repo.insert(pool, table, fields_map)   -> Result<Map<String,String>, String>
Repo.update(pool, table, id, fields_map) -> Result<Map<String,String>, String>
Repo.delete(pool, table, id)  -> Result<Map<String,String>, String>
Repo.transaction(pool, fn(conn) -> ...)  -> Result<T, String>
Repo.preload(pool, rows, assocs, meta)  -> List<Map<String,String>>
```

**Changeset (Phase 99):**
```
Changeset.cast(data, params, [:name, :email])
  |> Changeset.validate_required([:name, :email])
  |> Changeset.validate_length(:name, 1, 100)
  |> Changeset.validate_format(:email, "@")
  |> Changeset.validate_inclusion(:role, ["admin", "member"])
Changeset.valid(cs)           -> Bool
Changeset.errors(cs)          -> Map<String,String>
Changeset.changes(cs)         -> Map<String,String>
Repo.insert_changeset(pool, cs, table, field_types) -> Result<Map<String,String>, String>
```

**Migration DSL (Phase 101):**
```
pub fn up(pool) do
  Migration.create_table(pool, "users", [
    "id:UUID:PRIMARY KEY DEFAULT gen_random_uuid()",
    "name:TEXT:NOT NULL",
    "email:TEXT:NOT NULL UNIQUE"
  ])?
  Migration.create_index(pool, "users", "idx_users_email", ["email"], "")
end

pub fn down(pool) do
  Migration.drop_table(pool, "users")
end
```

### Anti-Patterns to Avoid

**Anti-Pattern 1: Converting PG-specific functions to ORM calls that lose functionality.**
The ORM's Repo.insert generates `INSERT INTO table (cols) VALUES ($1,...) RETURNING *`. But Mesher's `create_user` uses `crypt($2, gen_salt('bf', 12))` for password hashing inline. The ORM cannot express this. These functions must either: (a) use Query.fragment for the PG-specific parts, or (b) keep a thin raw-SQL wrapper. Do NOT try to move bcrypt logic to Mesh application code -- it belongs in PG for security.

**Anti-Pattern 2: Losing UPSERT semantics by splitting into separate SELECT + INSERT.**
The `upsert_issue` function uses `INSERT ... ON CONFLICT DO UPDATE SET event_count = issues.event_count + 1`. This is atomic and cannot be decomposed into `Repo.get` followed by `Repo.insert` or `Repo.update` without losing atomicity. This must remain as raw SQL or use a `Repo.query_raw` escape hatch.

**Anti-Pattern 3: Replacing complex analytical queries with multiple simple Repo calls.**
Queries like `event_counts_by_bucket` (GROUP BY with date_trunc), `level_distribution` (percentage calculations), and `adjacent_events` (ROW_NUMBER window function) are inherently SQL. Replacing them with application-level aggregation would be slower and more complex. Use raw Pool.query or Query.fragment for these.

**Anti-Pattern 4: Converting JSONB extraction utility calls to ORM.**
The 4 JSONB utility calls (`SELECT COALESCE($1::jsonb->>'key', '') AS val`) are using PG as a JSON parser, not querying tables. These are NOT candidates for ORM conversion. They should either remain as Pool.query or be replaced with a Mesh-native JSON parser if available.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Table-to-struct mapping | Manual Map.get chains | from_row (deriving(Row)) | Already generated by deriving(Row), handles all fields |
| Schema metadata | Hardcoded strings | deriving(Schema) | Compile-time generated, always consistent with struct |
| CRUD operations | Raw SQL strings | Repo.insert/get/update/delete | ORM handles quoting, parameterization, RETURNING |
| Query composition | String concatenation | Query builder pipe chains | Type-safe, composable, no SQL injection risk |
| Input validation | Manual if/else checks | Changeset pipeline | Consistent error accumulation, type coercion |
| DDL management | Imperative ensure_schema | Migration files | Versioned, reversible, team-friendly |
| Batch preloading | N+1 query loops | Repo.preload | Single WHERE IN query per association |

## Common Pitfalls

### Pitfall 1: Session Table Has Non-Standard Primary Key

**What goes wrong:** Session uses `token TEXT PRIMARY KEY`, not the default `id UUID PRIMARY KEY`. If `deriving(Schema)` defaults to `primary_key :id`, all Session queries will fail.
**Why it happens:** The ORM defaults to `"id"` as primary key (Phase 96 decision).
**How to avoid:** Use `primary_key :token` schema option on the Session struct. Verify `Session.__primary_key__()` returns `"token"` before using `Repo.get`.
**Warning signs:** `Repo.get(pool, "sessions", token)` generates `WHERE "id" = $1` instead of `WHERE "token" = $1`.

### Pitfall 2: User.password_hash Must Not Appear in Default SELECT

**What goes wrong:** `Repo.all` with `User.__fields__()` returns password_hash in results, leaking sensitive data in API responses.
**Why it happens:** `deriving(Schema)` includes ALL struct fields in `__fields__()`.
**How to avoid:** Either: (a) exclude password_hash from the struct and handle it separately in auth queries, (b) use `Query.select()` to explicitly list fields without password_hash for non-auth queries, or (c) accept that the storage layer returns it but the API serialization (deriving(Json)) excludes it. Recommendation: (b) -- define a `user_safe_fields` list constant and use it in all non-auth queries.
**Warning signs:** API responses containing password_hash values.

### Pitfall 3: UPSERT Cannot Be Expressed in Repo.insert

**What goes wrong:** `upsert_issue` uses `INSERT ... ON CONFLICT DO UPDATE SET event_count = event_count + 1, last_seen = now()`. Repo.insert has no ON CONFLICT support.
**Why it happens:** The ORM's insert builds simple `INSERT INTO ... VALUES ... RETURNING *`.
**How to avoid:** Keep `upsert_issue` as a raw `Pool.query` call wrapped in a function that follows the same signature pattern as other converted functions. Document it as a raw-SQL escape hatch. Future ORM enhancement could add `Repo.upsert`.
**Warning signs:** Lost events or duplicate issues if UPSERT is converted to SELECT-then-INSERT.

### Pitfall 4: Keyset Pagination Requires Composite WHERE Conditions

**What goes wrong:** `list_issues_paginated` uses `WHERE (last_seen, id::text) < ($5::timestamptz, $6)` -- a row-value comparison for keyset pagination. The ORM's Query.where does not support tuple comparisons.
**Why it happens:** Query.where generates simple `column op $N` conditions, not tuple comparisons.
**How to avoid:** Use `Query.fragment("(last_seen, id::text) < ($5::timestamptz, $6)", [...])` for the pagination condition, or keep the keyset pagination logic as raw SQL.
**Warning signs:** Incorrect pagination ordering or missing results at page boundaries.

### Pitfall 5: Event Insert Uses JSON Extraction in SQL

**What goes wrong:** `insert_event_from_json` does `INSERT INTO events SELECT $1::jsonb->>'field' ...` -- extracting fields from a JSON parameter directly in the INSERT. Repo.insert expects a Map of explicit field values.
**Why it happens:** The current design avoids parsing JSON in Mesh code; PG does the extraction.
**How to avoid:** Either: (a) parse the JSON event payload in Mesh code first, then use Repo.insert with the extracted values, or (b) keep this as raw Pool.query. Recommendation: (a) is preferred since the EventPayload struct already has all fields parsed -- use the struct fields directly.
**Warning signs:** Performance regression if JSON parsing moves to application code.

### Pitfall 6: Alert Rule Queries Use JSONB Column Extraction

**What goes wrong:** Alert rule queries access `condition_json` and `action_json` columns with `$1::jsonb->>'field'` in WHERE and SELECT clauses. Repo.get returns these as opaque strings.
**Why it happens:** JSONB columns are stored as Mesh String fields but their contents need runtime inspection.
**How to avoid:** After fetching alert rules via Repo, parse the JSONB strings in Mesh code using the `::jsonb->>'key'` utility pattern (or a future JSON.get function). Alternatively, keep the JSONB extraction queries as raw SQL.
**Warning signs:** Extra round-trips to PG for what was previously done in a single query.

### Pitfall 7: Migration Order Matters for Foreign Keys

**What goes wrong:** Migration files must create tables in dependency order. If `issues` migration runs before `projects`, the FK constraint `REFERENCES projects(id)` fails.
**Why it happens:** Timestamp-ordered migrations don't inherently respect FK dependencies.
**How to avoid:** Use a single initial migration that creates all tables in dependency order (matching the existing ensure_schema order), OR use multiple migrations with carefully ordered timestamps. Recommendation: single initial migration for the existing schema, separate migrations for future changes.
**Warning signs:** Migration runner fails with "relation does not exist" errors.

### Pitfall 8: Partial Indexes and GIN Indexes in Migration DSL

**What goes wrong:** Migration DSL's `Migration.create_index` may not support partial index conditions (`WHERE revoked_at IS NULL`) or GIN index types (`USING GIN(tags jsonb_path_ops)`).
**Why it happens:** Phase 101 research shows basic index creation; partial and GIN indexes may require raw SQL.
**How to avoid:** Check Phase 101 implementation for index options support. If not available, use `Pool.execute(pool, "CREATE INDEX ...", [])` in migration up/down functions alongside Migration DSL calls. This is acceptable -- migration files can mix DSL and raw SQL.
**Warning signs:** Missing indexes on api_keys (partial), events (GIN).

## Code Examples

### Example 1: Organization Schema Struct (Before -> After)

**Before:**
```
struct Organization do
  id :: String
  name :: String
  slug :: String
  created_at :: String
end deriving(Json, Row)
```

**After:**
```
struct Organization do
  table "organizations"
  primary_key :id
  timestamps true

  id :: String
  name :: String
  slug :: String

  has_many :projects, Project
  has_many :org_memberships, OrgMembership
end deriving(Schema, Json, Row)
```

### Example 2: Session Schema Struct (Non-Standard PK)

**After:**
```
struct Session do
  table "sessions"
  primary_key :token

  token :: String
  user_id :: String
  created_at :: String
  expires_at :: String

  belongs_to :user, User
end deriving(Schema, Json, Row)
```

### Example 3: Simple CRUD Replacement

**Before (queries.mpl):**
```
pub fn create_org(pool :: PoolHandle, name :: String, slug :: String) -> String!String do
  let rows = Pool.query(pool, "INSERT INTO organizations (name, slug) VALUES ($1, $2) RETURNING id::text", [name, slug])?
  let row = List.head(rows)
  Ok(Map.get(row, "id"))
end

pub fn get_org_by_id(pool :: PoolHandle, id :: String) -> Map<String, String>!String do
  let rows = Pool.query(pool, "SELECT id::text, name, slug, created_at::text FROM organizations WHERE id = $1::uuid", [id])?
  if List.length(rows) > 0 do
    Ok(List.head(rows))
  else
    Err("organization not found")
  end
end

pub fn list_orgs(pool :: PoolHandle) -> List<Map<String, String>>!String do
  let rows = Pool.query(pool, "SELECT id::text, name, slug, created_at::text FROM organizations ORDER BY name", [])?
  Ok(rows)
end
```

**After (queries.mpl):**
```
pub fn create_org(pool :: PoolHandle, name :: String, slug :: String) -> String!String do
  let fields = %{"name" => name, "slug" => slug}
  let row = Repo.insert(pool, Organization.__table__(), fields)?
  Ok(Map.get(row, "id"))
end

pub fn get_org_by_id(pool :: PoolHandle, id :: String) -> Map<String, String>!String do
  Repo.get(pool, Organization.__table__(), id)
end

pub fn list_orgs(pool :: PoolHandle) -> List<Map<String, String>>!String do
  let q = Query.from(Organization.__table__())
    |> Query.order_by(:name, :asc)
  Repo.all(pool, q)
end
```

### Example 4: Complex Query Requiring Raw SQL Escape Hatch

**Before:**
```
pub fn upsert_issue(pool, project_id, fingerprint, title, level) -> String!String do
  let sql = "INSERT INTO issues (project_id, fingerprint, title, level) VALUES ($1::uuid, $2, $3, $4) ON CONFLICT (project_id, fingerprint) DO UPDATE SET event_count = issues.event_count + 1, last_seen = now(), title = EXCLUDED.title, level = EXCLUDED.level RETURNING id::text"
  let rows = Pool.query(pool, sql, [project_id, fingerprint, title, level])?
  let row = List.head(rows)
  Ok(Map.get(row, "id"))
end
```

**After (keeps raw SQL for UPSERT):**
```
pub fn upsert_issue(pool, project_id, fingerprint, title, level) -> String!String do
  let sql = "INSERT INTO issues (project_id, fingerprint, title, level) VALUES ($1::uuid, $2, $3, $4) ON CONFLICT (project_id, fingerprint) DO UPDATE SET event_count = issues.event_count + 1, last_seen = now(), title = EXCLUDED.title, level = EXCLUDED.level RETURNING id::text"
  let rows = Pool.query(pool, sql, [project_id, fingerprint, title, level])?
  let row = List.head(rows)
  Ok(Map.get(row, "id"))
end
```

### Example 5: Keyset Pagination with Query.fragment

**Before:**
```
pub fn list_issues_paginated(pool, project_id, status, level, assigned_to, cursor, cursor_id, limit_str) do
  let sql = "SELECT ... FROM issues WHERE project_id = $1::uuid AND status = $2 AND ($3 = '' OR level = $3) AND ($4 = '' OR assigned_to = $4::uuid) AND (last_seen, id::text) < ($5::timestamptz, $6) ORDER BY last_seen DESC, id DESC LIMIT $7"
  Pool.query(pool, sql, [project_id, status, level, assigned_to, cursor, cursor_id, limit_str])
end
```

**After (uses fragment for tuple comparison and conditional filters):**
```
pub fn list_issues_paginated(pool, project_id, status, level, assigned_to, cursor, cursor_id, limit_str) do
  # Complex keyset pagination with conditional filters -- raw SQL is clearer
  let sql = "SELECT ... FROM issues WHERE project_id = $1::uuid AND status = $2 AND ($3 = '' OR level = $3) AND ($4 = '' OR assigned_to = $4::uuid) AND (last_seen, id::text) < ($5::timestamptz, $6) ORDER BY last_seen DESC, id DESC LIMIT $7"
  Pool.query(pool, sql, [project_id, status, level, assigned_to, cursor, cursor_id, limit_str])
end
```

Note: Some complex queries are better left as raw SQL. The success criteria says "approximately 100-150 lines of ORM query code" -- this implies the ORM replaces the simple/medium queries, while complex analytics remain as raw SQL.

### Example 6: Migration File for Initial Schema

```
# migrations/20260216120000_create_initial_schema.mpl
# Creates all tables, indexes, and extensions for Mesher.

pub fn up(pool) do
  # Extensions
  Pool.execute(pool, "CREATE EXTENSION IF NOT EXISTS pgcrypto", [])?

  # Tables (in FK dependency order)
  Migration.create_table(pool, "organizations", [
    "id:UUID:PRIMARY KEY DEFAULT gen_random_uuid()",
    "name:TEXT:NOT NULL",
    "slug:TEXT:NOT NULL UNIQUE",
    "created_at:TIMESTAMPTZ:NOT NULL DEFAULT now()"
  ])?

  Migration.create_table(pool, "users", [
    "id:UUID:PRIMARY KEY DEFAULT gen_random_uuid()",
    "email:TEXT:NOT NULL UNIQUE",
    "password_hash:TEXT:NOT NULL",
    "display_name:TEXT:NOT NULL",
    "created_at:TIMESTAMPTZ:NOT NULL DEFAULT now()"
  ])?

  # ... (remaining tables in FK order)

  # Indexes
  Migration.create_index(pool, "org_memberships", "idx_org_memberships_user", ["user_id"], "")?
  # ... (remaining indexes)

  # Partial and GIN indexes (may need raw SQL)
  Pool.execute(pool, "CREATE UNIQUE INDEX IF NOT EXISTS idx_projects_slug ON projects(slug) WHERE slug IS NOT NULL", [])?
  Pool.execute(pool, "CREATE INDEX IF NOT EXISTS idx_events_tags ON events USING GIN(tags jsonb_path_ops)", [])?

  Ok(0)
end

pub fn down(pool) do
  # Drop in reverse FK order
  Migration.drop_table(pool, "alerts")?
  Migration.drop_table(pool, "alert_rules")?
  Migration.drop_table(pool, "events")?
  Migration.drop_table(pool, "issues")?
  Migration.drop_table(pool, "api_keys")?
  Migration.drop_table(pool, "projects")?
  Migration.drop_table(pool, "sessions")?
  Migration.drop_table(pool, "org_memberships")?
  Migration.drop_table(pool, "users")?
  Migration.drop_table(pool, "organizations")?
  Ok(0)
end
```

## State of the Art

| Current State | After Rewrite | Impact |
|--------------|---------------|--------|
| 11 structs with deriving(Json, Row) only | 11 structs with deriving(Schema, Json, Row) | Compile-time metadata for table/field/PK/relationships |
| 82 lines of imperative DDL in schema.mpl | Versioned migration files using Migration DSL | Reversible, team-friendly schema management |
| 627 lines of raw SQL in queries.mpl | ~100-150 lines of ORM code + ~100 lines of raw SQL for complex queries | 60-75% reduction in SQL code |
| 62 Pool.query/Pool.execute call sites | ~25 Repo calls + ~25 raw SQL (complex) + ~5 JSONB utilities | Significant reduction in direct Pool usage |
| No input validation layer | Changeset validation for user-facing mutations | Type coercion, required fields, format validation |
| No relationship metadata | Full relationship graph with FK inference | Enables Repo.preload for API responses |
| Single ensure_schema() function | Migration files with up/down | Proper schema versioning |

## Open Questions

### 1. Which complex queries should remain as raw SQL vs use ORM?

**What we know:** The success criteria says "approximately 100-150 lines of ORM query code." The current 627 lines include ~25 simple CRUD functions (directly convertible, ~100 lines of ORM), ~10 PG-function-dependent queries (need fragment or raw SQL), and ~25 complex analytics queries.

**What's unclear:** Should complex analytics queries (date_trunc bucketing, JSONB tag queries, window functions, keyset pagination) be converted to Query.fragment chains or left as raw Pool.query?

**Recommendation:** Convert the ~25 simple CRUD functions to full ORM (yielding ~100-150 lines of ORM code). Leave the ~25 complex queries as raw Pool.query wrapped in the same function signatures. The ~10 PG-function queries can use a hybrid approach (ORM for structure + fragment for PG functions). This satisfies the success criteria's "approximately 100-150 lines" target while maintaining correctness for complex SQL.

### 2. How should the JSONB utility queries (non-table Pool.query) be handled?

**What we know:** 4 calls in non-storage modules use `Pool.query` to extract JSON fields from string parameters (not table data). The ORM is designed for table operations.

**What's unclear:** Should these be converted to Repo calls, left as Pool.query, or replaced with a Mesh JSON parsing function?

**Recommendation:** Leave as Pool.query. These are utility calls, not database operations. They use PG's JSON parsing capability as a convenience. Converting them to ORM would be forcing a square peg into a round hole. A future Mesh JSON module could replace them, but that is out of scope for Phase 102.

### 3. Should RetentionSettings be a separate Schema struct or a projection?

**What we know:** RetentionSettings doesn't exist as a type today. Retention data (retention_days, sample_rate) lives as columns on the projects table. The success criteria lists it as one of the 11 Schema types.

**What's unclear:** Is it a virtual/projection type or an actual table?

**Recommendation:** Create RetentionSettings as a Schema struct with `table "projects"` (same table as Project). Its `__fields__()` would return only `["retention_days", "sample_rate"]`, and it would be used for settings-specific queries. This satisfies the success criteria while reflecting the actual data model.

### 4. How should password hashing (crypt/gen_salt) be handled in the ORM layer?

**What we know:** `create_user` and `authenticate_user` use PG's crypt() and gen_salt() functions inline in SQL. Repo.insert generates simple parameterized INSERTs that cannot call PG functions on parameter values.

**What's unclear:** Should password hashing move to Mesh application code, stay in PG via raw SQL, or use a hybrid approach?

**Recommendation:** Keep password hashing in PG via raw SQL for the create_user and authenticate_user functions. These are security-critical paths where the established behavior should not change. All other User CRUD operations (get, list, update non-password fields) use the ORM.

### 5. How should the migration handle ALTER TABLE additions?

**What we know:** The current schema.mpl has 5 ALTER TABLE statements that added columns to alert_rules and projects tables after initial creation. In a migration system, these would be separate migration files.

**What's unclear:** Should the initial migration include these columns in the CREATE TABLE (since we are creating a fresh migration history), or should there be separate migrations matching the historical evolution?

**Recommendation:** Include all columns in the initial CREATE TABLE migration. Since we are replacing the imperative schema.mpl with a fresh migration system, the first migration should reflect the CURRENT desired state, not the historical evolution. Historical ALTER TABLE additions are an artifact of the imperative approach.

## Quantitative Analysis

### Query Conversion Breakdown

| Category | Count | Lines (Before) | Lines (After) | Approach |
|----------|-------|----------------|---------------|----------|
| Simple CRUD (insert/get/list/update/delete) | 25 | ~300 | ~100-120 | Full ORM (Repo + Query) |
| PG-function queries (crypt, gen_random_bytes, interval) | 10 | ~120 | ~100 | Hybrid (ORM structure + raw SQL for PG functions) |
| Complex analytics (GROUP BY, window, JSONB, pagination) | 22 | ~180 | ~180 | Raw Pool.query (preserve existing SQL) |
| JSONB utility (non-table JSON parsing) | 5 | ~27 | ~27 | Raw Pool.query (not table operations) |
| **Total** | **62** | **627** | **~407-427** | **~35% reduction** |

### Schema Struct Conversion

| Struct | Current Deriving | New Deriving | Schema Options | Relationships |
|--------|-----------------|--------------|----------------|---------------|
| Organization | Json, Row | Schema, Json, Row | table "organizations" | has_many :projects, :org_memberships |
| User | Json, Row | Schema, Json, Row | table "users" | has_many :org_memberships, :sessions |
| OrgMembership | Json, Row | Schema, Json, Row | table "org_memberships" | belongs_to :user, :org |
| Session | Json, Row | Schema, Json, Row | table "sessions", primary_key :token | belongs_to :user |
| Project | Json, Row | Schema, Json, Row | table "projects" | belongs_to :org; has_many :api_keys, :issues, :events, :alert_rules, :alerts |
| ApiKey | Json, Row | Schema, Json, Row | table "api_keys" | belongs_to :project |
| Event | Json, Row | Schema, Json, Row | table "events" | belongs_to :project, :issue |
| Issue | Json, Row | Schema, Json, Row | table "issues" | belongs_to :project; has_many :events |
| AlertRule | Json, Row | Schema, Json, Row | table "alert_rules" | belongs_to :project; has_many :alerts |
| Alert | Json, Row | Schema, Json, Row | table "alerts" | belongs_to :rule, :project |
| RetentionSettings | (new) | Schema, Json, Row | table "projects" | (none) |

### Migration File Breakdown

| Migration | Tables | Indexes | Raw SQL Lines |
|-----------|--------|---------|---------------|
| Initial schema creation | 10 CREATE TABLE | 17 CREATE INDEX | ~80-100 |
| (Future additions would be separate files) | - | - | - |

## Recommended Plan Structure

### Plan 102-01: Schema Struct Conversion + Relationship Declarations
- Add `deriving(Schema)` to all 11 type structs
- Add schema options (table, primary_key) where non-default
- Add relationship declarations (belongs_to, has_many)
- Create RetentionSettings as new struct type
- Verify all `__table__()`, `__fields__()`, `__primary_key__()` return correct values via e2e tests

### Plan 102-02: Migration Files Replace storage/schema.mpl
- Create `mesher/migrations/` directory
- Write initial migration creating all 10 tables with full DDL
- Write index migration(s) for all 17+ indexes
- Handle special cases: partial indexes, GIN indexes, pgcrypto extension
- Update main.mpl to use `meshc migrate up` instead of `ensure_schema()`
- Delete or deprecate storage/schema.mpl

### Plan 102-03: Simple CRUD Query Conversion (~25 functions)
- Convert all simple insert/get/list/update/delete functions to Repo/Query
- Preserve existing function signatures (parameters and return types)
- Add Changeset validation for user-facing mutations (create_org, create_user, etc.)
- Test each converted function maintains identical behavior

### Plan 102-04: Complex Query Conversion + Hybrid Approach (~35 functions)
- Convert medium-complexity queries using Query.fragment where needed
- Keep complex analytics queries as raw Pool.query
- Convert PG-function queries to hybrid approach
- Handle password hashing, session creation, API key generation as raw SQL

### Plan 102-05: Service/API Module Migration + End-to-End Verification
- Update service modules that bypass storage/queries.mpl
- Convert direct Pool.query calls in ingestion/ and api/ modules
- Update WriterService to use Repo.insert
- End-to-end verification: run full Mesher application
- Verify all functionality: ingestion, error grouping, REST API, streaming, alerting, retention, clustering

## Sources

### Primary (HIGH confidence)
- Codebase: `mesher/storage/queries.mpl` (627 lines) -- complete raw SQL inventory, 62 functions with Pool.query/Pool.execute
- Codebase: `mesher/storage/schema.mpl` (82 lines) -- 10 CREATE TABLE + 5 ALTER TABLE + 17 CREATE INDEX + partitioning helper
- Codebase: `mesher/storage/writer.mpl` (24 lines) -- batch event writer with direct Pool.execute
- Codebase: `mesher/types/project.mpl` (75 lines) -- Organization, User, OrgMembership, Session, Project, ApiKey structs
- Codebase: `mesher/types/event.mpl` (52 lines) -- EventPayload, StackFrame, ExceptionInfo, Event structs
- Codebase: `mesher/types/issue.mpl` (22 lines) -- Issue struct
- Codebase: `mesher/types/alert.mpl` (23 lines) -- AlertRule, Alert structs
- Codebase: `mesher/services/` -- all 7 service actors (org, project, user, event_processor, retention, writer, stream_manager)
- Codebase: `mesher/ingestion/` -- pipeline.mpl, routes.mpl, ws_handler.mpl (5 direct Pool.query calls)
- Codebase: `mesher/api/` -- team.mpl, alerts.mpl, settings.mpl, dashboard.mpl, detail.mpl, search.mpl (2 direct Pool.query calls)
- Codebase: `mesher/main.mpl` -- application entry point showing ensure_schema() initialization
- Phase 96 research: `.planning/phases/96-compiler-additions/96-RESEARCH.md` -- deriving(Schema) infrastructure
- Phase 97 research: `.planning/phases/97-schema-metadata-sql-generation/97-RESEARCH.md` -- schema options, SQL generation
- Phase 98 research: `.planning/phases/098-query-builder-repo/98-RESEARCH.md` -- Query builder + Repo API
- Phase 99 research: `.planning/phases/099-changesets/99-RESEARCH.md` -- Changeset validation pipeline
- Phase 100 research: `.planning/phases/100-relationships-preloading/100-RESEARCH.md` -- relationship metadata + Repo.preload
- Phase 101 research: `.planning/phases/101-migration-system/101-RESEARCH.md` -- Migration DSL + runner + CLI
- E2E tests: `crates/meshc/tests/e2e.rs` -- ORM pipeline compilation tests showing actual API

### Secondary (MEDIUM confidence)
- Codebase: `crates/mesh-rt/src/db/` -- runtime implementation files (repo.rs, query.rs, changeset.rs, migration.rs, orm.rs)
- Codebase: `crates/mesh-typeck/src/infer.rs` -- type registrations for Query/Repo/Changeset modules
- Codebase: `crates/mesh-codegen/src/mir/lower.rs` -- MIR lowering for Schema metadata + module function registration

## Metadata

**Confidence breakdown:**
- Type struct inventory: HIGH -- all 11 types identified, all fields documented, all relationship FKs traced
- Query inventory: HIGH -- all 62 functions in queries.mpl categorized, all 5 external Pool.query sites identified
- Schema DDL inventory: HIGH -- all 44 Pool.execute calls in schema.mpl documented with table structures
- ORM API availability: HIGH -- phases 96-101 research and e2e tests confirm Query/Repo/Changeset/Migration APIs
- Simple CRUD conversion: HIGH -- ~25 functions have direct 1:1 ORM equivalents
- Complex query conversion: MEDIUM -- some queries may need ORM features not yet tested in production (fragment, raw SQL escapes)
- Migration DSL capabilities: MEDIUM -- partial indexes and GIN indexes may need raw SQL fallback
- End-to-end behavior preservation: MEDIUM -- full verification requires running the application with real data

**Research date:** 2026-02-16
**Valid until:** 2026-03-16 (Mesher codebase is stable, ORM infrastructure is fully built)
