# Phase 15: HTTP Actor Model - Research

**Researched:** 2026-02-07
**Domain:** Runtime internals -- bridging HTTP accept loop with actor scheduler
**Confidence:** HIGH

## Summary

Phase 15 replaces the `std::thread::spawn` per-connection model in the HTTP server with the existing lightweight actor system (corosensei coroutines on M:N scheduler). The codebase already has all the infrastructure needed: `snow_actor_spawn`, supervisor support with `trap_exit` and linking, and a working HTTP server with `tiny_http`. The change is localized to one function (`snow_http_serve` in `crates/snow-rt/src/http/server.rs`) and the `handle_request` helper.

The key challenge is that `tiny_http::Request` contains blocking I/O (reading body, writing response) which will block the coroutine's OS worker thread. This is the exact issue noted in the existing code comments. The solution is to keep `tiny_http` for socket accept/HTTP parsing but move the request/response handling into actors that wrap the blocking I/O in a way that cooperates with the scheduler. Since each worker thread runs multiple coroutines, a blocking I/O call in one coroutine blocks all other coroutines on that worker. The pragmatic approach is to accept this limitation (identical to BEAM's NIF behavior for short blocking operations) and ensure connection handlers are short-lived.

**Primary recommendation:** Replace `std::thread::spawn` with `snow_actor_spawn` in `snow_http_serve`, wrapping `handle_request` as an actor entry function. Add a supervisor for connection actors with `one_for_one` strategy so a crash in one handler does not propagate. Keep `tiny_http` as-is -- do not change HTTP libraries.

## Standard Stack

### Core
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| tiny_http | 0.12 | HTTP server (accept loop, parsing) | Already in use, works well, no reason to change |
| corosensei | 0.3 | Stackful coroutines for actors | Already in use, core of actor system |
| crossbeam-deque | 0.8 | Work-stealing scheduler queues | Already in use |
| parking_lot | 0.12 | Fast Mutex/RwLock | Already in use throughout actor system |

### Supporting
| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| ureq | 2 | HTTP client | Already in use for HTTP.get/HTTP.post -- unchanged by this phase |

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| tiny_http | mio/tokio | Async I/O would avoid coroutine thread-blocking, but requires rewriting the entire HTTP stack AND the actor system to be async-aware. Massive scope creep for Phase 15. Not recommended. |
| tiny_http | hyper | Same async problem. Also much more complex API. |
| Wrapping blocking I/O | Non-blocking I/O via polling | Would require custom HTTP parser and event loop integration. Not justified for Phase 15. |

## Architecture Patterns

### Current Architecture (What Exists)

```
snow_http_serve(router, port)
  |
  |-- tiny_http::Server::http(addr)
  |-- for request in server.incoming_requests()
  |     |-- std::thread::spawn(move || handle_request(router, request))
  |
  (blocks forever)
```

Key files:
- `/Users/sn0w/Documents/dev/snow/crates/snow-rt/src/http/server.rs` -- the HTTP server runtime
- `/Users/sn0w/Documents/dev/snow/crates/snow-rt/src/http/router.rs` -- URL pattern matching
- `/Users/sn0w/Documents/dev/snow/crates/snow-rt/src/actor/mod.rs` -- actor ABI (spawn, send, receive)
- `/Users/sn0w/Documents/dev/snow/crates/snow-rt/src/actor/scheduler.rs` -- M:N work-stealing scheduler
- `/Users/sn0w/Documents/dev/snow/crates/snow-rt/src/actor/supervisor.rs` -- supervision trees

### Target Architecture (What We Build)

```
snow_http_serve(router, port)
  |
  |-- tiny_http::Server::http(addr)
  |-- Create http_connection_supervisor (one_for_one, transient children)
  |-- Accept loop runs on a dedicated OS thread (NOT an actor)
  |     |-- for request in server.incoming_requests()
  |     |     |-- Serialize request data (method, path, body, headers, query)
  |     |     |-- snow_actor_spawn(connection_handler_entry, serialized_data, ...)
  |     |     |-- Link spawned actor to supervisor
  |     |
  |     (blocks forever on this dedicated thread)
```

### Pattern 1: Actor-per-connection via snow_actor_spawn

