# Phase 103 Research: Refactor ORM to Eliminate Raw SQL

## Objective

Eliminate all remaining `Pool.query` and `Pool.execute` calls from Mesher application code by extending the ORM/Query builder to express every SQL pattern currently written as raw strings. After this phase, all database access in Mesher should flow through `Repo.*` / `Query.*` APIs (except migrations and partition DDL which are inherently raw SQL).

---

## Current State After Phase 102

Phase 102 converted simple CRUD to ORM but explicitly deferred complex patterns:

- **102-02 decision:** "ORM conversions only for simple CRUD; raw SQL kept for PG functions, JOINs, casts, conditional updates, analytics."
- **102-03 decision:** "JSONB utility Pool.query calls preserved in 5 non-storage files (they parse JSON parameters, not query tables)."
- **102-03 decision:** "writer.mpl insert_event preserved as raw Pool.execute (JSONB extraction INSERT not expressible in ORM)."

The ORM was designed to be extended. Phase 98 built the Query builder with 13 slots (including fragment/join/group/having). Phase 99 added Repo CRUD with changeset support. Phase 100 added preloading. Phase 101 added migrations.

---

## Complete Raw SQL Inventory

### A. Storage/queries.mpl -- Core Query Functions (55 raw SQL calls)

#### A1. Issue Helpers (2 calls)
| Function | SQL Pattern | Complexity |
|----------|------------|------------|
| `count_unresolved_issues` | `SELECT count(*)::text ... WHERE status = 'unresolved'` | Query builder + Repo.count with literal WHERE |
| `get_issue_project_id` | `SELECT project_id::text ... WHERE id = $1::uuid` | Repo.get with column selection |

#### A2. API Key Queries (3 calls)
| Function | SQL Pattern | Complexity |
|----------|------------|------------|
| `create_api_key` | INSERT with `gen_random_bytes(24)` PG function | PG function in VALUES |
| `get_project_by_api_key` | SELECT with JOIN on api_keys | Query.join already supported |
| `revoke_api_key` | UPDATE with `now()` | PG function in SET |

#### A3. User/Auth Queries (3 calls)
| Function | SQL Pattern | Complexity |
|----------|------------|------------|
| `create_user` | INSERT with `crypt($2, gen_salt('bf', 12))` | PG crypto functions in VALUES |
| `authenticate_user` | SELECT WHERE `crypt($2, password_hash)` | PG function in WHERE |
| (simple CRUD already on ORM) | | |

#### A4. Session Queries (3 calls)
| Function | SQL Pattern | Complexity |
|----------|------------|------------|
| `create_session` | INSERT with `encode(gen_random_bytes(32), 'hex')` | PG function in VALUES |
| `validate_session` | SELECT WHERE `expires_at > now()` | PG function comparison in WHERE |
| `delete_session` | DELETE WHERE token = $1 | Simple -- could use Query + Repo |

#### A5. Issue Management (9 calls)
| Function | SQL Pattern | Complexity |
|----------|------------|------------|
| `upsert_issue` | INSERT ... ON CONFLICT DO UPDATE ... RETURNING | UPSERT with conditional SET (CASE WHEN) |
| `is_issue_discarded` | SELECT 1 WHERE status = 'discarded' | Repo.exists with Query |
| `resolve_issue` | UPDATE WHERE status != 'resolved' | Conditional WHERE |
| `archive_issue` | UPDATE SET status WHERE id | Simple conditional UPDATE |
| `unresolve_issue` | UPDATE SET status WHERE id | Simple conditional UPDATE |
| `assign_issue` | UPDATE SET assigned_to = $2::uuid or NULL | Conditional NULL/value |
| `discard_issue` | UPDATE SET status WHERE id | Simple conditional UPDATE |
| `delete_issue` | DELETE events + DELETE issue (2 statements) | Multi-statement delete with FK |
| `list_issues_by_status` | SELECT with type casts and COALESCE | Casts + COALESCE |

#### A6. Spike Detection (1 call)
| Function | SQL Pattern | Complexity |
|----------|------------|------------|
| `check_volume_spikes` | Complex subquery with JOIN, GROUP BY, HAVING, correlated subselect | Very high -- analytics |

