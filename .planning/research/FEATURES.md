# Feature Landscape

**Domain:** Core monitoring/observability SaaS platform (Mesher) -- log ingestion, error tracking, real-time streaming, alerting, dashboards. NOT full observability (no distributed tracing, no APM, no infrastructure metrics).
**Researched:** 2026-02-14
**Confidence:** HIGH for core feature set (well-documented domain with Sentry, GlitchTip, Highlight.io as references). MEDIUM for SDK design specifics (depends on Mesh's capabilities as a client library target). HIGH for architecture patterns (standard ingestion pipeline patterns).

## Existing System Baseline

Before defining features, here is what Mesh already provides (verified from PROJECT.md):

- **HTTP server:** Hand-rolled HTTP/1.1 parser with TLS (HTTPS), path parameters, method routing, middleware pipeline. Actor-per-connection model with crash isolation.
- **WebSocket server:** RFC 6455 with TLS (wss://), rooms/channels with join/leave/broadcast, heartbeat, actor-per-connection. Cross-node room broadcast via distributed actors.
- **PostgreSQL driver:** Pure wire protocol, SCRAM-SHA-256 auth, TLS, connection pooling (min/max/timeout), transactions with panic-safe rollback, `deriving(Row)` for struct mapping, `Pool.query_as` for one-step query+hydration.
- **SQLite driver:** Bundled (zero system deps), parameterized queries.
- **JSON serde:** `deriving(Json)` for automatic encode/decode, nested structs, Option, List, Map, tagged union sum types.
- **Actor system:** Lightweight actors with typed message passing, supervision trees with let-it-crash, process monitoring, linked processes, selective receive.
- **Distributed actors:** Location-transparent PIDs, TLS-encrypted inter-node connections, cookie auth, mesh formation, global process registry, remote spawn, cross-node supervision.
- **Language features:** HM type inference, pattern matching with exhaustiveness, sum types, traits with associated types, iterators, From/Into, generic monomorphization, modules, pipe operator.
- **Timers:** `Timer.sleep`, `Timer.send_after` for delayed messages, receive timeouts.

### What Mesh Does NOT Have (Relevant Gaps)

- **No HTTP client** -- cannot make outbound HTTP requests (needed for webhook alerting). Would need to be built or use a shell-out pattern.
- **No email sending** -- alerting via email requires SMTP or external service integration.
- **No full-text search engine** -- PostgreSQL `LIKE`/`tsvector` or build custom indexing.
- **No background job queue** -- actors with timers serve this purpose (evaluate alert rules on intervals).
- **No template engine** -- email templates would be string interpolation.
- **No rate limiting primitive** -- must be built from actor state + timers.
- **No cursor/pagination primitive** -- must be built from SQL OFFSET/LIMIT or keyset pagination.

---

## Table Stakes

Features users expect from a monitoring platform. Missing = product feels incomplete or unusable.

### 1. Event Ingestion API

The foundational capability -- accepting error events and log entries from client applications.

| Feature | Why Expected | Complexity | Mesh Dependency | Notes |
|---------|--------------|------------|-----------------|-------|
| POST `/api/v1/events` endpoint | Every monitoring platform has an HTTP ingestion endpoint. This is literally the entry point for all data. | **Low** | HTTP server, JSON serde | Accept JSON event payloads with required fields: `event_id`, `timestamp`, `platform`, `level`, `message`. |
| Authentication via DSN/API key | Events must be associated with a project. DSN (Data Source Name) embeds project ID + secret key in a URL. Sentry pioneered this pattern and every competitor uses it. | **Low** | HTTP middleware | DSN format: `https://<key>@<host>/<project_id>`. Parse from `X-Mesher-Auth` header or query string. |
| Event validation and normalization | Reject malformed events, normalize timestamps, trim oversized fields, set defaults. | **Med** | Pattern matching, sum types | Validate required fields, normalize timestamp to UTC, truncate message at 8KB, tags at 200 chars. |
| Bulk event ingestion | SDKs batch events for efficiency. Must accept arrays of events in a single request. | **Low** | JSON serde (List) | POST `/api/v1/events/bulk` accepting `List<Event>`. Process sequentially or fan out to actors. |
| Rate limiting per project | Protect the system from runaway clients. Must enforce events-per-minute limits per project and return 429 with `Retry-After` header. | **Med** | Actor state + timers | Per-project actor tracking event count per window. Sliding window or token bucket algorithm. |
| Response with event ID | Client needs confirmation the event was accepted and an ID for correlation. | **Low** | JSON serde | Return `{"id": "<event_id>"}` on 202 Accepted. |
| WebSocket ingestion for streaming | High-throughput clients benefit from persistent connections. Actor-per-connection model is natural here. | **Med** | WebSocket server, rooms | Connect once, stream events as JSON frames. Natural fit for Mesh's actor-per-connection. |

**Confidence: HIGH** -- Standard REST ingestion pattern. Mesh's HTTP server, JSON serde, and actor model directly support this.

### 2. Error Grouping and Fingerprinting

The feature that separates a monitoring platform from a log aggregator. Without grouping, users drown in individual events.

| Feature | Why Expected | Complexity | Mesh Dependency | Notes |
|---------|--------------|------------|-----------------|-------|
| Automatic fingerprinting from stack trace | Sentry's primary grouping mechanism. Identical stack traces (same file + function + line across frames) produce the same fingerprint hash. Users expect "100 occurrences of this error" not "100 separate errors." | **High** | Pattern matching, string ops | Hash stack trace frames: normalize frame data (strip line numbers for some languages, keep function names), compute SHA-256 of concatenated frame signatures. |
| Fallback to exception type + message | When no stack trace available, group by error type and message (with variable parts stripped). Standard Sentry fallback hierarchy. | **Med** | Pattern matching, string ops | Regex-strip numbers, UUIDs, hex addresses, file paths from error messages before hashing. |
| Fallback to raw message | Last resort when neither stack trace nor exception type available. Group by normalized message content. | **Low** | String ops | Strip parameters, compute hash of remaining text. |
| Custom fingerprint override | SDKs should be able to set an explicit fingerprint array on events, overriding automatic grouping. Sentry supports this and power users rely on it. | **Low** | JSON serde (List<String>) | If `fingerprint` field present in event, use it directly instead of computing. |
| Issue creation from first event | First event with a new fingerprint creates an "Issue" -- the aggregate container. Subsequent events with the same fingerprint increment the issue's event count. | **Med** | PostgreSQL, transactions | Insert into `issues` table on first occurrence. Use `ON CONFLICT DO UPDATE` for atomic upsert. |
| Event count and first/last seen timestamps | Each issue tracks total events, first seen, and last seen. Essential for triage. | **Low** | PostgreSQL | `UPDATE issues SET event_count = event_count + 1, last_seen = NOW() WHERE fingerprint = $1`. |

**Confidence: HIGH** -- Sentry's grouping algorithm is well-documented in their developer docs. The fallback hierarchy (stack trace -> exception -> message) is the industry standard.

### 3. Issue Lifecycle and Triage

Users need to track issues through a workflow: new -> acknowledged -> resolved -> regressed.

| Feature | Why Expected | Complexity | Mesh Dependency | Notes |
|---------|--------------|------------|-----------------|-------|
| Issue states: unresolved, resolved, archived | Minimum viable workflow. Sentry uses: unresolved, resolved, archived (with escalating sub-state). Start simpler. | **Low** | PostgreSQL, sum types | `status` column as enum: `Unresolved`, `Resolved`, `Archived`. Sum type maps directly. |
| Resolve an issue | Mark as fixed. If the same fingerprint appears again, auto-reopen (regression). | **Low** | PostgreSQL | `UPDATE issues SET status = 'Resolved', resolved_at = NOW()`. |
| Regression detection | If a resolved issue gets a new event, automatically change status back to `Unresolved` and flag as regressed. Users expect this -- it is how Sentry ensures fixed bugs stay fixed. | **Med** | PostgreSQL, event processing | During event ingestion: if matching issue is `Resolved`, set to `Unresolved` and set `is_regressed = true`. |
| Archive (mute) an issue | Low-priority or noisy issues can be archived to clean up the issue list. Should auto-unarchive if event volume spikes (escalation). | **Med** | PostgreSQL, actor timer | Track event velocity. If archived issue receives >10x normal volume in 1 hour, unarchive and flag as escalating. |
| Assign issue to user | Team members need to own issues. Simple foreign key to user. | **Low** | PostgreSQL | `assigned_to` column on issues table. |
| Delete and discard | Remove an issue and optionally discard all future events matching that fingerprint. Prevents known-noise from consuming quota. | **Med** | PostgreSQL, in-memory set | Maintain a discard set (fingerprints to silently drop). Check during ingestion before processing. |

**Confidence: HIGH** -- Sentry's issue lifecycle is thoroughly documented in their blog series and docs.

### 4. Project and Team Organization

Multi-tenant structure for organizing monitoring data.

| Feature | Why Expected | Complexity | Mesh Dependency | Notes |
|---------|--------------|------------|-----------------|-------|
| Organizations (tenants) | Top-level container. All data is scoped to an org. Required for any SaaS platform. | **Low** | PostgreSQL | `organizations` table with name, slug, created_at. |
| Projects within organizations | Each monitored application is a project. Issues, events, and settings are project-scoped. Sentry, Datadog, and every competitor uses this hierarchy. | **Low** | PostgreSQL | `projects` table with org_id FK, name, platform, DSN key. |
| DSN key generation per project | Each project gets a unique DSN for SDK configuration. Format: `https://<public_key>@<host>/<project_id>`. | **Low** | Crypto (random bytes) | Generate 32-char hex key on project creation. Store in `project_keys` table. |
| Team membership with roles | Users belong to orgs with roles: owner, admin, member. Controls who can manage projects, resolve issues, configure alerts. | **Med** | PostgreSQL, middleware | `memberships` table with user_id, org_id, role. Middleware checks role on protected endpoints. |
| API key / auth token management | Programmatic access for CI/CD, scripts, and custom integrations. | **Low** | PostgreSQL, crypto | `api_tokens` table with hashed token, user_id, scopes, expiry. |

**Confidence: HIGH** -- Standard SaaS multi-tenancy pattern. Well-understood RBAC model.

### 5. Search and Filtering

Users must be able to find specific events and issues. A monitoring platform without search is useless at scale.

| Feature | Why Expected | Complexity | Mesh Dependency | Notes |
|---------|--------------|------------|-----------------|-------|
| Issue list with filters | Filter issues by: status (unresolved/resolved/archived), level (error/warning/info), first seen, last seen, assigned user, event count. | **Med** | PostgreSQL | Dynamic SQL query building with WHERE clauses. Use parameterized queries for safety. |
| Full-text search on event messages | Users search for "NullPointerException" or "timeout" across all events. Table stakes for any log/error tool. | **Med** | PostgreSQL tsvector | Use PostgreSQL's built-in full-text search: `tsvector` column on events, `GIN` index, `to_tsquery` for searches. Avoids external search engine. |
| Tag-based filtering | Events carry tags (environment, release, server, custom). Users filter by `environment:production` or `release:v2.3.1`. | **Med** | PostgreSQL, JSONB or separate table | Store tags as JSONB on events table, or normalize into `event_tags` table with GIN index for fast lookup. |
| Time range filtering | Every query should be scoped to a time range. Default to last 24 hours. | **Low** | PostgreSQL | `WHERE timestamp BETWEEN $1 AND $2`. Index on timestamp column. |
| Pagination | Issue and event lists must paginate. Keyset pagination (cursor-based) preferred over OFFSET for performance at scale. | **Med** | PostgreSQL | Use `WHERE id > $cursor ORDER BY id LIMIT $page_size`. Return next cursor in response. |
| Sort by frequency, last seen, first seen | Users need to prioritize by most frequent, most recent, or newest issues. | **Low** | PostgreSQL | `ORDER BY event_count DESC` / `ORDER BY last_seen DESC` / `ORDER BY first_seen DESC`. |

**Confidence: HIGH** -- PostgreSQL full-text search is well-documented and sufficient for a monitoring platform's scale. No need for Elasticsearch at MVP.

### 6. Real-Time Event Streaming

Users expect to see events appear in their dashboard immediately, not after a page refresh.

| Feature | Why Expected | Complexity | Mesh Dependency | Notes |
|---------|--------------|------------|-----------------|-------|
| WebSocket stream of new events | Live tail -- events appear in real-time as they are ingested. Every modern monitoring tool has this. Sentry has live event feed, Datadog has live tail. | **Med** | WebSocket server, rooms | One WebSocket room per project. When an event is ingested, broadcast to the project's room. Mesh's room system handles this natively. |
| Filtered streaming | Users want to stream only errors (not warnings), or only events from production environment. | **Med** | WebSocket, pattern matching | Client sends filter criteria on connect. Server-side actor applies filters before forwarding events. |
| New issue notifications | When a brand new issue is created (not just a new event on existing issue), push a notification to connected dashboards. | **Low** | WebSocket rooms | Broadcast `{type: "new_issue", issue: {...}}` to project room on first-occurrence events. |
| Issue count updates | When an existing issue gets more events, update the count in real-time on the dashboard without full page reload. | **Low** | WebSocket rooms | Broadcast `{type: "issue_update", issue_id: ..., event_count: ...}` to project room. |
| Connection management and backpressure | Don't overwhelm slow clients. Buffer or drop old events if client can't keep up. | **Med** | Actor mailbox, WebSocket | Per-client actor mailbox provides natural backpressure. If mailbox fills, drop oldest undelivered events. |

**Confidence: HIGH** -- Mesh's WebSocket room system and actor-per-connection model are purpose-built for this. This is the strongest dogfooding opportunity.

### 7. Alerting System

Notify teams when something goes wrong. Without alerting, users must constantly watch the dashboard.

| Feature | Why Expected | Complexity | Mesh Dependency | Notes |
|---------|--------------|------------|-----------------|-------|
| Alert rules with conditions | "Alert when issue X has >100 events in 1 hour" or "Alert on any new issue with level=fatal." Sentry's default: alert on first occurrence of any new issue. | **High** | Actor system, timers, PostgreSQL | Alert rules stored in DB. Evaluator actor runs on a timer (e.g., every 60s), queries event counts against rule conditions. |
| Threshold-based alerts | "Error rate > N events per M minutes." The most common alert type. | **Med** | PostgreSQL aggregate queries | `SELECT COUNT(*) FROM events WHERE project_id = $1 AND timestamp > NOW() - interval '1 hour'`. Compare against threshold. |
| New issue alert | Alert immediately when a never-before-seen issue appears. This is Sentry's default alert rule. | **Low** | Event processing pipeline | During ingestion, if fingerprint is new (INSERT succeeded, not UPDATE), trigger alert. |
| Regression alert | Alert when a resolved issue regresses (reappears). Critical for teams that mark issues as fixed. | **Low** | Event processing pipeline | During regression detection (see Issue Lifecycle), trigger alert. |
| Alert notification via WebSocket | In-app notifications in the dashboard. Simplest notification channel, fully within Mesh's capabilities. | **Low** | WebSocket rooms | Broadcast alert to org-wide notification room. |
| Alert notification via webhook | POST alert payload to a user-configured URL. Enables Slack, Discord, PagerDuty integration without building each one. | **High** | **Needs HTTP client** | This requires outbound HTTP capability. Either build a minimal HTTP client in Mesh, or shell out to `curl`. Major gap. |
| Alert cooldown / deduplication | Don't send the same alert every 60 seconds. Enforce a cooldown period (e.g., alert at most once per hour per rule). | **Med** | Actor state, timers | Track last_triggered timestamp per rule. Skip if within cooldown window. |
| Alert states: active, acknowledged, resolved | Track whether alerts have been seen and addressed. Auto-resolve when condition no longer met. | **Med** | PostgreSQL, actor timers | `alerts` table with state machine. Evaluator actor checks if condition is still true, auto-resolves if not. |

**Confidence: MEDIUM** -- Alert rule evaluation is straightforward. The major concern is **webhook notification requiring an HTTP client**, which Mesh does not have. In-app (WebSocket) alerts are fully supported. Webhook alerts are the industry standard notification mechanism and will need the HTTP client gap addressed.

### 8. Dashboard and Visualization Data

The API must serve pre-aggregated data that a Vue frontend can render as charts and widgets.

| Feature | Why Expected | Complexity | Mesh Dependency | Notes |
|---------|--------------|------------|-----------------|-------|
| Event volume over time | Time-series data: events per hour/day for a project. Powers the main overview chart. Every monitoring dashboard has this. | **Med** | PostgreSQL, `date_trunc` | `SELECT date_trunc('hour', timestamp) as bucket, COUNT(*) FROM events WHERE project_id = $1 GROUP BY bucket ORDER BY bucket`. |
| Error breakdown by level | Pie/bar chart: how many fatal vs error vs warning vs info events. | **Low** | PostgreSQL | `SELECT level, COUNT(*) FROM events WHERE project_id = $1 GROUP BY level`. |
| Top issues by frequency | "Most frequent errors" list. The primary triage view. | **Low** | PostgreSQL | `SELECT * FROM issues WHERE project_id = $1 ORDER BY event_count DESC LIMIT 10`. |
| Events by tag (environment, release) | Breakdown by deployment context. "How many errors in production vs staging?" | **Med** | PostgreSQL, JSONB or tags table | Aggregate on tag values. If JSONB: `SELECT tags->>'environment', COUNT(*) GROUP BY 1`. |
| Issue event timeline | For a single issue: events over time. Shows if the issue is getting worse or better. | **Med** | PostgreSQL | `SELECT date_trunc('hour', timestamp), COUNT(*) FROM events WHERE issue_id = $1 GROUP BY 1`. |
| Project health summary | At-a-glance: total unresolved issues, events in last 24h, new issues today. Dashboard overview widget. | **Low** | PostgreSQL | Three simple COUNT queries aggregated into one response. |

**Confidence: HIGH** -- Pure SQL aggregation queries. PostgreSQL handles this well at moderate scale. The Vue frontend consumes JSON and renders with a charting library (Chart.js or similar).

### 9. Event Detail View

When a user clicks on an event, they need full context.

| Feature | Why Expected | Complexity | Mesh Dependency | Notes |
|---------|--------------|------------|-----------------|-------|
| Full event payload display | Show all event data: message, stack trace, tags, extra context, user info, breadcrumbs, timestamp, level. | **Low** | PostgreSQL, JSON serde | Store full event JSON in a `payload` JSONB column. Return directly to frontend. |
| Stack trace rendering | Formatted stack trace with file names, line numbers, function names. The core debugging view. | **Low** | JSON serde (frontend concern) | Backend stores stack trace as structured JSON (list of frames). Frontend renders with syntax highlighting. |
| Breadcrumbs (event trail) | Chronological list of actions/events leading up to the error. SDKs send these as part of the event payload. | **Low** | JSON serde | Stored as part of event payload. `breadcrumbs: [{timestamp, category, message, level}]`. |
| Tags display | Key-value pairs showing environment, release, server, custom tags. | **Low** | JSON serde | Already part of event payload. Frontend renders as tag chips. |
| Navigation between events in an issue | "Next event" / "Previous event" buttons within an issue. | **Low** | PostgreSQL | `SELECT id FROM events WHERE issue_id = $1 AND id > $2 ORDER BY id LIMIT 1`. |
| User context | Which user experienced the error. SDKs send user info (id, email, IP). | **Low** | JSON serde | Part of event payload. `user: {id, email, ip_address, username}`. |

**Confidence: HIGH** -- This is essentially storing and retrieving JSON documents. The complexity is in the frontend rendering, not the backend.

### 10. Data Retention

Events accumulate fast. Must manage storage lifecycle.

| Feature | Why Expected | Complexity | Mesh Dependency | Notes |
|---------|--------------|------------|-----------------|-------|
| Configurable retention period per project | "Keep events for 30/60/90 days." After that, delete. Sentry and all competitors offer this. | **Med** | PostgreSQL, actor timer | `retention_days` column on projects. Background actor runs daily: `DELETE FROM events WHERE project_id = $1 AND timestamp < NOW() - interval '$N days'`. |
| Preserve issue summaries after event deletion | When old events are purged, keep the issue record (fingerprint, count, first/last seen). Losing issue history is unacceptable. | **Low** | PostgreSQL | Only delete from `events` table. `issues` table is preserved. |
| Storage usage display | Show users how much storage each project uses. Needed for quota management. | **Med** | PostgreSQL | `SELECT pg_total_relation_size('events')` or track per-project with a materialized count. |
| Event sampling at ingestion | When volume is extreme, sample (keep 1 in N events). Reduces storage while preserving statistical accuracy. | **Med** | Actor state, random | Per-project sample rate config. During ingestion, generate random float, drop if > sample_rate. Track dropped count. |

**Confidence: HIGH** -- Standard data lifecycle management. PostgreSQL handles bulk deletes and partitioning well.

---

## Differentiators

Features that set Mesher apart from the competition. Not expected, but valuable.

| Feature | Value Proposition | Complexity | Mesh Dependency | Notes |
|---------|-------------------|------------|-----------------|-------|
| **Multi-node event processing** | Distribute event ingestion across multiple Mesh nodes using distributed actors. No single point of failure. Demonstrates Mesh's clustering capability under real load. | **High** | Distributed actors, global registry | Ingestion actors on multiple nodes. Global registry for service discovery. Cross-node event routing. This is THE differentiator as a dogfooding exercise. |
| **Actor-per-connection streaming** | Each WebSocket dashboard connection is its own actor with its own state and filters. Crashes in one connection never affect others. True isolation. | **Low** | WebSocket server, actors | Already how Mesh WebSocket works. Just needs filter state per actor. |
| **Supervision tree resilience** | If the event processing pipeline crashes, it automatically restarts via supervision. Zero manual intervention. Live demo of Mesh's fault tolerance. | **Med** | Supervision trees, crash recovery | Design processing pipeline as supervised actor tree. Intentionally stress-test crash recovery. |
| **Zero-dependency backend** | Single Mesh binary for the entire backend. No Kafka, no Redis, no Elasticsearch, no Zookeeper. Compare to self-hosted Sentry's 12+ services. GlitchTip-level simplicity. | **Med** | All existing Mesh features | PostgreSQL is the only external dependency. Actor mailboxes replace message queues. Actor state replaces Redis caches. PG full-text search replaces Elasticsearch. |
| **Live alert rule evaluation** | Alert rules evaluated by a dedicated actor that receives events via message passing, not by polling the database. Lower latency than cron-based evaluation. | **Med** | Actor system, pattern matching | Alert evaluator actor subscribes to event stream. Maintains in-memory counters. Fires alerts in real-time. |
| **Cross-node WebSocket broadcast** | Dashboard users connected to different Mesh nodes all see the same real-time events. Demonstrates cross-node room broadcast. | **Med** | Distributed actors, cross-node rooms | Already supported by Mesh's DIST_ROOM_BROADCAST. Just wire it into the event pipeline. |
| **Built with the language it monitors** | Meta-dogfooding: Mesher is built in Mesh, and Mesher monitors Mesh applications. The SDK is written in Mesh. Unique narrative. | **Low** | All of Mesh | Marketing differentiator and ultimate stress test. |

---

## Anti-Features

Features to explicitly NOT build. Each would expand scope beyond "core monitoring" into "full observability platform."

| Anti-Feature | Why Avoid | What to Do Instead |
|--------------|-----------|-------------------|
| **Distributed tracing / APM** | Requires span collection, trace assembly, waterfall visualization, service maps. Enormous scope. Sentry bolted this on later, separate from core error tracking. This is a full product. | Focus on error events and log entries. If users want tracing, they use Jaeger/Zipkin alongside Mesher. |
| **Infrastructure metrics** | CPU, memory, disk, network monitoring requires agents on hosts, time-series database, different query patterns. Datadog's core business, not ours. | Mesher monitors application errors, not infrastructure. Point users to Prometheus/Grafana for infra. |
| **Session replay** | Recording user browser sessions (DOM snapshots, mouse movements, clicks) requires specialized SDKs, massive storage, and a complex replay player. Highlight.io's differentiator. | Breadcrumbs provide lightweight context. Session replay is a separate product. |
| **Source map processing** | Unminifying JavaScript stack traces requires uploading, storing, and processing source maps during ingestion. Significant complexity for one platform. | Accept stack traces as-is. Document that source map processing is deferred. Users can use Sentry for JS-heavy needs. |
| **Release tracking and deploy integration** | Associating events with git commits, PRs, and deploy timestamps. Sentry's "suspect commits" feature. Requires deep SCM integration. | Accept `release` as a string tag on events. No commit-level resolution. |
| **Profiling** | Code-level performance profiling (CPU/memory flame graphs). Sentry added this as a separate product. Requires agent-level instrumentation. | Out of scope. Focus on error events. |
| **Custom dashboards builder** | Drag-and-drop dashboard creation with arbitrary widget placement. Grafana's core product. Enormous frontend complexity. | Provide fixed, well-designed dashboard layouts. Customization limited to filter/time-range selection. |
| **Log aggregation with query language** | Building a full log query language (like Datadog's or Splunk's) with parsing, regex, aggregation pipelines. This is a search engine project. | Full-text search on event messages via PostgreSQL. Structured tag filtering. No custom query DSL. |
| **Multi-region data residency** | Storing data in specific geographic regions for GDPR compliance. Requires multi-region infrastructure and routing logic. | Single-region deployment. Document data location. Compliance is a v2+ concern. |
| **Billing and usage-based pricing** | Metering, invoicing, plan management, Stripe integration. Full SaaS billing is a product in itself. | Track usage for display purposes. No billing system. Mesher is a dogfooding project, not a revenue product. |
| **Email alerting** | Requires SMTP client, email templates, delivery tracking, bounce handling. Mesh has no SMTP capability. | Use webhooks for external notification. In-app WebSocket alerts are the primary channel. |
| **Mobile SDK** | iOS/Android SDKs require platform-specific crash reporting (signal handlers, NDK, etc.). Each is a separate project. | Mesher SDKs target server-side applications only (Mesh, then possibly Python/Node). |
| **AI-powered grouping** | Sentry uses ML to improve grouping beyond fingerprints. Requires ML pipeline, training data, inference infrastructure. | Use deterministic fingerprinting (stack trace hash, exception type, message normalization). AI is a v2+ feature. |

---

## Feature Dependencies

```
Project/Org Setup (foundation -- everything depends on this)
  |
  +-> DSN Key Generation (projects need keys for SDK auth)
  |     |
  |     +-> Event Ingestion API (needs DSN to authenticate events)
  |           |
  |           +-> Event Validation & Normalization
  |           |     |
  |           |     +-> Error Grouping / Fingerprinting (needs validated events)
  |           |     |     |
  |           |     |     +-> Issue Creation (first occurrence of fingerprint)
  |           |     |     |     |
  |           |     |     |     +-> Issue Lifecycle (resolve, archive, assign)
  |           |     |     |     |
  |           |     |     |     +-> Regression Detection (needs resolved issues + new events)
  |           |     |     |
  |           |     |     +-> Discard Set (needs fingerprints to filter against)
  |           |     |
  |           |     +-> Real-Time Streaming (broadcast ingested events to WebSocket rooms)
  |           |     |     |
  |           |     |     +-> Filtered Streaming (needs events + filter criteria)
  |           |     |     |
  |           |     |     +-> Issue Count Updates (needs issue IDs from grouping)
  |           |     |
  |           |     +-> Alert Rule Evaluation (needs event stream to evaluate against rules)
  |           |           |
  |           |           +-> Alert Notifications (WebSocket first, webhook later)
  |           |           |
  |           |           +-> Alert Cooldown / Dedup (needs alert history)
  |           |
  |           +-> Event Storage in PostgreSQL (needs validated, normalized events)
  |                 |
  |                 +-> Search and Filtering (needs stored events)
  |                 |     |
  |                 |     +-> Full-Text Search (needs tsvector index on events)
  |                 |     |
  |                 |     +-> Tag-Based Filtering (needs tags stored/indexed)
  |                 |
  |                 +-> Dashboard Aggregation Queries (needs event data)
  |                 |
  |                 +-> Data Retention (needs events to age out)
  |
  +-> Team Membership / RBAC (needs orgs and users)
        |
        +-> Issue Assignment (needs users and issues)

Multi-Node Clustering (can be layered on top of single-node at any time)
  |
  +-> Distributed Event Ingestion (multiple ingestion nodes)
  |
  +-> Cross-Node WebSocket Broadcast (real-time streaming across nodes)
  |
  +-> Global Process Registry for service discovery

SDK (parallel track -- can be built independently)
  |
  +-> Mesh SDK (captures errors, sends to ingestion API)
  |
  +-> JavaScript SDK (optional, for monitoring web apps)
```

**Key ordering insight:** The dependency chain is linear and deep. Project setup must come first, then ingestion, then grouping, then everything else. Real-time streaming and alerting both depend on the ingestion pipeline being complete. Multi-node clustering is an overlay that can be added to any phase without changing the single-node architecture. The SDK is a parallel workstream.

---

## MVP Recommendation

Prioritize by dependency order and dogfooding value:

1. **Project/Org data model + auth** -- Foundation. Create orgs, projects, DSN keys, user accounts, API tokens. Without this, nothing else works. Low complexity, pure PostgreSQL schema + CRUD endpoints.

2. **Event ingestion API** -- Core pipeline. HTTP POST endpoint accepting events, validating, storing in PostgreSQL. This immediately stress-tests HTTP server, JSON serde, connection pooling, and actor concurrency. Highest dogfooding value per line of code.

3. **Error grouping and issue creation** -- The feature that makes this a monitoring platform, not a log database. Fingerprint computation using pattern matching and string operations. Issue upsert with PostgreSQL transactions. Exercises `deriving(Row)`, `deriving(Json)`, sum types.

4. **Real-time WebSocket streaming** -- The most impressive demo feature and best dogfooding of WebSocket rooms. Events flow from ingestion to dashboard in real-time via actor message passing. Exercises WebSocket, rooms, actor-per-connection.

5. **Issue lifecycle (resolve/archive/regress)** -- Makes the platform usable for actual triage. State machine in PostgreSQL. Regression detection during ingestion. Low incremental complexity.

6. **Search and filtering + dashboard data** -- Makes the platform useful for investigation. PostgreSQL full-text search, tag filtering, time-series aggregation. Exercises complex SQL with parameterized queries.

7. **Alerting (rules + WebSocket notifications)** -- Closes the loop: users don't have to watch the dashboard. Actor-based rule evaluator with timer-driven evaluation. Start with in-app WebSocket notifications only (defer webhooks until HTTP client exists).

8. **Data retention** -- Background actor with daily cleanup. Exercises Timer.send_after for scheduling.

9. **Multi-node clustering** -- The ultimate dogfooding phase. Distribute ingestion and streaming across multiple Mesh nodes. Exercises distributed actors, global registry, cross-node rooms.

**Defer to follow-up:**
- Webhook alerting (requires HTTP client capability in Mesh -- significant language work)
- SDK for external languages (Python, Node, etc.)
- Source map processing
- Advanced dashboard customization
- Log aggregation query language
- Email notifications

---

## Complexity Summary

| Feature Area | Complexity | Primary Mesh Features Exercised |
|--------------|------------|-------------------------------|
| Project/Org/Auth | Low | HTTP server, PostgreSQL, JSON serde, middleware |
| Event Ingestion | Med | HTTP server, WebSocket, actors, connection pooling, JSON serde |
| Error Grouping | High | Pattern matching, string ops, PostgreSQL transactions, sum types |
| Issue Lifecycle | Low | PostgreSQL, sum types, pattern matching |
| Real-Time Streaming | Med | WebSocket rooms, actor-per-connection, message passing |
| Search & Filtering | Med | PostgreSQL full-text search, parameterized queries |
| Dashboard Data | Med | PostgreSQL aggregation, JSON serde |
| Alerting | High | Actor system, timers, supervision, PostgreSQL |
| Data Retention | Low | Actor timers, PostgreSQL bulk operations |
| Multi-Node | High | Distributed actors, global registry, cross-node rooms, clustering |
| Event Detail View | Low | PostgreSQL, JSON serde |

---

## Sources

- [Sentry Issue Grouping](https://docs.sentry.io/concepts/data-management/event-grouping/) -- HIGH confidence, authoritative
- [Sentry Grouping Developer Docs](https://develop.sentry.dev/backend/application-domains/grouping/) -- HIGH confidence, authoritative
- [Sentry Event Payloads](https://develop.sentry.dev/sdk/event-payloads/) -- HIGH confidence, authoritative
- [Sentry SDK Expected Features](https://develop.sentry.dev/sdk/expected-features/) -- HIGH confidence, authoritative
- [Sentry Envelope Protocol](https://develop.sentry.dev/sdk/envelopes/) -- HIGH confidence, authoritative
- [Sentry Rate Limiting](https://develop.sentry.dev/sdk/expected-features/rate-limiting/) -- HIGH confidence, authoritative
- [Sentry Issue States](https://docs.sentry.io/product/issues/states-triage/) -- HIGH confidence, authoritative
- [Sentry Workflow: Resolve](https://blog.sentry.io/the-sentry-workflow-resolve/) -- HIGH confidence
- [Sentry Fingerprint Rules](https://docs.sentry.io/concepts/data-management/event-grouping/fingerprint-rules/) -- HIGH confidence
- [GlitchTip Architecture](https://glitchtip.com/documentation/hosted-architecture/) -- HIGH confidence
- [GlitchTip](https://glitchtip.com/) -- HIGH confidence, open source reference
- [Highlight.io vs Sentry](https://www.highlight.io/compare/highlight-vs-sentry) -- MEDIUM confidence
- [Sentry Self-Hosted Developer Docs](https://develop.sentry.dev/self-hosted/) -- HIGH confidence
- [Datadog vs Sentry Comparison](https://betterstack.com/community/comparisons/datadog-vs-sentry/) -- MEDIUM confidence
- [System Design: Monitoring and Alerting](https://algomaster.io/learn/system-design-interviews/design-monitoring-and-alerting-system) -- MEDIUM confidence
- [Datadog Dashboard Widgets](https://docs.datadoghq.com/dashboards/widgets/) -- HIGH confidence, authoritative
- [Log Retention Policies](https://www.groundcover.com/learn/logging/log-retention-policies) -- MEDIUM confidence
- [ClickHouse Full-Text Search](https://www.cloudquery.io/blog/why-and-how-we-built-our-own-full-text-search-engine-with-clickhouse) -- MEDIUM confidence
- [Multi-Tenant RBAC Design](https://workos.com/blog/how-to-design-multi-tenant-rbac-saas) -- MEDIUM confidence
- [SDK Best Practices](https://www.speakeasy.com/blog/sdk-best-practices) -- MEDIUM confidence
- [Observability Trends 2026](https://www.ibm.com/think/insights/observability-trends) -- MEDIUM confidence
- [Google SRE Monitoring](https://sre.google/sre-book/monitoring-distributed-systems/) -- HIGH confidence, authoritative
