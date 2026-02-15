# Phase 91: REST API - Research

**Researched:** 2026-02-14
**Domain:** REST API endpoints for search/filtering, keyset pagination, dashboard aggregations, event detail views, team management, and API tokens -- all implemented in Mesh (.mpl) with PostgreSQL
**Confidence:** HIGH

## Summary

Phase 91 adds the complete REST API layer to Mesher, exposing all platform data (issues, events, organizations, projects, users, teams) through HTTP endpoints with filtering, search, pagination, and aggregation. The current codebase has a minimal issue listing endpoint (GET /api/v1/projects/:project_id/issues, hardcoded to `status=unresolved`) and issue state transition routes (POST /api/v1/issues/:id/resolve, etc.) from Phase 89, plus event ingestion routes (POST /api/v1/events) from Phase 88. Phase 91 must expand this to 19 requirements across four domains: search/filtering (SEARCH-01 through SEARCH-05), dashboard aggregations (DASH-01 through DASH-06), event detail views (DETAIL-01 through DETAIL-06), and organization/team management (ORG-04, ORG-05).

The most significant discovery is that **Request.query IS fully implemented** in the Mesh runtime, despite decision [89-02] stating "Mesh lacks query string parsing." The `Request.query(request, "param_name")` function exists in the typechecker (infer.rs line 549), codegen (lower.rs), intrinsics.rs, and runtime (server.rs line 219). It returns `Option<String>` and works by looking up a key in the request's `query_params` map (which is populated during HTTP request parsing from the URL query string). This means all filter/search parameters can be properly extracted from query strings rather than requiring workarounds.

The primary technical patterns for this phase are: (1) PostgreSQL-heavy query construction with SQL WHERE clause building via conditional string concatenation in Mesh, (2) keyset pagination using `WHERE (sort_col, id) < ($cursor_val, $cursor_id)` patterns with LIMIT, (3) PostgreSQL aggregate functions (COUNT, date_trunc GROUP BY) for dashboard data, (4) full-text search via PostgreSQL tsvector/tsquery (requires a schema migration to add a tsvector column on events.message), (5) JSONB operators for tag filtering (`tags @> '{"key":"value"}'`), and (6) manual JSON response serialization following the established pattern from routes.mpl.

**Primary recommendation:** Organize the new endpoints into three new route handler files (api/search.mpl, api/dashboard.mpl, api/detail.mpl) plus extend the existing files for team/token management. Build SQL queries with conditional WHERE clause construction in Mesh helper functions. Use Request.query for all filter parameters. Implement keyset pagination for list endpoints. Add a tsvector index for full-text search. Keep manual JSON serialization (Json.encode does not reliably work cross-module for imported struct types).

## Standard Stack

### Core
| Component | Version/Detail | Purpose | Why Standard |
|-----------|---------------|---------|--------------|
| Mesh language | v8.0 (current) | All application code | Dogfooding -- entire REST API in Mesh |
| Request.query | Built-in (infer.rs line 549) | Extract filter/search params from query string | Returns Option<String>, fully implemented in runtime |
| Request.param | Built-in | Extract path parameters (project_id, issue_id, event_id) | Existing, used in Phase 89 |
| HTTP.on_get | Built-in | GET endpoints for list/detail/dashboard | Existing, used for handle_list_issues |
| HTTP.on_post | Built-in | POST endpoints for create/update operations | Existing, heavily used |
| Pool.query | Built-in | All data retrieval queries | Returns `List<Map<String, String>>`, established pattern |
| Pool.execute | Built-in | All data modification queries | Returns `Result<Int, String>`, established pattern |
| PostgreSQL date_trunc + GROUP BY | PostgreSQL built-in | Dashboard time-bucket aggregations | Standard approach for hourly/daily event volume |
| PostgreSQL tsvector/tsquery | PostgreSQL built-in | Full-text search on event messages (SEARCH-02) | Standard PostgreSQL full-text search |
| PostgreSQL JSONB operators | PostgreSQL built-in | Tag filtering with `@>` containment (SEARCH-03) | Existing GIN index on events.tags |
| PipelineRegistry | Existing service | Pool handle lookup for HTTP handlers | Established pattern -- `Process.whereis("mesher_registry")` |

### Supporting
| Component | Detail | Purpose | When to Use |
|-----------|--------|---------|-------------|
| String.from(Int) | Built-in | Convert integer counts/offsets to String for SQL params | Pagination cursor, counts |
| String.to_int | Built-in | Parse pagination limit from query params | Default handling |
| String.length | Built-in | Check if optional filter params are non-empty | Conditional WHERE clause building |
| String.contains | Built-in | Basic substring matching if needed | Fallback search |
| List.map | Built-in | Transform query result rows into JSON strings | Response serialization |
| List.length | Built-in | Check result set size | Empty result handling |
| Map.get | Built-in | Extract fields from query result rows | Row -> struct/JSON construction |
| Map.has_key | Built-in | Check if optional JSON fields exist | Safe field extraction |
| HTTP.response | Built-in | Build HTTP responses (status + body) | All endpoints |
| HTTP.response_with_headers | Built-in (Phase 88) | Add Content-Type and pagination headers | List endpoints with cursor info |

