# Architecture Patterns

**Domain:** Monitoring/observability SaaS platform (Mesher) -- built entirely in Mesh
**Researched:** 2026-02-14
**Overall confidence:** HIGH (architecture derived directly from verified Mesh language primitives and established monitoring platform patterns)

## Executive Summary

Mesher is a monitoring/observability platform where the entire backend is written in Mesh (.mpl files compiled by meshc). The architecture maps directly onto Mesh's actor system: every conceptual component (ingestion endpoint, event processor, aggregator, alerting evaluator, WebSocket streamer) is one or more actors, supervised by a layered supervision tree. Data flows from HTTP/WebSocket ingestion actors through processing actors into PostgreSQL via connection pools, and simultaneously streams to dashboard clients via WebSocket rooms.

The architecture is intentionally actor-heavy. Where a traditional web framework would use middleware chains and thread pools, Mesher uses dedicated actor types with message passing. This creates maximum stress on the actor runtime (scheduler fairness, GC under load, mailbox throughput) and exercises every Mesh language feature: sum types for event modeling, generics for typed processing pipelines, traits for extensible fingerprinting, iterators for data transformation, services for stateful aggregation, supervisors for fault tolerance, distributed actors for multi-node clustering, and deriving(Json)/deriving(Row) for serialization boundaries.

The key architectural decision is the **pipeline-of-actors** pattern: ingestion actors accept events, normalize them, and forward to processor actors. Processors perform fingerprinting, grouping, and enrichment, then fan out to three destinations: (1) storage actors that batch-write to PostgreSQL, (2) streaming actors that broadcast to WebSocket rooms, and (3) alerting actors that evaluate rules against aggregated state. Each stage is independently supervised, so a crash in alerting never impacts ingestion or storage.

## Recommended Architecture

### System Overview Diagram

```
                        INGESTION LAYER
                        ===============
  HTTP Clients ──> [HTTP Server (actor-per-conn)]
                        |
  WS Clients   ──> [WS Ingestion Server (actor-per-conn)]
                        |
                   ┌────┴────┐
                   │ Router  │  (dispatches by project_id)
                   │ Actor   │
                   └────┬────┘
                        |
                   PROCESSING LAYER
                   ================
           ┌────────────┼────────────┐
           v            v            v
    [Processor    [Processor    [Processor
     Actor #1]    Actor #2]    Actor #N]
     (fingerprint, group, enrich)
           |            |            |
           └────────────┼────────────┘
                        |
                   ┌────┴────────────────┐
                   |                     |
              FAN-OUT LAYER         ALERTING LAYER
              =============         ==============
         ┌────────┴────────┐    [AlertEvaluator
         v                 v     Service (timer)]
   [StorageWriter    [StreamBroadcaster        |
    Service]          Actor]              [AlertNotifier
         |                 |               Actor]
         v                 v
   [PostgreSQL       [WS Dashboard
    Pool]             Server rooms]
                          |
                     Dashboard Clients
```

### Actor Topology

```
RootSupervisor (one_for_one)
├── IngestionSupervisor (one_for_one)
│   ├── HTTP Server (actor-per-connection, crash-isolated)
│   ├── WS Ingestion Server (actor-per-connection, crash-isolated)
│   └── EventRouter Service (stateful: project routing table)
│
├── ProcessingSupervisor (one_for_one)
│   ├── ProcessorPool (N processor actors, one_for_all within pool)
│   └── Fingerprinter Service (stateful: fingerprint cache)
│
├── StorageSupervisor (rest_for_one)
│   ├── PgPool (connection pool - must start first)
│   ├── SchemaManager (runs migrations on startup)
│   └── StorageWriter Service (batch accumulator + flush)
│
├── StreamingSupervisor (one_for_one)
│   ├── WS Dashboard Server (actor-per-connection)
│   └── StreamBroadcaster Actor (room management)
│
├── AlertingSupervisor (one_for_one)
│   ├── AlertRuleStore Service (stateful: rule definitions)
│   ├── AlertEvaluator Service (timer-driven: checks rules)
│   └── AlertNotifier Actor (sends notifications)
│
└── ClusterSupervisor (one_for_one)      [multi-node only]
    ├── NodeManager Actor (mesh formation)
    └── ClusterSync Service (global registry + state sync)
```

### Component Boundaries