#### A7. Event Field Extraction (1 call)
| Function | SQL Pattern | Complexity |
|----------|------------|------------|
| `extract_event_fields` | CASE WHEN with jsonb_typeof, jsonb_array_elements, string_agg, subquery | Very high -- server-side computation |

#### A8. Search & Pagination (4 calls across 2 functions)
| Function | SQL Pattern | Complexity |
|----------|------------|------------|
| `list_issues_filtered` | Conditional WHERE with `($2 = '' OR ...)` pattern, keyset pagination | Complex conditional + tuple comparison |
| `search_events_fulltext` | `to_tsvector`, `plainto_tsquery`, `ts_rank`, interval | Full-text search with PG functions |
| `filter_events_by_tag` | `tags @> $2::jsonb` containment operator | JSONB containment |
| `list_events_for_issue` | Keyset pagination with tuple comparison | Tuple comparison WHERE |

#### A9. Dashboard Analytics (6 calls)
| Function | SQL Pattern | Complexity |
|----------|------------|------------|
| `event_volume_hourly` | `date_trunc`, GROUP BY, interval | PG time functions + aggregation |
| `error_breakdown_by_level` | GROUP BY level, count | Aggregation |
| `top_issues_by_frequency` | ORDER BY event_count DESC LIMIT | Could use Query builder |
| `event_breakdown_by_tag` | `tags->>$2`, `tags ? $2` JSONB operators | JSONB operators |
| `issue_event_timeline` | Simple SELECT with ORDER BY, LIMIT | Could use Query builder |
| `project_health_summary` | 3 correlated subselects in single SELECT | Very high -- multi-subquery |

#### A10. Event Detail (2 calls)
| Function | SQL Pattern | Complexity |
|----------|------------|------------|
| `get_event_detail` | SELECT with multiple COALESCE for JSONB fields | COALESCE expressions |
| `get_event_neighbors` | 2 correlated subselects with tuple comparison | Very high -- window navigation |

#### A11. Team/Membership (2 calls)
| Function | SQL Pattern | Complexity |
|----------|------------|------------|
| `update_member_role` | UPDATE with SQL-side role validation `$2 IN (...)` | Conditional validation in SQL |
| `get_members_with_users` | JOIN org_memberships + users | JOIN query |

#### A12. Alert System (8 calls)
| Function | SQL Pattern | Complexity |
|----------|------------|------------|
| `create_alert_rule` | INSERT with JSONB extraction from body | JSONB extraction INSERT |
| `list_alert_rules` | SELECT with type casts, COALESCE | Casts + COALESCE |
| `toggle_alert_rule` | UPDATE with `$2::boolean` cast | Boolean cast |
| `evaluate_threshold_rule` | CASE WHEN with subqueries, interval arithmetic | Very high -- analytics |
| `fire_alert` | INSERT with `jsonb_build_object` + UPDATE last_fired_at | PG function + multi-statement |
| `check_new_issue` | SELECT 1 WHERE first_seen = last_seen | Column-to-column comparison |
| `get_event_alert_rules` | SELECT WHERE `condition_json->>'condition_type' = $2` | JSONB path access in WHERE |
| `should_fire_by_cooldown` | SELECT WHERE `last_fired_at < now() - interval` | Interval arithmetic |

#### A13. Retention & Storage (6 calls)
| Function | SQL Pattern | Complexity |
|----------|------------|------------|
| `delete_expired_events` | DELETE with interval expression `($2 \|\| ' days')::interval` | Dynamic interval |
| `get_expired_partitions` | pg_inherits system catalog query with regex | System catalog + regex |
| `drop_partition` | DDL: `DROP TABLE IF EXISTS` | DDL -- migration territory |
| `get_all_project_retention` | SELECT id, retention_days FROM projects | Simple -- Query builder |
| `get_project_storage` | SELECT count(*), estimate from events | Aggregation |
| `update_project_settings` | UPDATE with JSONB extraction from body | JSONB extraction in SET |

#### A14. Settings (2 calls)
| Function | SQL Pattern | Complexity |
|----------|------------|------------|
| `get_project_settings` | SELECT retention_days, sample_rate | Simple select with columns |
| `check_sample_rate` | SELECT random() < subquery | PG function + subquery |