**What:** Each incoming HTTP connection spawns a lightweight actor instead of an OS thread.

**When to use:** This is the core pattern for Phase 15.

**Implementation approach:**

The accept loop in `snow_http_serve` currently runs `for request in server.incoming_requests()` on the calling thread and spawns OS threads. The new approach:

1. The accept loop stays on its own OS thread (it must block on `incoming_requests()` which is a synchronous iterator over a TCP listener).
2. For each request, serialize the essential request data into a buffer (method, path, body, headers, query params).
3. Also pass the `tiny_http::Request` handle somehow so the actor can call `request.respond()`.
4. Call `snow_actor_spawn(handler_entry, args_ptr, args_size, priority)` to create a lightweight actor.

**Critical design decision: How to pass `tiny_http::Request` to the actor.**

`tiny_http::Request` is `!Send` (it contains a TCP stream handle). The current thread-spawn approach works because `move ||` closure captures the request. But `snow_actor_spawn` takes raw `*const u8` args and the actor may run on a different worker thread.

**Solution:** Use `Box::into_raw` to convert the `tiny_http::Request` into a raw pointer, pass it as the args to the spawn request. The worker thread that picks up the spawn request will own the request. This is safe because:
- The `SpawnRequest` is already `unsafe impl Send` (the scheduler already does this for fn_ptr and args_ptr).
- The request pointer is only used by a single actor -- no concurrent access.
- The actor's entry function takes ownership via `Box::from_raw`.

```rust
// In snow_http_serve accept loop:
for request in server.incoming_requests() {
    let boxed_req = Box::new(request);
    let req_ptr = Box::into_raw(boxed_req) as *const u8;
    let args = ConnectionArgs { router_addr, req_ptr };
    let boxed_args = Box::new(args);
    let args_ptr = Box::into_raw(boxed_args) as *const u8;
    let args_size = std::mem::size_of::<ConnectionArgs>() as u64;

    // Spawn actor via the global scheduler
    let sched = global_scheduler();
    let pid = sched.spawn(
        connection_handler_entry as *const u8,
        args_ptr,
        args_size,
        1, // Normal priority
    );
    // Link to supervisor for crash isolation
}
```

### Pattern 2: Supervisor for Connection Actors (Crash Isolation)

**What:** A supervisor watches all connection handler actors. If one crashes (panic, bad handler code), the supervisor handles the exit signal without affecting other connections.

**When to use:** Required by HTTP-02 (crash isolation per connection).

**Implementation approach:**

Option A: **Full supervisor via `snow_supervisor_start`** -- Create a `simple_one_for_one` supervisor that spawns connection actors as dynamic children. This provides automatic restart (not needed for HTTP connections) and crash isolation.

Option B: **Lightweight trap_exit actor** -- Create a dedicated "HTTP supervisor" actor that sets `trap_exit = true` and links to each connection actor. When a connection actor crashes, the supervisor receives an exit signal as a message (instead of crashing itself). This is simpler and avoids the complexity of the full supervisor config serialization.

**Recommendation: Option B** -- HTTP connections are transient and should NOT be restarted. A full supervisor with `Transient` restart type would work but is overkill. A lightweight trap_exit actor that simply logs crashes is cleaner.

Actually, even simpler: **Option C: Just don't link connection actors to anything.** If a connection actor panics, `std::panic::catch_unwind` in the coroutine runner already prevents it from crashing the scheduler. The `handle_process_exit` function in the scheduler handles cleanup. The only concern is whether the panic propagates to linked actors -- but if the connection actors are not linked to anything, they are fully isolated by default.

**Revised recommendation: Option C with catch_unwind wrapping.**

The scheduler's `worker_loop` already handles actor completion via `handle_process_exit`. If an actor's entry function panics, the coroutine's catch_unwind (if present) prevents it from unwinding through the scheduler. Looking at the code, the coroutine entry function does NOT have a catch_unwind wrapper -- a panic would unwind through corosensei, which would be undefined behavior.

**Therefore: The connection handler entry function MUST wrap the handler call in `std::panic::catch_unwind`.** This is the crash isolation mechanism. If the handler panics, the catch_unwind catches it, the actor completes normally (with an error logged), and other connections are unaffected.