### No New Runtime Extensions Required

All required functionality exists in the current Mesh runtime and PostgreSQL. Query string parsing (Request.query), all HTTP methods (GET/POST/PUT/DELETE), JSON encoding, and string operations are all available. No Rust runtime modifications needed.

## Architecture Patterns

### Recommended Project Structure
```
mesher/
  api/
    search.mpl                 # NEW: SEARCH-01..05 endpoints (issue filtering, event search, tag filter, pagination)
    dashboard.mpl              # NEW: DASH-01..06 endpoints (aggregation queries, health summary)
    detail.mpl                 # NEW: DETAIL-01..06 endpoints (event detail, stack traces, breadcrumbs, nav)
    team.mpl                   # NEW: ORG-04, ORG-05 endpoints (team management, API tokens)
  storage/
    queries.mpl                # EXTEND: add search queries, dashboard aggregation queries, event detail queries
    schema.mpl                 # EXTEND: add tsvector column + index on events, add message_tsv trigger
  types/
    (existing types unchanged)
  ingestion/
    routes.mpl                 # MINOR: may add imports but largely unchanged
  main.mpl                     # EXTEND: register new API routes
```

### Pattern 1: Query String Parameter Extraction with Defaults
**What:** Extract optional filter parameters from query string with fallback defaults
**When to use:** Every list/search endpoint
**Example:**
```mesh
# Extract filter parameters with defaults.
# Request.query returns Option<String> -- use case match for default.
fn get_status_filter(request) -> String do
  let opt = Request.query(request, "status")
  case opt do
    Some(s) -> s
    None -> "unresolved"
  end
end

fn get_limit(request) -> Int do
  let opt = Request.query(request, "limit")
  case opt do
    Some(s) ->
      let parsed = String.to_int(s)
      case parsed do
        Some(n) -> if n > 100 do 100 else n end
        None -> 25
      end
    None -> 25
  end
end
```

### Pattern 2: Conditional SQL WHERE Clause Construction
**What:** Build SQL queries dynamically by appending WHERE conditions based on which filter params are present
**When to use:** SEARCH-01 (multi-filter issue listing), SEARCH-03 (tag filtering)
**Why:** Mesh has no query builder library. SQL strings must be constructed via concatenation.
**Example:**
```mesh
# Build WHERE clause for issue filtering.
# Mesh has no mutable variables -- thread SQL string through let bindings.
# Each filter appends an AND condition if non-empty.
fn build_issue_query(project_id :: String, status :: String, level :: String, assigned_to :: String) -> String do
  let base = "SELECT id::text, project_id::text, fingerprint, title, level, status, event_count::text, first_seen::text, last_seen::text, COALESCE(assigned_to::text, '') as assigned_to FROM issues WHERE project_id = '" <> project_id <> "'::uuid"
  let q1 = if String.length(status) > 0 do base <> " AND status = '" <> status <> "'" else base end
  let q2 = if String.length(level) > 0 do q1 <> " AND level = '" <> level <> "'" else q1 end
  let q3 = if String.length(assigned_to) > 0 do q2 <> " AND assigned_to = '" <> assigned_to <> "'::uuid" else q2 end
  q3 <> " ORDER BY last_seen DESC"
end
```

**IMPORTANT: SQL injection concern.** The above pattern inlines user input directly into SQL. Since Mesh's Pool.query supports parameterized queries ($1, $2, etc.), a safer pattern uses a fixed set of parameters:
```mesh
# Safer: use CASE expressions in SQL to conditionally apply filters
# This avoids SQL injection by never interpolating user input into SQL strings.
fn list_issues_filtered(pool :: PoolHandle, project_id :: String, status :: String, level :: String, assigned_to :: String, limit_str :: String) do
  let sql = "SELECT id::text, project_id::text, fingerprint, title, level, status, event_count::text, first_seen::text, last_seen::text, COALESCE(assigned_to::text, '') as assigned_to FROM issues WHERE project_id = $1::uuid AND ($2 = '' OR status = $2) AND ($3 = '' OR level = $3) AND ($4 = '' OR assigned_to = $4::uuid) ORDER BY last_seen DESC LIMIT $5::int"
  Pool.query(pool, sql, [project_id, status, level, assigned_to, limit_str])
end
```

### Pattern 3: Keyset Pagination
**What:** Cursor-based pagination using WHERE (sort_col, id) < ($cursor, $cursor_id) for stable pagination under concurrent inserts
**When to use:** SEARCH-05 (issue and event lists)
**Example:**
```mesh
# Keyset pagination for issues ordered by last_seen DESC.
# Client sends ?cursor=2026-02-14T10:00:00Z&cursor_id=abc-123
# Next page: WHERE (last_seen, id) < ($cursor, $cursor_id)
fn list_issues_paginated(pool :: PoolHandle, project_id :: String, status :: String, cursor :: String, cursor_id :: String, limit_str :: String) do
  let sql = if String.length(cursor) > 0 do
    "SELECT id::text, project_id::text, fingerprint, title, level, status, event_count::text, first_seen::text, last_seen::text, COALESCE(assigned_to::text, '') as assigned_to FROM issues WHERE project_id = $1::uuid AND ($2 = '' OR status = $2) AND (last_seen, id) < ($3::timestamptz, $4::uuid) ORDER BY last_seen DESC, id DESC LIMIT $5::int"
  else
    "SELECT id::text, project_id::text, fingerprint, title, level, status, event_count::text, first_seen::text, last_seen::text, COALESCE(assigned_to::text, '') as assigned_to FROM issues WHERE project_id = $1::uuid AND ($2 = '' OR status = $2) ORDER BY last_seen DESC, id DESC LIMIT $5::int"
  end
  Pool.query(pool, sql, [project_id, status, cursor, cursor_id, limit_str])
end
```

