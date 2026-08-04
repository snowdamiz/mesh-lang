---
title: Web
description: HTTP servers and clients, JSON, WebSocket servers and clients, routing, middleware, TLS, limits, and graceful shutdown in Mesh
---

# Web

Mesh includes HTTP and WebSocket servers and scheduler-aware outbound clients, so you can build web applications without external packages. The uppercase `HTTP` module is the inbound server surface; lowercase `Http` is the outbound client.

> **Autonomous clusters:** This page explains web primitives. Continue with [Autonomous Clusters](/docs/autonomous-clusters/) for adaptive ingress routing and admission control.

## HTTP Server

Create an HTTP server by building a router, adding routes, and starting the server with `HTTP.serve`:

```mesh
fn handler(request) do
  HTTP.response(200, "Hello from Mesh!")
end

fn main() do
  let r = HTTP.router()
  let r = HTTP.route(r, "/", handler)
  HTTP.serve(r, 8080)
end
```

The server listens on the specified port and dispatches incoming requests to the matching handler function. Each handler receives a `Request` and returns a `Response`.

### Creating Responses

Use `HTTP.response` to create a response with a status code and body. Responses default to `application/json; charset=utf-8`, even when the body is plain text. Use `HTTP.response_with_headers` to add headers or override `Content-Type`:

```mesh
fn handler(request) do
  let headers = Map.new()
    |> Map.put("Cache-Control", "no-store")
    |> Map.put("Content-Type", "application/json; charset=utf-8")
  HTTP.response_with_headers(200, json { status: "ok" }, headers)
end
```

For encrypted envelopes and other binary payloads, use `Request.body_bytes(request)` and `HTTP.response_bytes(status, bytes)`. Binary responses default to `application/octet-stream`; use `HTTP.response_bytes_with_headers(status, bytes, headers)` when the response also needs headers such as `Cache-Control: no-store`.

Common status codes: `200` (OK), `201` (Created), `400` (Bad Request), `401` (Unauthorized), `404` (Not Found), `500` (Internal Server Error).

## Routing

### Basic Routes

Use `HTTP.route` to register a handler for a path. The router checks exact paths first, parameterized paths second, and wildcards last. Within each category, the first registered match wins:

```mesh
fn home_handler(request) do
  HTTP.response(200, "home")
end

fn health_handler(request) do
  HTTP.response(200, json { status: "ok" })
end

fn main() do
  let r = HTTP.router()
  let r = HTTP.route(r, "/", home_handler)
  let r = HTTP.route(r, "/health", health_handler)
  HTTP.serve(r, 8080)
end
```

### Method-Specific Routes

Use `HTTP.on_get`, `HTTP.on_post`, `HTTP.on_put`, and `HTTP.on_delete` to match specific HTTP methods:

```mesh
fn me_handler(request) do
  HTTP.response(200, "me")
end

fn user_handler(request) do
  let param = Request.param(request, "id")
  case param do
    Some(id) -> HTTP.response(200, id)
    None -> HTTP.response(400, "no-id")
  end
end

fn post_handler(request) do
  HTTP.response(200, "posted")
end

fn fallback_handler(request) do
  HTTP.response(200, "fallback")
end

fn main() do
  let r = HTTP.router()
  let r = HTTP.on_get(r, "/users/me", me_handler)
  let r = HTTP.on_get(r, "/users/:id", user_handler)
  let r = HTTP.on_post(r, "/data", post_handler)
  let r = HTTP.route(r, "/*", fallback_handler)
  HTTP.serve(r, 8080)
end
```

Route precedence: static paths like `/users/me` are matched before parameterized paths like `/users/:id`. The wildcard `/*` matches any path not matched by other routes.

### Path Parameters

Use `:param` syntax in route paths to capture dynamic segments. Access captured values with `Request.param`:

```mesh
fn user_handler(request) do
  let param = Request.param(request, "id")
  case param do
    Some(id) -> HTTP.response(200, id)
    None -> HTTP.response(400, "missing id")
  end
end

fn main() do
  let r = HTTP.router()
  let r = HTTP.on_get(r, "/users/:id", user_handler)
  HTTP.serve(r, 8080)
end
```