For the supervision requirement (HTTP-02), the simplest correct approach:
1. Wrap the connection handler in `catch_unwind` -- this provides crash isolation per connection.
2. Optionally link connection actors to a supervisor that uses `trap_exit` -- this provides observability and proper exit propagation.

### Pattern 3: Backward-Compatible API

**What:** The `snow_http_serve(router, port)` ABI signature does not change. Snow programs that call `HTTP.serve(r, 18080)` continue to work identically.

**When to use:** Required by success criterion 3.

**Implementation:** The change is entirely internal to the runtime. The `snow_http_serve` function signature stays the same. The router pointer handling stays the same. The `handle_request` logic stays the same. Only the dispatch mechanism changes from `std::thread::spawn` to `snow_actor_spawn`.

### Recommended Project Structure Changes

```
crates/snow-rt/src/http/
  server.rs     # MODIFY: Replace thread::spawn with actor spawn, add connection_handler_entry
  mod.rs        # MODIFY: Update architecture comment
  router.rs     # UNCHANGED
  client.rs     # UNCHANGED
```

### Anti-Patterns to Avoid

- **Making the accept loop an actor:** The `incoming_requests()` iterator blocks forever on TCP accept. If this were an actor, it would block the worker thread and starve other actors on that worker. The accept loop MUST stay on a dedicated OS thread.
- **Using async/await or tokio:** This phase is about using the EXISTING actor system, not building an async runtime. tiny_http's synchronous API is fine.
- **Restarting crashed HTTP connections:** Unlike service actors, HTTP connections are fire-and-forget. A crashed connection should return a 500 error (if possible) or simply close the TCP connection. Do NOT configure restart for connection actors.
- **Passing `tiny_http::Request` by value through actor mailboxes:** The request contains a TCP stream. It cannot be serialized through the message passing system. Pass it as a raw pointer in the spawn args.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Actor spawning | Custom connection pooling | `snow_actor_spawn` via `Scheduler::spawn` | Already tested with 100K actors at ~2.78s |
| Crash isolation | Custom error boundary | `std::panic::catch_unwind` in actor entry | Standard Rust panic isolation, same as how BEAM catches per-process crashes |
| HTTP parsing | Custom HTTP parser | `tiny_http` (already in use) | Proven, tested, handles edge cases |
| Process supervision | Custom health monitoring | Existing supervisor or `trap_exit` | Full OTP-style supervision already implemented |
| Request routing | New routing logic | `SnowRouter::match_route` (unchanged) | Already working correctly |

**Key insight:** Nearly all the infrastructure already exists. This phase is about wiring existing components together, not building new ones. The risk is in the FFI/pointer passing between the accept loop thread and the actor coroutines, not in the high-level architecture.

## Common Pitfalls

### Pitfall 1: tiny_http::Request is !Send, and Blocking I/O in Coroutines

**What goes wrong:** `tiny_http::Request` owns a TCP stream and is not `Send`. Passing it across thread boundaries requires `unsafe`. Additionally, reading the request body and writing the response are blocking I/O operations that will block the coroutine's worker thread.

**Why it happens:** The actor system uses corosensei coroutines which are `!Send` and thread-pinned. A blocking I/O call in a coroutine blocks the entire worker thread, not just the coroutine. This means other actors on the same worker are starved during the I/O.

**How to avoid:**
1. For the `!Send` issue: Use `Box::into_raw` / `Box::from_raw` to transfer ownership across thread boundaries. This is already the pattern used by `SpawnRequest` which is `unsafe impl Send`.
2. For the blocking I/O issue: Accept the limitation for Phase 15. HTTP request/response handling is typically fast (< 1ms for most handlers). The M:N scheduler distributes actors across all CPU cores, so one blocked worker does not stop the entire system. This is exactly how the BEAM handles NIFs that do short blocking work.

**Warning signs:** If a Snow HTTP handler does slow I/O (e.g., database queries, external API calls), it will block the worker thread. This is a known limitation documented in Phase 15, to be addressed in a future async I/O phase if needed.

### Pitfall 2: Panic Safety in Connection Handlers

**What goes wrong:** A Snow handler function pointer (compiled from Snow code) may panic (e.g., division by zero, match failure). If the panic unwinds through the corosensei coroutine without being caught, it is undefined behavior (unwinding across FFI boundaries).

