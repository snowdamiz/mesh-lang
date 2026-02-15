# Phase 88: Ingestion Pipeline - Research

**Researched:** 2026-02-14
**Domain:** HTTP/WebSocket event ingestion, DSN authentication, validation, rate limiting, and supervised actor-based processing -- all implemented in the Mesh language
**Confidence:** HIGH

## Summary

Phase 88 builds the event ingestion pipeline for Mesher: the HTTP and WebSocket endpoints that accept error events from external clients, authenticate them via API keys (DSN-style), validate payloads, enforce per-project rate limits, and route validated events into the StorageWriter service for batched persistence. Everything is written in Mesh (.mpl files) using existing runtime capabilities: `HTTP.router()`, `HTTP.on_post()`, `HTTP.serve()`, `WS.serve()`, `service` blocks, `supervisor` blocks, and the actor system.

The critical technical challenge is a **runtime gap**: the current `MeshHttpResponse` struct only has `status` and `body` fields -- no response headers. The ingestion pipeline requires a `Retry-After` header on 429 responses (INGEST-04). This means Phase 88 must either (a) extend the Rust runtime to support response headers, or (b) embed the retry-after value in the response body as a workaround. Option (a) is strongly recommended since response headers are a fundamental HTTP feature needed beyond just this phase. Additionally, the `write_response` function's status text lookup needs entries for 202 ("Accepted") and 429 ("Too Many Requests").

The second major concern is **rate limiting state management**. Since Mesh has no mutable variables, rate limiting state must be managed through a service actor (GenServer pattern). A `RateLimiter` service per project tracks event counts within sliding time windows using the existing `Timer.sleep` + recursive actor pattern for window expiry. The `get_project_by_api_key` query from Phase 87 provides the authentication bridge -- each incoming request extracts the API key, looks up the project, then checks the rate limiter before processing.

**Primary recommendation:** First extend the runtime with response headers support (MeshHttpResponse gets a headers map field, write_response emits custom headers, add 202/429 status text). Then build the ingestion layer in Mesh: authentication middleware, validation functions, rate limiter service, HTTP routes, WebSocket handler, and supervision tree wrapping the entire pipeline.

## Standard Stack

### Core
| Component | Version/Detail | Purpose | Why Standard |
|-----------|---------------|---------|--------------|
| Mesh language | v8.0 (current) | All application code | Dogfooding milestone -- entire ingestion pipeline in Mesh |
| HTTP.router + HTTP.on_post | Built-in | POST /api/v1/events, POST /api/v1/events/bulk | Mesh HTTP server with method-specific routing |
| HTTP.use (middleware) | Built-in | Authentication, rate limiting as middleware chain | Existing middleware infrastructure from Phase 52 |
| WS.serve | Built-in | WebSocket event streaming | Actor-per-connection with crash isolation |
| service | Built-in | RateLimiter, EventProcessor stateful actors | GenServer pattern: call (sync), cast (async), state threading |
| supervisor | Built-in | Supervision tree for pipeline actors | one_for_one strategy, restart limits, crash isolation |
| Request.header | Built-in | Extract API key from X-Sentry-Auth or Authorization header | Returns Option<String>, case match for Some/None |
| Request.body | Built-in | Extract JSON event payload | Returns String body content |
| Json.parse | Built-in | Parse incoming JSON payloads | Returns Result with parsed Json value |
| deriving(Json) | Built-in | EventPayload from_json for typed deserialization | Auto-generated from_json for struct fields |
| Pool.query | Built-in | API key lookup, project validation | PostgreSQL queries via connection pool |
| StorageWriter.store | Phase 87 | Route validated events to batched persistence | Per-project writer actors already built |

### Supporting
| Component | Detail | Purpose | When to Use |
|-----------|--------|---------|-------------|
| Timer.sleep | Built-in | Rate limit window expiry, periodic cleanup | Sliding window token replenishment |
| String.starts_with | Built-in | API key prefix validation (mshr_) | Quick reject of malformed keys |
| String.length | Built-in | Payload size validation | Reject oversized event bodies |
| String.to_int | Built-in | Parse numeric values from headers/params | Rate limit configuration |
| List.length | Built-in | Bulk event count validation | Cap bulk request size |
| Map.get | Built-in | Extract fields from parsed JSON | Validation of required fields |
| Json.encode | Built-in | Serialize response bodies | Error responses, event ID responses |

