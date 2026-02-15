# Phase 93: Data Retention & Cleanup - Research

**Researched:** 2026-02-15
**Domain:** Per-project data retention policies, partition-based event cleanup, issue summary preservation, storage visibility, and event sampling for the Mesher monitoring platform (Mesh language + PostgreSQL)
**Confidence:** HIGH

## Summary

Phase 93 adds data retention and cleanup to the Mesher monitoring platform. The four requirements (RETAIN-01 through RETAIN-04) span project-level configuration (retention period and sampling rate), automated cleanup (dropping old events while preserving issue summaries), and storage visibility (per-project usage reporting). Everything is implemented in Mesh (.mpl files) with PostgreSQL as the sole data store.

The central design challenge is that the events table uses **time-only daily partitions** (decision from Phase 87), not composite project+time partitions. This means we cannot simply `DROP TABLE events_YYYYMMDD` to enforce per-project retention -- the partition contains events from ALL projects. The correct approach is a **hybrid strategy**: (1) use `DELETE FROM events WHERE project_id = $1 AND received_at < $2` for per-project retention within active partitions, and (2) `DROP TABLE events_YYYYMMDD` for partitions older than the MAXIMUM retention across all projects (90 days). The DELETE approach within partitions benefits from existing indexes (`idx_events_project_received`) and partition pruning on `received_at`. A periodic actor (same Timer.sleep + recursive pattern as flush_ticker, spike_checker, alert_evaluator) runs the cleanup on a daily schedule.

For issue summary preservation (RETAIN-02), the issues table is NOT partitioned and NOT cascade-deleted with events. The events table has no foreign key to issues (no `REFERENCES issues(id)` -- verified in schema.mpl). Issue rows (with event_count, first_seen, last_seen) survive event deletion naturally. No special migration or data copy is needed.

For storage visibility (RETAIN-03), PostgreSQL's `pg_total_relation_size()` on individual partition tables (e.g., `events_20260215`) summed per-project is impractical because partitions are shared across projects. Instead, use `pg_column_size()` aggregation or `count(*) * avg_row_size` estimation queries filtered by `project_id`, which leverage the existing project index.

For event sampling (RETAIN-04), the sampling decision happens at ingestion time in the HTTP handler, before events reach the EventProcessor. A random number check against the project's configured `sample_rate` (0.0-1.0) determines whether to process or discard each event. Mesh has no random number generation, so PostgreSQL's `random()` function is used via a lightweight query.

**Primary recommendation:** Add `retention_days` (default 90) and `sample_rate` (default 1.0 = 100%) columns to the `projects` table. Create a `retention_cleaner` actor that runs daily, queries each project's retention setting, deletes expired events per-project, then drops any whole partitions older than 90 days. Add storage estimation queries and HTTP endpoints following the established handler patterns.

## Standard Stack

### Core
| Component | Version/Detail | Purpose | Why Standard |
|-----------|---------------|---------|--------------|
| Mesh language | v9.0 (current) | Backend application language | All Mesher code is Mesh |
| PostgreSQL | 18+ | Data storage, partition management, storage size functions | Native partitioning, `pg_total_relation_size()`, `random()` |
| Pool | Built-in | Connection pooling | `Pool.query`, `Pool.execute` for all DB operations |
| service/actor | Built-in | Periodic cleanup actor | Timer.sleep + recursive call pattern (established) |
| HTTP.router | Built-in | API endpoints for settings and storage | Pipe-chained router pattern |

### Supporting
| Component | Detail | Purpose | When to Use |
|-----------|--------|---------|-------------|
| pg_total_relation_size() | PostgreSQL built-in | Partition size measurement | Storage reporting per partition |
| pg_inherits | System catalog | List child partitions of events table | Enumerate partitions for cleanup/sizing |
| random() | PostgreSQL built-in | Sampling decision | Event ingestion sampling (Mesh has no RNG) |
| ALTER TABLE ... ADD COLUMN | DDL | Schema migration | Adding retention_days, sample_rate to projects |

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| Per-project DELETE within shared partitions | Composite LIST(project)+RANGE(time) partitioning | Would require schema migration, partition explosion for many projects, breaks existing partition management code |
| PostgreSQL random() for sampling | Application-level RNG | Mesh has no random number generation; PG random() is lightweight |
| Exact per-project storage calculation | Row count estimation from pg_stat_user_tables | Exact count requires full scan; estimation is faster but less accurate |
| Daily cleanup actor | pg_cron extension | pg_cron requires extension install; Mesh actor pattern is already established and consistent with existing architecture |

