# Requirements: Mesher

**Defined:** 2026-02-14
**Core Value:** Expressive, readable concurrency -- writing concurrent programs should feel as natural and clean as writing sequential code, with the safety net of supervision and fault tolerance built into the language.

## v9.0 Requirements

Requirements for Mesher monitoring platform. Each maps to roadmap phases.

### Event Ingestion

- [ ] **INGEST-01**: User can send error events via POST /api/v1/events with DSN authentication
- [ ] **INGEST-02**: System validates and normalizes events (required fields, UTC timestamps, size limits)
- [ ] **INGEST-03**: User can send bulk events in a single request via POST /api/v1/events/bulk
- [ ] **INGEST-04**: System enforces per-project rate limits and returns 429 with Retry-After header
- [ ] **INGEST-05**: User can stream events over a persistent WebSocket connection
- [ ] **INGEST-06**: System responds with event ID on 202 Accepted

### Error Grouping

- [ ] **GROUP-01**: System automatically fingerprints events from stack trace frames (file + function + normalized message)
- [ ] **GROUP-02**: System falls back to exception type then raw message when no stack trace is present
- [ ] **GROUP-03**: User can override automatic fingerprinting with a custom fingerprint array
- [ ] **GROUP-04**: System creates a new Issue on first occurrence of a fingerprint
- [ ] **GROUP-05**: System tracks event count, first seen, and last seen per Issue

### Issue Lifecycle

- [ ] **ISSUE-01**: User can transition issues between unresolved, resolved, and archived states
- [ ] **ISSUE-02**: System detects regressions when a resolved issue receives a new event
- [ ] **ISSUE-03**: System auto-escalates archived issues on volume spike
- [ ] **ISSUE-04**: User can assign issues to team members
- [ ] **ISSUE-05**: User can delete and discard issues (suppress future events for that fingerprint)

### Organization & Projects

- [ ] **ORG-01**: User can create and manage organizations (tenants)
- [ ] **ORG-02**: User can create projects within organizations with platform metadata
- [ ] **ORG-03**: System generates unique DSN keys per project for SDK configuration
- [ ] **ORG-04**: User can manage team membership with roles (owner, admin, member)
- [ ] **ORG-05**: User can create and manage API tokens for programmatic access

### Search & Filtering

- [ ] **SEARCH-01**: User can filter issues by status, level, time range, and assignment
- [ ] **SEARCH-02**: User can full-text search event messages via PostgreSQL tsvector
- [ ] **SEARCH-03**: User can filter events by tag key-value pairs
- [ ] **SEARCH-04**: System defaults time-range filters to last 24 hours
- [ ] **SEARCH-05**: System uses keyset pagination for issue and event lists

### Real-Time Streaming

- [ ] **STREAM-01**: User can subscribe to a WebSocket stream of new events per project
- [ ] **STREAM-02**: User can apply filters to the WebSocket stream (level, environment, etc.)
- [ ] **STREAM-03**: System pushes new issue notifications to connected dashboards
- [ ] **STREAM-04**: System pushes issue count updates in real-time
- [ ] **STREAM-05**: System applies backpressure by dropping old events for slow clients

### Alerting

- [ ] **ALERT-01**: User can create alert rules with configurable conditions
- [ ] **ALERT-02**: User can set threshold-based alerts (event count > N in M minutes)
- [ ] **ALERT-03**: System triggers alerts on new issue creation and issue regression
- [ ] **ALERT-04**: System delivers alert notifications via WebSocket to connected dashboards
- [ ] **ALERT-05**: System enforces alert cooldown and deduplication windows
- [ ] **ALERT-06**: User can manage alert states (active, acknowledged, resolved)

### Dashboard Data

- [ ] **DASH-01**: System provides event volume over time (hourly/daily buckets)
- [ ] **DASH-02**: System provides error breakdown by level
- [ ] **DASH-03**: System provides top issues ranked by frequency
- [ ] **DASH-04**: System provides event breakdown by tag (environment, release)
- [ ] **DASH-05**: System provides per-issue event timeline
- [ ] **DASH-06**: System provides project health summary (unresolved count, 24h events, new today)

### Event Detail

- [ ] **DETAIL-01**: User can view full event payload
- [ ] **DETAIL-02**: User can view formatted stack traces with file, line, and function info
- [ ] **DETAIL-03**: User can view breadcrumbs (chronological event trail)
- [ ] **DETAIL-04**: User can view event tags as key-value pairs
- [ ] **DETAIL-05**: User can navigate between events within an issue (next/previous)
- [ ] **DETAIL-06**: User can view user context (id, email, IP) on events