### Pattern 4: Dashboard Aggregation Queries
**What:** Use PostgreSQL date_trunc, COUNT, GROUP BY for dashboard data
**When to use:** DASH-01 through DASH-06
**Example:**
```mesh
# DASH-01: Event volume over time (hourly buckets, last 24h default)
fn event_volume_hourly(pool :: PoolHandle, project_id :: String) do
  Pool.query(pool, "SELECT date_trunc('hour', received_at)::text AS bucket, count(*)::text AS count FROM events WHERE project_id = $1::uuid AND received_at > now() - interval '24 hours' GROUP BY bucket ORDER BY bucket", [project_id])
end

# DASH-02: Error breakdown by level
fn error_breakdown(pool :: PoolHandle, project_id :: String) do
  Pool.query(pool, "SELECT level, count(*)::text AS count FROM events WHERE project_id = $1::uuid AND received_at > now() - interval '24 hours' GROUP BY level ORDER BY count DESC", [project_id])
end

# DASH-03: Top issues by frequency
fn top_issues(pool :: PoolHandle, project_id :: String) do
  Pool.query(pool, "SELECT i.id::text, i.title, i.level, i.status, i.event_count::text, i.last_seen::text FROM issues i WHERE i.project_id = $1::uuid AND i.status = 'unresolved' ORDER BY i.event_count DESC LIMIT 10", [project_id])
end

# DASH-06: Project health summary
fn project_health(pool :: PoolHandle, project_id :: String) do
  Pool.query(pool, "SELECT (SELECT count(*)::text FROM issues WHERE project_id = $1::uuid AND status = 'unresolved') AS unresolved_count, (SELECT count(*)::text FROM events WHERE project_id = $1::uuid AND received_at > now() - interval '24 hours') AS events_24h, (SELECT count(*)::text FROM issues WHERE project_id = $1::uuid AND first_seen > now() - interval '24 hours') AS new_today", [project_id])
end
```

### Pattern 5: Full-Text Search via tsvector (SEARCH-02)
**What:** Add a tsvector column on events for full-text search of event messages
**When to use:** SEARCH-02 (full-text search)
**Requires:** Schema migration to add the column and trigger
**Example:**
```sql
-- Schema migration (in create_schema)
ALTER TABLE events ADD COLUMN IF NOT EXISTS message_tsv tsvector
  GENERATED ALWAYS AS (to_tsvector('english', message)) STORED;
CREATE INDEX IF NOT EXISTS idx_events_message_tsv ON events USING GIN(message_tsv);
```
```mesh
# Search events by message text
fn search_events(pool :: PoolHandle, project_id :: String, query :: String, limit_str :: String) do
  Pool.query(pool, "SELECT id::text, project_id::text, issue_id::text, level, message, fingerprint, received_at::text FROM events WHERE project_id = $1::uuid AND message_tsv @@ plainto_tsquery('english', $2) ORDER BY received_at DESC LIMIT $3::int", [project_id, query, limit_str])
end
```

**NOTE:** Generated columns require PostgreSQL 12+. Since we use uuidv7() (which requires pg_uuidv7 extension), the PostgreSQL version is modern enough. An alternative is a trigger-based approach if generated columns cause issues with partitioned tables.

**IMPORTANT PARTITIONING CONCERN:** The events table is partitioned by received_at range. Generated columns may not propagate to partitions correctly in all PostgreSQL versions. A safer approach is to add the tsvector column to the CREATE TABLE definition (so new partitions inherit it) and use a separate UPDATE for existing partitions. Alternatively, skip the stored column entirely and use `to_tsvector('english', message)` inline in the WHERE clause -- this is slower but avoids schema migration complexity. The GIN index on the tsvector column provides the performance benefit. For this phase, the inline approach (`WHERE to_tsvector('english', message) @@ plainto_tsquery(...)`) is simplest and avoids partition complications. An expression index can be added later if needed.