**Why it happens:** The coroutine entry function (`CoroutineHandle::new`) does NOT wrap the actor entry function in `catch_unwind`. This was not a problem for regular actors because the Snow compiler generates well-typed code, but user HTTP handlers may encounter runtime errors.

**How to avoid:** Wrap the connection handler's call to the Snow handler function in `std::panic::catch_unwind`. If a panic occurs:
1. Catch it.
2. Try to send a 500 Internal Server Error response back to the client (if the request object is still usable).
3. Let the actor complete normally (return from entry function).
4. The scheduler handles cleanup via `handle_process_exit`.

**Warning signs:** Crashes in HTTP handlers that take down the entire server instead of just the one connection.

### Pitfall 3: Memory Allocation in Connection Actors

**What goes wrong:** The `handle_request` function currently uses `snow_gc_alloc` (global arena) to allocate SnowHttpRequest, SnowString objects, etc. When running inside an actor, `snow_gc_alloc_actor` should be used instead so allocations go to the per-actor heap and are cleaned up when the actor exits.

**Why it happens:** The current code was written for the thread-per-connection model where there is no actor context. Moving to actors means there IS an actor context, and the per-actor heap should be used.

**How to avoid:** In the connection handler actor entry function, set the current PID (the scheduler does this automatically before calling the entry function). Then `snow_gc_alloc_actor` will correctly allocate from the actor's heap. However, `handle_request` currently calls `snow_gc_alloc` directly. Either:
1. Change `handle_request` to use `snow_gc_alloc_actor` (preferred -- allocations are scoped to the connection).
2. Leave as `snow_gc_alloc` (works but memory is not per-actor). This is acceptable since the GC does not collect yet anyway (Phase 5 no-collect arena).

**Recommendation:** Change to `snow_gc_alloc_actor` for correctness and forward-compatibility with future per-actor GC.

### Pitfall 4: The Accept Loop Must Not Be an Actor

**What goes wrong:** If the accept loop (which calls `server.incoming_requests()`) runs inside an actor coroutine, it blocks the worker thread indefinitely. This starves all other actors on that worker.

**Why it happens:** `incoming_requests()` is a blocking iterator that calls `accept()` on the TCP listener. It will block until a new connection arrives.

**How to avoid:** Run the accept loop on a dedicated OS thread (`std::thread::spawn`). Only the per-connection handlers become actors. The accept loop thread does:
1. Block on `incoming_requests()`
2. For each request, call into the scheduler to spawn an actor
3. Repeat

This is the same pattern as Erlang/OTP: the TCP acceptor is a special process that bridges OS-level I/O to the actor world.

### Pitfall 5: Scheduler Must Be Initialized

**What goes wrong:** `snow_http_serve` currently does not require the actor scheduler. After this change, it will call `snow_actor_spawn` which requires `GLOBAL_SCHEDULER` to be initialized.

**Why it happens:** The current server is standalone (just uses `std::thread::spawn`). The actor-based server needs the M:N scheduler.

**How to avoid:** Either:
1. Ensure `snow_rt_init_actor` is called before `snow_http_serve`. Check if the Snow compiler already emits this call in the generated `main` wrapper. (It should -- all Snow programs with actors call this.)
2. Have `snow_http_serve` lazily initialize the scheduler if not already initialized.

**Recommendation:** Option 1. The compiler should already emit `snow_rt_init_actor` for any program that uses actors. Since Phase 15 makes HTTP implicitly use actors, ensure the compiler emits `snow_rt_init_actor` when `HTTP.serve` is used. OR, have `snow_http_serve` itself call `snow_rt_init_actor(0)` at the top (it is idempotent).

### Pitfall 6: Shutdown Coordination

**What goes wrong:** When the Snow program exits (main returns), `snow_rt_run_scheduler` signals shutdown and waits for all actors to complete. But the HTTP accept loop thread is not an actor -- it's a plain OS thread blocking on `incoming_requests()`. It will block forever.

**Why it happens:** The accept loop thread is outside the scheduler's control.

**How to avoid:** Store the `tiny_http::Server` handle and call `server.unblock()` during shutdown to break the `incoming_requests()` loop. Alternatively, use `Arc<Server>` and drop it or call its shutdown method. `tiny_http::Server` has an `unblock()` method that causes the iterator to return `None`.