`Request.param` returns an `Option` -- `Some(value)` if the parameter exists, `None` otherwise. Use pattern matching to handle both cases.

### Request Accessors

The `Request` module provides accessors for reading request data:

| Function | Returns | Description |
|----------|---------|-------------|
| `Request.method(request)` | `String` | HTTP method (GET, POST, etc.) |
| `Request.path(request)` | `String` | Request path |
| `Request.body(request)` | `String` | Request body |
| `Request.body_bytes(request)` | `Bytes` | Byte-exact request body |
| `Request.header(request, name)` | `Option<String>` | Header value by name |
| `Request.query(request, name)` | `Option<String>` | Query parameter by name |
| `Request.param(request, name)` | `Option<String>` | Path parameter by name |
| `HTTP.request_id(request)` | `String` | Runtime-generated request correlation ID |
| `HTTP.idempotency_key(request)` | `Option<String>` | Validated idempotency key, when supplied |

The server bounds each request: the request line and complete header section are limited to 8 KiB, at most 100 headers are accepted, and the body is limited to 1 MiB. Requests using `Transfer-Encoding` are rejected; send a bounded `Content-Length`.

`HTTP.idempotency_key` reads the case-insensitive `Idempotency-Key` header. Keys must contain 1–255 visible ASCII bytes with no surrounding whitespace; an invalid key makes the server return `400` before the route handler runs.

### Graceful Shutdown

Call `Process.install_shutdown_signals()` before serving to translate native `SIGINT` and `SIGTERM` into a shutdown request. `HTTP.serve` and `HTTP.serve_tls` then stop accepting new connections and drain connections they already accepted. Application code can trigger the same path with `Process.request_shutdown()`.

## Middleware

Middleware functions wrap request handling with cross-cutting concerns like logging, authentication, or CORS. Add middleware with `HTTP.use`:

```mesh
fn logger(request :: Request, next) -> Response do
  next(request)
end

fn auth_check(request :: Request, next) do
  let path = Request.path(request)
  let is_secret = String.starts_with(path, "/secret")
  if is_secret do
    HTTP.response(401, "Unauthorized")
  else
    next(request)
  end
end

fn handler(request :: Request) do
  HTTP.response(200, "hello-world")
end

fn secret_handler(request :: Request) do
  HTTP.response(200, "secret-data")
end

fn main() do
  let r = HTTP.router()
  let r = HTTP.use(r, logger)
  let r = HTTP.use(r, auth_check)
  let r = HTTP.route(r, "/hello", handler)
  let r = HTTP.route(r, "/secret", secret_handler)
  HTTP.serve(r, 8080)
end
```

### Middleware Signature

A middleware function takes two arguments:

- **`request`** -- the incoming `Request`
- **`next`** -- a continuation function that passes the request to the next middleware or the final handler

Call `next(request)` to continue the chain. Return a `Response` directly (without calling `next`) to short-circuit the chain, as shown in the `auth_check` example above.

Middleware runs in the order added with `HTTP.use`. In the example above, every request passes through `logger` first, then `auth_check`, and finally the matched route handler.

## JSON

### JSON Object Literals

Use `json { }` to construct JSON objects without manual string escaping or heredoc interpolation. The result auto-coerces to `String` and can be passed directly to `HTTP.response`:

```mesh
fn api_handler(request) do
  HTTP.response(200, json { status: "ok", count: 42 })
end

fn error_handler(request) do
  HTTP.response(400, json { error: "bad request" })
end
```