### B. Storage/writer.mpl (1 call)
| Function | SQL Pattern | Complexity |
|----------|------------|------------|
| `insert_event` | INSERT ... SELECT with JSONB extraction subquery | Very high -- JSONB extraction |

### C. Storage/schema.mpl (2 calls)
| Function | SQL Pattern | Complexity |
|----------|------------|------------|
| `create_partition` | DDL: CREATE TABLE ... PARTITION OF | DDL -- inherently raw |
| `create_partitions_loop` | `SELECT to_char(now() + interval)` | Date computation |

### D. Non-storage JSONB Parsing (4 calls)
| Location | SQL Pattern | Complexity |
|----------|------------|------------|
| `ingestion/routes.mpl` (handle_assign_issue) | `$1::jsonb->>'user_id'` | JSONB field extraction |
| `ingestion/ws_handler.mpl` | `$1::jsonb->'filters'->>'level'` | JSONB nested extraction |
| `ingestion/pipeline.mpl` | `$1::jsonb->>$2` | JSONB dynamic key |
| `api/alerts.mpl` | `$1::jsonb->>'enabled'` | JSONB field extraction |
| `api/team.mpl` | `$1::jsonb->>$2` | JSONB dynamic key |

### E. Migration Files (inherently raw SQL)
| Location | SQL Pattern |
|----------|------------|
| `migrations/20260216120000_create_initial_schema.mpl` | DDL: CREATE TABLE, CREATE INDEX, etc. |

---

## SQL Pattern Classification

### Tier 1: Expressible with Current Query Builder (7 functions)
These can be converted today with existing `Query.*` + `Repo.*` APIs:

1. **`count_unresolved_issues`** -- `Query.from("issues") |> Query.where(:project_id, id) |> Query.where(:status, "unresolved") |> Repo.count(pool, q)`
2. **`get_issue_project_id`** -- `Repo.get_by` then extract field
3. **`is_issue_discarded`** -- `Query.from("issues") |> Query.where(:project_id, id) |> Query.where(:fingerprint, fp) |> Query.where(:status, "discarded") |> Repo.exists(pool, q)`
4. **`top_issues_by_frequency`** -- `Query.from("issues") |> Query.where(...) |> Query.order_by(:event_count, :desc) |> Query.limit(...) |> Repo.all(pool, q)`
5. **`issue_event_timeline`** -- `Query.from("events") |> Query.where(:issue_id, id) |> Query.order_by(:received_at, :desc) |> Query.limit(...) |> Repo.all(pool, q)`
6. **`get_all_project_retention`** -- `Query.from("projects") |> Query.select([...]) |> Repo.all(pool, q)`
7. **`delete_session`** -- `Repo.delete` or Query-based delete

### Tier 2: Need Query Builder Extensions (20+ functions)
These need new Query/Repo capabilities:

**2a. Type Casts (`::uuid`, `::text`, `::int`, `::boolean`, `::timestamptz`)**
- Affects nearly every raw SQL call. The runtime passes all values as strings, and PostgreSQL needs explicit casts for UUID, integer, boolean, and timestamp types.
- **Current workaround:** Query builder does not emit type casts; `Pool.query` raw SQL includes them inline.
- **Solution:** Schema-aware casting -- the Query builder / Repo layer should know column types (from `deriving(Schema)` metadata) and emit appropriate casts automatically. OR: a `Query.cast` or param type annotation system.

**2b. PG Functions in WHERE (`now()`, `crypt()`, interval arithmetic)**
- `validate_session`: `expires_at > now()`
- `should_fire_by_cooldown`: `last_fired_at < now() - interval '1 minute' * $2::int`
- `authenticate_user`: `password_hash = crypt($2, password_hash)`
- **Solution:** `Query.fragment` already exists but is unused. These could use `Query.fragment` for the WHERE clause. Alternatively, a `Query.where_raw(clause, params)` method.

**2c. PG Functions in INSERT VALUES (`gen_random_bytes`, `crypt`, `gen_salt`, `now()`)**
- `create_api_key`: `'mshr_' || encode(gen_random_bytes(24), 'hex')`
- `create_user`: `crypt($2, gen_salt('bf', 12))`
- `create_session`: `encode(gen_random_bytes(32), 'hex')`
- **Solution:** Need a way to specify raw SQL expressions as column values in Repo.insert. E.g., `Repo.insert_raw` or a Map value wrapper indicating "this is a SQL expression, not a parameter."