This is an important detail: the accept loop thread must be joinable for clean shutdown. Options:
1. Store the accept thread `JoinHandle` and join it during `snow_rt_run_scheduler`.
2. Make the accept loop thread a daemon thread (it dies when the process exits). This is the simplest approach and matches the current behavior (HTTP server runs until process exits).

**Recommendation:** Daemon thread approach (simplest, matches current behavior). The HTTP server is intended to run for the lifetime of the process. When the process exits, the OS cleans up the socket.

## Code Examples

### Example 1: Connection Handler Actor Entry Function

```rust
/// Actor entry function for handling a single HTTP connection.
///
/// Receives a raw pointer to ConnectionArgs containing the router
/// address and a boxed tiny_http::Request.
extern "C" fn connection_handler_entry(args: *const u8) {
    if args.is_null() {
        return;
    }

    // Reconstruct the args struct.
    let args = unsafe { Box::from_raw(args as *mut ConnectionArgs) };
    let router_ptr = args.router_addr as *mut u8;
    let request = unsafe { *Box::from_raw(args.request_ptr as *mut tiny_http::Request) };

    // Wrap handler in catch_unwind for crash isolation.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        handle_request(router_ptr, request);
    }));

    if let Err(panic_info) = result {
        eprintln!("[snow-rt] HTTP handler panicked: {:?}", panic_info);
        // The request is consumed -- TCP connection will be reset.
        // This is acceptable: the client receives a connection reset,
        // but other connections are unaffected.
    }
}

/// Arguments passed to the connection handler actor.
#[repr(C)]
struct ConnectionArgs {
    router_addr: usize,
    request_ptr: usize,
}
```

### Example 2: Modified Accept Loop

```rust
#[no_mangle]
pub extern "C" fn snow_http_serve(router: *mut u8, port: i64) {
    // Ensure the actor scheduler is initialized.
    crate::actor::snow_rt_init_actor(0);

    let addr = format!("0.0.0.0:{}", port);
    let server = match tiny_http::Server::http(&addr) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[snow-rt] Failed to start HTTP server on {}: {}", addr, e);
            return;
        }
    };

    eprintln!("[snow-rt] HTTP server listening on {}", addr);

    let router_addr = router as usize;

    for request in server.incoming_requests() {
        // Box the request for transfer to actor thread.
        let request_ptr = Box::into_raw(Box::new(request)) as usize;

        // Build args struct for the actor.
        let args = ConnectionArgs {
            router_addr,
            request_ptr,
        };
        let args_ptr = Box::into_raw(Box::new(args)) as *const u8;
        let args_size = std::mem::size_of::<ConnectionArgs>() as u64;

        // Spawn a lightweight actor for this connection.
        let sched = global_scheduler();
        sched.spawn(
            connection_handler_entry as *const u8,
            args_ptr,
            args_size,
            1, // Normal priority
        );
    }
}
```

### Example 3: handle_request with Actor-Aware Allocation

```rust
fn handle_request(router_ptr: *mut u8, mut request: tiny_http::Request) {
    unsafe {
        let router = &*(router_ptr as *const SnowRouter);

        // Use snow_gc_alloc (or snow_gc_alloc_actor when in actor context)
        // to allocate SnowHttpRequest fields.
        // ... (same logic as current implementation)
    }
}
```

Note: The `handle_request` function body remains almost identical to the current implementation. The only changes are:
1. It receives a `tiny_http::Request` by value instead of by move-closure
2. Optionally uses `snow_gc_alloc_actor` instead of `snow_gc_alloc`

### Example 4: Snow Program (Unchanged)

```snow
fn handler(request) do
  HTTP.response(200, "{\"status\":\"ok\"}")
end

fn main() do
  let r = HTTP.router()
  let r = HTTP.route(r, "/health", handler)
  HTTP.serve(r, 18080)
end
```

