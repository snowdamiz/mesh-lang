# Project Research Summary

**Project:** Mesher -- Monitoring/Observability SaaS Platform
**Domain:** Core monitoring platform for log ingestion, error tracking, real-time streaming, alerting
**Researched:** 2026-02-14
**Confidence:** HIGH

## Executive Summary

Mesher is a monitoring and observability platform designed as the ultimate dogfooding exercise for the Mesh programming language. The entire backend is written in Mesh (.mpl files), using no additional frameworks or runtimes -- just the Mesh compiler, PostgreSQL for storage, and a separate Vue 3 frontend. This is a classic event ingestion pipeline with error grouping, real-time WebSocket streaming, alerting, and multi-tenant organization. The architecture is intentionally actor-heavy: every component (HTTP ingestion, event processing, database writing, WebSocket streaming, alert evaluation) is one or more supervised actors communicating via message passing.

The recommended approach is to build incrementally from foundation to complexity. Start with the data model (projects, events, issues), then build the ingestion pipeline (HTTP POST endpoint, event validation, actor-based processing), add error grouping (fingerprinting via pattern matching and string operations), implement real-time streaming (WebSocket rooms with actor-per-connection), layer on alerting (timer-driven rule evaluation), and finally add multi-node clustering as an overlay. The Vue 3 frontend is a separate workstream that consumes the REST API and WebSocket endpoints. The technology stack leverages existing Mesh capabilities: the HTTP server, WebSocket server with rooms, PostgreSQL driver with connection pooling, JSON serde via deriving(Json), and the actor system with supervision trees. The frontend uses Vue 3 + Vite + TypeScript with shadcn-vue (already established in the docs site), ECharts for time-series charting, TanStack libraries for data tables and virtual scrolling, and native fetch for API calls.

The key risks are the dual-bug problem (app bugs versus compiler bugs, since this is the first large Mesh application), timer management (Timer.send_after spawns OS threads, not sustainable for hundreds of recurring timers), and database schema design (must use time-based partitioning from day one or face query/retention nightmares). Mitigation strategies: maintain a compiler bug journal with minimal reproductions, use a single recurring timer actor for alerting instead of per-rule timers, and design PostgreSQL schema with native PARTITION BY RANGE on timestamps. The project should expect to surface compiler/runtime issues and fix them immediately rather than working around them long-term.

## Key Findings

### Recommended Stack

The stack is dictated by what Mesh already provides plus established frontend technologies. The backend is 100% Mesh with zero external dependencies beyond PostgreSQL. The frontend is a standalone Vue 3 SPA in the monorepo, communicating via REST and WebSocket.

**Core technologies:**

- **Mesh (.mpl source files)** -- Entire backend, compiled to native binary by meshc. Exercises HTTP server, WebSocket with rooms, PostgreSQL driver with pooling, JSON serde, actor concurrency, supervision trees, distributed actors for multi-node clustering.
- **PostgreSQL 16+** -- Storage layer with native range partitioning (no TimescaleDB), connection pooling, full-text search via tsvector. Schema designed for high write volume with time-series partitioning.
- **Vue 3 (^3.5.28) + Vite (^6.2) + TypeScript (^5.9.3)** -- Frontend framework, already used in docs site. Reactivity model fits real-time dashboard needs. Monorepo consistency.
- **ECharts (^6.0.0) + vue-echarts (^8.0.1)** -- Charting for time-series event volume, error breakdowns, issue timelines. Apache 2.0 licensed, handles large datasets, supports brush/zoom needed for monitoring.
- **shadcn-vue + Tailwind CSS 4.1** -- UI components (already established in docs site). Copy-paste component library built on reka-ui primitives with Tailwind styling.
- **TanStack Vue libraries** -- @tanstack/vue-table for sortable/filterable data tables (official shadcn-vue pattern), @tanstack/vue-virtual for log viewer virtual scrolling (60 FPS with 100K+ items).
- **date-fns (^4.1.0)** -- Date formatting with first-class timezone support (critical for monitoring events from different regions).
- **Native fetch** -- No axios/ky needed. Single backend, thin composable wrapper for auth headers and JSON parsing.