### Pattern 6: Event Detail with Navigation (DETAIL-01 through DETAIL-06)
**What:** Fetch full event payload including JSONB fields, with next/previous navigation within an issue
**When to use:** DETAIL-05 (event navigation)
**Example:**
```mesh
# Fetch full event details by ID
fn get_event_detail(pool :: PoolHandle, event_id :: String) do
  Pool.query(pool, "SELECT id::text, project_id::text, issue_id::text, level, message, fingerprint, COALESCE(exception::text, '{}') AS exception, COALESCE(stacktrace::text, '[]') AS stacktrace, COALESCE(breadcrumbs::text, '[]') AS breadcrumbs, COALESCE(tags::text, '{}') AS tags, COALESCE(extra::text, '{}') AS extra, COALESCE(user_context::text, '{}') AS user_context, COALESCE(sdk_name, '') AS sdk_name, COALESCE(sdk_version, '') AS sdk_version, received_at::text FROM events WHERE id = $1::uuid", [event_id])
end

# DETAIL-05: Get next/previous event IDs within an issue
fn get_adjacent_events(pool :: PoolHandle, issue_id :: String, event_id :: String, received_at :: String) do
  Pool.query(pool, "SELECT (SELECT id::text FROM events WHERE issue_id = $1::uuid AND (received_at, id) > ($3::timestamptz, $2::uuid) ORDER BY received_at, id LIMIT 1) AS next_id, (SELECT id::text FROM events WHERE issue_id = $1::uuid AND (received_at, id) < ($3::timestamptz, $2::uuid) ORDER BY received_at DESC, id DESC LIMIT 1) AS prev_id", [issue_id, event_id, received_at])
end
```

### Pattern 7: Manual JSON Response Serialization
**What:** Build JSON response strings via string concatenation for structs
**When to use:** All response bodies (Json.encode does not reliably work cross-module)
**Example:**
```mesh
# Serialize an event row (Map<String, String>) to JSON string.
# JSONB fields (exception, stacktrace, etc.) are already valid JSON strings
# from PostgreSQL -- embed them directly without quoting.
fn event_to_json(row) -> String do
  let id = Map.get(row, "id")
  let level = Map.get(row, "level")
  let message = Map.get(row, "message")
  let exception = Map.get(row, "exception")
  let stacktrace = Map.get(row, "stacktrace")
  let breadcrumbs = Map.get(row, "breadcrumbs")
  let tags = Map.get(row, "tags")
  let user_context = Map.get(row, "user_context")
  let received_at = Map.get(row, "received_at")
  "{\"id\":\"" <> id <> "\",\"level\":\"" <> level <> "\",\"message\":\"" <> message <> "\",\"exception\":" <> exception <> ",\"stacktrace\":" <> stacktrace <> ",\"breadcrumbs\":" <> breadcrumbs <> ",\"tags\":" <> tags <> ",\"user_context\":" <> user_context <> ",\"received_at\":\"" <> received_at <> "\"}"
end
```

**IMPORTANT:** JSONB fields from PostgreSQL arrive as valid JSON strings (e.g., `{"key":"value"}` or `[{...}]`). These should be embedded directly without additional quoting. String fields (id, level, message, received_at) need JSON string quoting with `\"`.

### Pattern 8: Route Handler Structure (Established Pattern)
**What:** Each handler: get registry -> get pool -> extract params -> query -> serialize -> respond
**When to use:** Every new endpoint
**Example:**
```mesh
pub fn handle_search_events(request) do
  let reg_pid = Process.whereis("mesher_registry")
  let pool = PipelineRegistry.get_pool(reg_pid)
  let project_id = Request.param(request, "project_id")
  let query_opt = Request.query(request, "q")
  let query = case query_opt do Some(q) -> q; None -> "" end
  if String.length(query) == 0 do
    HTTP.response(400, "{\"error\":\"missing search query\"}")
  else
    let result = search_events_query(pool, project_id, query, "25")
    case result do
      Ok(rows) -> HTTP.response(200, rows_to_json_array(rows))
      Err(e) -> HTTP.response(500, "{\"error\":\"" <> e <> "\"}")
    end
  end
end
```

### Anti-Patterns to Avoid
- **SQL string interpolation with user input:** NEVER inline query string parameters directly into SQL. Always use parameterized queries ($1, $2, etc.) with Pool.query. Use SQL-side conditionals (`$2 = '' OR status = $2`) for optional filters.
- **Offset-based pagination:** Use keyset (cursor) pagination, not OFFSET. OFFSET is O(n) and produces inconsistent results under concurrent writes.
- **Json.encode for cross-module structs:** The `ToJson__to_json__TypeName` function may not be in known_functions when compiling a module that imports the type. Use manual JSON serialization via string concatenation (established pattern from routes.mpl).
- **Complex case expressions in handlers:** Per decision [88-02], extract multi-line logic into helper functions. Keep handler bodies simple.
- **Returning raw DB rows:** Always serialize through explicit JSON construction. Map<String, String> from Pool.query cannot be directly Json.encode'd into proper nested JSON.
- **Forgetting ::text casts on UUID columns:** Per decision [87-01], always cast UUID columns to ::text in SELECT for compatibility with Map<String, String> return type.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Full-text search | Custom LIKE/ILIKE matching | PostgreSQL tsvector + plainto_tsquery | Handles stemming, ranking, word boundaries |
| Tag filtering | Iterate events and check tags in Mesh | PostgreSQL `tags @> '{"key":"value"}'::jsonb` + GIN index | Leverages existing GIN index (idx_events_tags), O(log n) |
| Time bucketing | Manual date math in Mesh | PostgreSQL `date_trunc('hour', received_at)` + GROUP BY | Handles timezone, DST, edge cases |
| Cursor encoding | Base64 encode cursor in Mesh | Raw timestamp + ID pair in query params | Simpler, no encoding needed, cursor_id is a UUID string |
| JSON serialization | Try Json.encode cross-module | Manual string concatenation | Cross-module to_json resolution unreliable |
| API key generation | Random string in Mesh | PostgreSQL `encode(gen_random_bytes(24), 'hex')` | No random/crypto in Mesh runtime; existing pattern from queries.mpl |
| Role validation | Complex Mesh-level checks | PostgreSQL CHECK constraint or SQL WHERE clause | DB enforces valid roles |