This program works identically before and after Phase 15. The API does not change.

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| std::thread::spawn per connection | Actor per connection (corosensei) | Phase 15 | Lightweight actors (64 KiB stack) instead of OS threads (~8 MiB stack). Crash isolation per connection. Same API. |
| No crash isolation for HTTP | catch_unwind + actor isolation | Phase 15 | A panicking handler no longer crashes the server |
| HTTP independent of actor system | HTTP uses actor system | Phase 15 | Unifies the runtime model -- everything is an actor |

**Deprecated/outdated after Phase 15:**
- The `std::thread::spawn` approach in server.rs is replaced entirely
- The comment in `mod.rs` about "pragmatic choice" for thread-per-connection is no longer applicable

## Open Questions

1. **Should snow_http_serve block the calling thread or return immediately?**
   - What we know: Currently blocks forever (loops on `incoming_requests()`). This is called from `main()` in Snow programs.
   - What's unclear: Should the accept loop run on the calling thread (current behavior) or spawn a background thread?
   - Recommendation: Keep current behavior (block the calling thread). This matches user expectations and the existing e2e test. The accept loop runs on the main thread, and actors run on the scheduler's worker threads. This is the standard pattern in web frameworks.

2. **Should we switch from snow_gc_alloc to snow_gc_alloc_actor in handle_request?**
   - What we know: `snow_gc_alloc_actor` will correctly allocate from the actor's heap when running inside an actor context. Currently, `handle_request` uses `snow_gc_alloc` (global arena).
   - What's unclear: Whether the global arena allocations cause issues when the actor system is active.
   - Recommendation: Switch to `snow_gc_alloc_actor` for forward-compatibility. It falls back to the global arena if no actor context exists, so it is a safe change.

3. **Should the connection actor call the Snow handler function directly, or should it wrap it?**
   - What we know: The Snow handler is a function pointer with signature `fn(request_ptr) -> response_ptr`. The connection actor needs to call this and then respond to the client.
   - What's unclear: Whether calling a Snow function pointer from within an actor entry function requires any special setup (e.g., reduction checking, yielder context).
   - Recommendation: The actor entry function already runs inside a coroutine with a yielder set up. The Snow handler function will naturally participate in reduction counting (the compiler inserts `snow_reduction_check` calls). No special setup needed. Just call the function pointer as-is.

## Sources

### Primary (HIGH confidence)
- Direct codebase analysis of `/Users/sn0w/Documents/dev/snow/crates/snow-rt/src/http/server.rs` -- current HTTP server implementation
- Direct codebase analysis of `/Users/sn0w/Documents/dev/snow/crates/snow-rt/src/actor/mod.rs` -- actor ABI (spawn, send, receive)
- Direct codebase analysis of `/Users/sn0w/Documents/dev/snow/crates/snow-rt/src/actor/scheduler.rs` -- M:N scheduler, worker loop, SpawnRequest pattern
- Direct codebase analysis of `/Users/sn0w/Documents/dev/snow/crates/snow-rt/src/actor/stack.rs` -- corosensei coroutine management
- Direct codebase analysis of `/Users/sn0w/Documents/dev/snow/crates/snow-rt/src/actor/supervisor.rs` -- supervision tree implementation
- Direct codebase analysis of `/Users/sn0w/Documents/dev/snow/crates/snow-rt/src/actor/link.rs` -- exit signal propagation
- Direct codebase analysis of `/Users/sn0w/Documents/dev/snow/crates/snow-rt/src/gc.rs` -- GC allocation (global arena + per-actor heap)
- Direct codebase analysis of `/Users/sn0w/Documents/dev/snow/crates/snow-rt/src/http/router.rs` -- URL pattern matching
- Direct codebase analysis of `/Users/sn0w/Documents/dev/snow/crates/snow-rt/Cargo.toml` -- dependency versions

### Secondary (MEDIUM confidence)
- corosensei crate documentation (from training data) -- coroutine `!Send` behavior, panic behavior through coroutines
- tiny_http crate documentation (from training data) -- `incoming_requests()` blocking behavior, `Request` ownership semantics

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH - all libraries already in use, no new dependencies needed
- Architecture: HIGH - all components verified through direct codebase analysis, patterns are straightforward
- Pitfalls: HIGH - identified through systematic analysis of threading/coroutine interaction, GC scoping, and shutdown semantics

**Research date:** 2026-02-07
**Valid until:** 2026-03-07 (30 days -- stable domain, no external dependency changes)