### Runtime Extensions Required
| Extension | Location | Purpose | Why Needed |
|-----------|----------|---------|------------|
| Response headers | MeshHttpResponse + write_response | Retry-After header on 429 | INGEST-04 requires Retry-After header |
| Status 202 text | write_response status_text match | "Accepted" response text | INGEST-06 requires 202 Accepted |
| Status 429 text | write_response status_text match | "Too Many Requests" text | INGEST-04 requires 429 status |
| HTTP.response_with_headers | New Mesh API | Construct response with headers map | Mesh-level API for setting headers |

## Architecture Patterns

### Recommended Project Structure
```
mesher/
  main.mpl                      # Entry point -- starts services, HTTP server
  types/
    event.mpl                   # EventPayload, Event (existing from Phase 87)
    project.mpl                 # Project, ApiKey (existing)
    ...
  storage/
    queries.mpl                 # get_project_by_api_key (existing)
    writer.mpl                  # insert_event (existing)
    ...
  services/
    writer.mpl                  # StorageWriter service (existing)
    org.mpl                     # OrgService (existing)
    project.mpl                 # ProjectService (existing)
    user.mpl                    # UserService (existing)
    rate_limiter.mpl            # NEW: RateLimiter service (per-project rate state)
    event_processor.mpl         # NEW: EventProcessor service (validate + route to writer)
  ingestion/
    auth.mpl                    # NEW: API key authentication middleware
    validation.mpl              # NEW: Event payload validation functions
    routes.mpl                  # NEW: HTTP route handlers (POST /api/v1/events, bulk)
    ws_handler.mpl              # NEW: WebSocket event streaming handler
    pipeline.mpl                # NEW: Supervision tree for ingestion actors
```

### Pattern 1: Authentication Middleware
**What:** Extract API key from request header, look up project, attach project context
**When to use:** Every ingestion endpoint request
**Example:**
```mesh
# Mesh middleware signature: fn(request, next) -> Response
fn auth_middleware(request :: Request, next) do
  let key_header = Request.header(request, "x-sentry-auth")
  case key_header do
    Some(key) ->
      case get_project_by_api_key(pool, key) do
        Ok(project) -> next(request)    # Project found, continue
        Err(_) -> HTTP.response(401, "{\"error\":\"Invalid API key\"}")
      end
    None -> HTTP.response(401, "{\"error\":\"Missing API key\"}")
  end
end
```
**Constraint:** Middleware in Mesh is a function `(Request, next) -> Response`. The `next` parameter is a closure calling the next middleware or final handler. The middleware cannot attach data to the request (Request struct is immutable with no user-data field). The project lookup result must be re-derived in the handler, or a different approach must be used.

### Pattern 2: Service-Based Rate Limiter
**What:** Per-project sliding window rate limiter as a GenServer service actor
**When to use:** Rate limiting ingestion per project
**Example:**
```mesh
# State tracks event count and window start time.
# Since Mesh has no mutable variables, state is threaded through service handlers.
struct RateLimitState do
  counts :: Map<String, Int>       # project_id -> event count in current window
  window_start :: Int              # epoch seconds of current window start
  window_seconds :: Int            # window duration
  max_events :: Int                # max events per window per project
end

service RateLimiter do
  fn init(window_seconds :: Int, max_events :: Int) -> RateLimitState do
    RateLimitState {
      counts: Map.new(),
      window_start: 0,
      window_seconds: window_seconds,
      max_events: max_events
    }
  end

  # Check if project is within rate limit. Returns (allowed :: Bool, retry_after :: Int).
  call CheckLimit(project_id :: String) :: Bool do |state|
    # ... sliding window logic, return (new_state, allowed)
  end
end
```

### Pattern 3: Timer-Based Window Reset
**What:** Recursive actor that periodically resets rate limit windows (same pattern as flush_ticker)
**When to use:** Sliding window expiry for rate limiter
**Example:**
```mesh
# Same pattern as flush_ticker from StorageWriter
actor rate_window_ticker(limiter_pid, interval :: Int) do
  Timer.sleep(interval)
  RateLimiter.reset_window(limiter_pid)
  rate_window_ticker(limiter_pid, interval)
end
```

