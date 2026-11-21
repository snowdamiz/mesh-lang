# Mesh HTTP and WebSockets

`HTTP` serves inbound HTTP. `Http` sends outbound HTTP requests. `Ws` and `WsClient` serve and connect to WebSockets, respectively.

## HTTP routing and middleware

`HTTP.route(router, path, handler)` matches any method. `HTTP.on_get`, `HTTP.on_post`, `HTTP.on_put`, and `HTTP.on_delete` match one method. Router functions return the router and compose with `|>`. Exact paths take precedence over parameterized paths and wildcards. Within each category, the first registered match wins. [Routing and path parameters](https://meshlang.dev/docs/web/#routing)

A handler receives `Request` and returns `Response`. Middleware registered with `HTTP.use(router, middleware)` receives the request and a continuation. Call `next(request)` to continue, or return a response to stop the chain. Middleware runs in registration order. [Middleware](https://meshlang.dev/docs/web/#middleware)

```mesh
fn users(request :: Request) -> Response do
	case Request.param(request, "id") do
		Some(id) -> HTTP.response(200, json { id: id })
		None -> HTTP.response(400, json { error: "missing id" })
	end
end

fn check_token(request :: Request, next) -> Response do
	case Request.header(request, "X-Demo-Token") do
		Some("demo") -> next(request)
		_ -> HTTP.response(401, json { error: "missing or invalid token" })
	end
end

fn main() do
	Process.install_shutdown_signals()
	let router = HTTP.router()
		|> HTTP.use(check_token)
		|> HTTP.on_get("/users/:id", users)
	HTTP.serve(router, 8080)
end
```

## Requests, responses, and server lifecycle

| API | Behavior |
| --- | --- |
| `Request.method`, `Request.path`, `Request.body` | Return strings. Use `Request.body_bytes` for the exact request bytes. |
| `Request.header`, `Request.query`, `Request.param` | Return `Option<String>` for a header, query parameter, or path parameter. Header lookup ignores case. |
| `HTTP.request_id`, `HTTP.idempotency_key` | Return a correlation ID as `String` and a validated caller key as `Option<String>`, respectively. |
| `HTTP.response(status, body)` | Build a text response with default content type `application/json; charset=utf-8`. A `json { ... }` value in a string argument supplies its JSON text. |
| `HTTP.response_bytes(status, bytes)` | Build a byte-exact response with default content type `application/octet-stream`. |
| `HTTP.response_with_headers`, `HTTP.response_bytes_with_headers` | Take a third argument of `Map<String, String>` for custom headers, including a content type override. |

The server limits the request line and headers to 8 KiB each, accepts at most 100 headers, and limits the body to 1 MiB. Send a bounded `Content-Length` for request bodies. Requests with `Transfer-Encoding` are rejected. Invalid idempotency keys return `400` before handler dispatch. [Request accessors and limits](https://meshlang.dev/docs/web/#request-accessors) · [Response constructors](https://meshlang.dev/docs/web/#creating-responses)

`HTTP.serve(router, port)` blocks while serving. `HTTP.serve_tls(router, port, cert_path, key_path)` uses PEM files. A handler panic returns `500` and leaves the server available. After `Process.install_shutdown_signals()`, `SIGINT` and `SIGTERM` stop new accepts and drain accepted connections. `Process.request_shutdown()` triggers the same path. [TLS](https://meshlang.dev/docs/web/#tls) · [Graceful shutdown](https://meshlang.dev/docs/web/#graceful-shutdown)

## Clustered routes

Put `HTTP.clustered(handler)` or `HTTP.clustered(N, handler)` directly in the handler argument of `HTTP.route` or `HTTP.on_*`. The handler is a named public function taking `Request` and returning `Response`. The count is a positive integer literal and includes the executor's record. The omitted count uses `[cluster].default_replicas`, or 2 without one. A handler cannot be registered with conflicting counts. Failed placement returns `503`. For routes that can replay writes, use a validated caller `Idempotency-Key`. [Clustered routes and response headers](https://meshlang.dev/docs/web/#clustered-routes) · [Bootstrap and replay rules](clustering.md#clustered-routes-and-continuity)

## Outbound HTTP

`Http.build(method, url)` creates a single-use request handle. Methods include `:get`, `:head`, `:post`, `:put`, `:patch`, `:delete`, and `:options`. Set headers and query parameters with `Http.header` and `Http.query`. Set a request body with `Http.body`, `Http.body_bytes`, or `Http.json`. Bound requests with `Http.timeout`, `Http.stage_timeout`, `Http.max_redirects`, and `Http.max_response_bytes`. A redirect limit of `0` disables redirects. [Fluent builder and limits](https://meshlang.dev/docs/web/#fluent-builder)

```mesh
fn main() do
	let request = Http.build(:get, "http://127.0.0.1:8080/users/42")
		|> Http.header("X-Demo-Token", "demo")
		|> Http.timeout(5_000)
		|> Http.max_redirects(0)
	case Http.send(request) do
		Ok(response) -> println("#{response.status}: #{response.body}")
		Err(error) -> println(error)
	end
end
```

`Http.send` and `Http.send_with(client, request)` yield and return `Result<HttpResponse, String>`. The response has `status`, `headers`, `body`, and `body_bytes`. HTTP error statuses such as `404` arrive as `Ok(response)`. Transport failures, timeouts, and size-limit violations return `Err`. The text body is empty if the bytes are invalid UTF-8. Open a reusable connection pool with `Http.client()` and release it with `Http.client_close(client)`. [Response contract](https://meshlang.dev/docs/web/#fluent-builder) · [Keep-alive client](https://meshlang.dev/docs/web/#keep-alive-client)

`Http.stream(request, callback)` provides UTF-8 chunks. `Http.stream_bytes` provides `Bytes`. The I/O thread waits for each callback before reading more. Return `"stop"` to end the stream or `"continue"` to keep reading. `Http.cancel(handle)` cancels a pending or active request or stream. Streaming failures end the stream and can appear in process-wide `Http.metrics()`. Use `Http.retry_class(method, error)` when implementing your own retry policy. [Streaming](https://meshlang.dev/docs/web/#streaming) · [Cancellation](https://meshlang.dev/docs/web/#cancellation) · [Retries and metrics](https://meshlang.dev/docs/web/#retries-and-metrics)

## WebSockets

`Ws.serve(on_connect, on_message, on_close, port)` starts a separate listener and returns after binding. Keep the process alive with other application work. `Ws.serve_tls` takes additional PEM certificate and key paths. Each connection has an isolated actor.

| Callback | Contract |
| --- | --- |
| `on_connect(conn, path, headers)` | Runs after upgrade. Return a nonzero integer to accept, or `0` to close with code 1008. |
| `on_message(conn, message)` | Handles a message. Use `Ws.send(conn, text)` to reply. |
| `on_close(conn, code, reason)` | Receives the close code and reason. Room membership is removed automatically. |

`Ws.join` and `Ws.leave` manage room membership. `Ws.broadcast` and `Ws.broadcast_except` notify room members across connected nodes and return a count of local write failures. Applications that need durable delivery must handle catch-up. [WebSocket server](https://meshlang.dev/docs/web/#websocket) · [Rooms and broadcasting](https://meshlang.dev/docs/web/#rooms-and-broadcasting)

`WsClient.connect(url, options)` opens `ws://` or certificate-validated `wss://` and consumes the handle from `WsClient.options()`. Configure it with `WsClient.connect_timeout`, `WsClient.heartbeat_timeout`, `WsClient.queue_capacity`, and `WsClient.max_message_bytes` before connecting. `WsClient.send_text` and `WsClient.send_bytes` send frames. `WsClient.recv(conn, timeout_ms)` returns `Result<WsMessage, String>` with a `text`, `binary`, or `close` kind. One receiver may wait per connection. Queue overflow closes the connection with a backpressure error. `WsClient.close` releases the handle. Use `WsClient.reconnect_delay` for backoff and restore subscriptions after reconnecting. [WebSocket client API and limits](https://meshlang.dev/docs/web/#websocket-client)