**What NOT to add:** TimescaleDB (extension dependency, operational complexity), InfluxDB/ClickHouse (Mesh has no driver), OpenTelemetry SDK (over-engineered for log/error use case), Nuxt (SSR unnecessary for dashboard SPA), Socket.io (Mesh uses raw RFC 6455 WebSocket), GraphQL (REST is simpler), Redis/message queue (actor mailboxes handle in-memory processing).

### Expected Features

Mesher is a monitoring platform, not a full observability platform. No distributed tracing, no APM, no infrastructure metrics, no session replay. Focus on error events and log entries.

**Must have (table stakes):**

- Event ingestion API (POST /api/v1/events) with DSN authentication and rate limiting
- Error grouping via fingerprinting (hash of stack trace + error type + normalized message)
- Issue lifecycle (open, resolved, archived) with regression detection
- Project/org multi-tenancy with RBAC
- Search and filtering (full-text via PostgreSQL tsvector, tag-based, time-range)
- Real-time WebSocket streaming of new events to dashboards (actor-per-connection, rooms)
- Alerting with threshold-based rules and webhook/WebSocket notifications
- Data retention with configurable per-project expiration
- Dashboard aggregation queries (event volume over time, error breakdowns, top issues)
- Event detail view with stack traces, breadcrumbs, tags, user context

**Should have (competitive/dogfooding differentiators):**

- Multi-node event processing via distributed actors (location-transparent send, global registry)
- Actor-per-connection WebSocket with crash isolation
- Supervision tree resilience (self-healing event pipeline)
- Zero-dependency backend (single binary, no Kafka/Redis/Elasticsearch)
- Live alert rule evaluation (actor receives event stream, maintains in-memory counters)
- Cross-node WebSocket broadcast (dashboards on different nodes see same real-time events)

**Defer to v2+ (anti-features for this milestone):**

- Distributed tracing/APM (span collection, service maps)
- Infrastructure metrics (CPU/memory monitoring)
- Session replay (DOM snapshots)
- Source map processing for JavaScript
- Release tracking with git commit integration
- Profiling (CPU/memory flame graphs)
- Custom dashboard builder (drag-and-drop widgets)
- Log query language (stick to full-text search + structured tags)
- Multi-region data residency
- Billing/usage-based pricing
- Email alerting (requires SMTP, which Mesh lacks)
- Mobile SDKs

### Architecture Approach

Pipeline-of-actors pattern with layered supervision. Every conceptual component is an actor or service (stateful actor). Data flows from ingestion actors through processing actors to three destinations: storage (batch write to PostgreSQL), streaming (WebSocket broadcast to rooms), and alerting (rule evaluation with in-memory state).

**Major components:**

1. **Ingestion Layer** -- HTTP Server (actor-per-connection) and WebSocket Ingestion Server accept events, validate, route to EventRouter service which dispatches by project_id to processor pool.
2. **Processing Layer** -- Processor actors fingerprint events (pattern matching on stack traces, string operations for normalization), resolve fingerprint to issue_id via Fingerprinter service (Map cache), enrich events with metadata, fan out to downstream.
3. **Storage Layer** -- StorageWriter service accumulates events in List buffer, flushes in batches to PostgreSQL via connection pool. Uses PostgreSQL transactions with ON CONFLICT for issue upserts.
4. **Streaming Layer** -- StreamBroadcaster actor receives enriched events, broadcasts to WebSocket rooms. WS Dashboard Server (actor-per-connection) joins clients to rooms with filters.
5. **Alerting Layer** -- AlertRuleStore service maintains rule cache, AlertEvaluator service runs on timer (single recurring timer actor, not per-rule timers), AlertNotifier sends notifications (WebSocket first, webhooks require HTTP client capability).
6. **Cluster Layer** (multi-node) -- NodeManager actor handles mesh formation via Node.connect, ClusterSync service uses Global.register for service discovery, cross-node room broadcast automatic via DIST_ROOM_BROADCAST.

**Key patterns:** Actor-per-connection with crash isolation (HTTP/WS), service as stateful singleton (EventRouter, Fingerprinter, StorageWriter use GenServer pattern), timer-driven periodic work (single timer actor for alerting, batch flush timer), pipeline fan-out via message passing (processor sends to storage/streaming/alerting independently), authentication middleware (extract API key, validate against projects table).