**2d. COALESCE and Default Expressions in SELECT**
- `get_event_detail`: `COALESCE(exception::text, 'null')`, `COALESCE(tags::text, '{}')`
- `list_issues_by_status`: `COALESCE(assigned_to::text, '')`
- **Solution:** `Query.select_raw` for computed/aliased columns, or post-processing in MPL code (read nulls as empty strings).

**2e. Conditional WHERE Patterns**
- `list_issues_filtered`: `($2 = '' OR status = $2)` pattern for optional filters
- `resolve_issue`: `WHERE status != 'resolved'`
- `update_member_role`: SQL-side role validation
- **Solution:** Build queries conditionally in MPL code using `if` to add where clauses only when filters are non-empty. The `!= resolved` pattern needs `Query.where_op(:status, :neq, "resolved")` which already exists.

**2f. JOIN Queries**
- `get_project_by_api_key`: projects JOIN api_keys
- `get_members_with_users`: org_memberships JOIN users
- `list_alerts`: alerts JOIN alert_rules
- **Solution:** `Query.join` already exists in the runtime. The compiler already supports it. These can be converted.

**2g. GROUP BY / Aggregation**
- `event_volume_hourly`: GROUP BY date_trunc bucket
- `error_breakdown_by_level`: GROUP BY level
- `event_breakdown_by_tag`: GROUP BY tag_value
- `get_project_storage`: count(*)
- **Solution:** `Query.group_by` and `Query.having` already exist. Need `Query.select_raw` for aggregation expressions (`count(*)::text AS count`).

**2h. Keyset Pagination with Tuple Comparison**
- `list_issues_filtered`: `(last_seen, id) < ($5::timestamptz, $6::uuid)`
- `list_events_for_issue`: `(received_at, id) < ($2::timestamptz, $3::uuid)`
- **Solution:** `Query.fragment` for the tuple comparison, or a dedicated `Query.where_tuple_lt` helper.

### Tier 3: Fundamentally Complex -- Keep as Raw or Use Fragment (8 functions)
These are complex analytics/computation SQL that should remain as `Query.fragment` escape hatches or dedicated raw queries:

1. **`upsert_issue`** -- ON CONFLICT ... DO UPDATE with CASE WHEN in SET
2. **`extract_event_fields`** -- Multi-branch CASE WHEN with jsonb_typeof, jsonb_array_elements, string_agg, correlated subselect
3. **`check_volume_spikes`** -- Correlated subquery in WHERE with GROUP BY HAVING
4. **`project_health_summary`** -- 3 correlated scalar subselects in SELECT
5. **`evaluate_threshold_rule`** -- CASE WHEN with subqueries and interval arithmetic
6. **`get_event_neighbors`** -- 2 correlated subselects with tuple comparison
7. **`insert_event` (writer.mpl)** -- INSERT SELECT with JSONB extraction subquery
8. **`get_expired_partitions`** -- System catalog query with regex

### Tier 4: Utility / Non-table Queries (5 calls) -- Separate concern
The JSONB parsing calls in non-storage files (`$1::jsonb->>'key'`) are not database queries -- they use PostgreSQL as a JSON parser. These should be replaced with a Mesh-native JSON field extraction utility (e.g., a `Json.get(body, "key")` function).

### Tier 5: DDL / Migrations (inherently raw)
Migration files and partition DDL remain raw SQL by nature. Not in scope.

---

## ORM Extension Points Needed

### 1. `Query.select_raw(q, expressions)` -- Raw SELECT expressions
Allow raw SQL expressions in the SELECT clause for aggregations, casts, and aliases:
```
Query.from("events")
  |> Query.select_raw(["count(*)::text AS count", "level"])
  |> Query.group_by(:level)
```
**Runtime:** New slot or reuse `SLOT_SELECT` with a flag distinguishing raw from column-name. Simpler: store raw select strings that bypass `quote_ident`.
**Compiler:** New intrinsic `mesh_query_select_raw`.