Values are serialized based on their Mesh type: `String` → quoted, `Int`/`Float` → unquoted number, `Bool` → `true`/`false`, `nil` → `null`, `Option<T>` → `null` or value, `List<T>` → array, struct with `deriving(Json)` → nested object. See [JSON Literals](/docs/language-basics/#json-literals) in the Language Basics guide for the full type table.

Nested `json { }` values embed raw — no double-encoding:

```mesh
let inner = json { code: 200 }
let outer = json { result: inner, ok: true }
# outer is: {"result":{"code":200},"ok":true}
```

> **Note:** Keys must be bare identifiers. Reserved keywords (`type`, `fn`, `let`, etc.) cannot be used as keys — use heredoc strings for JSON objects with keyword-named fields.

### Json Module

`Json.parse` creates a structured `Json` value. Navigation and scalar conversion are checked, so a missing field, out-of-range array index, or unexpected type returns `Err`:

```mesh
fn main() do
  case Json.parse("{\"users\":[{\"name\":\"Ada\"}],\"cursor\":null}") do
    Ok(root) ->
      case Json.object_get(root, "users") do
        Ok(users) ->
          case Json.array_get(users, 0) do
            Ok(user) ->
              case Json.object_get(user, "name") do
                Ok(name) -> case Json.as_string(name) do
                  Ok(text) -> println(text)
                  Err(error) -> println(error)
                end
                Err(error) -> println(error)
              end
            Err(error) -> println(error)
          end
        Err(error) -> println(error)
      end
    Err(error) -> println("invalid JSON: #{error}")
  end
end
```

`JSON` is an alias of the same module. `Json.encode` additionally accepts structs with `deriving(Json)`.

| Function | Returns | Description |
|----------|---------|-------------|
| `Json.parse(text)` | `Result<Json, String>` | Parse one JSON value |
| `Json.encode(value)` | `String` | Encode a `Json` value or a value with JSON support |
| `Json.encode_string(value)` | `String` | Encode a JSON string scalar |
| `Json.encode_int(value)` | `String` | Encode an integer scalar |
| `Json.encode_bool(value)` | `String` | Encode a boolean scalar |
| `Json.encode_map(value)` | `String` | Encode a map |
| `Json.encode_list(value)` | `String` | Encode a list |
| `Json.object_get(value, key)` | `Result<Json, String>` | Read an object member |
| `Json.array_get(value, index)` | `Result<Json, String>` | Read a checked zero-based array element |
| `Json.array_length(value)` | `Result<Int, String>` | Read an array's length |
| `Json.is_null(value)` | `Bool` | Test for JSON `null` |
| `Json.as_string(value)` | `Result<String, String>` | Require a JSON string |
| `Json.as_int(value)` | `Result<Int, String>` | Require an integral JSON number in `Int` range |
| `Json.as_float(value)` | `Result<Float, String>` | Require a JSON number |
| `Json.as_bool(value)` | `Result<Bool, String>` | Require a JSON boolean |

The older `Json.get(json_text, key)`, `Json.get_nested(json_text, first, second)`, and `Json.is_string(json_text, key)` helpers operate on JSON text and return lossy scalar strings. Prefer `Json.parse` plus the checked structured accessors for new code.

### Struct Serialization with deriving(Json)

Structs that derive `Json` get automatic `to_json` and `from_json` methods:

```mesh
struct User do
  name :: String
  age :: Int
  active :: Bool
end deriving(Json)

fn main() do
  # Encode to JSON string
  let user = User { name: "Alice", age: 30, active: true }
  let json_str = Json.encode(user)
  println(json_str)

  # Decode from JSON string
  let result = User.from_json("{\"name\":\"Bob\",\"age\":25,\"active\":false}")
  case result do
    Ok(u) -> println("${u.name}")
    Err(e) -> println("Error: ${e}")
  end
end
```

For HTTP handlers, combine JSON encoding with `HTTP.response` to return JSON responses:

```mesh
fn api_handler(request) do
  let body = Request.body(request)
  # Process the JSON body...
  HTTP.response(200, json { status: "ok" })
end
```

## WebSocket

Mesh includes a built-in WebSocket server for real-time bidirectional communication. Create a WebSocket server with `Ws.serve`, providing three lifecycle callbacks:

```mesh
fn on_connect(conn, path, headers) do
  println("connected on #{path}")
  let _ = Ws.send(conn, "Welcome!")
  1
end

fn on_message(conn, msg) do
  let _ = Ws.send(conn, msg)
end

fn on_close(conn, code, reason) do
  println("client disconnected: #{code} #{reason}")
end

fn main() do
  Ws.serve(on_connect, on_message, on_close, 9001)
end
```

### Lifecycle Callbacks

| Callback | Arguments | Purpose |
|----------|-----------|---------|
| `on_connect` | `(conn, path, headers)` | Called after the upgrade with the request path and `Map<String, String>` headers; return nonzero to accept or `0` to reject with close code 1008 |
| `on_message` | `(conn, msg)` | Called for each message from the client. |
| `on_close` | `(conn, code, reason)` | Called with the close code and reason. Cleanup is automatic. |

Each WebSocket connection runs as an isolated actor. If a handler crashes, only that connection is affected -- the server continues accepting new connections.

`Ws.serve` starts its accept loop on a background thread and returns after binding the port. Keep the native process alive with your HTTP server or other application work.

### Sending Messages

Use `Ws.send` to send a text message to a specific connection:

```mesh
fn on_message(conn, msg) do
  let status = Ws.send(conn, "Echo: " <> msg)
  if status != 0 do
    println("send failed")
  end
end
```

### Rooms and Broadcasting

Rooms provide pub/sub messaging. Connections can join named rooms and broadcast messages to all room members:

```mesh
fn on_connect(conn, path, headers) do
  let _ = Ws.join(conn, "lobby")
  let _ = Ws.send(conn, "Welcome to the lobby!")
  1
end

fn on_message(conn, msg) do
  # Broadcast to all connections in the room
  let _ = Ws.broadcast("lobby", msg)
end

fn on_close(conn, code, reason) do
  # Room membership is automatically cleaned up on disconnect
  println("client left")
end

fn main() do
  Ws.serve(on_connect, on_message, on_close, 9001)
end
```

| Function | Returns | Description |
|----------|---------|-------------|
| `Ws.send(conn, message)` | `Int` | Send text; `0` indicates success |
| `Ws.join(conn, room)` | `Int` | Subscribe a connection to a named room |
| `Ws.leave(conn, room)` | `Int` | Unsubscribe a connection from a room |
| `Ws.broadcast(room, msg)` | `Int` | Send to every room member; returns the local write-failure count |
| `Ws.broadcast_except(room, msg, conn)` | `Int` | Send to all room members except one local connection |

Room membership is automatically cleaned up when a connection disconnects -- you do not need to manually call `Ws.leave` in the `on_close` callback.

In a distributed cluster, `Ws.broadcast` automatically forwards messages to room members on other nodes.

## WebSocket Client

`WsClient` is the outbound client surface. It accepts `ws://` and certificate-validated `wss://` URLs, yields while connecting or waiting for a message, and bounds both message size and the inbound queue.

```mesh
fn exchange(connection :: Int) -> Int!String do
  WsClient.send_text(connection, "subscribe")?
  (("0102" |> Bytes.from_hex())? |2> WsClient.send_bytes(connection))?

  let message = WsClient.recv(connection, 5_000)?
  if message.kind == "text" do
    case Bytes.to_utf8(message.data) do
      Ok(text) -> println(text)
      Err(error) -> println(error)
    end
  else if message.kind == "binary" do
    println(Bytes.to_hex(message.data))
  else
    println("closed: #{message.close_code}:#{message.close_reason}")
  end

  WsClient.close(connection, 1_000, "done")?
  Ok(0)
end

fn main() do
  let options = WsClient.options()
    |> WsClient.connect_timeout(5_000)
    |> WsClient.heartbeat_timeout(30_000)
    |> WsClient.max_message_bytes(1_048_576)
    |> WsClient.queue_capacity(256)

  case WsClient.connect("wss://example.com/feed", options) do
    Ok(connection) -> case exchange(connection) do
      Ok(_) -> println("done")
      Err(error) -> println(error)
    end
    Err(error) -> println(error)
  end
end
```

| Function | Description |
|----------|-------------|
| `WsClient.options()` | Create a single-use options handle |
| `WsClient.connect_timeout(options, ms)` | Set the DNS/TCP/TLS/upgrade timeout |
| `WsClient.heartbeat_timeout(options, ms)` | Set the ping/pong liveness timeout |
| `WsClient.max_message_bytes(options, bytes)` | Bound inbound and outbound text or binary messages, including fragments |
| `WsClient.queue_capacity(options, messages)` | Bound unread inbound messages |
| `WsClient.connect(url, options)` | Connect and consume the options handle |
| `WsClient.send_text(connection, text)` | Send a masked text frame |
| `WsClient.send_bytes(connection, bytes)` | Send a masked binary frame |
| `WsClient.recv(connection, timeout_ms)` | Receive a `WsMessage`; only one receiver may wait per connection |
| `WsClient.close(connection, code, reason)` | Send a close frame and release the handle |
| `WsClient.reconnect_delay(attempt, base_ms, max_ms, jitter_ppm)` | Return bounded exponential backoff with jitter |

Options default to a 10-second connect timeout, 30-second heartbeat timeout, 1 MiB message limit, and 256-message queue. Connect timeouts must be 1–120,000 ms, heartbeat timeouts 1,000–300,000 ms, message limits 1 byte–16 MiB, and queue capacities 1–65,536. Setters retain the fluent handle; `connect` performs validation and consumes it.

`WsMessage.kind` is `"text"`, `"binary"`, or `"close"`; payload bytes are in `data`, and close details are in `close_code` and `close_reason`. Queue overflow closes the connection with a `BACKPRESSURE` error rather than dropping data silently. Heartbeat timeout and other disconnects are observable errors.

Reconnect is deliberately explicit. After an interruption, the caller chooses whether to reconnect, uses `reconnect_delay`, restores subscriptions, and checks source sequence numbers for regressions. The helper accepts attempts `0..62`, positive `base_ms <= max_ms`, and jitter from `0` to `1_000_000` parts per million. The runtime never restores subscriptions or retries writes implicitly.

## TLS

`HTTP.serve_tls` serves HTTPS with a PEM certificate and private key:

```mesh
fn handler(request) do
  HTTP.response(200, "Secure hello!")
end

fn main() do
  let r = HTTP.router()
  let r = HTTP.route(r, "/", handler)
  HTTP.serve_tls(r, 8443, "cert.pem", "key.pem")
end
```

The server performs TLS negotiation with rustls. The current inbound `Ws` module exposes plain `Ws.serve`; terminate WSS at a trusted reverse proxy. The outbound `WsClient` supports certificate-validated `wss://` connections directly.

## HTTP Client

Mesh provides a fluent builder API for making outbound HTTP requests via the `Http` module (note: lowercase `Http`, distinct from the `HTTP` server module).

### Fluent Builder

Builder functions return the same single-use request handle, so pipe them:

```mesh
fn main() do
  let request = Http.build(:get, "https://api.example.com/data")
    |> Http.header("Authorization", "Bearer token")
    |> Http.query("market", "SOL/USDC")
    |> Http.timeout(30_000)
    |> Http.stage_timeout(:resolve, 2_000)
    |> Http.stage_timeout(:connect, 5_000)
    |> Http.stage_timeout(:send, 5_000)
    |> Http.stage_timeout(:first_byte, 10_000)
    |> Http.stage_timeout(:body, 10_000)
    |> Http.max_redirects(5)
    |> Http.max_response_bytes(1_048_576)

  case Http.send(request) do
    Ok(response) -> println("#{response.status}: #{response.body}")
    Err(error) -> println("error: #{error}")
  end
end
```

| Function | Description |
|----------|-------------|
| `Http.build(method, url)` | Create a request for `:get`, `:head`, `:post`, `:put`, `:patch`, `:delete`, or `:options` |
| `Http.header(req, key, value)` | Add a request header |
| `Http.query(req, key, value)` | Add a percent-encoded query parameter |
| `Http.body(req, value)` | Set a POST, PUT, or PATCH body |
| `Http.body_bytes(req, value)` | Set a byte-exact POST, PUT, or PATCH body |
| `Http.json(req, value)` | Set a body and the JSON content type |
| `Http.timeout(req, ms)` | Set the total timeout |
| `Http.stage_timeout(req, stage, ms)` | Set `:resolve`, `:connect`, `:send`, `:first_byte`, or `:body` timeout |
| `Http.max_redirects(req, count)` | Set the request-local redirect limit; `0` disables redirects |
| `Http.max_response_bytes(req, bytes)` | Set the buffered or streamed response limit |
| `Http.send(req)` | Yield while executing; return `Result<HttpResponse, String>` |

`HttpResponse` has `status`, `body`, `body_bytes`, and `headers` fields. `body_bytes` always preserves the exact response bytes; `body` is empty when the payload is not valid UTF-8. HTTP status errors such as 404 are successful protocol responses and therefore return `Ok`; inspect `status`. Network, timeout, and body-limit failures return `Err`.

Timeouts must be between 1 and 120,000 milliseconds. Requests follow the HTTP client's default of up to 10 redirects unless `Http.max_redirects` sets a request-local limit from 0 through 20. Responses default to an 8 MiB limit and cannot be configured above 64 MiB.

### POST Requests

```mesh
fn main() do
  let request = Http.build(:post, "https://api.example.com/items")
    |> Http.json(json { name: "widget", price: 9 })
    |> Http.max_response_bytes(65_536)

  case Http.send(request) do
    Ok(response) -> println("created: #{response.body}")
    Err(error) -> println("error: #{error}")
  end
end
```

### Streaming

`Http.stream` emits valid UTF-8 `String` chunks. `Http.stream_bytes` emits binary-safe `Bytes`. Each callback completes before the next bounded 8 KiB chunk is read, providing backpressure.

```mesh
fn main() do
  let handle = Http.build(:get, "https://example.com/large-file")
    |> Http.max_response_bytes(8_388_608)
    |> Http.stream_bytes(fn chunk do
      println(Bytes.to_hex(chunk))
      "continue"
    end)

  Timer.sleep(100)
  Http.cancel(handle)
end
```

Return `"stop"` from a callback to end the stream, or pass the returned handle to `Http.cancel`. Streaming begins on a dedicated I/O thread and does not block a scheduler worker. The callback API has no error channel; a setup or read failure ends the stream and is visible through `Http.metrics`.

### Cancellation

The request handle is also its cancellation handle. Retain a copy and send it to another actor before calling `Http.send` or `Http.send_with`; that actor can call `Http.cancel(request)` while the caller is suspended. Cancellation wakes the caller immediately. The bounded stage timeout still limits any underlying OS I/O that is already in progress.

Calling `Http.cancel` or `Http.client_close` more than once is safe.

### Keep-Alive Client

Reuse one connection pool across requests to the same origin:

```mesh
fn fetch(client :: Int, url :: String) do
  case Http.build(:get, url) |2> Http.send_with(client) do
    Ok(response) -> println(response.body)
    Err(error) -> println(error)
  end
end

fn main() do
  let client = Http.client()
  fetch(client, "https://api.example.com/data")
  fetch(client, "https://api.example.com/health")
  Http.client_close(client)
end
```

| Function | Description |
|----------|-------------|
| `Http.client()` | Create a keep-alive HTTP client handle |
| `Http.send_with(client, req)` | Send request reusing the client's connection pool |
| `Http.stream(req, callback)` | Stream UTF-8 text with callback backpressure |
| `Http.stream_bytes(req, callback)` | Stream `Bytes` with callback backpressure |
| `Http.cancel(handle)` | Cancel a pending/active request or stream |
| `Http.client_close(client)` | Close the client and release connections |

### Retries and Metrics

Mesh never retries HTTP requests automatically. `Http.retry_class(method, error)` returns `"safe_retry"` for transient GET, HEAD, and OPTIONS failures, `"unsafe_retry"` for transient writes, and `"do_not_retry"` for permanent failures. The caller owns retry count, backoff, and idempotency policy.

`Http.metrics()` returns cumulative process-wide metrics:

| Field | Meaning |
|-------|---------|
| `requests`, `in_flight`, `cancellations` | Request count, current gauge, and cancellation count |
| `dns_micros`, `connect_micros`, `tls_micros` | Time spent creating new connections; pooled requests do not repeat these stages |
| `first_byte_micros`, `total_micros` | Cumulative time to first byte and total request time |
| `dns_failures`, `connect_failures`, `tls_failures`, `timeouts` | Classified failure counts |
| `response_bytes` | Successfully read or streamed bytes |

## What's Next?

- [Databases](/docs/databases/) -- SQLite, PostgreSQL, connection pooling, and struct mapping
- [Concurrency](/docs/concurrency/) -- actors, message passing, and supervision trees
- [Syntax Cheatsheet](/docs/cheatsheet/) -- quick reference for all Mesh syntax