## Architecture Patterns

### Recommended File Changes
```
mesher/
├── storage/
│   ├── schema.mpl          # ADD: retention_days + sample_rate columns on projects
│   ├── queries.mpl          # ADD: retention queries, storage queries, settings CRUD
│   └── writer.mpl           # (unchanged)
├── services/
│   └── retention.mpl        # NEW: retention_cleaner actor + cleanup helpers
├── api/
│   └── settings.mpl         # NEW: project settings + storage API handlers
├── ingestion/
│   └── routes.mpl           # MODIFY: add sampling check before event processing
├── types/
│   └── project.mpl          # (no struct change needed -- settings are DB-only)
└── main.mpl                 # MODIFY: spawn retention_cleaner, register new routes
```

### Pattern 1: Hybrid Retention Cleanup
**What:** Per-project event deletion within shared daily partitions + whole-partition drops for globally expired data.
**When to use:** Daily retention cleanup cycle.

```mesh
# Step 1: For each project, delete events older than project's retention
# The WHERE clause on received_at enables partition pruning
fn cleanup_project(pool :: PoolHandle, project_id :: String, retention_days :: String) -> Int!String do
  Pool.execute(pool, "DELETE FROM events WHERE project_id = $1::uuid AND received_at < now() - ($2 || ' days')::interval", [project_id, retention_days])
end

# Step 2: Drop partitions older than the maximum retention (90 days)
# These partitions have zero rows from any project after step 1
fn drop_old_partition(pool :: PoolHandle, partition_name :: String) -> Int!String do
  Pool.execute(pool, "DROP TABLE IF EXISTS " <> partition_name, [])
end
```

### Pattern 2: Timer-Driven Cleanup Actor
**What:** Same pattern as flush_ticker, spike_checker, alert_evaluator -- Timer.sleep + recursive actor call.
**When to use:** Daily retention cleanup (86400000ms = 24 hours).

```mesh
# Retention cleaner actor -- runs daily.
# Uses Timer.sleep + recursive call pattern (established in flush_ticker).
actor retention_cleaner(pool :: PoolHandle) do
  Timer.sleep(86400000)
  let result = run_retention_cleanup(pool)
  case result do
    Ok(n) -> log_cleanup_result(n)
    Err(e) -> log_cleanup_error(e)
  end
  retention_cleaner(pool)
end
```

### Pattern 3: Sampling at Ingestion
**What:** Before routing an event to EventProcessor, check the project's sample_rate. Use PostgreSQL's `random()` to generate a random number and compare against the rate.
**When to use:** Every event ingestion request.

```mesh
# Check if event should be sampled (kept) based on project's sample_rate.
# Returns true if the event should be processed, false if it should be dropped.
fn should_sample(pool :: PoolHandle, project_id :: String) -> Bool!String do
  let rows = Pool.query(pool, "SELECT (random() < COALESCE((SELECT sample_rate FROM projects WHERE id = $1::uuid), 1.0)) AS keep", [project_id])?
  if List.length(rows) > 0 do
    Ok(Map.get(List.head(rows), "keep") == "t")
  else
    Ok(true)
  end
end
```

### Pattern 4: Project Settings via ALTER TABLE ADD COLUMN
**What:** Add `retention_days` and `sample_rate` to the existing `projects` table via idempotent DDL in schema.mpl.
**When to use:** Schema creation at startup.

```mesh
# In create_schema, after creating the projects table:
Pool.execute(pool, "ALTER TABLE projects ADD COLUMN IF NOT EXISTS retention_days INTEGER NOT NULL DEFAULT 90", [])?
Pool.execute(pool, "ALTER TABLE projects ADD COLUMN IF NOT EXISTS sample_rate REAL NOT NULL DEFAULT 1.0", [])?
```