### Pattern 4: Supervision Tree for Pipeline
**What:** OTP-style supervision tree wrapping all ingestion actors
**When to use:** Fault tolerance for the entire pipeline
**Example:**
```mesh
supervisor IngestSup do
  strategy: one_for_one
  max_restarts: 10
  max_seconds: 60

  child rate_limiter do
    start: fn -> RateLimiter.start(60, 1000) end
    restart: permanent
    shutdown: 5000
  end

  child event_processor do
    start: fn -> EventProcessor.start(pool, writer_pid) end
    restart: permanent
    shutdown: 5000
  end
end
```

### Pattern 5: WebSocket Event Streaming
**What:** WS.serve with on_connect auth, on_message event processing
**When to use:** INGEST-05 persistent WebSocket connection
**Example:**
```mesh
# WS.serve takes on_connect, on_message, on_close callbacks and a port
# Each connection gets its own actor (crash-isolated by the runtime)
fn ws_on_connect(conn) do
  # Auth happens here -- return value determines accept/reject
  # Connection args include headers from upgrade request
  conn
end

fn ws_on_message(conn, message) do
  # Parse JSON event, validate, route to pipeline
  let parse_result = Json.parse(message)
  case parse_result do
    Ok(json) -> process_ws_event(conn, json)
    Err(_) -> WS.send(conn, "{\"error\":\"Invalid JSON\"}")
  end
end

fn ws_on_close(conn) do
  # Cleanup
end
```

### Anti-Patterns to Avoid
- **Mutable state outside services:** Mesh has no mutable variables. All rate limiting counters, buffers, and state MUST live inside service actors. Do not attempt global variables or process dictionary hacks.
- **Blocking in middleware:** The auth middleware calls `Pool.query` which is synchronous but runs within the actor's coroutine. Each HTTP connection already gets its own actor, so blocking one does not affect others. However, avoid unbounded database calls in hot paths.
- **Timer.send_after for service dispatch:** Timer.send_after delivers raw bytes that cannot match service cast dispatch tags. Use the recursive `Timer.sleep` + cast pattern (flush_ticker pattern) instead. This is a known constraint from Phase 87.
- **Attaching context to requests:** The MeshHttpRequest struct is read-only with a fixed set of fields. You cannot add a "project" field to pass auth context from middleware to handler. Instead, the handler must re-derive the project from the API key, or use a registry/actor to cache lookups.
- **? operator in retry logic:** Per decision 87.2-02, use explicit case matching in retry functions where the Err branch calls retry logic. Only use ? in the flush_loop where errors should propagate.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| UUID generation | Custom UUID in Mesh | PostgreSQL `uuidv7()` in INSERT | Mesh has no random/UUID runtime; PG 18 has native uuidv7() |
| JSON parsing | Custom parser | `Json.parse()` + `EventPayload.from_json()` | Built-in serde-backed parser with deriving(Json) |
| Connection pooling | Custom pool | `Pool.open()` + `Pool.query()` | Built-in connection pool with checkout/checkin |
| Crash isolation | Custom try/catch | HTTP server actor-per-connection + supervisor | Runtime provides catch_unwind per connection actor |
| Password hashing | Custom hash | PostgreSQL `pgcrypto` | No Argon2/bcrypt in Mesh runtime |
| Event batching | Custom batch queue | `StorageWriter.store()` cast | Per-project writer with size+timer flush already built |

**Key insight:** The Mesh runtime handles crash isolation at the connection level automatically. Each HTTP connection and each WebSocket connection runs in its own actor with `catch_unwind`. If a handler panics (e.g., during validation), the connection returns a 500 but other connections are unaffected. The supervisor tree adds a second layer: if the pipeline service actors themselves crash, they are automatically restarted.

## Common Pitfalls

### Pitfall 1: Response Headers Not Supported
**What goes wrong:** Attempting to return a 429 with Retry-After header using `HTTP.response(429, body)` -- the header is silently dropped because MeshHttpResponse has no headers field.
**Why it happens:** The runtime was designed for simple JSON API responses with hardcoded Content-Type. Response headers were never needed before.
**How to avoid:** Extend the runtime FIRST before writing Mesh-level ingestion code. Add a `headers` field to `MeshHttpResponse`, a `mesh_http_response_with_headers` constructor, and update `write_response` to emit custom headers.
**Warning signs:** Tests pass but `curl -v` shows no Retry-After header in 429 responses.