| Component | Responsibility | Communicates With | Mesh Features Exercised |
|-----------|---------------|-------------------|------------------------|
| HTTP Ingestion Server | Accept POST /api/events, parse JSON, validate | EventRouter (send) | actor-per-conn, HTTP routing, middleware, deriving(Json), ? operator |
| WS Ingestion Server | Accept streaming events over WebSocket | EventRouter (send) | actor-per-conn WS, on_connect/on_message/on_close callbacks |
| EventRouter Service | Route events to correct processor by project_id | ProcessorPool actors (send) | service (GenServer), Map state, pattern matching on project_id |
| Processor Actor | Fingerprint, group, enrich events | StorageWriter (send), StreamBroadcaster (send), AlertEvaluator (send) | sum types, pattern matching, iterators, traits (Fingerprint interface), closures |
| Fingerprinter Service | Maintain fingerprint -> issue_id cache | Processor actors (call/response) | service with Map state, String operations, hashing |
| StorageWriter Service | Batch events and flush to PostgreSQL | PgPool (Pool.execute, Pool.query) | service state (List accumulator), Timer.send_after for flush, deriving(Row), transactions |
| WS Dashboard Server | Serve real-time dashboards over WebSocket | StreamBroadcaster (messages arrive in mailbox) | actor-per-conn WS, Ws.join/broadcast rooms, pattern matching on subscriptions |
| StreamBroadcaster Actor | Fan out processed events to WS rooms | WS Dashboard Server rooms (Ws.broadcast) | Ws.broadcast, room naming by project/filter, cross-node via distributed rooms |
| AlertRuleStore Service | CRUD for alert rules, in-memory cache | AlertEvaluator (call), HTTP API handlers | service with List/Map state, deriving(Json) for rule serialization |
| AlertEvaluator Service | Periodically evaluate rules against recent data | AlertNotifier (send), PgPool (query) | service with timer (Timer.send_after loop), pattern matching on rule conditions, iterators |
| AlertNotifier Actor | Send alert notifications (webhook, log) | HTTP.post for webhook delivery | HTTP client, error handling with Result, retry logic |
| NodeManager Actor | Start node, connect to peers, monitor cluster | Global registry, Node.monitor | Node.start, Node.connect, Node.list, Node.monitor, Global.register |
| ClusterSync Service | Sync state across nodes on connect/disconnect | NodeManager events, Global registry | Global.register/whereis, send to remote PIDs, pattern matching on :nodedown |

### Data Flow: Event Ingestion to Storage

```
1. HTTP POST /api/v1/events
   Body: {"project_id": "abc", "level": "error", "message": "...", "stacktrace": "..."}

2. HTTP handler actor (spawned per connection):
   - Parse JSON body with Event.from_json(body)?
   - Validate required fields
   - Extract project_id
   - send(router_pid, ProcessEvent(event))
   - Return HTTP 202 Accepted

3. EventRouter Service receives ProcessEvent:
   - Look up processor PID for project_id from routing table
   - If no processor assigned, round-robin to ProcessorPool
   - Forward: send(processor_pid, RawEvent(event))

4. Processor Actor receives RawEvent:
   - Generate fingerprint (hash of message + stacktrace frames)
   - call Fingerprinter to resolve fingerprint -> issue_id (or create new)
   - Enrich event with issue_id, timestamp, severity
   - Fan out:
     a. send(storage_writer_pid, StoreEvent(enriched_event))
     b. send(stream_broadcaster_pid, BroadcastEvent(enriched_event))
     c. send(alert_evaluator_pid, CheckEvent(enriched_event))

5. StorageWriter Service receives StoreEvent:
   - Append to batch buffer (List<EnrichedEvent> state)
   - If batch_size >= 100 OR flush_timer fires:
     - Pool.transaction: INSERT batch into events table
     - UPDATE issues SET last_seen, event_count
     - Clear batch buffer

6. StreamBroadcaster receives BroadcastEvent:
   - Encode event as JSON
   - Ws.broadcast("project:{project_id}", json)
   - Ws.broadcast("project:{project_id}:errors", json) if level == Error
```

### Data Flow: Real-Time Dashboard Subscription

```
1. Client connects via WebSocket to WS Dashboard Server

2. on_connect(conn):
   - Ws.send(conn, "connected")

3. Client sends: {"subscribe": "project:abc"}

4. on_message(conn, msg):
   - Parse subscription request
   - Ws.join(conn, "project:abc")
   - Query recent events from PgPool for initial state
   - Ws.send(conn, initial_state_json)

5. When StreamBroadcaster calls Ws.broadcast("project:abc", event_json):
   - All subscribed dashboard connections receive the event
   - In multi-node cluster: broadcast automatically propagates via DIST_ROOM_BROADCAST
```

## Module Structure (Multi-File Project)

