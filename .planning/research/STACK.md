# Technology Stack

**Project:** Mesher -- Monitoring/Observability SaaS Platform
**Researched:** 2026-02-14
**Confidence:** HIGH (frontend), HIGH (backend/Mesh), MEDIUM (database schema patterns)

## Recommended Stack

### Backend (Mesh -- Already Validated)

The entire backend is written in Mesh (.mpl files compiled by meshc). No new Rust crates or compiler changes are needed. All required backend capabilities already exist in the Mesh language and runtime:

| Capability | Mesh Feature | Status |
|------------|-------------|--------|
| HTTP API (ingestion + REST) | `HTTP.on_get/on_post/on_put/on_delete`, middleware, path params, TLS | Shipped v2.0-v3.0 |
| WebSocket streaming | `Ws.serve`, actor-per-connection, rooms, TLS | Shipped v4.0 |
| PostgreSQL storage | `Pg.connect`, `Pool.new`, `Pool.query_as`, transactions, `deriving(Row)` | Shipped v2.0-v3.0 |
| JSON serde | `deriving(Json)`, encode/decode for structs, sum types, generics | Shipped v2.0 |
| Actor concurrency | Spawn, send, receive, supervision trees, crash isolation | Shipped v1.0 |
| Distributed clustering | Node.connect, Global.register, remote spawn, cross-node rooms | Shipped v5.0 |
| Timers | Timer.sleep, Timer.send_after, receive timeouts | Shipped v1.9 |
| Pattern matching | Case expressions, sum type matching, guards, exhaustiveness | Shipped v1.0 |
| Iterators/Collections | Lazy iterators, map/filter/reduce, Collect | Shipped v7.0 |
| Module system | Multi-file builds, pub visibility, imports | Shipped v1.8 |

**No new Mesh stdlib additions are required for v9.0.** The existing HTTP server, WebSocket server, PostgreSQL driver, JSON serde, actor system, and distributed actors cover all Mesher backend needs. If anything surfaces during implementation (e.g., a missing string function), it would be a minor stdlib addition, not a stack decision.

### Frontend (Vue 3 -- New Application)

The Mesher frontend is a separate Vue 3 SPA in the same monorepo, communicating with the Mesh backend via REST API and WebSocket. The existing website (VitePress docs site) is completely separate and should not share code with Mesher.

#### Core Framework

| Technology | Version | Purpose | Why |
|------------|---------|---------|-----|
| Vue | ^3.5.28 | UI framework | Already used for docs site; team expertise; excellent reactivity for real-time dashboards |
| Vite | ^6.2 | Build tool / dev server | Default for Vue 3; fast HMR; already used by VitePress under the hood |
| TypeScript | ^5.9.3 | Type safety | Already used in project; catches API contract mismatches at build time |
| vue-router | ^5.0.2 | Client-side routing | Official Vue router; v5 merges file-based routing into core; no breaking changes from v4 |
| Pinia | ^3.0.4 | State management | Official Vue state management; lightweight (1.5KB); composition API native; devtools support |

#### UI Components

| Technology | Version | Purpose | Why |
|------------|---------|---------|-----|
| Tailwind CSS | ^4.1 | Utility-first CSS | Already used in docs site; consistent styling approach across monorepo |
| shadcn-vue | (copy-paste, no version) | Component primitives | Already used in docs site (button, collapsible, dropdown-menu, scroll-area, sheet); provides accessible, unstyled building blocks |
| reka-ui | ^2.8.0 | Headless primitives (shadcn-vue dep) | Already a dependency; provides Dialog, Popover, Select, Tabs used by shadcn-vue |
| lucide-vue-next | ^0.564.0 | Icon library | Already a dependency; consistent iconography |

#### Data Visualization

| Technology | Version | Purpose | Why |
|------------|---------|---------|-----|
| Apache ECharts | ^6.0.0 | Charting engine | Most feature-complete open-source charting library; handles time-series line charts, bar charts, heatmaps, and large datasets efficiently; free (Apache 2.0) |
| vue-echarts | ^8.0.1 | Vue 3 wrapper for ECharts | Official Vue integration; smart option merging; reactive updates; 459 dependents on npm |

**Why ECharts over alternatives:**
- **vs Chart.js**: Chart.js is simpler but lacks time-series zoom/brush, heatmaps, and large dataset rendering needed for monitoring dashboards
- **vs ApexCharts**: ApexCharts is good for interactive charts but ECharts handles 100K+ data points with better performance via canvas rendering and progressive loading
- **vs Highcharts**: Highcharts requires a commercial license for SaaS; ECharts is Apache 2.0
- **vs Unovis**: Unovis is newer and less proven; ECharts has 63K+ GitHub stars and years of production use in monitoring tools (Apache projects, Alibaba, Baidu)

