---
title: Web
description: HTTP servers, routing, middleware, WebSocket, and TLS in Mesh
---

# Web

Mesh includes a built-in HTTP server and WebSocket server, so you can build web applications without external dependencies. This guide covers creating HTTP servers with routing and middleware, handling JSON, building real-time WebSocket applications with rooms and broadcasting, and securing connections with TLS.

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

Use `HTTP.response` to create a response with a status code and body:

```mesh
fn handler(request) do
  HTTP.response(200, json { status: "ok" })
end
```

Common status codes: `200` (OK), `201` (Created), `400` (Bad Request), `401` (Unauthorized), `404` (Not Found), `500` (Internal Server Error).

## Routing

### Basic Routes

Use `HTTP.route` to register a handler for a path. Routes are matched in the order they are added:

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
| `Request.header(request, name)` | `Option` | Header value by name |
| `Request.query(request, name)` | `Option` | Query parameter by name |
| `Request.param(request, name)` | `Option` | Path parameter by name |

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

Mesh also provides a `Json` module for encoding and decoding JSON data. Use `Json.encode` and `Json.parse` for serialization:

```mesh
fn main() do
  # Encode a map to a JSON string
  let m = Map.new()
  let m = Map.put(m, "name", "Alice")
  let m = Map.put(m, "age", "30")
  let json_str = Json.encode(m)
  println(json_str)

  # Parse a JSON string
  let result = Json.parse("{\"key\": \"value\"}")
  case result do
    Ok(data) -> println("parsed")
    Err(msg) -> println("error: ${msg}")
  end
end
```

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
# Derived from runtime API
fn on_connect(conn) do
  Ws.send(conn, "Welcome!")
end

fn on_message(conn, msg) do
  Ws.send(conn, msg)
end

fn on_close(conn) do
  println("client disconnected")
end

fn main() do
  Ws.serve(on_connect, on_message, on_close, 9001)
end
```

### Lifecycle Callbacks

| Callback | Arguments | Purpose |
|----------|-----------|---------|
| `on_connect` | `(conn)` | Called when a client connects. Use `conn` to send messages or join rooms. |
| `on_message` | `(conn, msg)` | Called for each message from the client. |
| `on_close` | `(conn)` | Called when the client disconnects. Cleanup is automatic. |

Each WebSocket connection runs as an isolated actor. If a handler crashes, only that connection is affected -- the server continues accepting new connections.

### Sending Messages

Use `Ws.send` to send a text message to a specific connection:

```mesh
# Derived from runtime API
fn on_message(conn, msg) do
  Ws.send(conn, "Echo: " <> msg)
end
```

### Rooms and Broadcasting

Rooms provide pub/sub messaging. Connections can join named rooms and broadcast messages to all room members:

```mesh
# Derived from runtime API
fn on_connect(conn) do
  Ws.join(conn, "lobby")
  Ws.send(conn, "Welcome to the lobby!")
end

fn on_message(conn, msg) do
  # Broadcast to all connections in the room
  Ws.broadcast("lobby", msg)
end

fn on_close(conn) do
  # Room membership is automatically cleaned up on disconnect
  println("client left")
end

fn main() do
  Ws.serve(on_connect, on_message, on_close, 9001)
end
```

| Function | Description |
|----------|-------------|
| `Ws.join(conn, room)` | Subscribe a connection to a named room |
| `Ws.leave(conn, room)` | Unsubscribe a connection from a room |
| `Ws.broadcast(room, msg)` | Send a message to all connections in a room |
| `Ws.broadcast_except(room, msg, conn)` | Send to all in a room except one connection |

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

`WsMessage.kind` is `"text"`, `"binary"`, or `"close"`; payload bytes are in `data`, and close details are in `close_code` and `close_reason`. Queue overflow closes the connection with a `BACKPRESSURE` error rather than dropping data silently. Heartbeat timeout and other disconnects are observable errors.

Reconnect is deliberately explicit. After an interruption, the caller chooses whether to reconnect, uses `reconnect_delay`, restores subscriptions, and checks source sequence numbers for regressions. The runtime never restores subscriptions or retries writes implicitly.

## TLS

Both the HTTP and WebSocket servers support TLS for encrypted connections. Provide paths to a PEM certificate and private key file:

### HTTPS

```mesh
# Derived from runtime API
fn handler(request) do
  HTTP.response(200, "Secure hello!")
end

fn main() do
  let r = HTTP.router()
  let r = HTTP.route(r, "/", handler)
  HTTP.serve_tls(r, 8443, "cert.pem", "key.pem")
end
```

### Secure WebSocket (WSS)

```mesh
# Derived from runtime API
fn on_connect(conn) do
  Ws.send(conn, "Secure connection!")
end

fn on_message(conn, msg) do
  Ws.send(conn, msg)
end

fn on_close(conn) do
  println("disconnected")
end

fn main() do
  Ws.serve_tls(on_connect, on_message, on_close, 9443, "cert.pem", "key.pem")
end
```

The TLS functions are identical to their non-TLS counterparts, with two additional arguments for the certificate and key file paths. The server handles TLS negotiation automatically using rustls.

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
| `Http.json(req, value)` | Set a body and the JSON content type |
| `Http.timeout(req, ms)` | Set the total timeout |
| `Http.stage_timeout(req, stage, ms)` | Set `:resolve`, `:connect`, `:send`, `:first_byte`, or `:body` timeout |
| `Http.max_response_bytes(req, bytes)` | Set the buffered or streamed response limit |
| `Http.send(req)` | Yield while executing; return `Result<HttpResponse, String>` |

`HttpResponse` has `status`, `body`, and `headers` fields. HTTP status errors such as 404 are successful protocol responses and therefore return `Ok`; inspect `status`. Network, timeout, invalid UTF-8, and body-limit failures return `Err`.

Timeouts must be between 1 and 120,000 milliseconds. Responses default to an 8 MiB limit and cannot be configured above 64 MiB.

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
  end)
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
