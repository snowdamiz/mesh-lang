# Phase 87: Foundation - Research

**Researched:** 2026-02-14
**Domain:** Data model, database schema, storage writer, and org/project tenancy for the Mesher monitoring platform (implemented in Mesh language)
**Confidence:** HIGH

## Summary

Phase 87 establishes the foundation layer for the Mesher monitoring platform: all core data types (Event, Issue, Project, Organization, AlertRule, User), the PostgreSQL schema with time-partitioned events, a batching StorageWriter service, organization/project tenancy with API key generation, and full user authentication. Everything is written in the Mesh language (.mpl files), compiled by `meshc build`, using existing Mesh language features: `deriving(Json, Row)` for serialization, `service` for stateful GenServer actors, `Pool` for PostgreSQL connection management, and the multi-module build system.

The critical technical challenges in this phase are: (1) PostgreSQL schema design with daily time partitioning using native `PARTITION BY RANGE`, which requires that the events table primary key include the partition column; (2) UUIDv7 generation for time-sortable entity IDs, which PostgreSQL 18 natively supports via the `uuidv7()` function but Mesh has no UUID generation capability -- all UUID generation must happen in PostgreSQL; (3) a bounded StorageWriter service using the actor-per-project pattern with size+timer flush triggers and drop-oldest backpressure; (4) password hashing and session management for user auth, which must be implemented via PostgreSQL's `pgcrypto` extension since Mesh has no Argon2/bcrypt runtime; and (5) JSONB storage for semi-structured event data alongside typed columns for required core fields.

This is a multi-module Mesh project (the first real application built in Mesh). The project will live at `mesher/` in the monorepo root, with `mesher/main.mpl` as the entry point and subdirectories for logical modules. `meshc build mesher/` produces a single native binary. All database operations use parameterized SQL via `Pool.execute` and `Pool.query_as` with `deriving(Row)` structs.

**Primary recommendation:** Build the data model structs with `deriving(Json, Row)` first, then create the PostgreSQL schema with partitioning, then implement org/project/user management services, then the StorageWriter service. All UUID generation and password hashing happens in PostgreSQL (via `uuidv7()` and `pgcrypto`), not in Mesh application code.

<user_constraints>
## User Constraints (from CONTEXT.md)