#### Virtual Scrolling (Log Viewer)

| Technology | Version | Purpose | Why |
|------------|---------|---------|-----|
| @tanstack/vue-virtual | ^3.13.18 | Virtual scrolling for log viewer | Headless virtualizer; 60FPS with 100K+ items; framework-agnostic core with Vue adapter; recommended by VueUse over their own useVirtualList |

**Why TanStack Virtual over alternatives:**
- **vs vue-virtual-scroller**: TanStack is more actively maintained, has better TypeScript support, and is framework-agnostic (same API patterns as TanStack Table/Query)
- **vs native intersection observer**: Manual implementation is error-prone for variable-height log entries with different severity colors/stack traces
- **vs Quasar virtual scroll**: Quasar is a full framework; we only need the virtualizer

#### Data Tables

| Technology | Version | Purpose | Why |
|------------|---------|---------|-----|
| @tanstack/vue-table | ^8.21.3 | Data table logic (sorting, filtering, pagination) | Headless; works with shadcn-vue Table component; 24K+ GitHub stars; official shadcn-vue data table pattern |

**Why TanStack Table:** shadcn-vue's official data table guide is built on TanStack Table. This gives us sorting, filtering, column visibility, and pagination with full TypeScript support while keeping our own shadcn-vue styled markup.

#### Date Handling

| Technology | Version | Purpose | Why |
|------------|---------|---------|-----|
| date-fns | ^4.1.0 | Date formatting and manipulation | Tree-shakable (import only what you use); functional API (no mutable objects); first-class timezone support in v4; 200+ functions; 35K+ GitHub stars |

**Why date-fns over Day.js:** date-fns v4 has built-in timezone support (critical for monitoring -- events from different timezones), better tree-shaking (smaller effective bundle), and a functional API that fits Vue's composition API pattern. Day.js requires plugins for timezone and relative time.

#### HTTP Client

| Technology | Version | Purpose | Why |
|------------|---------|---------|-----|
| Native fetch | (browser built-in) | REST API calls to Mesh backend | No library needed; modern browsers support fetch natively; AbortController for cancellation; Response.json() for parsing |

**Why NOT axios/ky/ofetch:** The Mesher frontend has a single backend (the Mesh API server). A thin fetch wrapper composable (e.g., `useMesherApi()`) provides all needed functionality: base URL, auth headers, JSON parsing, error handling. Adding a dependency for this is over-engineering.

### Database (PostgreSQL -- Schema Patterns)

No additional database technology is needed. The Mesh PostgreSQL driver with connection pooling handles all storage. The schema design is the important decision, not the technology.

| Decision | Recommendation | Why |
|----------|---------------|-----|
| Database engine | PostgreSQL (already supported) | Mesh has full PG driver with TLS, pooling, transactions, deriving(Row) |
| Time-series approach | Native `PARTITION BY RANGE` on timestamp | No extension dependencies (TimescaleDB adds operational complexity); native partitioning is sufficient for Mesher's scale |
| Partition granularity | Daily partitions for events/logs | Enables efficient data expiration (DROP partition vs DELETE rows); keeps per-partition indexes small |
| Partition management | Manual CREATE via Mesh startup code | pg_partman requires PG extension installation; manual partition creation (30 days ahead) is simpler for a self-hosted product |
| ID generation | BIGSERIAL or UUID v7 (time-sortable) | ULIDs/UUID v7 are time-sortable, enabling time-range queries on primary key; BIGSERIAL is simpler if timestamps are always included in queries |

**Why NOT TimescaleDB:** TimescaleDB adds an extension dependency that complicates deployment (requires installation on PG server, version compatibility concerns). Native PostgreSQL partitioning provides the core benefit (partition pruning on time-range queries, efficient data expiration) without any extensions. For Mesher's expected scale (thousands of events/second, not millions), native partitioning is sufficient.

### SDK / Agent (Lightweight HTTP Client)

The Mesher SDK sends events from user applications to the Mesher ingestion API. Design it as a simple HTTP POST client, not a complex agent.

| Decision | Recommendation | Why |
|----------|---------------|-----|
| Transport protocol | HTTPS POST with JSON body | Simple, universal, works from any language; no gRPC/protobuf complexity |
| Ingestion endpoint | `POST /api/v1/events` | Single endpoint; event type in payload, not URL path |
| Authentication | API key in header (`X-Mesher-Key: <key>`) | Simple; no OAuth complexity for server-to-server calls |
| Batching | Client-side batching (flush every 5s or 100 events) | Reduces HTTP overhead; prevents event loss on crash via flush-on-shutdown |
| Retry | Exponential backoff with jitter, max 3 retries | Standard pattern; prevents thundering herd |
| SDK languages (initial) | JavaScript/TypeScript only | Start with one; expand later |
| SDK size | < 5KB minified | Tiny footprint; no heavy dependencies |