```
mesher/
├── main.mpl                      # Entry point: start supervisors, HTTP/WS servers
├── config.mpl                    # Configuration loading
├── types/
│   ├── event.mpl                 # Event, RawEvent, EnrichedEvent structs + sum types
│   ├── issue.mpl                 # Issue struct, IssueStatus sum type
│   ├── alert.mpl                 # AlertRule, AlertCondition, AlertAction structs
│   ├── project.mpl               # Project struct
│   └── api.mpl                   # ApiResponse, ApiError types
├── ingestion/
│   ├── http_handler.mpl          # HTTP route handlers for event ingestion
│   ├── ws_handler.mpl            # WebSocket ingestion handlers
│   └── router.mpl                # EventRouter service
├── processing/
│   ├── processor.mpl             # Processor actor (fingerprint + enrich + fan-out)
│   ├── fingerprint.mpl           # Fingerprinter service + fingerprint algorithm
│   └── grouping.mpl              # Issue grouping logic
├── storage/
│   ├── writer.mpl                # StorageWriter service (batch + flush)
│   ├── queries.mpl               # SQL query functions
│   └── migrations.mpl            # Schema migration runner
├── streaming/
│   ├── dashboard_handler.mpl     # WS dashboard handlers
│   └── broadcaster.mpl           # StreamBroadcaster actor
├── alerting/
│   ├── rule_store.mpl            # AlertRuleStore service
│   ├── evaluator.mpl             # AlertEvaluator service
│   └── notifier.mpl              # AlertNotifier actor
├── api/
│   ├── routes.mpl                # HTTP API route registration
│   ├── events_api.mpl            # GET /api/v1/events, /issues endpoints
│   ├── projects_api.mpl          # Project CRUD endpoints
│   └── alerts_api.mpl            # Alert rule CRUD endpoints
└── cluster/
    ├── node_manager.mpl          # Node startup and mesh formation
    └── cluster_sync.mpl          # Cross-node state synchronization
```

**Module naming convention:** `mesher/types/event.mpl` -> `Types.Event`, imported as `import Types.Event` or `from Types.Event import { Event, RawEvent }`.

**Build command:** `meshc build mesher/` produces a single native binary.

## Type System Design

### Core Event Types (types/event.mpl)

```mesh
# Severity levels as sum type -- pattern matching + exhaustiveness
type Severity do
  Debug
  Info
  Warning
  Error
  Fatal
end deriving(Eq, Ord, Display, Json)

# Event status for lifecycle tracking
type EventStatus do
  Received
  Processed
  Stored
  Failed(String)
end deriving(Display, Json)

# Raw event from ingestion (before processing)
struct RawEvent do
  project_id :: String
  level :: Severity
  message :: String
  stacktrace :: String?     # Option<String>
  tags :: Map<String, String>
  timestamp :: String        # ISO 8601
  sdk_name :: String?
  sdk_version :: String?
end deriving(Json)

# Enriched event (after processing)
struct EnrichedEvent do
  id :: String               # UUID
  project_id :: String
  issue_id :: String          # Fingerprint-derived group
  level :: Severity
  message :: String
  stacktrace :: String?
  fingerprint :: String
  tags :: Map<String, String>
  received_at :: String
  processed_at :: String
end deriving(Json, Row)

# Database row for events table
struct EventRow do
  id :: String
  project_id :: String
  issue_id :: String
  level :: String
  message :: String
  stacktrace :: String?
  fingerprint :: String
  tags_json :: String
  received_at :: String
  processed_at :: String
end deriving(Row)
```

### Issue Types (types/issue.mpl)

```mesh
type IssueStatus do
  Open
  Resolved
  Ignored
  Regressed
end deriving(Eq, Display, Json)

struct Issue do
  id :: String
  project_id :: String
  fingerprint :: String
  title :: String              # First line of message
  level :: Severity
  status :: IssueStatus
  event_count :: Int
  first_seen :: String
  last_seen :: String
end deriving(Json, Row)
```

### Alert Types (types/alert.mpl)

```mesh
type AlertCondition do
  EventCountAbove(Int, Int)           # threshold, window_seconds
  ErrorRateAbove(Float, Int)          # rate, window_seconds
  NewIssueDetected
  IssueRegressed(String)              # issue_id
end deriving(Json)

type AlertAction do
  WebhookPost(String)                 # URL
  LogMessage(String)                  # log prefix
end deriving(Json)

struct AlertRule do
  id :: String
  project_id :: String
  name :: String
  condition :: AlertCondition
  action :: AlertAction
  enabled :: Bool
end deriving(Json, Row)
```

### API Response Types (types/api.mpl)

```mesh
struct ApiResponse<T> do
  data :: T
  meta :: ApiMeta
end deriving(Json)

struct ApiMeta do
  total :: Int
  page :: Int
  per_page :: Int
end deriving(Json)

type ApiError do
  BadRequest(String)
  NotFound(String)
  InternalError(String)
  Unauthorized
end deriving(Json)
```

### Message Types Between Actors

```mesh
# Router -> Processor messages
type ProcessorMsg do
  ProcessRaw(RawEvent)
  Shutdown
end

# Processor -> StorageWriter messages
type StorageMsg do
  StoreEvent(EnrichedEvent)
  FlushNow
end

# Processor -> StreamBroadcaster messages
type StreamMsg do
  BroadcastEvent(EnrichedEvent)
  BroadcastAlert(String, String)    # room, message
end

# Timer -> AlertEvaluator self-messages
type AlertMsg do
  Evaluate
  RuleUpdated(AlertRule)
  RuleDeleted(String)                # rule_id
end
```

## Trait-Based Extension Points

### Fingerprint Interface

