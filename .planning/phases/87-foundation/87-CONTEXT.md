# Phase 87: Foundation - Context

**Gathered:** 2026-02-14
**Status:** Ready for planning

<domain>
## Phase Boundary

Data model, database schema, storage writer, and org/project tenancy for the Mesher monitoring platform. All data types (Event, Issue, Project, Organization, AlertRule, User) and the PostgreSQL persistence layer exist so that subsequent phases can store and retrieve platform data. Includes full user auth and org membership.

</domain>

<decisions>
## Implementation Decisions

### Data model shape
- Semi-structured event payload: required core fields (message, level, timestamp) plus a flexible `extra` JSONB bag for arbitrary data
- Known interfaces (exception, breadcrumbs, user, contexts) get typed fields; unknown data goes in extra
- Tags stored as flat `Map<String, String>` — simple key-value pairs, easy to index and filter
- Severity levels follow Sentry convention: fatal, error, warning, info, debug (5 levels)

### Tenancy & DSN design
- Simple API key format per project (e.g., `mshr_abc123...`) — not Sentry-style DSN URLs
- Multiple API keys per project supported — allows rotation without downtime, separate keys per environment
- Full user auth included in this phase: users table with org membership, password hashing, session/token support
- API keys passed as header or query param for event ingestion

### Storage writer behavior
- Flush trigger: size + timer — flush when batch hits N events OR when timer fires, whichever comes first
- Flush failure: retry N times with backoff, then drop the batch and log error (prevents unbounded memory growth)
- Per-project StorageWriter actors — each project gets its own writer for isolation (one slow project can't block others)
- Bounded buffer with drop-oldest backpressure — cap at N events, drop oldest when full (monitoring data is time-sensitive)

### Schema partitioning
- Daily time partitions for the events table
- Partitions pre-created ahead of time by a scheduled job (no risk of missing partition on first event of new day)
- Time-only partitioning (no composite project+time) — project_id filtered via index
- UUIDv7 (time-sortable) for all entity primary keys — no ID collision across nodes

### Claude's Discretion
- Stack trace representation (structured frames vs raw string — optimize for querying and display)
- Org/project hierarchy depth (flat org→project or org→team→project)
- Exact batch size and timer interval for StorageWriter
- Retry count and backoff strategy for flush failures
- Buffer capacity limits
- Partition pre-creation horizon (how far ahead to create)
- Index strategy on events and issues tables
- Password hashing algorithm and session token format

</decisions>

<specifics>
## Specific Ideas

- Multiple API keys per project is important for zero-downtime key rotation and environment separation (production vs staging keys)
- Per-project writer actors provide fault isolation — one misbehaving project shouldn't affect others
- Drop-oldest on buffer full (not reject-new) — for monitoring, recent data matters more than old data

</specifics>

<deferred>
## Deferred Ideas

None — discussion stayed within phase scope

</deferred>

---

*Phase: 87-foundation*
*Context gathered: 2026-02-14*