**Event payload format (JSON):**
```json
{
  "type": "error",
  "timestamp": "2026-02-14T12:00:00.000Z",
  "level": "error",
  "message": "Connection refused",
  "fingerprint": "conn_refused_pg",
  "metadata": {
    "service": "api-gateway",
    "environment": "production",
    "runtime": "node/22.0.0"
  },
  "stacktrace": "Error: Connection refused\n  at connect (pg.js:42)\n  ..."
}
```

**Why NOT OpenTelemetry format:** OTLP is complex (protobuf, spans, trace context). Mesher is a log/error monitoring tool, not a full APM/tracing platform. A simple JSON format keeps the SDK tiny and the ingestion pipeline simple. Users who want OTLP can add a bridge later.

### Deployment

| Technology | Purpose | Why |
|------------|---------|-----|
| Single Mesh binary | Backend server | meshc compiles to a native binary; no runtime dependencies |
| Vite build (static files) | Frontend assets | `vite build` produces static HTML/JS/CSS served by Mesh HTTP server or a CDN |
| PostgreSQL 16+ | Database | Standard; supports native partitioning; widely available |
| TLS certificates | HTTPS/WSS | Mesh supports Http.serve_tls and Ws.serve_tls; use Let's Encrypt or self-signed for dev |
| Docker (optional) | Containerized deployment | Mesh binary + PG in docker-compose for easy local dev and deployment |

## Alternatives Considered

| Category | Recommended | Alternative | Why Not |
|----------|-------------|-------------|---------|
| Charting | ECharts + vue-echarts | Chart.js / vue-chartjs | Lacks time-series brush/zoom, heatmaps, and large dataset handling needed for monitoring |
| Charting | ECharts + vue-echarts | Highcharts | Commercial license required for SaaS products |
| Charting | ECharts + vue-echarts | Unovis | Newer, smaller ecosystem; less proven at scale |
| Virtual scroll | @tanstack/vue-virtual | vue-virtual-scroller | Less active maintenance; TanStack has better TS types |
| Data table | @tanstack/vue-table | ag-Grid | ag-Grid community is limited; enterprise is paid; shadcn-vue integrates with TanStack |
| State management | Pinia | Vuex | Vuex is legacy; Pinia is the official successor |
| Router | vue-router 5 | vue-router 4 | v5 is latest npm release; no breaking changes from v4 |
| Date library | date-fns | Day.js | Day.js lacks built-in timezone support; requires plugins |
| Date library | date-fns | VueUse useDateFormat | Too limited for timezone conversion and relative time ("5 minutes ago") |
| HTTP client | Native fetch | Axios | Unnecessary dependency for single-backend SPA |
| Time-series DB | PostgreSQL native partitioning | TimescaleDB | Extension dependency; operational complexity; not needed at Mesher's scale |
| Time-series DB | PostgreSQL native partitioning | InfluxDB / ClickHouse | Different database entirely; Mesh only has a PG driver |
| SDK format | Simple JSON over HTTPS | OpenTelemetry OTLP | OTLP is complex; Mesher is not a full APM; simple JSON keeps SDK < 5KB |
| Frontend framework | Vue 3 | React / Svelte | Existing team expertise with Vue; docs site already in Vue; monorepo consistency |

## Installation

### Frontend (new app in monorepo)

```bash
# Create the Mesher frontend app
mkdir -p mesher/frontend
cd mesher/frontend

# Initialize with Vite
npm create vite@latest . -- --template vue-ts

# Core dependencies
npm install vue@^3.5.28 vue-router@^5.0.2 pinia@^3.0.4

# UI
npm install tailwindcss@^4.1 @tailwindcss/vite@^4.1 @tailwindcss/typography@^0.5
npm install class-variance-authority@^0.7.1 clsx@^2.1.1 tailwind-merge@^3.4.0
npm install reka-ui@^2.8.0 lucide-vue-next@^0.564.0

# Data visualization
npm install echarts@^6.0.0 vue-echarts@^8.0.1

# Data tables & virtual scrolling
npm install @tanstack/vue-table@^8.21.3 @tanstack/vue-virtual@^3.13.18

# Date handling
npm install date-fns@^4.1.0

# Dev dependencies
npm install -D typescript@^5.9.3 @types/node@^25.2.3
```