**Key insight:** PostgreSQL does the heavy lifting for search, filtering, aggregation, and pagination. Mesh code is a thin HTTP handler layer that extracts parameters, builds parameterized SQL, calls Pool.query, and serializes results to JSON. Most business logic lives in SQL.

## Common Pitfalls

### Pitfall 1: Query String Parameters Not Available
**What goes wrong:** Assuming Request.query doesn't work because decision [89-02] says "Mesh lacks query string parsing."
**Why it happens:** This decision was incorrect or overly cautious. Request.query IS fully implemented at all pipeline stages (typechecker, codegen, runtime).
**How to avoid:** Use `Request.query(request, "param_name")` which returns `Option<String>`. Match on Some/None for defaults.
**Warning signs:** Working around with POST bodies or path segments when query params would be cleaner.

### Pitfall 2: SQL Injection via String Concatenation
**What goes wrong:** Building SQL WHERE clauses by concatenating user-supplied query string values directly into SQL strings.
**Why it happens:** The temptation to do `"WHERE status = '" <> status <> "'"` is strong when building dynamic queries.
**How to avoid:** Use parameterized queries exclusively. For optional filters, use the SQL pattern: `AND ($N = '' OR column = $N)`. Pass empty string for unused filters. All user input goes through Pool.query's parameter binding.
**Warning signs:** Any SQL string containing `<>` concatenation with user-supplied values.

### Pitfall 3: tsvector on Partitioned Table
**What goes wrong:** Adding a GENERATED ALWAYS AS column to the partitioned events table fails or doesn't propagate to existing partitions.
**Why it happens:** PostgreSQL generated columns on partitioned tables have version-specific behavior. New partitions inherit the column, but existing partitions may not.
**How to avoid:** Two options: (A) Use inline `to_tsvector('english', message)` in WHERE clause without a stored column (simpler, slightly slower), or (B) Add the tsvector column to the base table definition and create an expression index. Option A is recommended for Phase 91 -- add an expression index later if search performance becomes an issue.
**Warning signs:** Search queries returning no results on older partitions; ALTER TABLE errors on partitioned tables.

### Pitfall 4: Cross-Module Json.encode Failure
**What goes wrong:** Calling `Json.encode(issue)` where Issue is imported from Types.Issue produces incorrect JSON (missing fields or raw pointer value).
**Why it happens:** The `ToJson__to_json__Issue` function is generated in the Types.Issue module's compilation unit. When compiling a different module (e.g., api/search.mpl), this function may not be in `known_functions`, causing `Json.encode` to fall through to the raw `mesh_json_encode` runtime call which doesn't know struct field layout.
**How to avoid:** Always use manual JSON string concatenation for response bodies, following the `issue_to_json_str` pattern established in routes.mpl.
**Warning signs:** JSON responses with numeric values instead of objects, or empty `{}` for struct fields.

### Pitfall 5: JSONB Fields Need Special JSON Serialization
**What goes wrong:** Double-encoding JSONB fields. For example, `exception` from PostgreSQL is already a valid JSON string like `{"type_name":"TypeError","value":"..."}`. Wrapping it in quotes produces `"{\"type_name\":...}"` (a string containing JSON, not a JSON object).
**Why it happens:** Treating all Map.get results as String fields that need quoting.
**How to avoid:** JSONB fields (exception, stacktrace, breadcrumbs, tags, extra, user_context) arrive from PostgreSQL as raw JSON text. Embed them directly in the response JSON without additional quoting. Only non-JSON String fields (id, level, message, received_at) get `\"` quoting.
**Warning signs:** Frontend receives strings instead of objects for nested fields.

### Pitfall 6: Keyset Pagination with Missing/Invalid Cursors
**What goes wrong:** Passing an invalid UUID or timestamp as cursor causes a PostgreSQL error. Not handling the "no cursor" case leads to queries returning zero results.
**Why it happens:** First page request has no cursor; cursor values from previous response must be carefully extracted.
**How to avoid:** Always check if cursor params are empty/absent. First page uses no cursor condition. Return next_cursor values from each response. Use PostgreSQL CAST with error handling or validate cursor format in SQL.
**Warning signs:** First page returns no results; pagination skips items; PostgreSQL errors about invalid UUID format.

