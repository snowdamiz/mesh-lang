# Phase 60: Actor Integration - Research

**Researched:** 2026-02-12
**Domain:** WebSocket actor-per-connection architecture, reader thread bridge, callback lifecycle
**Confidence:** HIGH

## Summary

Phase 60 integrates the Phase 59 WebSocket protocol layer (frame codec, handshake, close) with Snow's actor system to create the `Ws.serve(handler, port)` API. Each WebSocket connection spawns a dedicated actor with crash isolation. The central technical challenge is the **reader thread bridge**: a mechanism to read WebSocket frames from a blocking TCP stream on an OS thread and deliver them into an actor's mailbox without blocking the M:N scheduler's worker threads.

The existing HTTP server (`crates/snow-rt/src/http/server.rs`) provides a proven pattern for actor-per-connection servers. It uses `TcpListener::incoming()` on the calling thread, wraps each accepted `TcpStream` in a boxed struct, passes it as a `usize` through `ConnectionArgs`, and spawns an actor via `sched.spawn()`. The WebSocket server will follow this same pattern for the accept loop and initial actor spawning, but diverges after the handshake because WebSocket connections are long-lived and bidirectional (unlike HTTP's request-response-close cycle).

**Primary recommendation:** Use the HTTP server's accept-loop + actor-spawn pattern. For the reader thread bridge, spawn a dedicated `std::thread` per connection that reads frames and pushes them into the actor's mailbox via `Arc<Mailbox>`. The actor coroutine uses the standard `receive` expression to process frames alongside actor-to-actor messages. Use reserved type tags in the high range (u64::MAX - N) to prevent collision with user message tags. The write half of the TCP stream is shared via `Arc<Mutex<TcpStream>>` so the actor can send frames without owning the stream.

## Standard Stack

### Core (already in codebase)
| Component | Location | Purpose | Why Standard |
|-----------|----------|---------|--------------|
| Frame codec | `crates/snow-rt/src/ws/frame.rs` | `read_frame`, `write_frame`, `apply_mask` | Phase 59 built |
| Handshake | `crates/snow-rt/src/ws/handshake.rs` | `perform_upgrade`, `validate_upgrade_request` | Phase 59 built |
| Close/dispatch | `crates/snow-rt/src/ws/close.rs` | `process_frame`, `send_close`, `validate_text_payload` | Phase 59 built |
| Actor system | `crates/snow-rt/src/actor/` | Scheduler, Process, Mailbox, Link, Stack | Established core |
| HTTP server pattern | `crates/snow-rt/src/http/server.rs` | Accept loop + actor-per-connection model | Template to follow |

### Supporting (already available as dependencies)
| Component | Purpose | When to Use |
|-----------|---------|-------------|
| `parking_lot::Mutex` | Stream write-half sharing | Protecting `TcpStream` for concurrent write access |
| `std::thread` | Reader thread per connection | Blocking frame reads off the scheduler |
| `std::net::TcpStream` | try_clone() for read/write split | Splitting stream into reader thread + actor writer |

### No New Dependencies Required

All required functionality is available through existing crate dependencies and the standard library. No new Cargo dependencies are needed.

## Architecture Patterns

### Recommended Module Structure
```
crates/snow-rt/src/ws/
    mod.rs          # existing -- add re-exports for new items
    frame.rs        # existing (Phase 59)
    handshake.rs    # existing (Phase 59) -- needs modification: return headers
    close.rs        # existing (Phase 59)
    server.rs       # NEW -- Ws.serve, accept loop, reader thread, actor entry
```

### Pattern 1: Accept Loop + Actor Spawn (from HTTP server)
**What:** The calling thread runs `TcpListener::incoming()` in a loop, spawning an actor per accepted connection.
**When to use:** All server entry points (HTTP, HTTPS, WebSocket).
**Source:** `crates/snow-rt/src/http/server.rs` lines 413-458

The `snow_ws_serve(handler, port)` function:
1. Calls `snow_rt_init_actor(0)` (idempotent scheduler init)
2. Binds `TcpListener` on `0.0.0.0:{port}`
3. For each accepted TCP stream:
   - Sets read timeout (30s)
   - Packs handler + stream into `WsConnectionArgs`
   - Calls `sched.spawn(ws_connection_entry, args_ptr, args_size, 1)`

### Pattern 2: Reader Thread Bridge (novel for this codebase)
**What:** A dedicated `std::thread` reads WebSocket frames from the TCP stream and pushes them as messages into the actor's mailbox. The actor processes them via `receive`.
**Why:** `read_frame` uses blocking `read_exact` on the TCP stream. If this runs on the actor coroutine (scheduler worker thread), it blocks the entire worker, preventing other actors from executing. The HTTP server avoids this because HTTP connections are short-lived (parse request, generate response, close). WebSocket connections are long-lived, so blocking is unacceptable.

**How it works:**
1. After the handshake completes, the actor's entry function splits the `TcpStream`:
   - **Read half:** `TcpStream::try_clone()` goes to a new `std::thread`
   - **Write half:** Original `TcpStream` wrapped in `Arc<Mutex<TcpStream>>`, stored in actor state
2. The reader thread loops:
   ```
   loop {
       match read_frame(&mut read_stream) {
           Ok(frame) => {
               match process_frame(&mut write_stream_clone, frame) {
                   Ok(Some(data_frame)) => {
                       // Encode frame as MessageBuffer with reserved type tag
                       // Push to actor mailbox via Arc<Mailbox>
                       // Wake actor if Waiting
                   }
                   Ok(None) => continue, // control frame handled (ping/pong)
                   Err(e) => {
                       // Connection ended (close) or protocol error
                       // Push disconnect message to mailbox
                       break;
                   }
               }
           }
           Err(e) => {
               // I/O error or timeout
               // Push disconnect message to mailbox
               break;
           }
       }
   }
   ```
3. The actor coroutine uses `snow_actor_receive(-1)` to get messages. Messages from the reader thread (WS frames) arrive alongside any actor-to-actor messages. The type tag distinguishes them.

**Critical design constraint:** The reader thread needs a clone of the write half to handle protocol-level responses (ping -> pong, close echo). The `process_frame` function in `close.rs` takes `&mut S: Write` and may write pong/close responses. The reader thread needs write access for these control frames.

**Solution:** `TcpStream::try_clone()` twice:
- Clone 1: Reader thread owns for reads
- Clone 2: Reader thread has write access for control frames (ping/pong/close echo)
- Original: Actor owns for `Ws.send`/`Ws.send_binary` via `Arc<Mutex<TcpStream>>`

Actually, `TcpStream::try_clone()` produces a duplicate handle to the same underlying socket. Both the original and clone can read and write independently. So:
- **Reader thread:** Owns one `TcpStream` clone for both reading frames AND writing control responses (pong, close echo)
- **Actor:** Owns the original `TcpStream` (or wrapped in `Arc<Mutex<>>`) for `Ws.send()`/`Ws.send_binary()`

This is safe because:
- Reads are single-threaded (only the reader thread reads)
- Writes from the reader thread (pong/close) and actor thread (`Ws.send`) may interleave, but `write_frame` writes complete frames atomically to the kernel buffer. For safety, both should go through the same `Arc<Mutex<TcpStream>>`.

**Revised solution:** Both reader thread and actor share an `Arc<Mutex<TcpStream>>` for writes. The reader thread owns a separate `TcpStream` clone for reads only.

### Pattern 3: Reserved Type Tags for WS Messages
**What:** Use reserved type tag values (in the u64::MAX range) for WebSocket frame messages, preventing collision with user-defined actor-to-actor messages.
**Existing precedent:** `EXIT_SIGNAL_TAG = u64::MAX` is already reserved for exit signals in `link.rs`.

Proposed reserved tags:
| Tag | Meaning |
|-----|---------|
| `u64::MAX` | Exit signal (existing) |
| `u64::MAX - 1` | WebSocket text frame |
| `u64::MAX - 2` | WebSocket binary frame |
| `u64::MAX - 3` | WebSocket disconnect (close/error) |
| `u64::MAX - 4` | WebSocket connect (on_connect notification) |

The type_tag derivation in `snow_actor_send` uses the first 8 bytes of the message data as the tag. User messages will never produce tags in the `u64::MAX - N` range unless they deliberately construct messages with 0xFF-leading bytes, which is extremely unlikely and would be a user error.

### Pattern 4: Callback-Based Handler API
**What:** The user passes a handler struct (or tuple of callbacks) to `Ws.serve`. The handler bundles `on_connect`, `on_message`, and `on_close` callbacks.
**Snow-level API:**

```snow
# Handler is a map or struct with callback functions
let handler = %{
  on_connect: fn(conn, headers) -> :ok | :reject,
  on_message: fn(conn, message) -> :ok,
  on_close: fn(conn, code, reason) -> :ok
}

Ws.serve(handler, 8080)
```

At the runtime level, the handler is passed as a pointer. The runtime extracts callback function pointers from the handler struct/map at connection time.

**Simpler alternative (recommended):** Pass three separate function pointers:

```snow
Ws.serve(on_connect, on_message, on_close, port)
```

This avoids needing to parse a Snow map at runtime. Each callback is a Snow closure (fn_ptr + env_ptr pair). The `snow_ws_serve` function receives 7 arguments: `on_connect_fn, on_connect_env, on_message_fn, on_message_env, on_close_fn, on_close_env, port`.

**Even simpler (recommended for Phase 60):** Use a single handler function pointer like HTTP, with the runtime dispatching to it for each event. The handler receives a tagged message:

Actually, the most natural Snow pattern following the existing codebase conventions is to have the actor use `receive` to process messages. The callbacks (`on_connect`, `on_message`, `on_close`) would be called by the actor's entry function at the appropriate lifecycle points. The handler is a single callback function that receives the connection and message data.

**Recommended approach:** Bundle all three callbacks into a single `WsHandler` struct:
```rust
#[repr(C)]
struct WsHandler {
    on_connect_fn: *mut u8,
    on_connect_env: *mut u8,
    on_message_fn: *mut u8,
    on_message_env: *mut u8,
    on_close_fn: *mut u8,
    on_close_env: *mut u8,
}
```

This mirrors how Snow closures work (fn_ptr + env_ptr pairs). `Ws.serve(handler, port)` where `handler` is a Snow-level struct containing three closures.

### Pattern 5: Connection Handle for Ws.send
**What:** The actor needs a handle to send frames back to its client.
**Design:** A connection handle is an opaque pointer to a `WsConnection` struct stored on the Rust heap (not the GC heap -- it contains a Mutex):

```rust
struct WsConnection {
    write_stream: Arc<Mutex<TcpStream>>,
    actor_pid: ProcessId,
}
```

`Ws.send(conn, message)` calls `snow_ws_send(conn_ptr, msg_ptr)` which:
1. Locks the write stream
2. Calls `write_frame(stream, WsOpcode::Text, payload, true)`
3. Returns success/failure

### Anti-Patterns to Avoid
- **Blocking read on actor coroutine:** Never call `read_frame` from within the actor's coroutine body. This would block the scheduler worker thread. Always use the reader thread bridge.
- **Shared mutable state without Mutex:** The `TcpStream` write half must be protected by a Mutex since both the reader thread (for pong/close) and actor thread (for `Ws.send`) may write.
- **GC-allocating the connection handle:** The `WsConnection` contains a `Mutex` and `Arc`, which must not be GC-managed. Use `Box::into_raw` like the HTTP server's `ConnectionArgs`.
- **Using the actor's heap for reader thread data:** The reader thread runs outside any actor context. It must use Rust heap allocation (Vec, Box), not `snow_gc_alloc_actor`. Message data is serialized into `MessageBuffer` (which uses Vec internally) and deep-copied into the actor's heap when `receive` processes the message.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Frame codec | Custom parser | `ws::frame::read_frame` / `write_frame` | Already built in Phase 59, handles all length encodings and masking |
| HTTP upgrade | Custom HTTP parser | `ws::handshake::perform_upgrade` | Already built in Phase 59 with RFC 6455 compliance |
| Close handshake | Custom close logic | `ws::close::process_frame` | Already handles ping/pong/close/UTF-8 validation |
| Actor spawning | Custom thread management | `sched.spawn()` | Existing M:N scheduler with crash isolation |
| Message delivery | Custom channel | `Arc<Mailbox>` + `snow_actor_send` pattern | Existing mailbox with FIFO ordering and wake semantics |
| Exit propagation | Custom cleanup | `link::propagate_exit` | Existing bidirectional links and exit signal delivery |

**Key insight:** Phase 59 built the protocol layer and the actor system is mature. Phase 60 is a *glue* phase that connects them. The novel component is only the reader thread bridge.

## Common Pitfalls

### Pitfall 1: Blocking the M:N Scheduler
**What goes wrong:** Calling `read_frame` (blocking I/O) directly from the actor's coroutine body blocks the scheduler worker thread. Other actors on that thread cannot run.
**Why it happens:** It's the natural/simple approach -- just read in the actor body like HTTP does. But HTTP connections are short-lived; WebSocket connections persist indefinitely.
**How to avoid:** Always use the reader thread bridge pattern. The reader thread is a separate `std::thread`, not a scheduler worker.
**Warning signs:** Other actors freeze when a WebSocket connection is idle (waiting for client data).

### Pitfall 2: Type Tag Collision
**What goes wrong:** WebSocket frame messages use the same type tags as user actor-to-actor messages, causing incorrect pattern matching or dropped frames.
**Why it happens:** The current `snow_actor_send` derives type tags from the first 8 bytes of message data. If WS frame data happens to produce the same tag as a user message, they become indistinguishable.
**How to avoid:** Use reserved type tags (u64::MAX - N range) for all WS-related mailbox messages. Push messages directly into the mailbox using `MessageBuffer::new(data, reserved_tag)` instead of going through `snow_actor_send`.
**Warning signs:** Actor processes WebSocket frames as regular messages or vice versa.

### Pitfall 3: Masking Direction Error
**What goes wrong:** Server sends masked frames to the client (or expects unmasked frames from the client).
**Why it happens:** RFC 6455 specifies client-to-server = masked, server-to-client = unmasked. Easy to get backwards.
**How to avoid:** `read_frame` already handles unmasking client frames. `write_frame` already writes unmasked server frames. Don't add masking to `Ws.send` / `Ws.send_binary`.
**Warning signs:** Client immediately closes connection after receiving first server frame.

### Pitfall 4: Reader Thread Outlives Actor
**What goes wrong:** The actor crashes or exits, but the reader thread continues running, potentially pushing messages into a dead mailbox.
**Why it happens:** The reader thread is a separate OS thread with its own lifetime. It doesn't know the actor has exited.
**How to avoid:** The reader thread holds an `Arc<AtomicBool>` "shutdown" flag. The actor's exit path (normal return, crash via catch_unwind) sets this flag. The reader thread checks it periodically (on each frame read or timeout). Additionally, when the actor exits, its `TcpStream` write half is dropped, which may cause the read half to error out on the reader thread.
**Warning signs:** Resource leak -- OS threads accumulate, file descriptors not released.

### Pitfall 5: Close Frame After Actor Crash
**What goes wrong:** When an actor panics (crash), the connection is dropped without sending a close frame to the client. The client sees an abrupt TCP disconnect.
**Why it happens:** `catch_unwind` catches the panic, but the close frame sending happens after the unwind, when the stream may already be partially invalid.
**How to avoid:** The actor entry function wraps the user callback in `catch_unwind`. On panic, it sends a close frame (1011 Internal Error) before dropping the connection. The write-half `Arc<Mutex<TcpStream>>` should still be valid after the panic because it's in a separate allocation.
**Warning signs:** Clients report `1006` (abnormal closure) instead of `1011` (internal error).

### Pitfall 6: perform_upgrade Doesn't Return Headers
**What goes wrong:** The `on_connect` callback needs access to request headers (for auth tokens, cookies, etc.), but `perform_upgrade` currently returns `Result<(), String>` and discards the parsed headers.
**Why it happens:** Phase 59 only needed to validate and respond to the upgrade. It didn't need to expose headers.
**How to avoid:** Modify `perform_upgrade` to return `Result<Vec<(String, String)>, String>` -- the parsed headers on success. Or add a new function `perform_upgrade_with_headers` that returns both the path and headers.
**Warning signs:** `on_connect` callback receives empty headers.

### Pitfall 7: Reader Thread Mailbox Push Without Wake
**What goes wrong:** The reader thread pushes a message into the actor's mailbox, but the actor remains in `Waiting` state because no one woke it.
**Why it happens:** `snow_actor_send` both pushes the message AND wakes the process. But the reader thread bypasses `snow_actor_send` (because it uses reserved type tags). It needs to manually wake the process.
**How to avoid:** After `mailbox.push(msg)`, the reader thread must: (1) lock the process, (2) check if state is `Waiting`, (3) set to `Ready`, (4) call `sched.wake_process(pid)`. This mirrors the wake logic in `snow_actor_send` (mod.rs lines 285-296).
**Warning signs:** Actor never receives WebSocket frames despite reader thread successfully reading them.

### Pitfall 8: Concurrent Writes Corrupting Frames
**What goes wrong:** The reader thread writes a pong frame while the actor simultaneously writes a data frame via `Ws.send`. The two writes interleave, producing garbled data on the wire.
**Why it happens:** Both the reader thread (for control frames) and actor (for data frames) write to the same TcpStream.
**How to avoid:** Both must use the same `Arc<Mutex<TcpStream>>` for writes. The Mutex ensures that `write_frame` calls are atomic (one complete frame written before another starts).
**Warning signs:** Client receives malformed frames intermittently, especially under load.

## Code Examples

### Example 1: Accept Loop (adapted from HTTP server)
```rust
// Source: crates/snow-rt/src/http/server.rs lines 413-458
#[no_mangle]
pub extern "C" fn snow_ws_serve(handler: *mut u8, port: i64) {
    crate::actor::snow_rt_init_actor(0);
    let addr = format!("0.0.0.0:{}", port);
    let listener = match std::net::TcpListener::bind(&addr) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("[snow-rt] Failed to start WS server on {}: {}", addr, e);
            return;
        }
    };
    eprintln!("[snow-rt] WebSocket server listening on {}", addr);
    let handler_addr = handler as usize;
    for tcp_stream in listener.incoming() {
        let tcp_stream = match tcp_stream {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[snow-rt] accept error: {}", e);
                continue;
            }
        };
        tcp_stream.set_read_timeout(Some(Duration::from_secs(30))).ok();
        let stream_ptr = Box::into_raw(Box::new(tcp_stream)) as usize;
        let args = WsConnectionArgs { handler_addr, stream_ptr };
        let args_ptr = Box::into_raw(Box::new(args)) as *const u8;
        let sched = actor::global_scheduler();
        sched.spawn(ws_connection_entry as *const u8, args_ptr, /* size */ 0, 1);
    }
}
```

### Example 2: Actor Entry with Reader Thread Bridge
```rust
extern "C" fn ws_connection_entry(args: *const u8) {
    if args.is_null() { return; }
    let args = unsafe { Box::from_raw(args as *mut WsConnectionArgs) };
    let mut stream = unsafe { *Box::from_raw(args.stream_ptr as *mut TcpStream) };

    // 1. Perform WebSocket upgrade handshake
    let headers = match perform_upgrade_with_headers(&mut stream) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("[snow-rt] WS upgrade failed: {}", e);
            return;
        }
    };

    // 2. Create shared write stream
    let write_stream = Arc::new(Mutex::new(stream.try_clone().expect("clone stream")));
    let read_stream = stream; // original for reading

    // 3. Get current actor's PID and mailbox
    let my_pid = stack::get_current_pid().expect("no PID");
    let sched = actor::global_scheduler();
    let mailbox = sched.get_process(my_pid).expect("no process").lock().mailbox.clone();

    // 4. Call on_connect callback (can reject)
    // ... invoke handler.on_connect(conn, headers) ...

    // 5. Spawn reader thread
    let shutdown = Arc::new(AtomicBool::new(false));
    let reader_shutdown = shutdown.clone();
    let reader_write = write_stream.clone();
    let reader_mailbox = mailbox.clone();
    std::thread::spawn(move || {
        reader_thread_loop(read_stream, reader_write, reader_mailbox, my_pid, reader_shutdown);
    });

    // 6. Actor message loop (processes WS frames + actor messages)
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        loop {
            let msg_ptr = snow_actor_receive(-1); // block until message
            if msg_ptr.is_null() { break; }
            // Dispatch based on type_tag:
            // - WS_TEXT_TAG / WS_BINARY_TAG -> call on_message
            // - WS_DISCONNECT_TAG -> call on_close, break
            // - EXIT_SIGNAL_TAG -> handle exit
            // - other -> regular actor message
        }
    }));

    // 7. Cleanup: signal reader thread, send close frame
    shutdown.store(true, Ordering::SeqCst);
    if result.is_err() {
        // Actor crashed: send 1011 close frame
        let _ = send_close(&mut *write_stream.lock(), WsCloseCode::INTERNAL_ERROR, "");
    }
}
```

### Example 3: Reader Thread Loop
```rust
fn reader_thread_loop(
    mut read_stream: TcpStream,
    write_stream: Arc<Mutex<TcpStream>>,
    mailbox: Arc<Mailbox>,
    actor_pid: ProcessId,
    shutdown: Arc<AtomicBool>,
) {
    loop {
        if shutdown.load(Ordering::SeqCst) { break; }

        match read_frame(&mut read_stream) {
            Ok(frame) => {
                let mut writer = write_stream.lock();
                match process_frame(&mut *writer, frame) {
                    Ok(Some(data_frame)) => {
                        drop(writer); // release lock before mailbox push
                        // Encode and push to mailbox
                        let tag = match data_frame.opcode {
                            WsOpcode::Text => WS_TEXT_TAG,
                            WsOpcode::Binary => WS_BINARY_TAG,
                            _ => WS_TEXT_TAG,
                        };
                        let buffer = MessageBuffer::new(data_frame.payload, tag);
                        mailbox.push(Message { buffer });
                        // Wake actor if Waiting
                        wake_actor(actor_pid);
                    }
                    Ok(None) => { /* ping/pong handled */ }
                    Err(_) => {
                        drop(writer);
                        // Push disconnect message
                        let buffer = MessageBuffer::new(Vec::new(), WS_DISCONNECT_TAG);
                        mailbox.push(Message { buffer });
                        wake_actor(actor_pid);
                        break;
                    }
                }
            }
            Err(e) => {
                // I/O error or timeout -- check if it's a real error or just timeout
                if shutdown.load(Ordering::SeqCst) { break; }
                // For read timeout: continue (allows checking shutdown flag)
                // For real error: push disconnect and break
                if e.contains("timed out") || e.contains("WouldBlock") {
                    continue;
                }
                let buffer = MessageBuffer::new(Vec::new(), WS_DISCONNECT_TAG);
                mailbox.push(Message { buffer });
                wake_actor(actor_pid);
                break;
            }
        }
    }
}
```

### Example 4: Ws.send Runtime Function
```rust
#[no_mangle]
pub extern "C" fn snow_ws_send(conn: *mut u8, msg: *const SnowString) -> i64 {
    if conn.is_null() || msg.is_null() { return -1; }
    let conn = unsafe { &*(conn as *const WsConnection) };
    let text = unsafe { (*msg).as_str() };
    let mut stream = conn.write_stream.lock();
    match write_frame(&mut *stream, WsOpcode::Text, text.as_bytes(), true) {
        Ok(()) => 0,
        Err(_) => -1,
    }
}

#[no_mangle]
pub extern "C" fn snow_ws_send_binary(conn: *mut u8, data: *const u8, len: i64) -> i64 {
    if conn.is_null() || data.is_null() { return -1; }
    let conn = unsafe { &*(conn as *const WsConnection) };
    let bytes = unsafe { std::slice::from_raw_parts(data, len as usize) };
    let mut stream = conn.write_stream.lock();
    match write_frame(&mut *stream, WsOpcode::Binary, bytes, true) {
        Ok(()) => 0,
        Err(_) => -1,
    }
}
```

### Example 5: Codegen Wiring (from HTTP precedent)
```rust
// In intrinsics.rs -- declare runtime functions
module.add_function("snow_ws_serve", void_type.fn_type(&[ptr_type.into(), i64_type.into()], false), Some(Linkage::External));
module.add_function("snow_ws_send", i64_type.fn_type(&[ptr_type.into(), ptr_type.into()], false), Some(Linkage::External));
module.add_function("snow_ws_send_binary", i64_type.fn_type(&[ptr_type.into(), ptr_type.into(), i64_type.into()], false), Some(Linkage::External));

// In lower.rs -- known functions
self.known_functions.insert("snow_ws_serve".to_string(), MirType::FnPtr(vec![MirType::Ptr, MirType::Int], Box::new(MirType::Unit)));
self.known_functions.insert("snow_ws_send".to_string(), MirType::FnPtr(vec![MirType::Ptr, MirType::String], Box::new(MirType::Int)));
self.known_functions.insert("snow_ws_send_binary".to_string(), MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr, MirType::Int], Box::new(MirType::Int)));

// In STDLIB_MODULES
const STDLIB_MODULES: &[&str] = &[..., "Ws"];

// In map_builtin_name
"ws_serve" => "snow_ws_serve".to_string(),
"ws_send" => "snow_ws_send".to_string(),
"ws_send_binary" => "snow_ws_send_binary".to_string(),
```

## Detailed Design Decisions

### perform_upgrade Must Return Headers
The current `perform_upgrade` signature is `fn perform_upgrade<S: Read + Write>(stream: &mut S) -> Result<(), String>`. For LIFE-01 (on_connect receives request headers), it must return the parsed headers. Two options:

1. **Modify in-place:** Change to `Result<(String, Vec<(String, String)>), String>` returning (path, headers). This is a breaking change to the function signature but there are no other callers yet.
2. **New function:** Add `perform_upgrade_with_headers` alongside the existing one.

**Recommendation:** Option 1 -- modify in-place. The function has no external callers beyond tests. Return the request path and headers on success.

### Handler Function Calling Convention
The `Ws.serve(handler, port)` API needs to pass three callbacks. Following the established Snow pattern for closures, each callback is a `{fn_ptr, env_ptr}` pair. The handler can be a Snow struct with three closure fields.

At the runtime level, `snow_ws_serve` receives the handler as a single pointer to a `WsHandler` struct (3 closure pairs = 6 pointers = 48 bytes). This matches how the HTTP server receives a router pointer.

### Actor Crash Isolation (ACTOR-01, ACTOR-05)
The actor entry function wraps all user callback invocations in `catch_unwind`. On panic:
1. Send close frame 1011 to client
2. Signal reader thread to stop
3. Allow normal process exit handling (which propagates exit signals to linked actors per ACTOR-06)

### Client Disconnect Causes Actor Exit (ACTOR-06)
When the reader thread detects a client disconnect (read error or close frame), it pushes a `WS_DISCONNECT_TAG` message to the actor's mailbox. The actor's message loop processes this, calls `on_close`, and returns from the entry function. The scheduler then handles the normal exit path, propagating exit signals to linked actors.

### on_connect Rejection (LIFE-02)
The `on_connect` callback returns a value indicating accept or reject. If rejected:
1. Send close frame 1000 (normal) or 403-style rejection
2. Drop the connection
3. Exit the actor immediately (don't start the reader thread)

Since the handshake has already completed (101 sent), rejection after `on_connect` means sending a close frame immediately. The alternative (rejecting before 101) would require `on_connect` to be called before `perform_upgrade`, which is not possible because the request hasn't been validated yet.

**Practical approach:** The `on_connect` callback runs after the handshake. If it returns an error, the connection actor sends close frame 1008 (Policy Violation) and exits.

## Open Questions

1. **Reader thread read timeout vs. blocking**
   - What we know: The reader thread needs to periodically check the shutdown flag. Using a short read timeout (e.g., 100ms) lets it check between reads. But the HTTP server uses 30s timeout.
   - What's unclear: What timeout value balances responsiveness to shutdown vs. overhead of repeated timeout handling?
   - Recommendation: Use 5-second read timeout on the reader thread's TcpStream clone. On timeout, check shutdown flag and continue. This gives reasonable shutdown responsiveness without excessive timeout handling.

2. **Handler struct layout at the Snow level**
   - What we know: Snow closures are `{fn_ptr, env_ptr}` pairs. The handler needs three closures.
   - What's unclear: Whether to use a Snow struct, a map, or three separate arguments.
   - Recommendation: Use a `#[repr(C)]` WsHandler struct with 6 pointer fields. The Snow compiler generates this struct when the user creates the handler. `Ws.serve(handler, port)` passes the struct pointer.

3. **Connection handle lifetime and GC**
   - What we know: The WsConnection is Rust-heap allocated (contains Arc<Mutex>). It can't be GC-managed.
   - What's unclear: How does the GC know not to collect it? If the connection handle pointer is on the actor's stack, the conservative GC might try to interpret it.
   - Recommendation: The connection handle is a raw pointer stored in a variable on the actor's coroutine stack. The conservative GC will see it as an opaque integer, not as a GC-managed object, because it doesn't point into any ActorHeap page. This is safe -- the GC only scans its own pages for object containment.

## Sources

### Primary (HIGH confidence)
- `crates/snow-rt/src/http/server.rs` -- Accept loop + actor-per-connection pattern, `ConnectionArgs` design, `catch_unwind` crash isolation
- `crates/snow-rt/src/actor/mod.rs` -- Actor spawn, send, receive, link, exit propagation APIs
- `crates/snow-rt/src/actor/scheduler.rs` -- M:N scheduler, worker loop, process state management, wake semantics
- `crates/snow-rt/src/actor/process.rs` -- Process struct, ProcessState, Mailbox, ExitReason
- `crates/snow-rt/src/actor/link.rs` -- Exit signal encoding, propagation, EXIT_SIGNAL_TAG sentinel
- `crates/snow-rt/src/actor/mailbox.rs` -- Thread-safe FIFO mailbox (parking_lot Mutex)
- `crates/snow-rt/src/actor/stack.rs` -- Coroutine handles, yield, thread-local PID
- `crates/snow-rt/src/actor/heap.rs` -- MessageBuffer, ActorHeap, GC
- `crates/snow-rt/src/ws/frame.rs` -- Frame codec (read_frame, write_frame)
- `crates/snow-rt/src/ws/handshake.rs` -- HTTP upgrade handshake
- `crates/snow-rt/src/ws/close.rs` -- Close handshake, process_frame, WsCloseCode
- `crates/snow-codegen/src/codegen/intrinsics.rs` -- LLVM function declarations for builtins
- `crates/snow-codegen/src/mir/lower.rs` -- MIR lowering, STDLIB_MODULES, map_builtin_name, known_functions
- `.planning/phases/59-protocol-core/59-01-PLAN.md` -- Frame codec design decisions
- `.planning/phases/59-protocol-core/59-02-PLAN.md` -- Handshake and close design decisions

### Secondary (MEDIUM confidence)
- Rust `std::net::TcpStream::try_clone()` -- documented to produce duplicate handle to same socket, both can read/write independently

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- all components exist in the codebase, no new dependencies
- Architecture: HIGH -- reader thread bridge follows established OS thread pattern from `snow_timer_send_after`, accept loop follows HTTP server exactly
- Pitfalls: HIGH -- identified from direct code analysis, not speculation
- Codegen wiring: HIGH -- exact precedent from HTTP server in intrinsics.rs and lower.rs

**Research date:** 2026-02-12
**Valid until:** Indefinite (codebase-internal research, no external dependencies)