### Data Retention

- [ ] **RETAIN-01**: User can configure retention period per project (30/60/90 days)
- [ ] **RETAIN-02**: System preserves issue summaries after event deletion
- [ ] **RETAIN-03**: User can view storage usage per project
- [ ] **RETAIN-04**: User can configure event sampling rate for high-volume projects

### Multi-Node Clustering

- [ ] **CLUSTER-01**: System supports node discovery and mesh formation via Node.connect
- [ ] **CLUSTER-02**: System uses global process registry for cross-node service discovery
- [ ] **CLUSTER-03**: System routes events across nodes for distributed processing
- [ ] **CLUSTER-04**: System broadcasts WebSocket events across nodes via distributed rooms
- [ ] **CLUSTER-05**: System spawns remote processors on other nodes under load

### Resilience

- [ ] **RESIL-01**: System uses supervision trees for the event processing pipeline
- [ ] **RESIL-02**: System provides crash isolation for actor-per-connection (HTTP and WebSocket)
- [ ] **RESIL-03**: System self-heals pipeline actors via automatic supervisor restart

### Vue Frontend

- [ ] **UI-01**: User can view project overview dashboard with charts and issue list
- [ ] **UI-02**: User can browse and search events with filters and pagination
- [ ] **UI-03**: User can view and manage issues (state transitions, assignment)
- [ ] **UI-04**: User can view event detail with stack trace, breadcrumbs, and tags
- [ ] **UI-05**: User can see real-time event streaming via WebSocket
- [ ] **UI-06**: User can manage alert rules (create, edit, delete)
- [ ] **UI-07**: User can manage organizations and projects
- [ ] **UI-08**: System renders time-series charts via ECharts

## v2+ Requirements

Deferred to future milestone. Tracked but not in current roadmap.

### Webhook Alerting

- **WEBHOOK-01**: User can configure webhook URLs for alert notifications
- **WEBHOOK-02**: System sends HTTP POST to configured webhooks on alert (requires Mesh HTTP client)

### SDK

- **SDK-01**: Mesh SDK captures errors and sends to ingestion API
- **SDK-02**: TypeScript SDK for monitoring web/Node applications

### Advanced Features

- **ADV-01**: Source map processing for JavaScript stack traces
- **ADV-02**: Release tracking with git commit association
- **ADV-03**: Custom dashboard builder (drag-and-drop widgets)
- **ADV-04**: Log query language with parsing and aggregation

## Out of Scope

Explicitly excluded. Documented to prevent scope creep.

| Feature | Reason |
|---------|--------|
| Distributed tracing / APM | Enormous scope -- span collection, trace assembly, service maps. Separate product. |
| Infrastructure metrics | CPU/memory monitoring requires host agents and different query patterns. Use Prometheus/Grafana. |
| Session replay | DOM snapshots and replay player. Requires specialized SDKs and massive storage. |
| Profiling (CPU/memory flame graphs) | Requires agent-level instrumentation. Separate product from error tracking. |
| Email alerting | Mesh has no SMTP capability. Use webhooks for external notification. |
| Mobile SDKs | iOS/Android crash reporting requires platform-specific signal handlers. Server-side only. |
| AI-powered grouping | Requires ML pipeline, training data, inference. Deterministic fingerprinting for v9.0. |
| Multi-region data residency | GDPR compliance requires multi-region infrastructure. Single-region for v9.0. |
| Billing / usage-based pricing | Metering and invoicing is a product in itself. Mesher is dogfooding, not revenue. |
| GraphQL API | REST is simpler and sufficient. No need for query flexibility of GraphQL. |
| Redis / message queue | Actor mailboxes handle in-memory processing. No external dependencies beyond PostgreSQL. |
| Elasticsearch | PostgreSQL tsvector is sufficient for full-text search at monitoring scale. |

## Traceability

Which phases cover which requirements. Updated during roadmap creation.

| Requirement | Phase | Status |
|-------------|-------|--------|
| — | — | — |

**Coverage:**
- v9.0 requirements: 68 total
- Mapped to phases: 0
- Unmapped: 68 (pending roadmap creation)

---
*Requirements defined: 2026-02-14*
*Last updated: 2026-02-14 after initial definition*