```mesh
# Extensible fingerprinting via trait
interface Fingerprinter do
  fn fingerprint(self, event :: RawEvent) -> String
end

# Default implementation: hash message + first N stacktrace frames
struct DefaultFingerprinter do
end

impl Fingerprinter for DefaultFingerprinter do
  fn fingerprint(self, event :: RawEvent) -> String do
    let base = event.message
    let trace_part = case event.stacktrace do
      Some(st) -> st |> String.split("\n") |> List.take(5) |> String.join("\n")
      None -> ""
    end
    # Use string concatenation as hash input
    # Hash the combined string for the fingerprint
    base <> "::" <> trace_part
  end
end
```

**Mesh features exercised:** traits (interface + impl), pattern matching on Option, pipe operator, iterators (split/take/join), closures.

## Database Schema

### PostgreSQL Tables

```sql
-- Projects table
CREATE TABLE projects (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    api_key     TEXT NOT NULL UNIQUE,
    created_at  TIMESTAMP NOT NULL DEFAULT NOW()
);

-- Events table (append-only, high write volume)
-- Partitioned by received_at for time-range queries and retention
CREATE TABLE events (
    id           TEXT NOT NULL,
    project_id   TEXT NOT NULL,
    issue_id     TEXT NOT NULL,
    level        TEXT NOT NULL,
    message      TEXT NOT NULL,
    stacktrace   TEXT,
    fingerprint  TEXT NOT NULL,
    tags_json    TEXT NOT NULL DEFAULT '{}',
    received_at  TIMESTAMP NOT NULL,
    processed_at TIMESTAMP NOT NULL,
    PRIMARY KEY (id, received_at)
) PARTITION BY RANGE (received_at);

-- Create daily partitions (managed by a startup migration actor)
-- CREATE TABLE events_2026_02_14 PARTITION OF events
--   FOR VALUES FROM ('2026-02-14') TO ('2026-02-15');

-- Issues table (updated on each event, grouped by fingerprint)
CREATE TABLE issues (
    id           TEXT PRIMARY KEY,
    project_id   TEXT NOT NULL REFERENCES projects(id),
    fingerprint  TEXT NOT NULL,
    title        TEXT NOT NULL,
    level        TEXT NOT NULL,
    status       TEXT NOT NULL DEFAULT 'open',
    event_count  INTEGER NOT NULL DEFAULT 0,
    first_seen   TIMESTAMP NOT NULL,
    last_seen    TIMESTAMP NOT NULL,
    UNIQUE(project_id, fingerprint)
);

-- Alert rules table
CREATE TABLE alert_rules (
    id           TEXT PRIMARY KEY,
    project_id   TEXT NOT NULL REFERENCES projects(id),
    name         TEXT NOT NULL,
    condition_json TEXT NOT NULL,
    action_json  TEXT NOT NULL,
    enabled      BOOLEAN NOT NULL DEFAULT true,
    created_at   TIMESTAMP NOT NULL DEFAULT NOW()
);

-- Alert history (for audit trail)
CREATE TABLE alert_history (
    id           TEXT PRIMARY KEY,
    rule_id      TEXT NOT NULL REFERENCES alert_rules(id),
    project_id   TEXT NOT NULL,
    triggered_at TIMESTAMP NOT NULL,
    details_json TEXT NOT NULL
);

-- Indexes for common query patterns
CREATE INDEX idx_events_project_time ON events (project_id, received_at DESC);
CREATE INDEX idx_events_issue ON events (issue_id, received_at DESC);
CREATE INDEX idx_issues_project_status ON issues (project_id, status);
CREATE INDEX idx_issues_project_last_seen ON issues (project_id, last_seen DESC);
CREATE INDEX idx_alert_rules_project ON alert_rules (project_id, enabled);
```

**Schema design rationale:**
- **Events partitioned by day:** PostgreSQL native range partitioning on `received_at`. No TimescaleDB needed -- Mesher manages partition creation at startup and via a daily timer. This gives efficient time-range queries and simple retention (drop old partitions).
- **All values as TEXT for Mesh compatibility:** Mesh's `deriving(Row)` maps `Map<String, String>` to struct fields. PostgreSQL returns all values as text in the wire protocol. Using TEXT for JSON columns (tags_json, condition_json, action_json) keeps the schema simple and maps directly to Mesh's JSON capabilities.
- **Issues table with UNIQUE(project_id, fingerprint):** The fingerprinter generates a deterministic fingerprint. On each new event, the processor does an upsert: INSERT new issue or UPDATE event_count/last_seen on existing.

### Batch Write Strategy

The StorageWriter Service accumulates events in a List and flushes either when the batch reaches a size threshold or a timer fires:

```mesh
service StorageWriter do
  fn init(pool, batch_size :: Int) -> WriterState do
    # Schedule first flush timer
    Timer.send_after(self(), 1000, FlushNow)
    WriterState { pool: pool, buffer: [], batch_size: batch_size }
  end

  cast StoreEvent(event :: EnrichedEvent) do |state|
    let new_buffer = [event] ++ state.buffer
    if List.length(new_buffer) >= state.batch_size do
      flush_batch(state.pool, new_buffer)
      WriterState { pool: state.pool, buffer: [], batch_size: state.batch_size }
    else
      WriterState { pool: state.pool, buffer: new_buffer, batch_size: state.batch_size }
    end
  end

  cast FlushNow() do |state|
    if List.length(state.buffer) > 0 do
      flush_batch(state.pool, state.buffer)
    end
    # Reschedule flush timer
    Timer.send_after(self(), 1000, FlushNow)
    WriterState { pool: state.pool, buffer: [], batch_size: state.batch_size }
  end
end
```

**Mesh features exercised:** service (GenServer), list operations, Timer.send_after for periodic work, pattern matching, struct update.

## Supervision Tree Design

### Root Supervisor

```mesh
supervisor RootSupervisor do
  strategy: one_for_one
  max_restarts: 10
  max_seconds: 60

  child ingestion do
    start: fn -> spawn(IngestionSupervisor) end
    restart: permanent
    shutdown: 10000
  end

  child processing do
    start: fn -> spawn(ProcessingSupervisor) end
    restart: permanent
    shutdown: 10000
  end

  child storage do
    start: fn -> spawn(StorageSupervisor) end
    restart: permanent
    shutdown: 10000
  end

  child streaming do
    start: fn -> spawn(StreamingSupervisor) end
    restart: permanent
    shutdown: 10000
  end

  child alerting do
    start: fn -> spawn(AlertingSupervisor) end
    restart: permanent
    shutdown: 10000
  end
end
```

**Strategy rationale:**
- **Root: one_for_one** -- each subsystem is independent. If alerting crashes, ingestion/storage/streaming continue.
- **Storage: rest_for_one** -- the PgPool must start before SchemaManager, which must run before StorageWriter. If the pool crashes, everything downstream restarts in order.
- **Processing: one_for_one** -- individual processor actors are independent. If one crashes, others continue processing.

### Fault Isolation Boundaries

```
Layer 1 (Critical):  Ingestion + Storage   -- must never go down together
Layer 2 (Important): Streaming             -- degraded UX but no data loss if down
Layer 3 (Deferrable): Alerting             -- alerts delayed but events still stored
Layer 4 (Optional):  Clustering            -- single-node still fully functional
```

The one_for_one root strategy ensures a crash in Layer 3 never cascades to Layer 1. Each layer supervisor independently manages its children's restart policy.

## Multi-Node Distribution Strategy

### Cluster Formation

```mesh
# In cluster/node_manager.mpl
actor NodeManager(node_name :: String, cookie :: String, peers :: List<String>) do
  # Start this node
  Node.start(node_name, cookie)

  # Register critical services globally
  Global.register("event_router", EventRouter.whereis())
  Global.register("storage_writer", StorageWriter.whereis())

  # Connect to peer nodes
  for peer in peers do
    Node.connect(peer)
  end

  # Monitor all connected nodes
  let nodes = Node.list()
  for node in nodes do
    Node.monitor(node)
  end

  # Handle cluster events
  loop(node_name, cookie, peers)
end

fn loop(name :: String, cookie :: String, peers :: List<String>) do
  receive do
    (:nodedown, node_name) ->
      println("Node disconnected: ${node_name}")
      # Re-register local services that may have been shadowed
      # Trigger rebalance of processor pool
      loop(name, cookie, peers)

    (:nodeup, node_name) ->
      println("Node connected: ${node_name}")
      Node.monitor(node_name)
      loop(name, cookie, peers)
  end
end
```

### Cross-Node Event Distribution

Three distribution strategies used in Mesher, each exercising different Mesh distributed features:

**1. Ingestion Load Distribution (Global Registry)**
- Each node runs an HTTP ingestion server on a different port
- A load balancer distributes requests across nodes
- EventRouter is registered globally: `Global.register("event_router", pid)`
- Any node can route events to the globally-registered router: `send(Global.whereis("event_router"), msg)`

**2. WebSocket Room Broadcast (Cross-Node Rooms)**
- Ws.broadcast automatically reaches room members on all nodes via DIST_ROOM_BROADCAST
- Dashboard clients connect to any node; room membership is node-local but broadcasts are cluster-wide
- Zero application code needed for cross-node broadcast -- the runtime handles it

**3. Remote Processor Spawning (Node.spawn)**
- Under heavy load, the EventRouter can spawn processors on remote nodes: `Node.spawn("worker@host:4001", processor_fn, [config])`
- Processors on remote nodes send results back to the local StorageWriter and StreamBroadcaster via location-transparent PIDs
- Remote supervision: the ProcessingSupervisor can monitor and restart remote processor actors

**Mesh features exercised:** Node.start/connect/list/monitor, Global.register/whereis, Ws.broadcast (cross-node), Node.spawn/spawn_link, location-transparent send, :nodedown/:nodeup pattern matching, remote supervision.

## Patterns to Follow

### Pattern 1: Actor-Per-Connection with Crash Isolation