### Pitfall 7: Default Time Range for Searches and Filters
**What goes wrong:** Queries without a time range scan all partitions, causing slow responses.
**Why it happens:** The events table is partitioned by received_at. Without a time filter, PostgreSQL must scan all partitions.
**How to avoid:** SEARCH-04 requires defaulting to last 24 hours. Always include `AND received_at > now() - interval '24 hours'` (or user-specified range) in event queries. This enables partition pruning.
**Warning signs:** Slow query responses; full table scans in EXPLAIN.

### Pitfall 8: Event ID Lookups on Partitioned Table
**What goes wrong:** `WHERE id = $1::uuid` on the partitioned events table scans ALL partitions because id alone doesn't enable partition pruning.
**Why it happens:** Events are partitioned by received_at, but id is not the partition key. The primary key is (id, received_at).
**How to avoid:** For event detail lookups (DETAIL-01), include received_at in the WHERE clause if available, or accept that a lookup by id alone may scan multiple partitions (this is acceptable for individual detail views). Alternatively, the event list response should include received_at so the detail link can include it.
**Warning signs:** Event detail queries are slow despite index.

### Pitfall 9: Helper Function Ordering (Define-Before-Use)
**What goes wrong:** Compiler error about undefined function when a handler calls a helper that is defined below it.
**Why it happens:** Mesh requires define-before-use (decision [90-03]).
**How to avoid:** Order functions bottom-up: leaf helpers first, then callers, then pub handlers last.
**Warning signs:** Compilation errors about unknown functions.

## Code Examples

### Full Issue Listing with Filters and Pagination
```mesh
# Helper: extract optional query param with default
fn query_or_default(request, param :: String, default :: String) -> String do
  let opt = Request.query(request, param)
  case opt do
    Some(v) -> v
    None -> default
  end
end

# Query: list issues with optional filters and keyset pagination
pub fn list_issues_filtered(pool :: PoolHandle, project_id :: String, status :: String, level :: String, cursor :: String, cursor_id :: String, limit_str :: String) do
  if String.length(cursor) > 0 do
    Pool.query(pool, "SELECT id::text, project_id::text, fingerprint, title, level, status, event_count::text, first_seen::text, last_seen::text, COALESCE(assigned_to::text, '') as assigned_to FROM issues WHERE project_id = $1::uuid AND ($2 = '' OR status = $2) AND ($3 = '' OR level = $3) AND (last_seen, id) < ($4::timestamptz, $5::uuid) ORDER BY last_seen DESC, id DESC LIMIT $6::int", [project_id, status, level, cursor, cursor_id, limit_str])
  else
    Pool.query(pool, "SELECT id::text, project_id::text, fingerprint, title, level, status, event_count::text, first_seen::text, last_seen::text, COALESCE(assigned_to::text, '') as assigned_to FROM issues WHERE project_id = $1::uuid AND ($2 = '' OR status = $2) AND ($3 = '' OR level = $3) ORDER BY last_seen DESC, id DESC LIMIT $4::int", [project_id, status, level, limit_str])
  end
end
```

### Dashboard Health Summary (DASH-06)
```mesh
# Single query returning all health metrics
fn project_health_summary(pool :: PoolHandle, project_id :: String) do
  Pool.query(pool, "SELECT (SELECT count(*) FROM issues WHERE project_id = $1::uuid AND status = 'unresolved')::text AS unresolved_count, (SELECT count(*) FROM events WHERE project_id = $1::uuid AND received_at > now() - interval '24 hours')::text AS events_24h, (SELECT count(*) FROM issues WHERE project_id = $1::uuid AND first_seen > now() - interval '24 hours')::text AS new_today", [project_id])
end
```

### Tag-Based Event Filtering (SEARCH-03)
```mesh
# Filter events by tag key-value pair using JSONB containment operator.
# The @> operator leverages the existing GIN index on events.tags.
fn filter_events_by_tag(pool :: PoolHandle, project_id :: String, tag_key :: String, tag_value :: String, limit_str :: String) do
  let tag_json = "{\"" <> tag_key <> "\":\"" <> tag_value <> "\"}"
  Pool.query(pool, "SELECT id::text, issue_id::text, level, message, tags::text, received_at::text FROM events WHERE project_id = $1::uuid AND tags @> $2::jsonb AND received_at > now() - interval '24 hours' ORDER BY received_at DESC LIMIT $3::int", [project_id, tag_json, limit_str])
end
```

### Full-Text Search (SEARCH-02)
```mesh
# Full-text search using PostgreSQL plainto_tsquery.
# Uses inline tsvector computation (avoids generated column on partitioned table).
fn search_events_fulltext(pool :: PoolHandle, project_id :: String, search_query :: String, limit_str :: String) do
  Pool.query(pool, "SELECT id::text, issue_id::text, level, message, received_at::text, ts_rank(to_tsvector('english', message), plainto_tsquery('english', $2))::text AS rank FROM events WHERE project_id = $1::uuid AND to_tsvector('english', message) @@ plainto_tsquery('english', $2) AND received_at > now() - interval '24 hours' ORDER BY rank DESC, received_at DESC LIMIT $3::int", [project_id, search_query, limit_str])
end
```