### Backend (Mesh -- no installation)

No new dependencies. The Mesher backend is pure Mesh code using existing stdlib modules:

```
# File structure for Mesh backend
mesher/backend/
  main.mpl              # Entry point: start HTTP + WS servers
  config.mpl            # Configuration loading
  ingestion/
    handler.mpl          # HTTP ingestion endpoint
    processor.mpl        # Event processing pipeline
  api/
    router.mpl           # REST API routes
    events.mpl           # Event query endpoints
    projects.mpl         # Project CRUD
    alerts.mpl           # Alert rule endpoints
  ws/
    handler.mpl          # WebSocket connection handler
    streaming.mpl        # Real-time event streaming
  storage/
    schema.mpl           # Database schema setup
    events.mpl           # Event storage queries
    projects.mpl         # Project storage queries
  alerting/
    engine.mpl           # Rule evaluation engine
    notifier.mpl         # Alert notification dispatch
  grouping/
    fingerprint.mpl      # Error fingerprinting
    dedup.mpl            # Deduplication logic
```

### SDK (TypeScript, separate package)

```bash
# Create SDK package
mkdir -p mesher/sdk
cd mesher/sdk
npm init -y

# No production dependencies -- pure TypeScript with native fetch
npm install -D typescript@^5.9.3
```

## What NOT to Add

| Avoid | Why | Do Instead |
|-------|-----|------------|
| TimescaleDB | Extension dependency; operational complexity; Mesh PG driver works with native PG | Use native PostgreSQL PARTITION BY RANGE |
| InfluxDB / ClickHouse | Mesh has no driver for these; adding one is scope creep | Use PostgreSQL with partitioning |
| Axios / ky / ofetch | Unnecessary dependency for single-backend SPA | Use native fetch with a thin composable wrapper |
| Full OpenTelemetry SDK | Over-engineered for Mesher's log/error use case | Simple JSON POST SDK |
| Nuxt | SSR is unnecessary for a dashboard SPA; adds build complexity | Plain Vite + Vue 3 |
| Quasar / Vuetify / PrimeVue | Full component frameworks conflict with shadcn-vue / Tailwind approach | Use shadcn-vue (already established in project) |
| Socket.io client | Mesh uses raw WebSocket (RFC 6455), not Socket.io protocol | Use native WebSocket API |
| D3.js directly | Too low-level for dashboard charts; ECharts provides the right abstraction | Use ECharts via vue-echarts |
| GraphQL | Adds query language complexity; REST is simpler for dashboard CRUD + event queries | Use REST endpoints |
| Redis / message queue | Mesh actors with supervision trees handle all in-memory processing; adding external state is unnecessary | Use Mesh actor mailboxes for event pipeline |
| Webpack | Vite is the standard Vue 3 build tool; Webpack adds configuration overhead | Use Vite |
| CSS-in-JS (styled-components, emotion) | Conflicts with Tailwind approach; worse performance | Use Tailwind CSS |

## Integration Points

### Frontend <-> Mesh Backend (REST API)

The Vue frontend communicates with the Mesh HTTP server via REST endpoints:

```
GET    /api/v1/projects                  # List projects
POST   /api/v1/projects                  # Create project
GET    /api/v1/projects/:id/events       # Query events (with filters)
GET    /api/v1/projects/:id/events/:eid  # Single event detail
GET    /api/v1/projects/:id/groups       # Error groups
GET    /api/v1/projects/:id/stats        # Dashboard statistics
POST   /api/v1/projects/:id/alerts       # Create alert rule
GET    /api/v1/projects/:id/alerts       # List alert rules
```

**CORS:** The Mesh HTTP server needs to add CORS headers via middleware. This is a Mesh middleware function, not a stack addition:
```
# Mesh middleware (pseudocode)
fn cors_middleware(req, next) do
  response = next(req)
  # Add Access-Control-Allow-Origin, etc.
  response
end
```

### Frontend <-> Mesh Backend (WebSocket)

The Vue frontend connects to the Mesh WebSocket server for real-time streaming:

```
wss://mesher.example.com/ws/stream?project=<id>&token=<auth>
```

- Mesh WS handler joins the connection to a room per project
- Events flow: Ingestion -> Processing Actor -> Ws.broadcast(room, event_json)
- Frontend receives JSON messages via native WebSocket API
- No Socket.io or other WS library needed on either side

### SDK -> Mesh Backend (Ingestion)

```
POST /api/v1/ingest
Headers:
  Content-Type: application/json
  X-Mesher-Key: <project_api_key>
Body: { events: [...] }  # Batched events array
```

The Mesh HTTP server deserializes via Json.decode, processes via actor pipeline, and stores in PostgreSQL.