### Anti-Patterns to Avoid
- **Do not add composite partitioning (project + time).** This would require a full schema migration, create partition explosion for many projects, and break all existing partition management code. The current time-only partitioning with per-project DELETE is sufficient.
- **Do not delete events via the events table's FK to issues.** There is no FK constraint from events to issues -- `issue_id` is not a REFERENCES column. This is correct and means event deletion does not cascade or require issue deletion.
- **Do not modify the Project struct to include retention_days/sample_rate.** These are configuration settings queried directly from the database. Adding them to the struct would require all existing Project construction sites to be updated. Query them separately when needed.
- **Do not use pg_partman or pg_cron extensions.** The project uses vanilla PostgreSQL with no extensions beyond pgcrypto. The established actor pattern (Timer.sleep + recursive call) is the Mesher way to schedule periodic tasks.
- **Do not calculate exact per-project storage by summing pg_column_size for all rows.** This requires a full sequential scan. Use count(*) with estimated average row size, or use PostgreSQL statistics views.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Random number generation | Custom PRNG in Mesh | PostgreSQL `random()` | Mesh has no RNG capability; PG random() is built-in |
| Partition enumeration | Manual date arithmetic | `pg_inherits` system catalog query | Reliable, handles edge cases, returns actual partition list |
| Storage size per partition | Manual file size estimation | `pg_total_relation_size()` on partition tables | Includes indexes and TOAST data; PostgreSQL built-in |
| Scheduled cleanup | Custom timer with complex state | Timer.sleep + recursive actor pattern | Already proven in 5+ actors (flush_ticker, spike_checker, health_checker, alert_evaluator, stream_drain_ticker) |
| Idempotent schema migration | Migration files/versioning | `ALTER TABLE ADD COLUMN IF NOT EXISTS` | Consistent with all existing schema.mpl DDL |

**Key insight:** PostgreSQL handles all the hard parts -- random number generation for sampling, partition catalog queries for enumeration, size functions for storage reporting, and efficient DELETE with partition pruning for retention. The Mesh code orchestrates timing and HTTP API, nothing more.

## Common Pitfalls

### Pitfall 1: DELETE on Shared Partitions Causes VACUUM Overhead
**What goes wrong:** Deleting millions of rows from events partitions creates dead tuples that autovacuum must clean up, potentially causing table bloat and performance degradation.
**Why it happens:** PostgreSQL's MVCC means DELETEd rows are not immediately reclaimed. Large DELETEs in shared partitions are unavoidable with time-only partitioning.
**How to avoid:** (1) Run cleanup during off-peak hours (the actor runs once daily). (2) Batch DELETEs per-project rather than one massive DELETE across all projects. (3) Drop whole partitions for the oldest data (past 90 days) -- this is a metadata operation with zero VACUUM overhead. (4) The partition index on `(project_id, received_at DESC)` ensures DELETEs are index-scanned, not seq-scanned.
**Warning signs:** `pg_stat_user_tables.n_dead_tup` growing large on event partitions; slow queries on recently-cleaned partitions.

### Pitfall 2: Partition Name Format Must Match Existing Convention
**What goes wrong:** The retention cleaner tries to drop a partition with the wrong name format and silently fails.
**Why it happens:** The existing `create_partition` function uses format `events_YYYYMMDD` (e.g., `events_20260215`). The cleanup must use the same format.
**How to avoid:** Use `pg_inherits` to query actual child table names rather than constructing them by date arithmetic. This is reliable regardless of naming convention.
**Warning signs:** `DROP TABLE IF EXISTS` succeeding (returns 0) but partitions still existing.

### Pitfall 3: Issue Summaries Are Already Preserved -- Don't Over-Engineer
**What goes wrong:** Developer adds a complex "archive issue summary before deleting events" step, creating redundant data and complexity.
**Why it happens:** The requirement says "preserve issue summaries after event deletion" which sounds like it requires special handling.
**How to avoid:** The issues table already stores event_count, first_seen, last_seen, title, level, status, fingerprint. There is NO foreign key from events to issues (events.issue_id is UUID NOT NULL but has no REFERENCES constraint). Deleting events does NOT affect the issues table at all. No special preservation logic is needed -- just verify this invariant.
**Warning signs:** Creating an `issue_summaries` table or an archival step.

### Pitfall 4: Sampling Rate Must Be Checked BEFORE Rate Limiting
**What goes wrong:** Events are rate-limited before sampling, so the rate limiter counts events that will be discarded by sampling, effectively double-throttling high-volume projects.
**Why it happens:** Wrong ordering of the ingestion pipeline checks.
**How to avoid:** Check sampling rate FIRST (before rate limiter check). If the event is sampled out, return 202 Accepted immediately (the client should not know about server-side sampling). Only count events that pass sampling against the rate limiter.
**Warning signs:** Projects with 0.1 sample_rate still hitting rate limits as if they had 1.0.