### Event Navigation (DETAIL-05)
```mesh
# Get next and previous event IDs within an issue for navigation.
fn get_event_neighbors(pool :: PoolHandle, issue_id :: String, current_received_at :: String, current_id :: String) do
  Pool.query(pool, "SELECT (SELECT id::text FROM events WHERE issue_id = $1::uuid AND (received_at, id) > ($2::timestamptz, $3::uuid) ORDER BY received_at, id LIMIT 1) AS next_id, (SELECT id::text FROM events WHERE issue_id = $1::uuid AND (received_at, id) < ($2::timestamptz, $3::uuid) ORDER BY received_at DESC, id DESC LIMIT 1) AS prev_id", [issue_id, current_received_at, current_id])
end
```

### Team Membership Management (ORG-04)
```mesh
# Update member role. Validates role is one of: owner, admin, member.
fn update_member_role(pool :: PoolHandle, membership_id :: String, new_role :: String) do
  Pool.execute(pool, "UPDATE org_memberships SET role = $2 WHERE id = $1::uuid AND $2 IN ('owner', 'admin', 'member')", [membership_id, new_role])
end

# Remove member from organization
fn remove_member(pool :: PoolHandle, membership_id :: String) do
  Pool.execute(pool, "DELETE FROM org_memberships WHERE id = $1::uuid", [membership_id])
end

# List API keys for a project
fn list_api_keys(pool :: PoolHandle, project_id :: String) do
  Pool.query(pool, "SELECT id::text, project_id::text, key_value, label, created_at::text, revoked_at::text FROM api_keys WHERE project_id = $1::uuid ORDER BY created_at DESC", [project_id])
end
```