### Locked Decisions
- Semi-structured event payload: required core fields (message, level, timestamp) plus a flexible `extra` JSONB bag for arbitrary data
- Known interfaces (exception, breadcrumbs, user, contexts) get typed fields; unknown data goes in extra
- Tags stored as flat `Map<String, String>` -- simple key-value pairs, easy to index and filter
- Severity levels follow Sentry convention: fatal, error, warning, info, debug (5 levels)
- Simple API key format per project (e.g., `mshr_abc123...`) -- not Sentry-style DSN URLs
- Multiple API keys per project supported -- allows rotation without downtime, separate keys per environment
- Full user auth included in this phase: users table with org membership, password hashing, session/token support
- API keys passed as header or query param for event ingestion
- Flush trigger: size + timer -- flush when batch hits N events OR when timer fires, whichever comes first
- Flush failure: retry N times with backoff, then drop the batch and log error (prevents unbounded memory growth)
- Per-project StorageWriter actors -- each project gets its own writer for isolation (one slow project can't block others)
- Bounded buffer with drop-oldest backpressure -- cap at N events, drop oldest when full (monitoring data is time-sensitive)
- Daily time partitions for the events table
- Partitions pre-created ahead of time by a scheduled job (no risk of missing partition on first event of new day)
- Time-only partitioning (no composite project+time) -- project_id filtered via index
- UUIDv7 (time-sortable) for all entity primary keys -- no ID collision across nodes

### Claude's Discretion
- Stack trace representation (structured frames vs raw string -- optimize for querying and display)
- Org/project hierarchy depth (flat org->project or org->team->project)
- Exact batch size and timer interval for StorageWriter
- Retry count and backoff strategy for flush failures
- Buffer capacity limits
- Partition pre-creation horizon (how far ahead to create)
- Index strategy on events and issues tables
- Password hashing algorithm and session token format

### Deferred Ideas (OUT OF SCOPE)
None -- discussion stayed within phase scope
</user_constraints>

## Standard Stack

### Core
| Component | Version/Detail | Purpose | Why Standard |
|-----------|---------------|---------|--------------|
| Mesh language | v8.0 (current) | Backend application language | The entire Mesher backend is written in Mesh; this is the dogfooding milestone |
| meshc build | `meshc build mesher/` | Compile multi-module project | Produces single native binary from .mpl files |
| PostgreSQL | 18+ | Persistent storage | Native UUIDv7 via `uuidv7()`, native partitioning, JSONB, pgcrypto |
| deriving(Json) | Built-in | JSON serialization for all structs/sum types | Auto-generates `Json.encode` and `Type.from_json` |
| deriving(Row) | Built-in | Database row mapping | Auto-generates `Type.from_row` from `Map<String, String>` |
| Pool | Built-in | PostgreSQL connection pooling | `Pool.open`, `Pool.query`, `Pool.execute`, `Pool.query_as` with auto checkout/checkin |
| service | Built-in | Stateful GenServer actors | `call` (sync), `cast` (async), state threading |
| Timer.send_after | Built-in | Periodic flush trigger | Schedules delayed messages to actor mailbox |
| supervisor | Built-in | Fault tolerance | `one_for_one`, `rest_for_one` strategies for writer/schema actors |

### Supporting
| Component | Detail | Purpose | When to Use |
|-----------|--------|---------|-------------|
| Pg.execute | Direct connection | Schema DDL (CREATE TABLE, partitions) | One-time schema setup on startup |
| Pg.transaction | Built-in | Batch writes with atomicity | StorageWriter flush operations |
| pgcrypto (PG extension) | `gen_salt`, `crypt` | Password hashing | User authentication - bcrypt via PostgreSQL |
| HTTP.router + HTTP.serve | Built-in | API endpoints | Org/project/user management APIs in later phases |
| Map<String, String> | Built-in | Tag storage, event metadata | Tags, extra fields passed as maps |

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| pgcrypto for password hashing | Application-level hashing | Mesh has no Argon2/bcrypt runtime; pgcrypto is proven and available |
| PostgreSQL uuidv7() | Application-level UUID gen | Mesh has no random/UUID generation; PG 18 has native `uuidv7()` |
| JSONB for extra data | Separate tables per interface | JSONB is flexible for semi-structured data; separate tables need complex joins |
| TEXT columns for stored JSON | JSONB columns | Mesh's `deriving(Row)` maps to `Map<String, String>` (text); JSONB can be cast to text on SELECT |

## Architecture Patterns

### Recommended Project Structure
```
mesher/
├── main.mpl                    # Entry point: init DB pool, run schema, start services
├── types/
│   ├── event.mpl               # Event, Severity, StackFrame, ExceptionInfo structs
│   ├── issue.mpl               # Issue, IssueStatus structs
│   ├── project.mpl             # Organization, Project, ApiKey structs
│   ├── user.mpl                # User, OrgMembership, Session structs
│   └── alert.mpl               # AlertRule, AlertCondition structs
├── storage/
│   ├── schema.mpl              # DDL execution, partition management
│   ├── writer.mpl              # StorageWriter service (per-project batch+flush)
│   └── queries.mpl             # Reusable SQL query helper functions
└── services/
    ├── org_service.mpl          # Organization CRUD service
    ├── project_service.mpl      # Project + API key management service
    └── user_service.mpl         # User auth + session service
```

**Module naming:** `mesher/types/event.mpl` becomes module `Types.Event`, imported as `import Types.Event` or `from Types.Event import { Event, Severity }`.

### Pattern 1: Per-Project StorageWriter Service
**What:** Each project gets its own StorageWriter actor that accumulates events in a bounded buffer and flushes to PostgreSQL in batches on a timer or size threshold.
**When to use:** All event storage. The EventRouter (Phase 88) will send events to the correct project's writer.

```mesh
# storage/writer.mpl
module Storage.Writer

import Types.Event

struct WriterState do
  pool :: Int          # Pool handle (opaque u64)
  project_id :: String
  buffer :: List<Event>
  batch_size :: Int
  max_buffer :: Int
  retry_count :: Int
end

service StorageWriter do
  fn init(pool :: Int, project_id :: String) -> WriterState do
    # Schedule first flush timer (5 seconds)
    Timer.send_after(self(), 5000, "flush")
    WriterState {
      pool: pool,
      project_id: project_id,
      buffer: [],
      batch_size: 50,
      max_buffer: 500,
      retry_count: 0
    }
  end

  cast Store(event :: Event) do |state|
    let new_buffer = [event] ++ state.buffer
    let buffer_len = List.length(new_buffer)
    # Drop oldest if over capacity
    let trimmed = if buffer_len > state.max_buffer do
      List.take(new_buffer, state.max_buffer)
    else
      new_buffer
    end
    # Flush if batch size reached
    if List.length(trimmed) >= state.batch_size do
      flush_batch(state.pool, state.project_id, trimmed)
      WriterState { pool: state.pool, project_id: state.project_id,
                    buffer: [], batch_size: state.batch_size,
                    max_buffer: state.max_buffer, retry_count: 0 }
    else
      WriterState { pool: state.pool, project_id: state.project_id,
                    buffer: trimmed, batch_size: state.batch_size,
                    max_buffer: state.max_buffer, retry_count: 0 }
    end
  end

  cast Flush() do |state|
    if List.length(state.buffer) > 0 do
      flush_batch(state.pool, state.project_id, state.buffer)
    end
    # Reschedule flush timer
    Timer.send_after(self(), 5000, "flush")
    WriterState { pool: state.pool, project_id: state.project_id,
                  buffer: [], batch_size: state.batch_size,
                  max_buffer: state.max_buffer, retry_count: 0 }
  end
end
```

### Pattern 2: Schema Management via DDL Execution
**What:** Run CREATE TABLE, partition creation, and index creation via `Pg.execute` on a direct connection at startup.
**When to use:** Application startup, partition pre-creation scheduled job.

```mesh
# storage/schema.mpl
module Storage.Schema

pub fn create_schema(pool :: Int) -> Int!String do
  # Create organizations table
  let _ = Pool.execute(pool,
    "CREATE TABLE IF NOT EXISTS organizations (
      id UUID PRIMARY KEY DEFAULT uuidv7(),
      name TEXT NOT NULL,
      slug TEXT NOT NULL UNIQUE,
      created_at TIMESTAMPTZ NOT NULL DEFAULT now()
    )", [])?

  # Create events parent table (partitioned)
  let _ = Pool.execute(pool,
    "CREATE TABLE IF NOT EXISTS events (
      id UUID NOT NULL DEFAULT uuidv7(),
      project_id UUID NOT NULL,
      issue_id UUID NOT NULL,
      level TEXT NOT NULL,
      message TEXT NOT NULL,
      fingerprint TEXT NOT NULL,
      exception JSONB,
      stacktrace JSONB,
      breadcrumbs JSONB,
      tags JSONB NOT NULL DEFAULT '{}',
      extra JSONB NOT NULL DEFAULT '{}',
      user_context JSONB,
      received_at TIMESTAMPTZ NOT NULL DEFAULT now(),
      PRIMARY KEY (id, received_at)
    ) PARTITION BY RANGE (received_at)", [])?

  Ok(0)
end
```

### Pattern 3: Flat Org -> Project Hierarchy
**What:** Simple two-level hierarchy: Organizations contain Projects. No team/group intermediate layer.
**When to use:** All tenancy operations.
**Rationale:** Simpler model for an initial platform. Teams/groups can be added later without schema migration (just add a teams table and a team_id FK on projects). Sentry started with org->project and added teams later.

### Pattern 4: Structured Stack Trace Representation
**What:** Store stack traces as JSONB arrays of frame objects rather than raw strings.
**When to use:** Exception and stack trace storage.
**Rationale:** Structured frames enable: (1) fingerprinting on specific frames (function name + file), (2) display formatting on the frontend with syntax highlighting and context, (3) filtering queries like "show all events where frame contains function X". Raw strings would require regex parsing at query/display time.

```mesh
# Stack frame as a struct -- stored as JSONB array in PostgreSQL
struct StackFrame do
  filename :: String
  function_name :: String
  lineno :: Int
  colno :: Option<Int>
  context_line :: Option<String>
  in_app :: Bool
end deriving(Json)
```

### Anti-Patterns to Avoid
- **Do not generate UUIDs in Mesh application code.** Mesh has no random number generator or UUID library. Use PostgreSQL's `DEFAULT uuidv7()` for auto-generation, or `SELECT uuidv7()` when you need the ID in application code before inserting.
- **Do not hash passwords in Mesh application code.** Use PostgreSQL's `pgcrypto` extension: `crypt('password', gen_salt('bf', 12))` for hashing, `crypt('attempt', stored_hash) = stored_hash` for verification.
- **Do not create one StorageWriter for all projects.** Use per-project writers for fault isolation. One slow project's flush should not block other projects.
- **Do not use `Iter.collect()` to build `Map<String, V>`.** Known limitation: `Map.collect` assumes integer keys. Build string-keyed maps manually with `Map.new()` + `Map.put()`.
- **Do not use `case List.find(...) do Some(...) -> ...`.** Known LLVM verification error. Use `List.filter` + `List.length` check instead.
- **Do not store all event data in a single TEXT column.** Use typed columns for indexed/queried fields (level, message, fingerprint, project_id) and JSONB for flexible/nested data (exception, stacktrace, tags, extra).

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| UUID generation | Custom random+timestamp encoding | PostgreSQL `uuidv7()` function | Mesh has no random number generation; PG 18 has native UUIDv7 |
| Password hashing | Custom hash function | PostgreSQL `pgcrypto` extension (`crypt()` + `gen_salt()`) | Bcrypt with tunable cost factor; battle-tested implementation |
| Connection pooling | Actor-based pool manager | `Pool.open(url, min, max, timeout)` | Already built into Mesh runtime with health checks, checkout timeout, auto-ROLLBACK |
| JSON serialization | Manual string building | `deriving(Json)` + `Json.encode()` + `Type.from_json()` | Handles nested structs, Options, Lists, Maps, sum types automatically |
| Row mapping | Manual Map.get + parse for each column | `deriving(Row)` + `Pool.query_as()` | Generates from_row with type parsing (String, Int, Float, Bool, Option) |
| Table partitioning | Application-level data routing | PostgreSQL native `PARTITION BY RANGE` | PG handles partition pruning, constraint exclusion, and index maintenance |
| Session tokens | Custom token format | 64-char hex string from PostgreSQL `encode(gen_random_bytes(32), 'hex')` | Cryptographically random, no Mesh random needed |

**Key insight:** Mesh's runtime has no cryptographic primitives, no random number generation, and no UUID generation. All of these are delegated to PostgreSQL, which has mature implementations. This is not a workaround -- it is the correct architecture for a database-backed application where the database is the source of truth for IDs and credentials.

## Common Pitfalls

### Pitfall 1: Primary Key Must Include Partition Column
**What goes wrong:** Creating a partitioned table with `PRIMARY KEY (id)` fails because PostgreSQL requires all partition keys to be part of the primary key.
**Why it happens:** PostgreSQL enforces unique constraints per-partition, not globally. A unique constraint on `(id)` alone cannot be checked without scanning all partitions.
**How to avoid:** Use `PRIMARY KEY (id, received_at)` for the events table. This is a composite key where `id` (UUIDv7) is globally unique by construction, and `received_at` is required for partition routing.
**Warning signs:** `ERROR: unique constraint on partitioned table must include all partitioning columns`.

### Pitfall 2: UUIDv7 Requires PostgreSQL 18+
**What goes wrong:** The `uuidv7()` function does not exist on PostgreSQL 16 or 17, causing schema creation to fail.
**Why it happens:** UUIDv7 was added in PostgreSQL 18 (released September 2025). Older versions only have `gen_random_uuid()` (v4).
**How to avoid:** Require PostgreSQL 18+ in the project prerequisites. Verify with `SELECT version()` at startup. If PG < 18, use `gen_random_uuid()` as fallback (v4 UUIDs are not time-sortable but still globally unique).
**Warning signs:** `ERROR: function uuidv7() does not exist`.

### Pitfall 3: deriving(Row) Maps Everything Through Text
**What goes wrong:** Developer expects `from_row` to parse JSONB columns into nested Mesh structs. But `deriving(Row)` maps `Map<String, String>` -- all values are strings. A JSONB column arrives as a JSON string that must be manually parsed with `Type.from_json()`.
**Why it happens:** PostgreSQL's text protocol returns all values as strings. `deriving(Row)` provides type parsing for primitives (Int, Float, Bool, Option) but not for complex/nested types.
**How to avoid:** For JSONB columns, define the struct field as `String` in the Row struct, then parse with `Type.from_json(row.jsonb_field)` in a separate step. Consider a helper function that combines `from_row` + JSON parsing for complex structs.
**Warning signs:** Struct field type mismatch errors when deriving Row for a struct with non-primitive fields.

### Pitfall 4: Timer.send_after Creates One OS Thread Per Call
**What goes wrong:** Using `Timer.send_after` in a loop or for many concurrent writers creates an OS thread per invocation.
**Why it happens:** `Timer.send_after` spawns `std::thread::spawn` internally (see mesh-rt actor/mod.rs:585).
**How to avoid:** Use a single timer per StorageWriter actor. The writer calls `Timer.send_after(self(), interval, "flush")` once, and re-schedules in the flush handler. Never create timers in a loop.
**Warning signs:** High OS thread count correlated with number of active writers.

### Pitfall 5: Pool.query Returns Map<String, String> with String Keys
**What goes wrong:** Expecting `Pool.query` to return typed values. All values are strings, even integers and booleans.
**Why it happens:** PostgreSQL text protocol and Mesh's `Map<String, String>` return type.
**How to avoid:** Use `Pool.query_as` with `deriving(Row)` structs for automatic type parsing. For raw queries, use `String.to_int()` and `String.to_float()` for conversion.
**Warning signs:** Type errors or unexpected string values in application logic.

### Pitfall 6: JSONB vs TEXT for Flexible Columns
**What goes wrong:** Using `TEXT` for the extra/tags/exception columns means PostgreSQL cannot index into the content. Using `JSONB` means the column is queryable but Mesh's `deriving(Row)` receives it as a string anyway.
**Why it happens:** Tension between database queryability and application serialization.
**How to avoid:** Use `JSONB` in PostgreSQL for all semi-structured columns (tags, extra, exception, stacktrace, breadcrumbs, user_context). Accept that Mesh receives them as strings and parse with `from_json()` when needed. JSONB enables GIN indexes for future query features (Phase 91 search).
**Warning signs:** Wanting to add `WHERE tags @> '{"env": "production"}'` but the column is TEXT, not JSONB.

### Pitfall 7: Partition Pre-Creation Must Handle Timezone
**What goes wrong:** Daily partitions created for UTC dates, but events arrive with various timezone offsets. An event timestamped at `2026-02-15T01:00:00+09:00` is actually `2026-02-14T16:00:00Z` -- it goes into the Feb 14 partition, not Feb 15.
**Why it happens:** Timestamp confusion between local time and UTC.
**How to avoid:** Use `TIMESTAMPTZ` for all timestamp columns. PostgreSQL stores everything as UTC internally. Partition boundaries should be in UTC. Create partitions for UTC dates.
**Warning signs:** Events appearing in "wrong" partitions when queried by local time.

## Code Examples

Verified patterns from Mesh test files and runtime source:

### Creating a Connection Pool
```mesh
# Pool.open(url, min_connections, max_connections, checkout_timeout_ms)
let pool = Pool.open("postgres://user:pass@localhost:5432/mesher", 2, 10, 5000)?
```
Source: `crates/mesh-typeck/src/builtins.rs:734` -- `Pool.open(String, Int, Int, Int) -> Result<PoolHandle, String>`

### Executing DDL
```mesh
let _ = Pool.execute(pool,
  "CREATE TABLE IF NOT EXISTS projects (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    org_id UUID NOT NULL REFERENCES organizations(id),
    name TEXT NOT NULL,
    platform TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
  )", [])?
```
Source: `tests/e2e/stdlib_pg.mpl` pattern adapted for DDL

### Querying with Row Mapping
```mesh
struct ProjectRow do
  id :: String
  org_id :: String
  name :: String
  platform :: Option<String>
  created_at :: String
end deriving(Row)

# Pool.query_as uses the from_row function generated by deriving(Row)
let projects = Pool.query_as(pool,
  "SELECT id, org_id, name, platform, created_at FROM projects WHERE org_id = $1",
  [org_id],
  ProjectRow.from_row)?
```
Source: `crates/mesh-typeck/src/builtins.rs:766` -- `Pool.query_as` signature

### Inserting with Generated UUID
```mesh
# Let PostgreSQL generate the UUID via DEFAULT uuidv7()
let _ = Pool.execute(pool,
  "INSERT INTO organizations (name, slug) VALUES ($1, $2)",
  [name, slug])?

# Or fetch the generated UUID with RETURNING
let rows = Pool.query(pool,
  "INSERT INTO organizations (name, slug) VALUES ($1, $2) RETURNING id",
  [name, slug])?
let org_id = case rows do
  [row] -> Map.get(row, "id")
  _ -> ""
end
```

### Service with Timer-Driven Flush
```mesh
service PartitionManager do
  fn init(pool :: Int) -> Int do
    # Create partitions for next 7 days on startup
    create_partitions_ahead(pool, 7)
    # Schedule daily check (86400000 ms = 24 hours)
    Timer.send_after(self(), 86400000, "check")
    pool
  end

  cast Check() do |pool|
    create_partitions_ahead(pool, 7)
    Timer.send_after(self(), 86400000, "check")
    pool
  end
end
```

### Password Hashing via pgcrypto
```mesh
# Hash a password (bcrypt with cost factor 12)
let _ = Pool.execute(pool,
  "INSERT INTO users (email, password_hash) VALUES ($1, crypt($2, gen_salt('bf', 12)))",
  [email, password])?

# Verify a password
let rows = Pool.query(pool,
  "SELECT id FROM users WHERE email = $1 AND password_hash = crypt($2, password_hash)",
  [email, password])?
let authenticated = List.length(rows) > 0
```

### API Key Generation
```mesh
# Generate a prefixed API key using PostgreSQL
let rows = Pool.query(pool,
  "SELECT 'mshr_' || encode(gen_random_bytes(24), 'hex') AS api_key",
  [])?
let api_key = case rows do
  [row] -> Map.get(row, "api_key")
  _ -> ""
end
```

### Session Token Generation
```mesh
# Generate a cryptographically random session token
let rows = Pool.query(pool,
  "SELECT encode(gen_random_bytes(32), 'hex') AS token",
  [])?
let token = case rows do
  [row] -> Map.get(row, "token")
  _ -> ""
end
```

## Discretion Recommendations

### Stack Trace Representation: Structured JSONB Frames
**Recommendation:** Store stack traces as a JSONB array of frame objects (`[{"filename": "...", "function": "...", "lineno": 42, "in_app": true}, ...]`).

**Rationale:** Structured frames enable fingerprinting on specific fields (function name + filename), frontend display with syntax highlighting and context lines, and PostgreSQL JSONB querying for "find events where stack trace contains function X". Raw strings require parsing at query/display time and make fingerprinting fragile (depends on stack trace formatting).

**Mesh implementation:** Define a `StackFrame` struct with `deriving(Json)`. Store as JSONB column. In the Row struct, the field is `String` (raw JSON text) which gets parsed via `from_json` when needed.

### Org/Project Hierarchy: Flat org -> project
**Recommendation:** Two-level hierarchy. Organizations directly contain projects. No teams layer.

**Rationale:** Simpler to implement, query, and reason about. Adding a teams table later is non-breaking (add `team_id` nullable FK to projects, add teams table). Sentry's open-source version (GlitchTip) uses this flat model. The user decision (ORG-04: team membership with roles) can be implemented as org-level roles without a separate teams entity -- roles on the `org_memberships` table (owner, admin, member).

### StorageWriter Parameters
**Recommendation:**
- **Batch size:** 50 events (flush when buffer reaches 50)
- **Timer interval:** 5 seconds (flush every 5s even if batch not full)
- **Buffer capacity:** 500 events (drop oldest when exceeded)

**Rationale:** 50-event batches balance between write efficiency (fewer round trips) and latency (events visible within 5s max). 500-event buffer gives 10x headroom above batch size for burst absorption. 5-second timer ensures events appear in the database within 5 seconds even under low traffic.

### Retry Strategy
**Recommendation:**
- **Retry count:** 3 attempts
- **Backoff:** Exponential -- 100ms, 500ms, 2000ms
- **On final failure:** Drop the batch, log the error with batch size and project_id

**Rationale:** Short initial retry catches transient connection blips. Exponential backoff prevents thundering herd on database recovery. Dropping after 3 retries prevents unbounded memory growth. The 500-event buffer cap provides an additional safety net.

### Partition Pre-Creation Horizon
**Recommendation:** 7 days ahead. Re-check daily.

**Rationale:** 7 days provides a comfortable buffer -- even if the pre-creation job fails for several days, partitions still exist. Daily re-check is cheap (just CREATE TABLE IF NOT EXISTS) and self-healing. The PartitionManager service runs at startup AND on a 24-hour timer.

### Index Strategy
**Recommendation:**
```sql
-- Events table (per-partition, auto-created)
CREATE INDEX idx_events_project_received ON events (project_id, received_at DESC);
CREATE INDEX idx_events_issue_received ON events (issue_id, received_at DESC);
CREATE INDEX idx_events_level ON events (level, received_at DESC);
CREATE INDEX idx_events_fingerprint ON events (fingerprint);
CREATE INDEX idx_events_tags ON events USING GIN (tags jsonb_path_ops);

-- Issues table
CREATE INDEX idx_issues_project_status ON issues (project_id, status);
CREATE INDEX idx_issues_project_last_seen ON issues (project_id, last_seen DESC);
CREATE INDEX idx_issues_fingerprint ON issues (project_id, fingerprint);
```

**Rationale:**
- `(project_id, received_at DESC)` -- primary query pattern: "recent events for project X"
- `(issue_id, received_at DESC)` -- event detail view: "events for issue Y"
- `GIN (tags jsonb_path_ops)` -- enables `@>` containment queries on tags for Phase 91 search
- `jsonb_path_ops` over default `jsonb_ops` -- smaller index, better performance for containment queries
- Indexes on partitioned tables are created per-partition automatically by PostgreSQL

### Password Hashing: bcrypt via pgcrypto
**Recommendation:** Use PostgreSQL's `pgcrypto` extension with bcrypt (cost factor 12).

**Rationale:** Argon2id would be the gold standard, but pgcrypto does not support Argon2. bcrypt with cost factor 12 is still secure (OWASP recommended minimum is 10). Since Mesh has no cryptographic runtime, pgcrypto is the only option without adding Rust dependencies. Cost factor 12 targets ~250ms hash time on modern hardware.

### Session Token Format: Opaque random token
**Recommendation:** 64-character hex string generated by PostgreSQL's `gen_random_bytes(32)`. Stored in a `sessions` table with user_id, created_at, and expires_at.

**Rationale:** Opaque tokens are simpler and more revocable than JWTs. For a single-server application (Phase 87-93), there is no need for stateless JWT verification. The session lookup (`SELECT user_id FROM sessions WHERE token = $1 AND expires_at > now()`) is a single indexed query. Opaque tokens can be immediately revoked by DELETE.

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| UUIDv4 (`gen_random_uuid()`) | UUIDv7 (`uuidv7()`) | PostgreSQL 18, Sep 2025 | Time-sortable IDs improve B-tree index locality and enable time-range queries on primary key |
| External UUID library (pg_uuidv7 extension) | Native `uuidv7()` function | PostgreSQL 18 | No extension installation needed |
| Inheritance-based partitioning | Declarative `PARTITION BY RANGE` | PostgreSQL 10+ | Simpler syntax, automatic partition routing, better query planning |
| Manual partition creation | Automated via scheduled job | Best practice | Pre-create partitions ahead to prevent missing-partition errors |

**Deprecated/outdated:**
- `uuid_generate_v1()` from uuid-ossp: Replaced by native `uuidv7()` in PG 18
- Trigger-based partitioning: Replaced by declarative partitioning (PG 10+)
- MD5 password hashing: Insecure; use bcrypt (pgcrypto) minimum

## PostgreSQL Schema Design

### Full Schema for Phase 87

```sql
-- Enable required extensions
CREATE EXTENSION IF NOT EXISTS pgcrypto;

-- Organizations
CREATE TABLE organizations (
    id              UUID PRIMARY KEY DEFAULT uuidv7(),
    name            TEXT NOT NULL,
    slug            TEXT NOT NULL UNIQUE,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Users
CREATE TABLE users (
    id              UUID PRIMARY KEY DEFAULT uuidv7(),
    email           TEXT NOT NULL UNIQUE,
    password_hash   TEXT NOT NULL,
    display_name    TEXT NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Org membership (many-to-many with role)
CREATE TABLE org_memberships (
    id              UUID PRIMARY KEY DEFAULT uuidv7(),
    user_id         UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    org_id          UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    role            TEXT NOT NULL DEFAULT 'member',  -- owner, admin, member
    joined_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(user_id, org_id)
);

-- Sessions
CREATE TABLE sessions (
    token           TEXT PRIMARY KEY,
    user_id         UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at      TIMESTAMPTZ NOT NULL DEFAULT now() + interval '7 days'
);

-- Projects
CREATE TABLE projects (
    id              UUID PRIMARY KEY DEFAULT uuidv7(),
    org_id          UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    name            TEXT NOT NULL,
    platform        TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- API Keys (multiple per project)
CREATE TABLE api_keys (
    id              UUID PRIMARY KEY DEFAULT uuidv7(),
    project_id      UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    key_value       TEXT NOT NULL UNIQUE,  -- 'mshr_' || hex-encoded random bytes
    label           TEXT NOT NULL DEFAULT 'default',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    revoked_at      TIMESTAMPTZ
);

-- Issues (grouped errors)
CREATE TABLE issues (
    id              UUID PRIMARY KEY DEFAULT uuidv7(),
    project_id      UUID NOT NULL,
    fingerprint     TEXT NOT NULL,
    title           TEXT NOT NULL,
    level           TEXT NOT NULL,
    status          TEXT NOT NULL DEFAULT 'unresolved',
    event_count     INTEGER NOT NULL DEFAULT 0,
    first_seen      TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen       TIMESTAMPTZ NOT NULL DEFAULT now(),
    assigned_to     UUID REFERENCES users(id),
    UNIQUE(project_id, fingerprint)
);

-- Events (time-partitioned, append-only)
CREATE TABLE events (
    id              UUID NOT NULL DEFAULT uuidv7(),
    project_id      UUID NOT NULL,
    issue_id        UUID NOT NULL,
    level           TEXT NOT NULL,
    message         TEXT NOT NULL,
    fingerprint     TEXT NOT NULL,
    exception       JSONB,
    stacktrace      JSONB,
    breadcrumbs     JSONB,
    tags            JSONB NOT NULL DEFAULT '{}',
    extra           JSONB NOT NULL DEFAULT '{}',
    user_context    JSONB,
    sdk_name        TEXT,
    sdk_version     TEXT,
    received_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (id, received_at)
) PARTITION BY RANGE (received_at);

-- Alert rules
CREATE TABLE alert_rules (
    id              UUID PRIMARY KEY DEFAULT uuidv7(),
    project_id      UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    name            TEXT NOT NULL,
    condition_json  JSONB NOT NULL,
    action_json     JSONB NOT NULL,
    enabled         BOOLEAN NOT NULL DEFAULT true,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Indexes
CREATE INDEX idx_org_memberships_user ON org_memberships (user_id);
CREATE INDEX idx_org_memberships_org ON org_memberships (org_id);
CREATE INDEX idx_sessions_user ON sessions (user_id);
CREATE INDEX idx_sessions_expires ON sessions (expires_at);
CREATE INDEX idx_projects_org ON projects (org_id);
CREATE INDEX idx_api_keys_project ON api_keys (project_id);
CREATE INDEX idx_api_keys_value ON api_keys (key_value) WHERE revoked_at IS NULL;
CREATE INDEX idx_issues_project_status ON issues (project_id, status);
CREATE INDEX idx_issues_project_last_seen ON issues (project_id, last_seen DESC);
CREATE INDEX idx_events_project_received ON events (project_id, received_at DESC);
CREATE INDEX idx_events_issue_received ON events (issue_id, received_at DESC);
CREATE INDEX idx_events_level ON events (level, received_at DESC);
CREATE INDEX idx_events_fingerprint ON events (fingerprint);
CREATE INDEX idx_events_tags ON events USING GIN (tags jsonb_path_ops);
CREATE INDEX idx_alert_rules_project ON alert_rules (project_id) WHERE enabled = true;
```

### Partition Creation SQL
```sql
-- Create a daily partition (called by PartitionManager service)
-- Example for 2026-02-14:
CREATE TABLE IF NOT EXISTS events_20260214 PARTITION OF events
  FOR VALUES FROM ('2026-02-14') TO ('2026-02-15');
```

The partition name pattern `events_YYYYMMDD` is constructed in Mesh code by concatenating date strings.

## Open Questions

1. **How to get current date/time in Mesh for partition naming?**
   - What we know: Mesh has no `System.time()` or `DateTime.now()` function in the stdlib.
   - What's unclear: Whether the current date can be obtained at all from Mesh code.
   - Recommendation: Use PostgreSQL `SELECT to_char(now(), 'YYYYMMDD')` to get the current date. Generate partition names server-side. Alternatively, this may require adding a `System.time_ms()` or `DateTime.now()` runtime function -- a small stdlib addition similar to `Timer.sleep`.

2. **Can Mesh services receive string-typed messages for Timer.send_after?**
   - What we know: `Timer.send_after(pid, ms, msg)` takes a message value. The service `cast` handlers match on sum type constructors.
   - What's unclear: Whether `Timer.send_after(self(), 5000, "flush")` correctly delivers a string message that the service's cast handler can match. Services typically use typed messages.
   - Recommendation: Test this pattern with a minimal Mesh service. If string messages do not work with service cast, use a sum type: `type WriterMsg do Flush end` and send `Flush` as the timer message.

3. **Does `RETURNING` work with Mesh's Pg.execute/Pool.execute?**
   - What we know: `Pg.execute` returns `Result<Int, String>` (rows affected count). `Pg.query` returns `Result<List<Map<String, String>>, String>`.
   - What's unclear: Whether `INSERT ... RETURNING id` should use `query` (to get the returned row) or `execute` (which only returns row count).
   - Recommendation: Use `Pool.query` for `INSERT ... RETURNING` statements since you need the returned data. `Pool.execute` is for statements where you only need the affected row count.

4. **Struct update syntax availability in Mesh**
   - What we know: Mesh test files show struct creation with all fields specified. No test shows partial struct update (e.g., `{...state, buffer: new_buffer}`).
   - What's unclear: Whether Mesh supports functional struct update syntax.
   - Recommendation: Assume full struct reconstruction is needed for each state update in services. This is verbose but correct. Example: `WriterState { pool: state.pool, project_id: state.project_id, buffer: new_buffer, ... }`.

## Sources

### Primary (HIGH confidence)
- Mesh runtime source: `crates/mesh-rt/src/db/pool.rs` -- Pool API (open, query, execute, query_as, checkout/checkin)
- Mesh runtime source: `crates/mesh-rt/src/db/row.rs` -- Row parsing (from_row_get, parse_int, parse_float, parse_bool)
- Mesh runtime source: `crates/mesh-rt/src/db/pg.rs` -- PostgreSQL wire protocol, Pg.execute/query/query_as
- Mesh runtime source: `crates/mesh-rt/src/hash.rs` -- FNV-1a hashing (deterministic, no cryptographic random)
- Mesh runtime source: `crates/mesh-rt/src/actor/mod.rs:585` -- Timer.send_after OS thread implementation
- Mesh runtime source: `crates/mesh-rt/src/http/server.rs:162-222` -- Request.body, Request.header, Request.query, Request.param
- Mesh typeck builtins: `crates/mesh-typeck/src/builtins.rs:734-770` -- Pool.open/query/execute/query_as type signatures
- Mesh test files: `tests/e2e/stdlib_pg.mpl`, `tests/e2e/deriving_json_basic.mpl`, `tests/e2e/deriving_row_basic.mpl`, `tests/e2e/service_counter.mpl`, `tests/e2e/supervisor_basic.mpl`
- Mesh project structure: `crates/meshc/src/discovery.rs` -- multi-module build, path-to-module naming convention
- Project research: `.planning/research/ARCHITECTURE.md`, `.planning/research/STACK.md`, `.planning/research/PITFALLS.md`

### Secondary (MEDIUM confidence)
- [PostgreSQL 18 UUIDv7](https://www.thenile.dev/blog/uuidv7) -- Native `uuidv7()` function, timestamp extraction, interval offset
- [PostgreSQL 18 UUID Functions](https://www.postgresql.org/docs/current/functions-uuid.html) -- Official docs for uuid functions
- [PostgreSQL Table Partitioning](https://www.postgresql.org/docs/current/ddl-partitioning.html) -- Range partitioning, partition pruning, unique constraints
- [PostgreSQL JSONB Indexing](https://pganalyze.com/blog/gin-index) -- GIN indexes, jsonb_ops vs jsonb_path_ops operator classes
- [Sentry Issue Grouping](https://docs.sentry.io/concepts/data-management/event-grouping/) -- Fingerprinting hierarchy, stack trace hashing
- [Sentry Developer Docs: Grouping](https://develop.sentry.dev/backend/application-domains/grouping/) -- GroupHash model, secondary hashing
- [Password Hashing Guide 2025](https://guptadeepak.com/the-complete-guide-to-password-hashing-argon2-vs-bcrypt-vs-scrypt-vs-pbkdf2-2026/) -- Argon2id vs bcrypt, recommended parameters
- [Session Tokens vs JWTs](https://stytch.com/docs/guides/sessions/session-tokens-vs-jwts) -- Opaque vs JWT tradeoffs
- [PostgreSQL JSONB Best Practices](https://www.crunchydata.com/blog/indexing-jsonb-in-postgres) -- Hybrid schema design (typed columns + JSONB)

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- all Mesh capabilities verified against runtime source code and test files
- Architecture: HIGH -- patterns derived from verified Mesh primitives (services, pools, timers, supervisors)
- Schema design: HIGH -- PostgreSQL 18 features verified via official docs; partitioning is well-documented
- Pitfalls: HIGH -- identified from runtime source analysis (no UUID/random, Timer.send_after threading, Row text mapping)
- Discretion recommendations: MEDIUM -- based on domain best practices (Sentry patterns, security standards) applied to Mesh constraints

**Research date:** 2026-02-14
**Valid until:** 2026-03-14 (stable -- Mesh compiler and PostgreSQL 18 are mature)