### 2. `Query.where_raw(q, clause, params)` -- Raw WHERE fragments
Allow raw SQL in WHERE clauses for PG functions and complex comparisons:
```
Query.from("sessions")
  |> Query.where(:token, token)
  |> Query.where_raw("expires_at > now()", [])
```
**Runtime:** Append raw clause to where_clauses list with a special prefix (e.g., "RAW:...") that the SQL builder passes through without quoting.
**Compiler:** New intrinsic `mesh_query_where_raw`.

### 3. `Repo.insert_raw(pool, table, fields, raw_fields)` or expression wrappers
Allow SQL expressions as column values in INSERT:
```
Repo.insert(pool, "api_keys", %{
  "project_id" => project_id,
  "key_value" => Repo.raw("'mshr_' || encode(gen_random_bytes(24), 'hex')"),
  "label" => label
})
```
**Alternative:** `Repo.insert_with_sql(pool, table, param_fields, sql_fields)` where `sql_fields` is a Map of column -> raw SQL expression.

### 4. `Repo.update_where(pool, table, set_fields, query)` -- UPDATE with Query conditions
Allow updating rows matching a Query (not just by primary key):
```
let q = Query.from("issues") |> Query.where(:id, issue_id) |> Query.where_op(:status, :neq, "resolved")
Repo.update_where(pool, "issues", %{"status" => "resolved"}, q)
```

### 5. `Repo.delete_where(pool, table, query)` -- DELETE with Query conditions
Allow deleting rows matching a Query:
```
let q = Query.from("events") |> Query.where(:issue_id, issue_id)
Repo.delete_where(pool, "events", q)
```

### 6. `Repo.execute_raw(pool, sql, params)` -- Explicit Raw SQL Escape Hatch
Keep `Pool.query`/`Pool.execute` available but provide a Repo-level equivalent for Tier 3 queries that are intentionally complex SQL:
```
Repo.execute_raw(pool, "INSERT INTO ... ON CONFLICT ...", [params])
```
This makes the intention explicit: "this query is too complex for the query builder."

### 7. JSON Field Extraction (Mesh-native, no DB roundtrip)
Replace JSONB parsing calls with a Mesh runtime intrinsic:
```
let user_id = Json.get_string(body, "user_id")
```
**Runtime:** Parse JSON string in Rust, extract field by key. New intrinsic `mesh_json_get_string`.
**Impact:** Eliminates 5 non-storage `Pool.query` calls entirely.

---

## Architecture Decision: "All ORM" vs "ORM + Raw Escape Hatch"

### Option A: Pure ORM -- All SQL Generated by Query Builder
- Extend Query builder to handle every pattern (subqueries, UPSERT, CASE WHEN, etc.)
- Very large scope; some patterns (correlated subselects, CASE WHEN in SET) would require adding a full SQL AST to the Query builder.
- Risk: the Query builder becomes more complex than writing raw SQL.

### Option B: ORM for CRUD + Query Builder for Reads + Raw Escape Hatch for Analytics (RECOMMENDED)
- Convert Tier 1 (7 functions) to existing Query/Repo APIs immediately.
- Add Tier 2 extensions (select_raw, where_raw, insert_raw, update_where, delete_where) to handle 20+ functions.
- Keep Tier 3 (8 functions) as explicit `Repo.execute_raw` calls with clear intent.
- Replace Tier 4 (5 calls) with Mesh-native JSON parsing.
- Net result: ~50+ raw SQL calls eliminated, ~8 intentional raw SQL calls remain (documented as "too complex for ORM").

### Option C: Minimal -- Just JSON Parsing + Easy Wins
- Convert Tier 1 (7 functions) + add JSON parsing intrinsic for Tier 4 (5 calls).
- Leave everything else as raw SQL.
- Smallest scope but least value -- still 45+ raw SQL calls in queries.mpl.

**Recommendation:** Option B. It provides the best ROI. The Tier 2 extensions are reusable language features, not one-off hacks. The Tier 3 functions are genuinely complex analytics that would be worse as generated SQL.

---

## Implementation Strategy

### Plan 1: JSON Field Extraction Intrinsic
**Goal:** Eliminate all JSONB-parsing `Pool.query` calls in non-storage modules.