### JSON Array Serialization Helper (Reusable Pattern)
```mesh
# Generic recursive JSON array builder from a list of strings (each a JSON object).
fn json_array_loop(items, i :: Int, total :: Int, acc :: String) -> String do
  if i < total do
    let item = List.get(items, i)
    let new_acc = if i > 0 do acc <> "," <> item else item end
    json_array_loop(items, i + 1, total, new_acc)
  else
    "[" <> acc <> "]"
  end
end

fn to_json_array(items) -> String do
  json_array_loop(items, 0, List.length(items), "")
end
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Single hardcoded issue list endpoint | Filtered, paginated issue listing with query params | Phase 91 | Full search and browse capability |
| No event detail view | Full event detail with JSONB fields, stack traces, breadcrumbs | Phase 91 | Users can investigate individual errors |
| No dashboard aggregations | PostgreSQL-based time bucketing, level breakdown, health metrics | Phase 91 | Dashboard data for monitoring overview |
| No full-text search | PostgreSQL tsvector-based event message search | Phase 91 | Users can find events by message content |
| No tag filtering | JSONB containment operator with GIN index | Phase 91 | Filter events by environment, release, etc. |
| No team management API | Membership CRUD with role enforcement | Phase 91 | Collaborative error monitoring |
| API token create only | Full API token lifecycle (create, list, revoke) | Phase 91 | Token rotation and management |

**Current state (pre-Phase 91):**
- One issue list endpoint: GET /api/v1/projects/:project_id/issues (hardcoded status=unresolved, no pagination)
- Issue state transitions: resolve, archive, unresolve, assign, discard, delete (POST endpoints)
- Event ingestion: POST /api/v1/events, POST /api/v1/events/bulk
- No event detail, search, dashboard, or team management endpoints
- OrgService, ProjectService, UserService exist but have no HTTP routes
- Database schema has all tables and indexes needed; events table is partitioned by received_at
- GIN index on events.tags already exists (idx_events_tags)
- No tsvector column or index on events.message

## Open Questions

1. **Event Detail by ID vs by ID+received_at**
   - What we know: The events table is partitioned by received_at. The primary key is (id, received_at). Looking up by id alone requires scanning all partitions.
   - What's unclear: Should the event detail API require both event_id and received_at (or a time hint), or just event_id?
   - Recommendation: Accept both. The event list responses include received_at, so detail links can include it. If only event_id is provided, query without received_at constraint (slower but correct). The API signature: GET /api/v1/events/:event_id?received_at=...

2. **Authentication for REST API Endpoints**
   - What we know: Ingestion uses API key auth (X-Sentry-Auth header -> project lookup). Dashboard/REST API endpoints need user session auth, not API key auth.
   - What's unclear: Should REST API endpoints use session tokens (from UserService.login) or API keys? Different endpoints may need different auth.
   - Recommendation: Use session token auth (validate_session) for user-facing endpoints (dashboard, team management). Use API key auth for programmatic access (ORG-05). Extract token from Authorization header, validate via validate_session query, get user_id. Check org_membership for authorization.

3. **JSON Response for Event JSONB Fields**
   - What we know: JSONB columns (exception, stacktrace, breadcrumbs, tags, extra, user_context) from Pool.query arrive as JSON-formatted strings. They should be embedded raw (not quoted) in the response JSON.
   - What's unclear: What if a JSONB field is NULL in the database? COALESCE to '{}' or '[]' handles this, but the calling code must know which default to use.
   - Recommendation: Use COALESCE with appropriate defaults in the SQL: `COALESCE(exception::text, 'null')`, `COALESCE(stacktrace::text, '[]')`, `COALESCE(breadcrumbs::text, '[]')`, `COALESCE(tags::text, '{}')`, etc.

4. **Time Range Parameter Format**
   - What we know: SEARCH-04 requires a default 24-hour time range. Users should be able to override this.
   - What's unclear: What format for user-specified time ranges? ISO 8601 timestamps? Relative durations (1h, 7d)?
   - Recommendation: Accept ISO 8601 timestamps via query params `?start=...&end=...`. Default start to `now() - interval '24 hours'`, default end to `now()`. PostgreSQL handles ISO 8601 parsing natively. Keep it simple: pass timestamps directly as SQL parameters.

5. **Pagination Response Format**
   - What we know: Keyset pagination needs cursor values in the response for the client to request the next page.
   - What's unclear: Standard format for pagination metadata.
   - Recommendation: Include pagination metadata in the JSON response: `{"data":[...], "next_cursor": "2026-02-14T10:00:00Z", "next_cursor_id": "abc-123", "has_more": true}`. The cursor is the last item's sort column value and ID. `has_more` is true when the result set equals the limit (meaning there may be more items).

## Sources

### Primary (HIGH confidence)
- `/Users/sn0w/Documents/dev/snow/mesher/main.mpl` -- Current route registration, service startup
- `/Users/sn0w/Documents/dev/snow/mesher/ingestion/routes.mpl` -- Existing HTTP handler patterns, JSON serialization approach, broadcast helpers
- `/Users/sn0w/Documents/dev/snow/mesher/storage/queries.mpl` -- All existing query patterns, struct construction from Map rows
- `/Users/sn0w/Documents/dev/snow/mesher/storage/schema.mpl` -- Full database schema DDL, existing indexes, partition management
- `/Users/sn0w/Documents/dev/snow/mesher/types/issue.mpl` -- Issue struct with deriving(Json, Row)
- `/Users/sn0w/Documents/dev/snow/mesher/types/event.mpl` -- Event struct (all String fields for Row), JSONB column layout
- `/Users/sn0w/Documents/dev/snow/mesher/types/user.mpl` -- User, OrgMembership, Session structs
- `/Users/sn0w/Documents/dev/snow/mesher/types/project.mpl` -- Organization, Project, ApiKey structs
- `/Users/sn0w/Documents/dev/snow/mesher/services/user.mpl` -- UserService call handlers (existing but no HTTP routes)
- `/Users/sn0w/Documents/dev/snow/mesher/services/project.mpl` -- ProjectService call handlers (existing but no HTTP routes)
- `/Users/sn0w/Documents/dev/snow/mesher/services/org.mpl` -- OrgService call handlers (existing but no HTTP routes)
- `/Users/sn0w/Documents/dev/snow/crates/mesh-typeck/src/infer.rs` (lines 549-552) -- Request.query type signature: `fn(Request, String) -> Option<String>` -- CONFIRMED IMPLEMENTED
- `/Users/sn0w/Documents/dev/snow/crates/mesh-rt/src/http/server.rs` (line 219) -- mesh_http_request_query runtime implementation -- CONFIRMED WORKING
- `/Users/sn0w/Documents/dev/snow/crates/mesh-typeck/src/infer.rs` (lines 519-526) -- HTTP.on_put and HTTP.on_delete ARE in typechecker (available if needed)
- `/Users/sn0w/Documents/dev/snow/.planning/STATE.md` -- All accumulated decisions from phases 87-90

### Secondary (MEDIUM confidence)
- `/Users/sn0w/Documents/dev/snow/.planning/phases/89-error-grouping-issue-lifecycle/89-RESEARCH.md` -- Issue lifecycle patterns, SQL upsert, fingerprint computation
- `/Users/sn0w/Documents/dev/snow/.planning/phases/90-real-time-streaming/90-RESEARCH.md` -- WebSocket patterns, broadcast architecture
- `/Users/sn0w/Documents/dev/snow/.planning/phases/88-ingestion-pipeline/88-RESEARCH.md` -- HTTP handler patterns, runtime extension requirements

### Tertiary (LOW confidence)
- Decision [89-02] "Mesh lacks query string parsing" -- **INCORRECT**: Request.query is fully implemented. This decision should be superseded.

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- all components verified against actual Mesh runtime codebase and typechecker
- Architecture: HIGH -- patterns derived from existing working Phase 88-90 code and established Mesh conventions
- Query patterns: HIGH -- PostgreSQL aggregation functions, tsvector search, JSONB operators are standard and well-documented
- Pagination: HIGH -- keyset pagination is a well-established pattern; PostgreSQL (last_seen, id) comparison works correctly
- Pitfalls: HIGH -- identified from direct analysis of codebase, cross-module limitations verified in codegen, partitioning concerns from schema analysis
- Request.query availability: HIGH -- verified at three pipeline stages (typechecker, codegen/lowering, runtime implementation)
- Cross-module Json.encode: MEDIUM -- inferred from codegen analysis (known_functions check at lower.rs line 5808); may work in some cases but safest to avoid

**Research date:** 2026-02-14
**Valid until:** 2026-03-14 (stable -- Mesh runtime changes are controlled by this project)