**Database schema:** Events table partitioned by timestamp (PARTITION BY RANGE, daily partitions), issues table with UNIQUE(project_id, fingerprint) for upsert, BRIN indexes on time columns, GIN indexes on JSONB tag columns. Batch writes via accumulated List with periodic flush. Retention via DROP old partitions (instant) not DELETE (hours).

### Critical Pitfalls

Top 5 risks that could derail the project:

1. **Dual-bug problem (app vs compiler)** -- This is the first large Mesh application. Every bug requires triage: is it my code or the compiler? Prevention: minimal reproduction discipline (isolate in standalone .mpl file), compiler bug journal, fix compiler bugs immediately instead of working around them. If a workaround would be more than a few lines, fix the compiler first.

2. **Timer.send_after thread explosion** -- Timer.send_after spawns an OS thread per call. Alerting with hundreds of rules firing every 10 seconds creates thousands of threads, exhausting OS limits. Prevention: Use a single recurring timer actor that evaluates all alert rules per tick, not one timer per rule. Batch alerts by evaluation interval.

3. **Unpartitioned events table** -- Without time-based partitioning, the events table becomes unusable after a week. Query latency grows linearly, autovacuum cannot keep pace, disk fills. Prevention: PARTITION BY RANGE on timestamp from day one. Create daily partitions at startup. Use BRIN indexes for timestamps. Retention via DROP partition, not DELETE + VACUUM.

4. **Map linear scan bottleneck** -- Mesh's Map uses vector with linear scan. Event metadata maps with 50+ keys make lookups O(n). 10 lookups per event at 10K events/second = 5M comparisons/second. Prevention: Limit metadata map sizes (max 32 tags), extract fields once into local variables, avoid repeated Map.get in hot paths. Profile before assuming this is the bottleneck -- may be fast enough.

5. **Alert storms from cascading rules** -- Single failure triggers hundreds of alerts (high error rate, slow response, connection failures all fire independently). Operators ignore all notifications. Prevention: Deduplication window (suppress duplicate alerts for same rule for 5 minutes), alert grouping (batch alerts that fire within 30 seconds), cooldown periods (do not re-fire same alert even if condition persists), start with fewer, broader rules.

## Implications for Roadmap

Based on combined research, the dependency chain is linear and deep. Project setup must come first, then ingestion, then grouping, then everything else. Multi-node clustering is an overlay that can be added to any phase without changing single-node architecture. The SDK is a parallel workstream.

### Phase 1: Foundation (Data Model + Database + Storage)

**Rationale:** Types are imported by every module. Database schema must support high write volume and time-series queries from the start -- retrofitting partitioning is a major migration. Storage writer establishes the batch-write pattern used by all subsequent phases.

**Delivers:** All struct definitions (Event, Issue, AlertRule, Project) with deriving(Json, Row), PostgreSQL schema with partitioning, StorageWriter service with batch accumulation and flush timer, database migrations runner.

**Addresses:** Database schema pitfall (Pitfall 4), establishes foundational types for all phases.

**Avoids:** Creating schema without partitioning, individual INSERT pattern that cannot scale.

**Research needed:** Standard pattern, skip research-phase. PostgreSQL partitioning is well-documented.

### Phase 2: Ingestion Pipeline (HTTP + Event Processing)

**Rationale:** Core pipeline must exist before any downstream features work. This immediately exercises HTTP server, JSON serde, actor concurrency, connection pooling. Highest dogfooding value per line of code.

**Delivers:** POST /api/v1/events endpoint with DSN authentication, rate limiting, validation, EventRouter service, Processor actor pool with fingerprinting and enrichment, fan-out to storage.

**Addresses:** Ingestion backpressure (Pitfall 14), Map linear scan (Pitfall 3), batch writes to storage.

**Uses:** HTTP server, middleware, deriving(Json), actor messaging, pattern matching, StorageWriter from Phase 1.

**Avoids:** No backpressure (spawn unlimited actors), synchronous call chains blocking pipeline, unbounded batch buffer.

**Research needed:** Standard pattern, skip research-phase. Event ingestion is well-understood (Sentry SDK docs).

### Phase 3: Error Grouping and Issue Lifecycle

**Rationale:** The feature that makes this a monitoring platform, not a log database. Fingerprinting exercises pattern matching, string operations, traits, PostgreSQL transactions with upserts.