**Scope:**
- Add `mesh_json_get_string(json_ptr, key_ptr) -> ptr` to mesh-rt (parse JSON, extract string field)
- Add `mesh_json_get_nested(json_ptr, path_ptr) -> ptr` for nested extraction
- Register as `Json.get_string` / `Json.get_nested` intrinsics
- Rewrite 5 call sites: `ingestion/routes.mpl`, `ingestion/ws_handler.mpl`, `ingestion/pipeline.mpl`, `api/alerts.mpl`, `api/team.mpl`

**Complexity:** Low-medium. JSON parsing in Rust (serde_json or manual) is straightforward.

### Plan 2: Query Builder Extensions
**Goal:** Add `select_raw`, `where_raw`, `where_tuple_lt` to the Query builder.

**Scope:**
- Runtime: extend `repo.rs` SQL builder to handle raw select expressions and raw where clauses
- Runtime: add `mesh_query_select_raw`, `mesh_query_where_raw` extern C functions
- Compiler: register new intrinsics, add lowering in `lower.rs`
- No new slots needed: reuse fragment mechanism or add a "raw" flag to existing select/where slots

**Complexity:** Medium. Follows established patterns from Phase 98 Query builder.

### Plan 3: Repo Write Extensions
**Goal:** Add `update_where`, `delete_where`, `insert_raw`, and `execute_raw` to Repo.

**Scope:**
- Runtime: `mesh_repo_update_where(pool, table, fields, query)` -- build UPDATE SET from fields Map, WHERE from Query slots
- Runtime: `mesh_repo_delete_where(pool, table, query)` -- build DELETE WHERE from Query slots
- Runtime: `mesh_repo_insert_raw(pool, table, param_fields, raw_fields)` -- INSERT with mixed param/expression values
- Runtime: `mesh_repo_execute_raw(pool, sql, params)` -- explicit escape hatch (alias for Pool.query but namespaced under Repo)
- Compiler: register all new intrinsics

**Complexity:** Medium. INSERT with raw expressions needs careful parameterization.

### Plan 4: Convert Storage/queries.mpl to ORM
**Goal:** Rewrite all eligible functions in queries.mpl to use ORM APIs.

**Scope:**
- Tier 1 conversions (7 functions): direct Query + Repo replacements
- Tier 2 conversions (20+ functions): use new select_raw, where_raw, update_where, delete_where, insert_raw
- Tier 3 functions (8 functions): convert from Pool.query to Repo.execute_raw for consistent API
- Validate: compile and run Mesher, verify all endpoints work

**Complexity:** High (bulk rewrite). Must be careful with type casts and parameter ordering.

### Plan 5: Convert Non-queries.mpl Remaining Raw SQL
**Goal:** Eliminate remaining raw SQL outside queries.mpl.

**Scope:**
- `storage/writer.mpl` insert_event: convert to Repo.execute_raw (intentionally complex)
- `storage/schema.mpl`: convert date computation to Mesh-native (or Repo.execute_raw)
- Final audit: grep for any remaining Pool.query/Pool.execute in application code

**Complexity:** Low. Mostly mechanical.

---

## Key Files

| File | Role | Path |
|------|------|------|
| Query builder runtime | 13-slot query object, immutable pipe composition | `crates/mesh-rt/src/db/query.rs` |
| Repo runtime | SQL execution from Query objects, CRUD, preload | `crates/mesh-rt/src/db/repo.rs` |
| ORM SQL builders | Pure Rust SQL generation (SELECT/INSERT/UPDATE/DELETE) | `crates/mesh-rt/src/db/orm.rs` |
| Changeset runtime | Validation pipeline for write operations | `crates/mesh-rt/src/db/changeset.rs` |
| Intrinsics registration | Compiler function declarations for Query/Repo | `crates/mesh-codegen/src/codegen/intrinsics.rs` |
| MIR lowering | Method call -> intrinsic call translation | `crates/mesh-codegen/src/mir/lower.rs` |
| Mesher queries | All centralized SQL (main target) | `mesher/storage/queries.mpl` |
| Mesher writer | JSONB extraction INSERT | `mesher/storage/writer.mpl` |
| Mesher schema | Partition DDL | `mesher/storage/schema.mpl` |
| Non-storage JSONB calls | JSON parsing via PG | `mesher/ingestion/routes.mpl`, `mesher/ingestion/ws_handler.mpl`, `mesher/ingestion/pipeline.mpl`, `mesher/api/alerts.mpl`, `mesher/api/team.mpl` |
| Type definitions | Schema metadata for ORM | `mesher/types/*.mpl` |