### Pitfall 5: Storage Estimation Query Performance
**What goes wrong:** Running `SELECT count(*), sum(pg_column_size(t.*)) FROM events WHERE project_id = $1` causes a full sequential scan across all partitions.
**Why it happens:** Without a `received_at` filter, partition pruning cannot kick in, and pg_column_size requires reading actual row data.
**How to avoid:** Use a two-tier approach: (1) Count events per project with `SELECT count(*) FROM events WHERE project_id = $1` which uses the `idx_events_project_received` index. (2) Multiply by an estimated average row size (query a small sample or use pg_stats). (3) Add partition-level sizes from `pg_total_relation_size()` for total table size context.
**Warning signs:** Storage query taking >10 seconds on large datasets.

### Pitfall 6: Mesh Cannot Do String Concatenation for Dynamic SQL Safely
**What goes wrong:** Building a `DROP TABLE events_YYYYMMDD` query via string concatenation opens SQL injection risk if the partition name comes from untrusted input.
**Why it happens:** Mesh has no parameterized DDL -- `Pool.execute` parameterization only works for DML values, not identifiers.
**How to avoid:** The partition names come from `pg_inherits` (a trusted system catalog), not from user input. Additionally, validate partition names match the expected format `events_YYYYMMDD` before constructing the DROP statement.
**Warning signs:** Partition names containing unexpected characters.

## Code Examples

Verified patterns from existing Mesher codebase:

### Schema Migration (Idempotent Column Addition)
```mesh
# Source: mesher/storage/schema.mpl (existing pattern for alert_rules)
Pool.execute(pool, "ALTER TABLE projects ADD COLUMN IF NOT EXISTS retention_days INTEGER NOT NULL DEFAULT 90", [])?
Pool.execute(pool, "ALTER TABLE projects ADD COLUMN IF NOT EXISTS sample_rate REAL NOT NULL DEFAULT 1.0", [])?
```
Matches the existing pattern on line 20-21 of schema.mpl where `cooldown_minutes` and `last_fired_at` were added to `alert_rules` via `ALTER TABLE ADD COLUMN IF NOT EXISTS`.

### Per-Project Retention Cleanup Query
```mesh
# Delete events older than the project's retention period.
# PostgreSQL interval cast from days string. Partition pruning on received_at.
fn delete_expired_events(pool :: PoolHandle, project_id :: String, retention_days :: String) -> Int!String do
  Pool.execute(pool, "DELETE FROM events WHERE project_id = $1::uuid AND received_at < now() - ($2 || ' days')::interval", [project_id, retention_days])
end
```

### Enumerate Partitions Older Than N Days
```mesh
# List partition table names for events partitions older than the given interval.
# Uses pg_inherits system catalog to get actual child table names.
fn get_expired_partitions(pool :: PoolHandle, max_days :: String) -> List<Map<String, String>>!String do
  let sql = "SELECT c.relname::text AS partition_name FROM pg_inherits i JOIN pg_class c ON c.oid = i.inhrelid JOIN pg_class p ON p.oid = i.inhparent WHERE p.relname = 'events' AND c.relname ~ '^events_[0-9]{8}$' AND to_date(substring(c.relname from '[0-9]{8}$'), 'YYYYMMDD') < (current_date - ($1 || ' days')::interval)"
  Pool.query(pool, sql, [max_days])
end
```

### Storage Usage Estimation
```mesh
# Get event count and estimated storage for a project.
# Uses count(*) with index scan + estimated avg row size.
fn get_project_storage(pool :: PoolHandle, project_id :: String) -> List<Map<String, String>>!String do
  let sql = "SELECT count(*)::text AS event_count, (count(*) * 1024)::text AS estimated_bytes FROM events WHERE project_id = $1::uuid"
  Pool.query(pool, sql, [project_id])
end
```
Note: 1024 bytes is a conservative average row size estimate for events with JSONB fields. A more precise approach queries `pg_stats` for average column widths.

### Project Settings Update Handler
```mesh
# Following the established pattern from Api.Alerts (handle_toggle_alert_rule).
# Uses SQL-side JSON extraction for request body parsing (decision [91-03]).
fn update_project_settings(pool :: PoolHandle, project_id :: String, body :: String) -> Int!String do
  Pool.execute(pool, "UPDATE projects SET retention_days = COALESCE(($2::jsonb->>'retention_days')::int, retention_days), sample_rate = COALESCE(($2::jsonb->>'sample_rate')::real, sample_rate) WHERE id = $1::uuid", [project_id, body])
end
```