**What:** Each HTTP request and WebSocket connection runs in its own actor. A panic in one handler kills only that actor; the server continues accepting new connections.

**When:** All HTTP and WebSocket servers in Mesher.

**Example:**
```mesh
fn handle_ingest(request) do
  let body = Request.body(request)
  let result = RawEvent.from_json(body)
  case result do
    Ok(event) ->
      let router_pid = Global.whereis("event_router")
      send(router_pid, ProcessRaw(event))
      HTTP.response(202, "{\"status\":\"accepted\"}")
    Err(e) ->
      HTTP.response(400, "{\"error\":\"${e}\"}")
  end
end
```

If `from_json` triggers a panic (malformed input), only this request's actor crashes. The supervisor restarts nothing (the actor was transient -- it dies after handling one request). The HTTP server spawns a new actor for the next request.

### Pattern 2: Service as Stateful Singleton

**What:** Use Mesh's `service` (GenServer) for components that maintain state: the EventRouter (routing table), Fingerprinter (fingerprint cache), StorageWriter (batch buffer), AlertRuleStore (rule cache), AlertEvaluator (timer state).

**When:** Any component that needs mutable state shared across messages.

**Example:**
```mesh
service EventRouter do
  fn init() -> Map<String, Int> do
    # project_id -> processor_pid mapping
    Map.new()
  end

  call Route(event :: RawEvent) :: Int do |routing_table|
    let project_id = event.project_id
    let pid = case Map.get(routing_table, project_id) do
      Some(p) -> p
      None -> pick_processor()  # round-robin from pool
    end
    send(pid, ProcessRaw(event))
    (pid, routing_table)
  end

  cast UpdateRoute(project_id :: String, pid :: Int) do |routing_table|
    Map.put(routing_table, project_id, pid)
  end
end
```

### Pattern 3: Timer-Driven Periodic Work

**What:** Use `Timer.send_after(self(), ms, msg)` in a receive loop to implement periodic tasks: batch flushing, alert evaluation, partition management, metric aggregation.

**When:** AlertEvaluator (check rules every N seconds), StorageWriter (flush every second), partition manager (create partitions daily).

**Example:**
```mesh
actor alert_evaluator(pool, rules :: List<AlertRule>) do
  Timer.send_after(self(), 10000, Evaluate)
  eval_loop(pool, rules)
end

fn eval_loop(pool, rules :: List<AlertRule>) do
  receive do
    Evaluate ->
      for rule in rules when rule.enabled do
        evaluate_rule(pool, rule)
      end
      Timer.send_after(self(), 10000, Evaluate)
      eval_loop(pool, rules)

    RuleUpdated(rule) ->
      let updated = List.map(rules, fn(r) do
        if r.id == rule.id do rule else r end
      end)
      eval_loop(pool, updated)

    RuleDeleted(rule_id) ->
      let filtered = List.filter(rules, fn(r) -> r.id != rule_id end)
      eval_loop(pool, filtered)
  end
end
```

**Mesh features exercised:** Timer.send_after, tail-recursive receive loop (TCE), for-in with filter (when clause), List.map/filter with closures, pattern matching on sum types.

### Pattern 4: Pipeline Fan-Out via Message Passing

**What:** After processing an event, the processor sends the result to multiple downstream actors (storage, streaming, alerting) using separate `send()` calls. Each downstream handles the message independently and at its own pace.

**When:** The processor -> storage/streaming/alerting fan-out point.

**Why not function calls:** Function calls would make the processor block on each downstream operation. Message passing is async -- the processor can handle the next event immediately while storage batches writes and alerting evaluates rules.

### Pattern 5: Authentication Middleware

**What:** HTTP middleware that extracts the API key from the Authorization header and resolves it to a project_id.

**When:** All API endpoints.

```mesh
fn auth_middleware(request :: Request, next) -> Response do
  let auth = Request.header(request, "authorization")
  case auth do
    Some(key) ->
      # In a real implementation, validate key against projects table
      # For now, pass through with the key as context
      next(request)
    None ->
      HTTP.response(401, "{\"error\":\"missing api key\"}")
  end
end
```

## Anti-Patterns to Avoid

### Anti-Pattern 1: Shared Mutable State Between Actors

**What:** Trying to share a database connection or mutable data structure between actors.

**Why bad:** Mesh's actor model enforces isolation. There is no shared memory. Attempting to pass connection handles between actors will fail because connections are opaque u64 handles tied to a specific OS thread/connection state.

**Instead:** Use the connection pool (`Pool.query`, `Pool.execute`). The pool handles checkout/checkin automatically and is thread-safe. Every actor that needs database access uses the pool handle (which is a u64 that can be safely passed in messages or closed over).

### Anti-Pattern 2: Synchronous Call Chains in the Hot Path

**What:** Using `Service.call()` (synchronous) for every step in the ingestion pipeline.

**Why bad:** Service calls block the caller until the callee responds. A chain of synchronous calls (ingest -> route -> process -> store) would serialize the entire pipeline and eliminate concurrency benefits.