**Delivers:** Fingerprint computation (hash of message + stack frames + error type), Fingerprinter service with cache, issue creation with ON CONFLICT upsert, regression detection during ingestion, issue state machine (open/resolved/archived/regressed).

**Addresses:** Error grouping accuracy (Pitfall 9), establishes issue-centric workflow.

**Uses:** Pattern matching, String operations, Map cache, sum types for IssueStatus, PostgreSQL transactions.

**Implements:** Fingerprinter trait with DefaultFingerprinter implementation (extensibility via traits).

**Avoids:** Over-grouping (too coarse fingerprints), under-grouping (line numbers in fingerprint), missing user override capability.

**Research needed:** Skip research-phase. Sentry grouping algorithm is thoroughly documented.

### Phase 4: Real-Time Streaming (WebSocket)

**Rationale:** Most impressive demo feature and best dogfooding of WebSocket rooms. Events flow from ingestion to dashboard in real-time via actor message passing. Exercises WebSocket, rooms, actor-per-connection, cross-node broadcast.

**Delivers:** WS Dashboard Server (actor-per-connection), StreamBroadcaster actor, room-based event broadcast, filtered streaming (clients subscribe to project:id, project:id:errors rooms), new issue notifications, issue count updates.

**Addresses:** WebSocket memory pressure (Pitfall 8), connection cleanup (Pitfall 21).

**Uses:** WebSocket server, Ws.join/broadcast, rooms, actor-per-connection crash isolation, pattern matching on subscriptions.

**Avoids:** Unbounded connections (enforce limits per project), missing on_close cleanup (resource leaks), sending to individual connections instead of rooms.

**Research needed:** Skip research-phase. WebSocket rooms are a known Mesh feature with examples.

### Phase 5: REST API (Query + CRUD)

**Rationale:** Makes the platform usable for investigation. Dashboard needs to query events, issues, projects. CRUD endpoints for projects and alert rules. Exercises HTTP routing, JSON encoding, complex SQL queries.

**Delivers:** GET /api/v1/events (search, filter, pagination), GET /api/v1/issues (sort, filter, pagination), POST/PUT/DELETE /api/v1/projects, POST/PUT/DELETE /api/v1/alerts, dashboard aggregation queries (event volume, error breakdowns, top issues).

**Addresses:** Search/filtering (PostgreSQL tsvector is well-documented), pagination with keyset cursors.

**Uses:** HTTP routing with path params, deriving(Json) for response encoding, Pool.query for reads, PostgreSQL full-text search, dynamic SQL query building.

**Avoids:** SQL injection (use parameterized queries), OFFSET pagination (use keyset), missing middleware (auth, CORS).

**Research needed:** Skip research-phase. REST API patterns are standard.

### Phase 6: Alerting System

**Rationale:** Closes the loop -- users don't have to watch the dashboard. Exercises timer-driven actor, service state management, pattern matching on sum types, HTTP client for webhooks (if capability exists).

**Delivers:** AlertRuleStore service (CRUD + in-memory cache), AlertEvaluator service (single recurring timer, evaluates all rules per tick), alert conditions (EventCountAbove, ErrorRateAbove, NewIssueDetected, IssueRegressed), alert actions (WebSocket notification, log message, webhook if HTTP client available), deduplication window, cooldown periods.

**Addresses:** Timer thread explosion (Pitfall 2), alert storms (Pitfall 5).

**Uses:** Timer.send_after in loop (single timer actor), service with List/Map state, pattern matching on AlertCondition sum type, Ws.broadcast for in-app alerts.

**Avoids:** One timer per rule (thread exhaustion), no deduplication (alert fatigue), no cooldown (re-fire spam).

**Research needed:** Consider research-phase if webhook HTTP client capability is uncertain. Timer-driven evaluation is standard pattern.

### Phase 7: Data Retention and Cleanup

**Rationale:** Background actor with daily cleanup. Exercises Timer.send_after for scheduling, bulk DELETE operations, partition management.

**Delivers:** Retention policy configuration per project (retention_days), background actor runs daily DELETE, partition manager creates future partitions at startup, storage usage tracking.

**Addresses:** Unbounded storage growth, query performance degradation.

**Uses:** Timer.send_after for daily schedule, PostgreSQL DELETE with WHERE timestamp, DROP old partitions.

**Avoids:** DELETE without WHERE (wipes all data), forgetting autovacuum impact, manual partition management.