### Pitfall 2: Middleware Cannot Pass Auth Context
**What goes wrong:** Auth middleware validates the API key and finds the project, but there is no way to pass the Project struct to the downstream handler. The handler must re-query the database.
**Why it happens:** MeshHttpRequest has a fixed repr(C) struct layout. Adding fields would break existing code. There is no request-context or "locals" mechanism.
**How to avoid:** Accept the double-query cost (one in middleware, one in handler), OR skip the middleware pattern and do inline auth in each handler. Alternatively, build a lightweight cache service actor that stores recent API key -> project lookups.
**Warning signs:** Database load doubles on every request.

### Pitfall 3: No Mutable State for Rate Counters
**What goes wrong:** Attempting to use a "global counter" or "shared Map" for rate limiting -- Mesh has no mutable variables or global state outside actors.
**Why it happens:** Developer instinct from imperative languages.
**How to avoid:** Use a service actor for rate limiting state. The `call CheckLimit(project_id)` pattern is synchronous and returns the result immediately. State is threaded through the service's internal handler.
**Warning signs:** Compilation errors about variable reassignment.

### Pitfall 4: Timer.send_after vs Service Dispatch
**What goes wrong:** Using `Timer.send_after` to schedule rate limit window resets -- the message arrives as raw bytes with the wrong type tag, and the service dispatch loop ignores it.
**Why it happens:** Timer.send_after delivers `MessageBuffer` with a generic tag, but services dispatch on specific type_tag values generated by the compiler.
**How to avoid:** Use the `flush_ticker` pattern: a separate actor that calls `Timer.sleep(interval)` then casts to the service, then recurses. This is a proven pattern from Phase 87.
**Warning signs:** Window resets never fire; rate limits never expire.

### Pitfall 5: Status Code Text Missing for 202 and 429
**What goes wrong:** `HTTP.response(202, body)` sends a response with status line "HTTP/1.1 202 OK" instead of "HTTP/1.1 202 Accepted" because the `write_response` match arm defaults to "OK" for unknown status codes.
**Why it happens:** The status text lookup in `write_response` only covers a limited set of codes (200, 201, 204, 301, 302, 400, 401, 403, 404, 405, 500).
**How to avoid:** Add 202 and 429 entries to the status text match in the runtime before using these status codes.
**Warning signs:** HTTP clients that strictly validate status text may behave unexpectedly.

### Pitfall 6: WebSocket Auth at Connection Time Only
**What goes wrong:** WebSocket connections are authenticated once during the on_connect callback. If an API key is revoked mid-session, the connection remains active.
**Why it happens:** WebSocket upgrade happens once; subsequent frames don't carry auth headers.
**How to avoid:** This is acceptable for monitoring (low-security concern). If needed, periodic re-validation can be done in on_message, but this adds latency. Document the trade-off.
**Warning signs:** Revoked API keys continue to stream events.

### Pitfall 7: Bulk Endpoint Event Count Explosion
**What goes wrong:** A client sends a bulk request with 10,000 events in one payload, overwhelming the StorageWriter buffer and causing drop-oldest to discard legitimate events.
**Why it happens:** No cap on bulk request event count.
**How to avoid:** Validate bulk event count (e.g., max 100 per request) before processing. Reject with 400 if exceeded.
**Warning signs:** StorageWriter drops increase; events disappear.

## Code Examples

### HTTP Route Registration
```mesh
# From existing test: stdlib_http_path_params.mpl
fn main() do
  let r = HTTP.router()
  let r = HTTP.use(r, auth_middleware)
  let r = HTTP.on_post(r, "/api/v1/events", handle_event)
  let r = HTTP.on_post(r, "/api/v1/events/bulk", handle_bulk)
  HTTP.serve(r, 8080)
end
```

### API Key Extraction from Header
```mesh
# Extract API key from X-Sentry-Auth header or Authorization header
fn extract_api_key(request) -> Option<String> do
  let auth = Request.header(request, "x-sentry-auth")
  case auth do
    Some(key) -> Some(key)
    None ->
      let bearer = Request.header(request, "authorization")
      case bearer do
        Some(token) -> Some(token)   # Would need to strip "Bearer " prefix
        None -> None
      end
  end
end
```