**Instead:** Use `send()` (async fire-and-forget) between pipeline stages. The ingestion handler sends to the router and immediately returns HTTP 202. The router sends to a processor and handles the next event. Only use synchronous `call` when the caller truly needs the response (e.g., fingerprint cache lookup where the processor needs the issue_id before continuing).

### Anti-Pattern 3: One Giant Actor for Everything

**What:** Putting all logic in a single actor or service that handles HTTP, processing, storage, and alerting.

**Why bad:** Defeats supervision isolation. A crash in alerting logic would kill the entire application. Also serializes all work through one mailbox.

**Instead:** Separate concerns into distinct actors with distinct supervision trees, as shown in the topology above. This is also the dogfooding goal -- exercise as many actors, supervisors, and message flows as possible.

### Anti-Pattern 4: Unbounded Batch Buffer

**What:** Accumulating events in the StorageWriter without a size limit, relying only on the timer to flush.

**Why bad:** Under high ingestion load, the batch buffer grows unboundedly, consuming actor heap memory. The GC will keep the list alive because it is reachable state.

**Instead:** Flush when batch_size >= threshold OR timer fires, whichever comes first. Also consider applying backpressure by having the EventRouter slow down if the StorageWriter's batch exceeds a high-water mark.

### Anti-Pattern 5: Blocking Database Calls in Ingestion Actors

**What:** Performing Pool.query inside the HTTP handler actor before returning the response.

**Why bad:** Database calls block the actor (the connection checkout + query + result). Under high concurrency with many ingestion actors, all pool connections could be checked out by ingestion actors, starving the storage writer and API query handlers.

**Instead:** Ingestion actors should only parse + validate + forward. Database writes happen in the StorageWriter service. API query endpoints (GET /issues) can query the database directly since they are lower-volume read operations.

## Stress-Test Points (Dogfooding Goals)

Every architectural decision is chosen to exercise specific Mesh features under load:

| Stress Target | Architecture Decision | Mesh Feature Exercised |
|---------------|----------------------|----------------------|
| Scheduler fairness | Thousands of concurrent HTTP ingestion actors | M:N scheduler, reduction checks, GC at yield points |
| Mailbox throughput | High-volume message passing between pipeline stages | Actor mailbox, selective receive |
| GC under pressure | Long-lived services with growing/shrinking state (batch buffer, caches) | Mark-sweep GC per actor, bounded memory |
| Supervision recovery | Intentional crash scenarios in processors | Supervisor restart, one_for_one/rest_for_one strategies |
| Database pool contention | Many actors competing for pooled connections | Pool checkout/checkin, timeout handling, transaction safety |
| Complex type hierarchies | Sum types with fields, generic structs, nested deriving | Type inference, monomorphization, deriving(Json)/deriving(Row) |
| Iterator pipelines | Event filtering, transformation, aggregation using Iter combinators | Lazy iterators, pipe operator, Collect |
| Pattern matching depth | Matching on nested sum types (AlertCondition variants, EventStatus) | Exhaustiveness checking, sum type field extraction |
| Service state management | Services maintaining Maps, Lists, counters as state | GenServer call/cast, state threading |
| Timer correctness | Periodic alert evaluation, batch flush timers | Timer.send_after, cooperative scheduling |
| WebSocket rooms | Hundreds of dashboard clients subscribing to filtered rooms | Ws.join/broadcast, room management, cross-node broadcast |
| Distributed messaging | Cross-node event routing and global process discovery | Node.spawn, Global.register/whereis, location-transparent PIDs |
| HTTP middleware | Request validation, auth, CORS through middleware pipeline | HTTP.use, trampoline-based chain |
| Error propagation | ? operator chaining through database calls, JSON parsing | Result/Option ?, From/Into error conversion |
| Multi-file compilation | 20+ module project with cross-module imports | Module system, pub visibility, qualified imports |
| Trait dispatch | Custom Fingerprinter trait with multiple implementations | interface, impl, static dispatch |

## Suggested Build Order