**Research needed:** Skip research-phase. Retention is a standard database operation.

### Phase 8: Multi-Node Clustering

**Rationale:** Ultimate dogfooding phase. Exercises distributed actors, global registry, cross-node rooms, location-transparent PIDs, node monitoring. Should be last because it layers on top of fully-working single-node system.

**Delivers:** NodeManager actor (Node.start, connect, monitor), ClusterSync service (Global.register for EventRouter, StorageWriter), cross-node event routing, cross-node WebSocket broadcast (automatic via DIST_ROOM_BROADCAST), remote processor spawning under load.

**Addresses:** Split brain (Pitfall 13), broadcast amplification (Pitfall 17), clock skew (Pitfall 20).

**Uses:** Node.connect, Global.register/whereis, Node.spawn, location-transparent send, :nodedown/:nodeup pattern matching.

**Avoids:** Expecting strong consistency (no consensus protocol), dual-fire alerts (designate single alerting node), relying on in-memory state (PostgreSQL is source of truth).

**Research needed:** Consider research-phase for split-brain handling patterns. Distributed consensus is complex.

### Phase 9: Vue Frontend

**Rationale:** Separate directory, not Mesh code. Consumes REST API and WebSocket from previous phases. Can be built in parallel with backend phases 1-6.

**Delivers:** Vue 3 SPA with Vite, dashboard views (project overview, event list, issue list, event detail, alert rules), real-time event streaming via WebSocket, ECharts time-series charts, TanStack data tables, shadcn-vue components.

**Uses:** Vue 3 composition API, vue-router 5, Pinia for state, native fetch for API, native WebSocket for streaming, ECharts for charting, TanStack Vue Table for sortable/filterable tables, TanStack Vue Virtual for log viewer.

**Avoids:** Sharing code with docs site (separate package.json), using Nuxt (SSR unnecessary), Socket.io client (use native WebSocket).

**Research needed:** Skip research-phase. Frontend stack is established technologies.

### Phase Ordering Rationale

- **Types first** because every module imports them.
- **Storage before ingestion** because writer must exist before events can be stored.
- **Ingestion before streaming** because events must flow into system before streaming out.
- **Grouping after ingestion** because fingerprinting operates on validated events.
- **Streaming after grouping** because broadcasts include issue_id from grouping.
- **REST API after ingestion** because it queries data that ingestion writes.
- **Alerting after REST API** because alert rules need CRUD endpoints.
- **Retention after storage** because it operates on stored events.
- **Clustering last** because it layers on top of fully-working single-node.
- **Frontend last** because it consumes API and WebSocket that must work first.

### Research Flags

**Phases likely needing deeper research during planning:**

- **Phase 8 (Multi-Node Clustering)** -- Split-brain handling without consensus protocol. Needs pattern research for distributed state consistency. Consider /gsd:research-phase for conflict resolution strategies.

**Phases with standard patterns (skip research-phase):**

- **Phase 1 (Foundation)** -- PostgreSQL partitioning well-documented, standard schema design.
- **Phase 2 (Ingestion)** -- HTTP ingestion is standard REST pattern, Sentry SDK docs cover event format.
- **Phase 3 (Grouping)** -- Sentry grouping algorithm thoroughly documented in developer docs.
- **Phase 4 (Streaming)** -- WebSocket rooms are known Mesh feature with E2E tests.
- **Phase 5 (REST API)** -- Standard CRUD patterns, PostgreSQL full-text search documented.
- **Phase 6 (Alerting)** -- Timer-driven evaluation is standard pattern, alerting best practices well-known.
- **Phase 7 (Retention)** -- Standard database cleanup pattern.
- **Phase 9 (Frontend)** -- Established Vue 3 ecosystem, shadcn-vue already used in docs site.

## Confidence Assessment

| Area | Confidence | Notes |
|------|------------|-------|
| Stack | HIGH | Backend is 100% existing Mesh capabilities (HTTP, WS, PG, actors, JSON). Frontend uses established Vue 3 ecosystem already proven in docs site. Versions verified against npm. |
| Features | HIGH | Feature set derived from authoritative Sentry developer docs and established monitoring platform patterns. Table stakes well-understood. Differentiators map directly to Mesh's actor/distributed capabilities. |
| Architecture | HIGH | Architecture derived directly from verified Mesh language primitives (actors, services, supervision, rooms). Patterns validated against existing E2E tests. Component boundaries clear. |
| Pitfalls | HIGH | Critical pitfalls identified from Mesh runtime source analysis (Timer.send_after thread spawn, Map linear scan, conservative GC) and monitoring domain research (partitioning, alert storms, error grouping). |