### Mesh Backend -> PostgreSQL

All queries use the existing `Pool.query_as` / `Pool.execute` with `deriving(Row)` for struct mapping. No ORM, no query builder -- raw parameterized SQL strings. This is the Mesh way (explicit, no magic).

## Version Compatibility Matrix

| Package | Compatible With | Notes |
|---------|----------------|-------|
| Vue 3.5.28 | Vite 6.x, Pinia 3.x, vue-router 5.x | Current stable |
| Pinia 3.0.4 | Vue 3 only (dropped Vue 2) | Straightforward upgrade from Pinia 2 |
| vue-router 5.0.2 | Vue 3 only | No breaking changes from vue-router 4 |
| echarts 6.0.0 | vue-echarts 8.0.1 | Major version pairing |
| @tanstack/vue-table 8.21.3 | Vue 3.x | Stable; v9 alpha exists but not ready |
| @tanstack/vue-virtual 3.13.18 | Vue 3.x | Stable |
| date-fns 4.1.0 | Any framework | Pure functions; no framework coupling |
| Tailwind CSS 4.1 | Vite 6.x via @tailwindcss/vite | Already proven in docs site |
| shadcn-vue | Tailwind 4.x, reka-ui 2.x | Copy-paste components; no version lock |

## Monorepo Structure

```
snow/
  crates/                    # Mesh compiler (Rust) -- existing
  website/                   # VitePress docs site -- existing
  mesher/                    # NEW: Mesher monitoring platform
    backend/                 # Mesh source files (.mpl)
      main.mpl
      ingestion/
      api/
      ws/
      storage/
      alerting/
      grouping/
    frontend/                # Vue 3 SPA
      package.json
      vite.config.ts
      src/
        main.ts
        App.vue
        router/
        stores/
        views/
        components/
        composables/
        lib/                 # shadcn-vue components
    sdk/                     # TypeScript SDK
      package.json
      src/
        index.ts
        client.ts
        types.ts
```

The frontend and SDK have separate `package.json` files. They do NOT share the website's `package.json` -- different apps with different dependencies.

## Sources

- [vue-echarts npm](https://www.npmjs.com/package/vue-echarts) -- v8.0.1, verified against echarts 6.0.0 (HIGH confidence)
- [echarts npm](https://www.npmjs.com/package/echarts) -- v6.0.0, Apache 2.0 license (HIGH confidence)
- [vue-router npm](https://www.npmjs.com/package/vue-router) -- v5.0.2, released Feb 2026 (HIGH confidence)
- [Pinia npm](https://www.npmjs.com/package/pinia) -- v3.0.4, Vue 3 only (HIGH confidence)
- [@tanstack/vue-virtual npm](https://www.npmjs.com/package/@tanstack/vue-virtual) -- v3.13.18 (HIGH confidence)
- [@tanstack/vue-table npm](https://www.npmjs.com/package/@tanstack/vue-table) -- v8.21.3 (HIGH confidence)
- [date-fns npm](https://www.npmjs.com/package/date-fns) -- v4.1.0 with timezone support (HIGH confidence)
- [shadcn-vue Data Table docs](https://www.shadcn-vue.com/docs/components/data-table) -- TanStack Table integration guide (HIGH confidence)
- [PostgreSQL Partitioning docs](https://www.postgresql.org/docs/current/ddl-partitioning.html) -- Native PARTITION BY RANGE (HIGH confidence)
- [Sentry SDK Overview](https://develop.sentry.dev/sdk/overview/) -- SDK design patterns and event payload format (HIGH confidence)
- [Sentry Event Payloads](https://develop.sentry.dev/sdk/data-model/event-payloads/) -- JSON event structure reference (HIGH confidence)
- [TanStack Virtual docs](https://tanstack.com/virtual/latest) -- Virtual scrolling for massive lists (HIGH confidence)
- [Luzmo Vue Chart Libraries Guide](https://www.luzmo.com/blog/vue-chart-libraries) -- 2025 comparison of Vue charting options (MEDIUM confidence)
- [PostgreSQL Partitioning for Time-Series](https://aws.amazon.com/blogs/database/speed-up-time-series-data-ingestion-by-partitioning-tables-on-amazon-rds-for-postgresql/) -- AWS best practices for PG partitioning (MEDIUM confidence)
- [pg_partman vs Hypertables](https://www.tigerdata.com/learn/pg_partman-vs-hypertables-for-postgres-partitioning) -- Comparison of partitioning approaches (MEDIUM confidence)

---
*Stack research for: Mesher Monitoring Platform (v9.0)*
*Researched: 2026-02-14*