```
Phase 1: Foundation (Types + Database + Storage)
  ├── types/*.mpl -- define all structs, sum types, deriving
  ├── storage/migrations.mpl -- schema creation
  ├── storage/queries.mpl -- SQL helper functions
  └── storage/writer.mpl -- StorageWriter service with batch + flush
  Dependencies: None (pure types + database)
  Exercises: structs, sum types, deriving(Json/Row), Pool, transactions, services

Phase 2: Ingestion Pipeline (HTTP + Processing)
  ├── ingestion/http_handler.mpl -- POST /api/v1/events
  ├── ingestion/router.mpl -- EventRouter service
  ├── processing/fingerprint.mpl -- Fingerprinter trait + DefaultFingerprinter
  ├── processing/processor.mpl -- Processor actor
  └── main.mpl (partial) -- start HTTP server + supervisors
  Dependencies: Phase 1 types + storage
  Exercises: HTTP server, middleware, actor messaging, traits, pattern matching

Phase 3: Real-Time Streaming (WebSocket)
  ├── streaming/broadcaster.mpl -- StreamBroadcaster actor
  ├── streaming/dashboard_handler.mpl -- WS dashboard server
  └── Integration with processor fan-out
  Dependencies: Phase 2 (events flow from processor)
  Exercises: WebSocket, rooms, Ws.broadcast, actor-per-connection

Phase 4: REST API (Query + CRUD)
  ├── api/routes.mpl -- route registration
  ├── api/events_api.mpl -- GET events, issues
  ├── api/projects_api.mpl -- project CRUD
  └── api/alerts_api.mpl -- alert rule CRUD
  Dependencies: Phase 1 (queries), Phase 2 (ingestion running)
  Exercises: HTTP routing, JSON encoding, Pool.query, deriving(Json)

Phase 5: Alerting System
  ├── alerting/rule_store.mpl -- AlertRuleStore service
  ├── alerting/evaluator.mpl -- AlertEvaluator with timer
  └── alerting/notifier.mpl -- AlertNotifier with HTTP webhook
  Dependencies: Phase 4 (alert rule CRUD), Phase 1 (queries)
  Exercises: Timer.send_after, service state, HTTP client, pattern matching on sum types

Phase 6: Multi-Node Clustering
  ├── cluster/node_manager.mpl -- Node.start, connect, monitor
  ├── cluster/cluster_sync.mpl -- Global registry for services
  └── Integration: cross-node WS broadcast, remote processor spawn
  Dependencies: Phases 1-5 (full single-node working)
  Exercises: Distributed actors, Global registry, Node.spawn, location-transparent PIDs

Phase 7: Vue Frontend
  └── Separate directory, not Mesh code
  Dependencies: Phase 4 (REST API), Phase 3 (WS streaming)
```

**Build order rationale:**
- Types come first because every other module imports them.
- Storage before ingestion because the writer must exist before events can be stored.
- Ingestion before streaming because events must flow into the system before they can be streamed out.
- REST API after ingestion because it queries data that ingestion writes.
- Alerting after REST API because alert rules need CRUD endpoints.
- Clustering last because it layers on top of a fully-working single-node system.
- Frontend last because it consumes the API and WebSocket that must already work.

## Scalability Considerations

| Concern | Single Node | 3-Node Cluster | 10-Node Cluster |
|---------|-------------|----------------|-----------------|
| Event ingestion | HTTP actor-per-conn, thousands concurrent | Load-balanced across nodes | Horizontal scale-out |
| Processing throughput | N processor actors (cpu-core scaling) | Remote-spawn processors on other nodes | Processor pools per node |
| Database writes | Batch writer with Pool (10-20 conns) | Shared PG, per-node pools | Single PG with larger pool, or PG replicas for reads |
| WebSocket streaming | Rooms on single node | DIST_ROOM_BROADCAST cross-node | Automatic via Mesh distributed rooms |
| Alert evaluation | Timer-driven in one service | Single evaluator (leader election via Global) | Same -- alerting is low-volume |
| State sync | N/A | Global.register for service discovery | Global registry with broadcast |

## Sources

- Direct analysis of Mesh runtime APIs: `crates/mesh-rt/src/http/`, `crates/mesh-rt/src/ws/`, `crates/mesh-rt/src/dist/`, `crates/mesh-rt/src/actor/`, `crates/mesh-rt/src/db/`
- Mesh language documentation: `website/docs/docs/concurrency/`, `website/docs/docs/web/`, `website/docs/docs/databases/`, `website/docs/docs/distributed/`
- Mesh language examples: `tests/e2e/supervisor_basic.mpl`, `tests/e2e/service_call_cast.mpl`, `tests/e2e/stdlib_http_server_runtime.mpl`, `tests/e2e/stdlib_pg.mpl`, `tests/e2e/deriving_json_sum_type.mpl`
- Mesh module system: `crates/meshc/src/main.rs` (meshc build <dir>)
- [Sentry issue grouping architecture](https://develop.sentry.dev/backend/application-domains/grouping/) -- fingerprinting hierarchy (fingerprint > stacktrace > exception > message), GroupHash model
- [Sentry event fingerprinting](https://docs.sentry.io/concepts/data-management/event-grouping/fingerprint-rules/) -- custom fingerprint rules, SDK-side vs server-side fingerprinting
- [PostgreSQL native partitioning for time-series](https://aws.amazon.com/blogs/database/designing-high-performance-time-series-data-tables-on-amazon-rds-for-postgresql/) -- range partitioning by timestamp, daily granularity, partition pruning
- [PostgreSQL partitioning strategies](https://medium.com/@connect.hashblock/9-postgres-partitioning-strategies-for-time-series-at-scale-c1b764a9b691) -- partition granularity selection, hot/warm/cold tiers
- Confidence: HIGH -- all Mesh language capabilities verified against existing test files and runtime FFI exports; monitoring domain patterns based on established architectures (Sentry, Datadog)