### Sampling Check at Ingestion
```mesh
# Uses PostgreSQL random() since Mesh has no RNG.
# Returns true/false as text from boolean column.
fn check_sample(pool :: PoolHandle, project_id :: String) -> Bool!String do
  let rows = Pool.query(pool, "SELECT random() < COALESCE((SELECT sample_rate FROM projects WHERE id = $1::uuid), 1.0) AS keep", [project_id])?
  if List.length(rows) > 0 do
    Ok(Map.get(List.head(rows), "keep") == "t")
  else
    Ok(true)
  end
end
```

### Timer-Driven Actor (Existing Pattern)
```mesh
# Source: mesher/ingestion/pipeline.mpl (spike_checker, alert_evaluator, health_checker)
actor retention_cleaner(pool :: PoolHandle) do
  Timer.sleep(86400000)  # 24 hours
  let result = run_retention_cleanup(pool)
  case result do
    Ok(n) -> log_cleanup_result(n)
    Err(e) -> log_cleanup_error(e)
  end
  retention_cleaner(pool)
end
```

## Critical Design Decisions

### Decision 1: Hybrid DELETE + DROP Strategy
**Context:** Events table uses time-only daily partitions (locked decision from Phase 87). Per-project retention requires different retention periods (30/60/90 days) for projects sharing the same partitions.

**Approach:** Two-phase cleanup:
1. **Per-project DELETE:** For each project, `DELETE FROM events WHERE project_id = $1 AND received_at < now() - retention_interval`. This uses the existing `idx_events_project_received` index and PostgreSQL partition pruning on `received_at`.
2. **Whole-partition DROP:** After per-project DELETEs, drop partitions older than 90 days (the maximum retention). These partitions are guaranteed to be empty after step 1.

**Why not sub-partitioning:** Adding LIST(project_id) partitioning would require schema migration, create O(projects * days) partitions (partition explosion), break all existing partition creation code, and provide minimal performance benefit (pg_partman docs explicitly warn against sub-partitioning for anything under petabyte scale).

**Confidence:** HIGH -- this is the standard approach for multi-tenant time-partitioned tables with varying retention.

### Decision 2: No Issue Summary Archival Needed
**Context:** RETAIN-02 requires "System preserves issue summaries after event deletion."

**Finding:** The issues table is completely independent from the events table. Events.issue_id is a UUID column with NO foreign key constraint (verified in schema.mpl line 17). The issues table stores all summary data (event_count, first_seen, last_seen, title, level, status, fingerprint, assigned_to). Deleting events has zero effect on the issues table.

**Implication:** RETAIN-02 is satisfied by the existing schema design. No migration, no archival step, no summary table needed. The planner should still add a verification step to confirm this invariant.

**Confidence:** HIGH -- verified directly in schema.mpl source code.

### Decision 3: PostgreSQL random() for Sampling
**Context:** RETAIN-04 requires per-project event sampling rate. Mesh has no random number generation.

**Approach:** Use `SELECT random() < sample_rate` in PostgreSQL. This is a lightweight query (no table scan) that returns a boolean. The sampling check runs once per event ingestion, adding ~1ms of latency per event.

**Alternative considered:** Deterministic sampling based on fingerprint hash (every Nth event). Rejected because it would bias against rare events and doesn't provide true random sampling.

**Confidence:** HIGH -- PostgreSQL random() is a well-known pattern; Mesh has no RNG (verified in prior research).

### Decision 4: Settings on Projects Table, Not Separate Config Table
**Context:** Need to store retention_days and sample_rate per project.

**Approach:** Add columns directly to the existing `projects` table via `ALTER TABLE ADD COLUMN IF NOT EXISTS`. This follows the same pattern used for `alert_rules` (cooldown_minutes, last_fired_at added via ALTER TABLE in schema.mpl lines 20-21).

**Why not a separate table:** A separate `project_settings` table would require a JOIN for every settings lookup and complicate the schema. Two columns on `projects` is simpler and the table is small (one row per project).

**Confidence:** HIGH -- follows established schema evolution pattern.

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Sentry: global retention only | Per-project retention (feature requested but not yet in Sentry) | N/A (Sentry still global) | Mesher goes beyond Sentry's current capability |
| Sentry: client-side sampling only | Server-side sampling at ingestion | Dynamic Sampling (Sentry) | Server-side sampling saves storage without client SDK changes |
| Manual partition management | pg_partman automation | Ongoing | Mesher uses DIY actor pattern instead -- consistent with existing architecture |
| DELETE for retention | DROP PARTITION for retention | PostgreSQL 10+ | Mesher uses hybrid: DELETE per-project + DROP for globally expired partitions |

## Open Questions

