---
title: Web
description: HTTP servers, routing, middleware, WebSocket, and TLS in Mesh
---

# Web

Mesh includes a built-in HTTP server and WebSocket server, so you can build web applications without external dependencies. This guide covers creating HTTP servers with routing and middleware, handling JSON, building real-time WebSocket applications with rooms and broadcasting, and securing connections with TLS.

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
  HTTP.response(200, "{\"status\":\"ok\"}")
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
  HTTP.response(200, "{\"status\":\"ok\"}")
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

Mesh provides a `Json` module for encoding and decoding JSON data. Use `Json.encode` and `Json.parse` for serialization:

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
  HTTP.response(200, "{\"status\":\"ok\"}")
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

Mesh also provides a simple HTTP client for making outbound requests:

```mesh
fn main() do
  let result = HTTP.get("http://example.com")
  case result do
    Ok(body) -> println("ok")
    Err(msg) -> println("error")
  end
end
```

`HTTP.get` returns a `Result` -- `Ok(body)` on success or `Err(message)` on failure.

## What's Next?

- [Databases](/docs/databases/) -- SQLite, PostgreSQL, connection pooling, and struct mapping
- [Concurrency](/docs/concurrency/) -- actors, message passing, and supervision trees
- [Syntax Cheatsheet](/docs/cheatsheet/) -- quick reference for all Mesh syntax