---

## Risks and Mitigations

### 1. Type Cast Correctness
**Risk:** Removing `::uuid`, `::text` casts from raw SQL may break queries if PostgreSQL cannot infer parameter types.
**Mitigation:** The Repo/Query SQL builder must emit casts. The `Pool.query` PG driver already sends parameters as text and relies on PG's type inference. Test each converted query against the actual database.

### 2. Parameter Ordering
**Risk:** The Query builder constructs SQL dynamically; parameter numbering ($1, $2...) must match the values list exactly.
**Mitigation:** The existing `build_select_sql_from_parts` already handles this correctly for WHERE/JOIN/HAVING. New extensions (where_raw, select_raw) must follow the same pattern. Add unit tests.

### 3. Performance Regression
**Risk:** Generated SQL may be less efficient than hand-written SQL (e.g., missing index hints, suboptimal join order).
**Mitigation:** PostgreSQL's query planner handles this well. The generated SQL will use the same parameterized pattern. Profile any queries that show degradation.

### 4. Fragment Injection Safety
**Risk:** `where_raw` and `select_raw` take string arguments -- if user input flows into these, SQL injection is possible.
**Mitigation:** All raw SQL fragments are hardcoded string literals in MPL source code. No user input flows into fragment strings. Parameters are always positional ($N).

### 5. Scope Creep
**Risk:** Trying to make the ORM handle every SQL pattern leads to building a full SQL AST, which is overkill.
**Mitigation:** Option B explicitly accepts that 8 queries stay as raw SQL via `Repo.execute_raw`. The escape hatch is by design, not a failure.

---

## Open Questions

### Q1: Should `Query.select_raw` use a new slot or reuse SLOT_SELECT?
**Option A:** New slot (SLOT_SELECT_RAW at index 13, expanding to 14 slots / 112 bytes). Cleanest separation.
**Option B:** Reuse SLOT_SELECT with a flag. If select_fields list entries start with "RAW:", pass through without quoting.
**Recommendation:** Option B -- simpler, no ABI change. The "RAW:" prefix convention is already used in WHERE clauses.

### Q2: Should type casts be auto-generated from Schema metadata?
**Option A:** Schema-aware casts -- Repo reads column types from `__schema__()` metadata and emits `::uuid`, `::text` casts automatically.
**Option B:** Manual casts -- MPL code specifies casts when needed (e.g., `Query.where_raw("project_id = $1::uuid", [id])`).
**Recommendation:** Option B for now. Auto-casting from schema metadata is a larger feature that can be added later. Manual casts in where_raw fragments are sufficient.

### Q3: Should `Repo.execute_raw` return rows or affected count?
**Option A:** Two functions: `Repo.query_raw` (returns rows) and `Repo.execute_raw` (returns affected count).
**Option B:** Single function with result type inference.
**Recommendation:** Option A -- matches the existing `Pool.query` / `Pool.execute` split.

### Q4: How to handle the JSONB parsing calls -- new intrinsic vs. stdlib?
**Option A:** Runtime intrinsic `Json.get_string` backed by serde_json in Rust.
**Option B:** MPL stdlib function using string manipulation (indexOf, slice).
**Recommendation:** Option A. JSON parsing needs to handle escaping, nesting, and edge cases. Rust + serde_json is robust. A runtime intrinsic avoids reimplementing JSON parsing in MPL.

---

## Dependencies

- Phase 102 must be complete (it is -- shipped 2026-02-16)
- Query builder (Phase 98), Repo (Phase 99), Preloading (Phase 100), Migrations (Phase 101) are all shipped
- No external dependencies

## Estimated Scope

- **Plan 1:** JSON intrinsic -- small (1 Rust module + 5 MPL rewrites)
- **Plan 2:** Query builder extensions -- medium (3-4 new runtime functions + compiler intrinsics)
- **Plan 3:** Repo write extensions -- medium (4 new runtime functions + compiler intrinsics)
- **Plan 4:** queries.mpl conversion -- large (35+ function rewrites)
- **Plan 5:** Final cleanup -- small (3 files)

Total: 5 plans, medium-to-large phase.