1. **What is the actual average row size for events?**
   - What we know: Events have 15 columns including JSONB fields (exception, stacktrace, breadcrumbs, tags, extra, user_context). Size varies dramatically by event content.
   - What's unclear: Whether 1KB is a good estimate for storage calculation.
   - Recommendation: Use `SELECT avg(pg_column_size(e.*))::int FROM events e LIMIT 1000` to get an empirical estimate on real data. For initial implementation, use 1024 bytes as placeholder and document it as approximate.

2. **Should the retention cleaner run at a fixed time or with a fixed interval?**
   - What we know: The Timer.sleep pattern gives a fixed interval (24 hours between cleanup runs). It does not guarantee a specific wall-clock time (e.g., 3 AM).
   - What's unclear: Whether running cleanup at arbitrary times causes issues.
   - Recommendation: Fixed interval (24 hours) is sufficient. The cleanup is idempotent -- running it at different times of day has no correctness impact. If wall-clock scheduling is needed in the future, it requires a `DateTime.now()` stdlib addition.

3. **Should sampled-out events return 202 or 200?**
   - What we know: The SDK client should not distinguish between "accepted and stored" vs "accepted and sampled out." Both should look like success.
   - Recommendation: Return 202 Accepted for sampled-out events (same as accepted events). The client does not need to know about server-side sampling. This matches Sentry's approach where Dynamic Sampling is transparent to the SDK.

## Sources

### Primary (HIGH confidence)
- Mesher codebase: `mesher/storage/schema.mpl` -- verified events table schema, partition creation, no FK from events to issues
- Mesher codebase: `mesher/storage/queries.mpl` -- verified query patterns, existing indexes, SQL conventions
- Mesher codebase: `mesher/ingestion/pipeline.mpl` -- verified Timer.sleep + recursive actor pattern (5 existing actors)
- Mesher codebase: `mesher/ingestion/routes.mpl` -- verified ingestion flow: auth -> rate limit -> validate -> process
- Mesher codebase: `mesher/main.mpl` -- verified startup sequence, route registration, service wiring
- Mesher codebase: `mesher/api/alerts.mpl` -- verified HTTP handler pattern with SQL-side JSON extraction
- [PostgreSQL 18 Partitioning Docs](https://www.postgresql.org/docs/current/ddl-partitioning.html) -- DROP/DETACH partition semantics
- [PostgreSQL Disk Usage Wiki](https://wiki.postgresql.org/wiki/Disk_Usage) -- pg_total_relation_size, pg_inherits queries

### Secondary (MEDIUM confidence)
- [Time-based retention strategies in Postgres](https://blog.sequinstream.com/time-based-retention-strategies-in-postgres/) -- DELETE vs DROP tradeoffs, VACUUM considerations
- [Beyond DELETE: Drop Partitions, Not Performance](https://www.simplethread.com/beyond-delete/) -- Partition-based retention best practices
- [Sentry Data Retention Help](https://sentry.zendesk.com/hc/en-us/articles/27118913621019-How-Long-Are-Errors-Events-Stored-in-Sentry) -- Sentry's 90-day retention model
- [Sentry Per-Project Retention Feature Request](https://github.com/getsentry/sentry/issues/76826) -- Confirms per-project retention is not yet in Sentry
- [Sentry Sampling Docs](https://docs.sentry.io/platforms/python/configuration/sampling/) -- SDK-side and server-side sampling patterns
- [Sentry Dynamic Sampling Architecture](https://develop.sentry.dev/application-architecture/dynamic-sampling/the-big-picture/) -- Server-side sampling decision model

### Tertiary (LOW confidence)
- Average event row size estimate (1024 bytes) -- needs empirical validation on real data

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- all Mesh capabilities verified against existing codebase (5+ timer actors, Pool.query/execute, HTTP.router)
- Architecture: HIGH -- hybrid DELETE+DROP strategy is well-documented for multi-tenant time-partitioned tables; schema evolution follows existing ALTER TABLE pattern
- Issue preservation: HIGH -- verified directly: no FK from events to issues in schema.mpl
- Pitfalls: HIGH -- identified from PostgreSQL documentation (VACUUM overhead, partition naming) and Mesh constraints (no RNG, no DateTime)
- Sampling: MEDIUM -- PostgreSQL random() approach is standard but adds a DB round-trip per event; performance impact needs validation

**Research date:** 2026-02-15
**Valid until:** 2026-03-15 (stable -- Mesh compiler and PostgreSQL 18 are mature)