### Event Validation
```mesh
# Validate required fields in parsed JSON payload
fn validate_event(payload :: EventPayload) -> String!String do
  if String.length(payload.message) == 0 do
    Err("missing required field: message")
  else
    let valid_levels = ["fatal", "error", "warning", "info", "debug"]
    if List.contains(valid_levels, payload.level) do
      Ok("valid")
    else
      Err("invalid level: must be fatal, error, warning, info, or debug")
    end
  end
end
```

### Supervisor Block Syntax
```mesh
# From existing test: supervisor_basic.mpl
supervisor IngestSup do
  strategy: one_for_one
  max_restarts: 10
  max_seconds: 60

  child rate_limiter do
    start: fn -> RateLimiter.start(60, 1000) end
    restart: permanent
    shutdown: 5000
  end
end

fn main() do
  let sup = spawn(IngestSup)
  println("Ingestion supervisor started")
end
```

### Service Call Pattern (Synchronous Rate Check)
```mesh
# From existing test: service_call_cast.mpl -- call returns a value synchronously
service RateLimiter do
  fn init(max_events :: Int) -> RateLimitState do
    RateLimitState { counts: Map.new(), max_events: max_events }
  end

  # Synchronous check -- handler blocks until result is returned
  call CheckLimit(project_id :: String) :: Bool do |state|
    let count = Map.get_or(state.counts, project_id, 0)
    if count >= state.max_events do
      (state, false)   # (new_state, reply_value)
    else
      let new_counts = Map.put(state.counts, project_id, count + 1)
      let new_state = RateLimitState { counts: new_counts, max_events: state.max_events }
      (new_state, true)
    end
  end
end
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| HTTP.route (all methods) | HTTP.on_get / HTTP.on_post (method-specific) | Phase 51 | Routes can be method-specific -- use on_post for ingestion |
| No middleware | HTTP.use(router, middleware_fn) | Phase 52 | Auth and rate limiting can be middleware |
| No response headers | Status + body only | Current limitation | Must extend runtime for Retry-After (429) |
| No supervisor syntax | supervisor blocks with OTP semantics | Phase 07 | Full one_for_one, one_for_all, rest_for_one, simple_one_for_one |
| No WebSocket | WS.serve with on_connect/on_message/on_close | Phase 59-62 | Actor-per-connection with crash isolation, heartbeat |
| Manual DB queries | Pool.query + deriving(Row) | Phase 54-58 | Typed query results with auto struct mapping |

**Deprecated/outdated:**
- `HTTP.route(r, path, handler)`: Still works but catches all methods. Use `HTTP.on_post` for POST-only routes.

## Open Questions

1. **Response headers runtime extension approach**
   - What we know: MeshHttpResponse is repr(C) with {status, body}. Adding a headers field extends the struct by 8 bytes (pointer to MeshMap). `write_response` must iterate the headers map and emit them.
   - What's unclear: Should the old `HTTP.response(status, body)` continue to work (headers=null)? Or should all responses use the new constructor? Backward compatibility suggests keeping both.
   - Recommendation: Add `headers` as a third optional field. Keep `mesh_http_response_new` setting headers to null. Add `mesh_http_response_with_headers` for the new constructor. Update `write_response` to emit extra headers when non-null.

2. **Auth context passing through middleware**
   - What we know: Middleware validates the API key. The handler also needs the project_id. There is no way to attach data to the Request.
   - What's unclear: Is the double-query overhead acceptable? Could a process-local cache help?
   - Recommendation: Implement a lightweight `AuthCache` service actor that caches recent `api_key -> Project` lookups. Middleware populates the cache; handler reads from cache instead of re-querying DB. Cache entries expire after 60 seconds.

3. **Rate limiter granularity**
   - What we know: Per-project rate limiting is required (INGEST-04). Sliding window is the standard approach.
   - What's unclear: Should the rate limiter be a single global service (one actor for all projects) or per-project actors (like StorageWriter)?
   - Recommendation: Single global RateLimiter service. Unlike StorageWriter (which buffers events and needs per-project isolation for writes), the rate limiter only tracks counts. A single service avoids actor proliferation and simplifies supervision. The service state is a `Map<String, RateEntry>` where RateEntry tracks count and window start.

4. **WebSocket authentication**
   - What we know: WS.serve provides on_connect, on_message, on_close callbacks. The on_connect callback receives a connection object.
   - What's unclear: Does on_connect have access to the HTTP upgrade request's headers (for API key extraction)? Need to verify in the runtime.
   - Recommendation: Investigate `ws_connection_entry` in the runtime to see if upgrade request headers are available to on_connect. If not, the first WS message could be an auth message.

5. **Payload size limit enforcement**
   - What we know: `String.length(Request.body(request))` gives the body size.
   - What's unclear: Does the HTTP server already have a body size limit? The `read_body` function in server.rs may have hardcoded limits.
   - Recommendation: Check `parse_request` in the runtime for existing body size limits. Add a Mesh-level check as well (e.g., reject bodies > 1MB).

## Sources

### Primary (HIGH confidence)
- `/Users/sn0w/Documents/dev/snow/crates/mesh-rt/src/http/server.rs` -- MeshHttpRequest, MeshHttpResponse, write_response, connection_handler_entry, mesh_http_serve
- `/Users/sn0w/Documents/dev/snow/crates/mesh-rt/src/ws/server.rs` -- WsStream, mesh_ws_serve, actor-per-connection WebSocket
- `/Users/sn0w/Documents/dev/snow/crates/mesh-rt/src/actor/supervisor.rs` -- SupervisorState, SupervisorConfig, handle_child_exit, restart strategies
- `/Users/sn0w/Documents/dev/snow/crates/mesh-rt/src/actor/child_spec.rs` -- ChildSpec, Strategy, RestartType, ShutdownType
- `/Users/sn0w/Documents/dev/snow/crates/mesh-rt/src/actor/service.rs` -- mesh_service_call, service dispatch protocol
- `/Users/sn0w/Documents/dev/snow/crates/mesh-codegen/src/mir/lower.rs` -- Known function mappings (request_header, request_body, json_parse, etc.)
- `/Users/sn0w/Documents/dev/snow/mesher/services/writer.mpl` -- StorageWriter service, flush_ticker pattern, retry logic
- `/Users/sn0w/Documents/dev/snow/mesher/storage/queries.mpl` -- get_project_by_api_key, create_api_key
- `/Users/sn0w/Documents/dev/snow/mesher/types/event.mpl` -- EventPayload, Event, Severity, StackFrame structs
- `/Users/sn0w/Documents/dev/snow/tests/e2e/supervisor_basic.mpl` -- Supervisor block syntax
- `/Users/sn0w/Documents/dev/snow/tests/e2e/stdlib_http_middleware.mpl` -- Middleware syntax
- `/Users/sn0w/Documents/dev/snow/tests/e2e/stdlib_http_path_params.mpl` -- HTTP.on_get, HTTP.on_post syntax
- `/Users/sn0w/Documents/dev/snow/tests/e2e/service_call_cast.mpl` -- Service call/cast syntax

### Secondary (MEDIUM confidence)
- [Sentry DSN Authentication](https://docs.sentry.io/api/auth/) -- DSN format and authentication patterns
- [Sentry DSN Explainer](https://docs.sentry.io/concepts/key-terms/dsn-explainer/) -- DSN structure: {PROTOCOL}://{PUBLIC_KEY}@{HOST}/{PROJECT_ID}
- [Rate Limiting Algorithms](https://api7.ai/blog/rate-limiting-guide-algorithms-best-practices) -- Token bucket vs sliding window comparison
- [Redis Rate Limiting](https://oneuptime.com/blog/post/2026-01-21-redis-rate-limiting/view) -- Rate limiting best practices

### Tertiary (LOW confidence)
- WebSocket authentication via upgrade headers -- needs verification against ws/server.rs `perform_upgrade` implementation

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- all components verified against actual Mesh runtime source code
- Architecture: HIGH -- patterns derived from existing working test fixtures and Phase 87 code
- Runtime gaps: HIGH -- verified by reading MeshHttpResponse struct definition (no headers field)
- Pitfalls: HIGH -- identified from direct analysis of runtime code and Mesh language constraints
- Rate limiting approach: MEDIUM -- standard algorithm, but implementation in Mesh's no-mutable-variables model is novel
- WebSocket auth: LOW -- need to verify upgrade request header access in on_connect callback

**Research date:** 2026-02-14
**Valid until:** 2026-03-14 (stable -- Mesh runtime changes are controlled by this project)