**Overall confidence:** HIGH

### Gaps to Address

- **HTTP client capability for webhook alerts** -- Mesh runtime has no outbound HTTP client. Alert webhooks (POST to Slack/Discord/PagerDuty) require this. Options: (1) build minimal HTTP client in Mesh runtime during dogfooding, (2) shell out to curl, (3) defer webhooks to v2 and start with WebSocket-only alerts. Recommend option 1 if webhooks are critical, option 3 for faster MVP.

- **List.find Option pattern matching codegen bug** -- Pre-existing LLVM verification error. Must be fixed before or during Phase 2 (ingestion pipeline) because finding events in lists is a bread-and-butter operation. Workaround is verbose (List.filter + List.length check). Track as high-priority compiler fix.

- **Map.collect integer key assumption** -- Collecting iterators into Map<String, V> produces incorrect results. Needed for Phase 3 (error grouping) to aggregate events by string fields. Workaround is manual Map building with fold. Track as runtime fix.

- **Multi-line pipe continuation** -- Parser limitation makes complex pipelines unreadable. Accept intermediate let bindings as standard pattern for now. Consider parser fix if pipe-heavy code becomes constant pain point.

- **Missing stdlib functions** -- List.group_by likely needed for Phase 3. Check stdlib during implementation. If missing, implement in Mesher codebase and track as candidate for stdlib addition.

All gaps have documented workarounds. None are project-blocking. The gaps themselves are valuable dogfooding findings.

## Sources

### Primary (HIGH confidence)

- Mesh runtime source analysis: crates/mesh-rt/src/{http, ws, db, actor, dist, collections}
- Mesh language documentation: website/docs/docs/{concurrency, web, databases, distributed}
- Mesh E2E tests: tests/e2e/{supervisor_basic, service_call_cast, stdlib_http_server_runtime, stdlib_pg, deriving_json_sum_type}.mpl
- PROJECT.md: tech debt list (lines 234-243), conservative GC decision (line 271)
- [Sentry Developer Docs](https://develop.sentry.dev/) -- Event payloads, grouping algorithm, SDK expected features, envelope protocol (authoritative)
- [PostgreSQL Partitioning Documentation](https://www.postgresql.org/docs/current/ddl-partitioning.html) -- Native PARTITION BY RANGE (authoritative)
- [vue-echarts npm](https://www.npmjs.com/package/vue-echarts) -- v8.0.1 compatibility verified
- [@tanstack/vue-table](https://www.npmjs.com/package/@tanstack/vue-table) -- v8.21.3, official shadcn-vue data table pattern
- [shadcn-vue Data Table docs](https://www.shadcn-vue.com/docs/components/data-table) -- TanStack Table integration

### Secondary (MEDIUM confidence)

- [PostgreSQL Write-Heavy Tuning](https://aws.amazon.com/blogs/database/speed-up-time-series-data-ingestion-by-partitioning-tables-on-amazon-rds-for-postgresql/) -- AWS best practices
- [pg_partman vs Hypertables](https://www.tigerdata.com/learn/pg_partman-vs-hypertables-for-postgres-partitioning) -- Partitioning approach comparison
- [Alert Fatigue Solutions 2025](https://incident.io/blog/alert-fatigue-solutions-for-dev-ops-teams-in-2025-what-works) -- Deduplication, actionability metrics
- [WebSocket Scale 2025](https://www.videosdk.live/developer-hub/websocket/websocket-scale) -- Connection limits, memory per connection
- [Luzmo Vue Chart Libraries Guide](https://www.luzmo.com/blog/vue-chart-libraries) -- 2025 comparison of Vue charting options
- [System Design: Monitoring and Alerting](https://algomaster.io/learn/system-design-interviews/design-monitoring-and-alerting-system) -- Architecture patterns
- [GlitchTip Architecture](https://glitchtip.com/documentation/hosted-architecture/) -- Open source Sentry alternative reference

---

*Research completed: 2026-02-14*
*Ready for roadmap: yes*
