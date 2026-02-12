# Phase 61: Production Hardening - Research

**Researched:** 2026-02-12
**Domain:** WebSocket TLS encryption (wss://), heartbeat ping/pong, message fragmentation reassembly
**Confidence:** HIGH

## Summary

Phase 61 adds three production-hardening features to the WebSocket server built in Phases 59-60: TLS encryption, heartbeat dead-connection detection, and fragmented message reassembly. All three features operate at the runtime layer (`crates/snow-rt/src/ws/`) with minimal codegen wiring for the new `Ws.serve_tls` API surface.

**TLS (SERVE-02)** follows the proven `snow_http_serve_tls` pattern: load PEM certs via `rustls-pki-types`, build an `Arc<ServerConfig>`, and wrap each accepted `TcpStream` in `rustls::StreamOwned<ServerConnection, TcpStream>`. However, a critical divergence from the HTTP TLS pattern exists: `StreamOwned` cannot be `try_clone()`-d like a plain `TcpStream`. The current plain-WS server splits the stream via `TcpStream::try_clone()` into a reader half (owned by the reader thread) and a write half (`Arc<Mutex<TcpStream>>`). For TLS, both reads and writes must go through a single `Arc<Mutex<StreamOwned>>`. The reader thread locks the mutex for each `read_frame()` call and releases between frames; the actor (for `Ws.send`) and reader thread (for pong/close echo) share the same mutex. This is the central architectural change.

**Heartbeat (BEAT-01 through BEAT-05)** requires the reader thread to send periodic Ping frames and track Pong responses. RFC 6455 Section 5.5.2-5.5.3 specifies that Pings can be sent at any time after the connection is established, and the peer MUST respond with a Pong echoing the payload. The implementation uses the existing 5-second read timeout cycle in the reader thread: on each timeout, check if a Ping is due (30s default interval) and if the Pong deadline has expired (10s default timeout). This operates inline with the read loop (BEAT-05) without requiring a separate timer thread.

**Fragmentation (FRAG-01 through FRAG-03)** requires the reader thread to reassemble continuation frames (opcode 0x0) into complete messages before pushing them to the actor's mailbox. Currently, `process_frame` in `close.rs` passes continuation frames through unmodified. The reassembly state machine tracks the initial opcode (text/binary), accumulates payload bytes, handles interleaved control frames (ping/pong/close), and enforces a 16 MiB maximum message size limit (close code 1009 on exceeded).

**Primary recommendation:** Implement all three features in `crates/snow-rt/src/ws/server.rs`. Introduce a `WsStream` enum (mirroring `HttpStream`) to abstract over plain `TcpStream` and `StreamOwned<ServerConnection, TcpStream>`. The reader thread loop gains a `FragmentState` struct for reassembly and timestamp tracking for heartbeat. The codegen pipeline needs 4 new entries: `snow_ws_serve_tls` runtime function, `known_functions` entry, intrinsic declaration, and `map_builtin_name` mapping.

## Standard Stack

### Core (already in codebase -- no new dependencies)
| Component | Version | Purpose | Why Standard |
|-----------|---------|---------|--------------|
| `rustls` | 0.23 | TLS encryption for wss:// | Already used by HTTP TLS server (Phase 56). Same `ServerConfig` + `ServerConnection` pattern. |
| `rustls-pki-types` | 1 | PEM certificate/key loading | Already used by `build_server_config` in `http/server.rs`. |
| `parking_lot::Mutex` | 0.12 | Shared stream access | Already used for `Arc<Mutex<TcpStream>>` in ws/server.rs. |
| `std::time::Instant` | stdlib | Ping/pong timing | Standard library -- no dependency needed. |
| `rand` | 0.9 | Random ping payload generation | Already a dependency in snow-rt (added Phase 59). |

### Supporting (already available)
| Component | Purpose | When to Use |
|-----------|---------|-------------|
| `std::io::{Read, Write}` | Generic stream trait bounds | All frame read/write functions already use these. |
| `base64` | 0.22 | (not needed for Phase 61 -- already present) | Only if payload encoding needed. |

### No New Dependencies Required

All required functionality is available through existing crate dependencies and the standard library. rustls 0.23 with `ServerConfig`, `ServerConnection`, and `StreamOwned` are already used in `http/server.rs`.

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| `Arc<Mutex<StreamOwned>>` for TLS | Separate reader/writer with channels | Channels add complexity and buffering. Mutex is simpler, already proven for the write half. Read-side locking is brief (one `read_frame` at a time). |
| Timestamp-based ping timing | Dedicated timer thread | Timer thread adds an OS thread per connection. Using the existing read timeout cycle is zero-cost. |
| Random ping payload | Monotonic counter payload | Counter is simpler but less collision-resistant. Random 4 bytes from `rand` is cheap and RFC-compliant. |

## Architecture Patterns

### Recommended Module Structure
```
crates/snow-rt/src/ws/
    mod.rs          # existing -- add WsStream enum, re-exports
    frame.rs        # existing (Phase 59) -- reduce MAX_PAYLOAD_SIZE to 16 MiB
    handshake.rs    # existing (Phase 59) -- unchanged
    close.rs        # existing (Phase 59) -- unchanged
    server.rs       # existing (Phase 60) -- add TLS serve, heartbeat, fragmentation
```

### Pattern 1: WsStream Enum (mirrors HttpStream)
**What:** An enum abstracting over plain TCP and TLS-wrapped streams, implementing `Read + Write`.
**When to use:** Everywhere the server needs to read/write frames on a connection that may be either plain or TLS.
**Source:** `crates/snow-rt/src/http/server.rs` lines 43-69

```rust
use rustls::{ServerConfig, ServerConnection, StreamOwned};

enum WsStream {
    Plain(TcpStream),
    Tls(StreamOwned<ServerConnection, TcpStream>),
}

impl Read for WsStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            WsStream::Plain(s) => s.read(buf),
            WsStream::Tls(s) => s.read(buf),
        }
    }
}

impl Write for WsStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            WsStream::Plain(s) => s.write(buf),
            WsStream::Tls(s) => s.write(buf),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            WsStream::Plain(s) => s.flush(),
            WsStream::Tls(s) => s.flush(),
        }
    }
}
```

**Critical constraint:** Unlike `TcpStream::try_clone()`, `StreamOwned` cannot be cloned. For TLS connections, the stream must be wrapped in `Arc<Mutex<WsStream>>` and shared between the reader thread and the actor. For plain connections, this same pattern works (no need for `try_clone` anymore). **This unifies the plain and TLS code paths.**

### Pattern 2: Unified Arc<Mutex<WsStream>> for Read and Write
**What:** Both reader thread and actor share a single `Arc<Mutex<WsStream>>` for all I/O.
**Why:** TLS state is not cloneable. A single mutex-protected stream is the only way to share between reader thread (reads + control frame writes) and actor (data frame writes via `Ws.send`).

**Trade-off analysis:**
- The reader thread holds the lock during `read_frame()`. This blocks `Ws.send()` during reads.
- In practice, `read_frame()` returns quickly when data is available (parse header + payload) or times out after 5 seconds.
- The 5-second read timeout means the worst case for `Ws.send()` blocking is 5 seconds if the reader thread is waiting on a timeout. **This is unacceptable.**
- **Solution:** For plain TCP connections, the reader thread extracts the raw `TcpStream` and sets read timeout on it directly. For TLS, we must set the read timeout on the **underlying** `TcpStream` before wrapping in `StreamOwned`, which propagates to TLS reads.
- Actually, the better solution: **use the existing pattern** for plain TCP (try_clone for read/write split) and **use Arc<Mutex<WsStream::Tls>>** only for TLS connections. This avoids the mutex contention problem for the common plain TCP case.

**Revised recommendation:** Keep `TcpStream::try_clone()` for plain connections (existing code). For TLS, wrap in `Arc<Mutex<WsStream>>` and set a short read timeout (5s as today) on the underlying `TcpStream` BEFORE wrapping in `StreamOwned`. The reader thread locks briefly for `read_frame`, releases the lock between frames, then the actor can lock for `Ws.send`.

**Even better revised recommendation:** For TLS, the reader thread should NOT hold the lock while blocking on read timeout. Instead:
1. Lock the mutex
2. Set `TcpStream` read timeout to a short value (100ms or the remaining heartbeat interval)
3. Call `read_frame`
4. Release the lock
5. If timeout: release lock, yield briefly, re-lock

Actually the simplest correct approach: since the underlying `TcpStream`'s read timeout is set BEFORE wrapping in `StreamOwned`, the `StreamOwned::read()` call will time out when the TCP socket times out. The reader thread locks, reads (may block up to read_timeout), unlocks. With a 5s read timeout, worst-case `Ws.send()` latency is 5 seconds. This is acceptable for a production server -- WebSocket send latency of up to 5 seconds during idle periods is fine. When the connection is active (frames arriving), `read_frame` returns almost instantly.

### Pattern 3: Heartbeat Inline with Read Timeout Cycle (BEAT-05)
**What:** Server ping/pong operates within the reader thread's existing read-timeout cycle.
**When to use:** Every WebSocket connection after the handshake.

```rust
struct HeartbeatState {
    last_ping_sent: Instant,
    last_pong_received: Instant,
    ping_interval: Duration,       // default 30s
    pong_timeout: Duration,        // default 10s
    pending_ping_payload: Option<[u8; 4]>,
}

impl HeartbeatState {
    fn new() -> Self {
        let now = Instant::now();
        Self {
            last_ping_sent: now,
            last_pong_received: now,
            ping_interval: Duration::from_secs(30),
            pong_timeout: Duration::from_secs(10),
            pending_ping_payload: None,
        }
    }

    fn should_send_ping(&self) -> bool {
        self.last_ping_sent.elapsed() >= self.ping_interval
    }

    fn is_pong_overdue(&self) -> bool {
        if let Some(_) = self.pending_ping_payload {
            self.last_ping_sent.elapsed() >= self.pong_timeout
        } else {
            false
        }
    }
}
```

The reader thread loop becomes:
```
loop {
    if shutdown { break; }
    if heartbeat.is_pong_overdue() {
        // Dead connection -- close with 1001 (Going Away)
        send_close(stream, 1001, "pong timeout");
        push_disconnect(mailbox);
        break;
    }
    if heartbeat.should_send_ping() {
        let payload = rand::random::<[u8; 4]>();
        write_frame(stream, Ping, &payload, true);
        heartbeat.last_ping_sent = Instant::now();
        heartbeat.pending_ping_payload = Some(payload);
    }
    match read_frame(stream) {
        Ok(frame) => {
            // On Pong: validate payload, update heartbeat
            // On other: dispatch as before
        }
        Err(timeout) => continue,
        Err(real_error) => break,
    }
}
```

### Pattern 4: Fragment Reassembly State Machine (FRAG-01, FRAG-02)
**What:** Accumulate continuation frames into a complete message, handling interleaved control frames.
**When to use:** Reader thread processes every frame before pushing to mailbox.

RFC 6455 Section 5.4 fragmentation rules:
1. First fragment: FIN=0, opcode = Text or Binary (establishes message type)
2. Continuation fragments: FIN=0, opcode = 0x0 (Continuation)
3. Final fragment: FIN=1, opcode = 0x0 (Continuation)
4. Control frames (ping/pong/close) MAY appear between fragments and MUST NOT be fragmented themselves
5. Cannot interleave fragments from different messages (no multiplexing without extensions)

```rust
struct FragmentState {
    /// The opcode of the first fragment (Text or Binary). None = not in a fragment sequence.
    initial_opcode: Option<WsOpcode>,
    /// Accumulated payload bytes from all fragments so far.
    buffer: Vec<u8>,
    /// Maximum total message size (default 16 MiB).
    max_message_size: usize,
}

impl FragmentState {
    fn new() -> Self {
        Self {
            initial_opcode: None,
            buffer: Vec::new(),
            max_message_size: 16 * 1024 * 1024, // 16 MiB
        }
    }

    fn is_assembling(&self) -> bool {
        self.initial_opcode.is_some()
    }
}
```

Frame processing with reassembly:
```
match frame.opcode {
    // Control frames: handle inline regardless of fragment state (FRAG-02)
    Ping | Pong | Close => {
        process_control_frame(stream, frame);
        // Do NOT interfere with fragment state
    }
    // Data frame with FIN=1 and non-continuation: unfragmented message
    Text | Binary if frame.fin && !frag.is_assembling() => {
        deliver_to_mailbox(frame);
    }
    // First fragment: FIN=0, opcode = Text|Binary
    Text | Binary if !frame.fin && !frag.is_assembling() => {
        frag.initial_opcode = Some(frame.opcode);
        frag.buffer = frame.payload;
        check_size_limit(&frag);
    }
    // Continuation fragment: FIN=0, opcode = Continuation
    Continuation if !frame.fin && frag.is_assembling() => {
        frag.buffer.extend_from_slice(&frame.payload);
        check_size_limit(&frag);
    }
    // Final fragment: FIN=1, opcode = Continuation
    Continuation if frame.fin && frag.is_assembling() => {
        frag.buffer.extend_from_slice(&frame.payload);
        check_size_limit(&frag);
        let complete = WsFrame {
            fin: true,
            opcode: frag.initial_opcode.take().unwrap(),
            payload: std::mem::take(&mut frag.buffer),
        };
        deliver_to_mailbox(complete);
    }
    // Protocol errors:
    Text | Binary if frag.is_assembling() => {
        // New message started while assembling previous -- protocol error
        send_close(stream, 1002, "new message during fragmented sequence");
    }
    Continuation if !frag.is_assembling() => {
        // Continuation without a preceding first fragment -- protocol error
        send_close(stream, 1002, "unexpected continuation frame");
    }
}
```

Size limit check (FRAG-03):
```rust
fn check_size_limit(frag: &FragmentState) -> Result<(), ()> {
    if frag.buffer.len() > frag.max_message_size {
        // Close code 1009 (Message Too Big)
        Err(())
    } else {
        Ok(())
    }
}
```

### Pattern 5: snow_ws_serve_tls (mirrors snow_http_serve_tls)
**What:** TLS WebSocket server entry point.
**Source:** `crates/snow-rt/src/http/server.rs` lines 473-557

The function signature follows the HTTP TLS pattern:
```rust
#[no_mangle]
pub extern "C" fn snow_ws_serve_tls(
    on_connect_fn: *mut u8,
    on_connect_env: *mut u8,
    on_message_fn: *mut u8,
    on_message_env: *mut u8,
    on_close_fn: *mut u8,
    on_close_env: *mut u8,
    port: i64,
    cert_path: *const SnowString,
    key_path: *const SnowString,
) { ... }
```

The accept loop:
1. Build `Arc<ServerConfig>` using `build_server_config` (can reuse the one from `http/server.rs` or duplicate)
2. For each accepted TCP stream:
   - Set read timeout BEFORE TLS wrapping
   - Create `ServerConnection::new(config.clone())`
   - Wrap in `StreamOwned::new(conn, tcp_stream)`
   - Wrap in `WsStream::Tls(tls_stream)`
   - Pack into `WsConnectionArgs` and spawn actor

### Anti-Patterns to Avoid
- **Trying to `try_clone()` a `StreamOwned`:** This does not compile. TLS connection state is not cloneable. Always use `Arc<Mutex<WsStream>>` for TLS connections.
- **Separate timer thread for heartbeat:** Adding an OS thread per connection for ping timing is wasteful. The read timeout cycle provides natural heartbeat checkpoints.
- **Buffering all fragments in memory without a size limit:** Without FRAG-03's 16 MiB limit, a malicious client can send an unbounded fragmented message to OOM the server.
- **Ignoring control frames during fragment reassembly:** RFC 6455 explicitly allows control frames between fragments. The reassembly state machine must handle ping/pong/close without corrupting the fragment buffer.
- **Using `process_frame` for Pong handling when heartbeat is active:** The existing `process_frame` in `close.rs` silently ignores Pong frames. For heartbeat, the reader thread needs to inspect Pong payloads to validate they match the sent Ping payload. Pong processing must happen before (or instead of) `process_frame`.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| TLS encryption | Custom TLS | `rustls 0.23` + `ServerConfig` + `StreamOwned` | Already integrated in HTTP TLS server. Same pattern, proven code. |
| Certificate loading | Custom PEM parser | `rustls_pki_types::PemObject` | Already used in `build_server_config`. Handles cert chains. |
| Frame codec | New frame parser | Existing `read_frame` / `write_frame` | Phase 59 built, handles all encodings. Generic over `Read`/`Write`. |
| Close handshake | New close logic | Existing `send_close` / `process_frame` | Phase 59 built, handles all close codes. |
| Random bytes for ping payload | Custom RNG | `rand::random::<[u8; 4]>()` | Already a dependency (Phase 59). 4 bytes is sufficient for payload matching. |

**Key insight:** Phase 61 is a *hardening* phase. The protocol layer (Phase 59) and actor integration (Phase 60) are complete. The work is: (a) wrap the accept loop with TLS configuration, (b) add state tracking to the reader thread for ping timing and fragment accumulation, and (c) modify the Pong handling path. No new protocol parsing is needed.

## Common Pitfalls

### Pitfall 1: TcpStream Read Timeout Must Be Set Before StreamOwned Wrapping
**What goes wrong:** Setting `set_read_timeout` on the `TcpStream` after it's been moved into `StreamOwned` is impossible -- `StreamOwned` owns the `TcpStream` and doesn't expose `set_read_timeout`.
**Why it happens:** `StreamOwned::new(conn, tcp_stream)` takes ownership. The `get_mut()` method returns `&mut TcpStream`, but `TcpStream::set_read_timeout` takes `&self`, so it CAN be called via `get_mut()`. However, setting timeout BEFORE wrapping is cleaner and matches the HTTP TLS pattern.
**How to avoid:** Always call `tcp_stream.set_read_timeout(...)` BEFORE `StreamOwned::new(conn, tcp_stream)`. This is exactly what `snow_http_serve_tls` does (server.rs line 519).
**Warning signs:** TLS reader thread never times out, heartbeat never fires, shutdown takes forever.

### Pitfall 2: Arc<Mutex<WsStream>> Contention Between Reader and Writer
**What goes wrong:** The reader thread holds the mutex while blocking on `read_frame` (up to the read timeout), starving `Ws.send()` calls from the actor.
**Why it happens:** `read_frame` calls `read_exact`, which blocks until data arrives or the read timeout fires.
**How to avoid:** Set a reasonable read timeout (5 seconds -- already the current value). In the worst case, `Ws.send()` waits 5 seconds. For most WebSocket applications, this is acceptable -- the server sends responses after receiving messages, and during idle periods a 5-second send delay is fine. If lower latency is needed in the future, the read timeout could be reduced (e.g., 1 second), at the cost of more timeout processing overhead.
**Warning signs:** `Ws.send()` calls appear laggy, especially when the client is idle.

### Pitfall 3: Pong Payload Validation (BEAT-03)
**What goes wrong:** Server sends Ping with payload `[0x12, 0x34, 0x56, 0x78]`, receives Pong with different payload (e.g., from an unsolicited Pong or a stale response), and incorrectly marks the connection as alive.
**Why it happens:** RFC 6455 allows unsolicited Pong frames. The server must verify the Pong payload matches the most recently sent Ping payload.
**How to avoid:** Store the last Ping payload in `HeartbeatState`. On receiving a Pong, compare payloads byte-by-byte. Only reset the pong timer if they match. Ignore unmatched Pongs (they may be unsolicited).
**Warning signs:** Dead connections not detected because stale/unsolicited Pongs reset the timer.

### Pitfall 4: Fragment State Corruption from Interleaved Control Frames
**What goes wrong:** A Ping arrives between fragment 1 and fragment 2 of a text message. The Ping is processed, but the fragment buffer is accidentally cleared or the initial opcode is lost.
**Why it happens:** Control frames and data frames share the same `process_frame` dispatch path. If fragmentation state is stored in the same struct that control frame handling modifies, state corruption can occur.
**How to avoid:** Keep `FragmentState` completely separate from control frame handling. Control frames (ping/pong/close) must NOT modify `FragmentState`. The dispatch logic should check: is this a control frame? If yes, handle it and return. If no, feed it to the fragment state machine.
**Warning signs:** Fragmented messages arrive corrupted. Text messages contain garbled data. Close code 1002 (protocol error) sent unexpectedly.

### Pitfall 5: 16 MiB Limit Must Be Checked Incrementally
**What goes wrong:** The size limit is only checked after the final fragment arrives, by which point the server has already buffered a potentially huge message.
**Why it happens:** Checking only at completion seems simpler.
**How to avoid:** Check `frag.buffer.len() + frame.payload.len()` BEFORE appending each fragment. If the total exceeds 16 MiB, send close code 1009 (Message Too Big) immediately, without buffering the oversized fragment.
**Warning signs:** Server OOM during fragment reassembly despite having a configured size limit.

### Pitfall 6: Continuation Frame UTF-8 Validation Timing
**What goes wrong:** A fragmented text message is reassembled, but UTF-8 validation happens on each fragment individually instead of the complete message. This rejects valid multi-byte UTF-8 sequences that span fragment boundaries.
**Why it happens:** The existing `process_frame` validates text frames immediately. For fragments, the initial fragment has opcode Text, but intermediate fragments have opcode Continuation.
**How to avoid:** UTF-8 validation for fragmented text messages must happen on the fully reassembled payload, not on individual fragments. The fragment state machine tracks that the initial opcode was Text, and after reassembly, validates the complete buffer with `std::str::from_utf8()`.
**Warning signs:** Multi-byte Unicode characters (emoji, CJK) that happen to span fragment boundaries cause spurious close code 1007 errors.

### Pitfall 7: build_server_config Duplication
**What goes wrong:** The TLS `build_server_config` function is duplicated between `http/server.rs` and `ws/server.rs`, creating maintenance burden.
**Why it happens:** The function is defined as a private function in `http/server.rs`.
**How to avoid:** Either (a) make `build_server_config` `pub(crate)` in `http/server.rs` and import it in `ws/server.rs`, or (b) extract it to a shared module (e.g., `crates/snow-rt/src/tls.rs`). Option (a) is simpler and follows the existing codebase style of direct cross-module imports.
**Warning signs:** Bug fixes in TLS config only applied to one server.

## Code Examples

### TLS Accept Loop
```rust
// Source: adapted from crates/snow-rt/src/http/server.rs lines 473-557
#[no_mangle]
pub extern "C" fn snow_ws_serve_tls(
    on_connect_fn: *mut u8,
    on_connect_env: *mut u8,
    on_message_fn: *mut u8,
    on_message_env: *mut u8,
    on_close_fn: *mut u8,
    on_close_env: *mut u8,
    port: i64,
    cert_path: *const SnowString,
    key_path: *const SnowString,
) {
    crate::actor::snow_rt_init_actor(0);

    let cert_str = unsafe { (*cert_path).as_str() };
    let key_str = unsafe { (*key_path).as_str() };

    let tls_config = match build_server_config(cert_str, key_str) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[snow-rt] Failed to load TLS certificates: {}", e);
            return;
        }
    };

    let addr = format!("0.0.0.0:{}", port);
    let listener = match TcpListener::bind(&addr) { ... };

    for tcp_stream in listener.incoming() {
        // Set read timeout BEFORE TLS wrapping
        tcp_stream.set_read_timeout(Some(Duration::from_secs(5))).ok();

        let conn = ServerConnection::new(Arc::clone(&tls_config)).unwrap();
        let tls_stream = StreamOwned::new(conn, tcp_stream);
        let ws_stream = WsStream::Tls(tls_stream);

        // Pack into args and spawn actor (same as plain WS)
    }
}
```

### Reader Thread with Heartbeat and Fragmentation
```rust
fn reader_thread_loop(
    stream: Arc<Mutex<WsStream>>,  // shared for TLS; or owned TcpStream for plain
    proc_arc: Arc<Mutex<Process>>,
    actor_pid: ProcessId,
    shutdown: Arc<AtomicBool>,
) {
    let mut heartbeat = HeartbeatState::new();
    let mut frag = FragmentState::new();

    loop {
        if shutdown.load(Ordering::SeqCst) { break; }

        // Check heartbeat timeout (BEAT-04)
        if heartbeat.is_pong_overdue() {
            let mut s = stream.lock();
            let _ = send_close(&mut *s, 1001, "pong timeout");
            drop(s);
            push_disconnect(&proc_arc, actor_pid);
            break;
        }

        // Send periodic ping (BEAT-01)
        if heartbeat.should_send_ping() {
            let payload = rand::random::<[u8; 4]>();
            let mut s = stream.lock();
            let _ = write_frame(&mut *s, WsOpcode::Ping, &payload, true);
            drop(s);
            heartbeat.last_ping_sent = Instant::now();
            heartbeat.pending_ping_payload = Some(payload);
        }

        // Read a frame
        let frame_result = {
            let mut s = stream.lock();
            read_frame(&mut *s)
        };

        match frame_result {
            Ok(frame) => {
                // Handle Pong for heartbeat (BEAT-03)
                if frame.opcode == WsOpcode::Pong {
                    if let Some(expected) = heartbeat.pending_ping_payload {
                        if frame.payload == expected {
                            heartbeat.last_pong_received = Instant::now();
                            heartbeat.pending_ping_payload = None;
                        }
                    }
                    continue; // Pong handled, don't pass to fragment/dispatch
                }

                // Handle control frames inline (FRAG-02)
                if matches!(frame.opcode, WsOpcode::Ping | WsOpcode::Close) {
                    let mut s = stream.lock();
                    match process_frame(&mut *s, frame) {
                        Ok(None) => continue,      // Ping -> Pong sent
                        Err(_) => { /* close */ break; }
                        _ => {}
                    }
                    continue;
                }

                // Fragment reassembly (FRAG-01, FRAG-03)
                match reassemble(&mut frag, frame) {
                    ReassembleResult::Complete(msg) => {
                        push_to_mailbox(&proc_arc, actor_pid, msg);
                    }
                    ReassembleResult::Accumulating => { /* waiting for more fragments */ }
                    ReassembleResult::TooLarge => {
                        let mut s = stream.lock();
                        let _ = send_close(&mut *s, 1009, "message too big");
                        drop(s);
                        push_disconnect(&proc_arc, actor_pid);
                        break;
                    }
                    ReassembleResult::ProtocolError(reason) => {
                        let mut s = stream.lock();
                        let _ = send_close(&mut *s, 1002, reason);
                        drop(s);
                        push_disconnect(&proc_arc, actor_pid);
                        break;
                    }
                }
            }
            Err(e) if e.contains("timed out") || e.contains("WouldBlock") => continue,
            Err(_) => {
                push_disconnect(&proc_arc, actor_pid);
                break;
            }
        }
    }
}
```

### Codegen Wiring for Ws.serve_tls
```rust
// In intrinsics.rs:
module.add_function("snow_ws_serve_tls", void_type.fn_type(&[
    ptr_type.into(), ptr_type.into(),  // on_connect fn/env
    ptr_type.into(), ptr_type.into(),  // on_message fn/env
    ptr_type.into(), ptr_type.into(),  // on_close fn/env
    i64_type.into(),                   // port
    ptr_type.into(), ptr_type.into(),  // cert_path, key_path
], false), Some(Linkage::External));

// In lower.rs known_functions:
self.known_functions.insert("snow_ws_serve_tls".to_string(),
    MirType::FnPtr(vec![
        MirType::Ptr, MirType::Ptr,  // on_connect fn/env
        MirType::Ptr, MirType::Ptr,  // on_message fn/env
        MirType::Ptr, MirType::Ptr,  // on_close fn/env
        MirType::Int,                // port
        MirType::Ptr, MirType::Ptr,  // cert_path, key_path (SnowString pointers)
    ], Box::new(MirType::Unit)));

// In map_builtin_name:
"ws_serve_tls" => "snow_ws_serve_tls".to_string(),
```

## Detailed Design Decisions

### Decision 1: WsStream Unification vs. Conditional Code Paths

**Option A (recommended): Introduce WsStream enum, unify both paths through Arc<Mutex<WsStream>>**
- PRO: Single code path for reader thread, heartbeat, fragmentation
- PRO: Simpler to reason about -- no conditional logic in the reader thread
- CON: Plain TCP connections now go through a mutex that wasn't needed before
- CON: Slight performance regression for plain TCP (mutex lock/unlock overhead)

**Option B: Keep plain TCP with try_clone, use Arc<Mutex<StreamOwned>> only for TLS**
- PRO: Zero overhead for plain TCP connections (existing behavior preserved)
- CON: Two code paths for reader thread loop (one with owned stream, one with Arc<Mutex>)
- CON: Heartbeat and fragmentation logic must work with both stream access patterns

**Recommendation:** Option A. The mutex overhead is negligible compared to network I/O. A single code path reduces bugs and simplifies testing. The reader thread and `Ws.send` rarely contend (reader blocks on timeout, sends happen in response to messages).

**Alternative to Option A:** If mutex contention is a concern, a hybrid approach: plain TCP still uses `try_clone()` for the read half (zero contention on reads), but the write half is always `Arc<Mutex<WsStream>>`. The reader thread gets a dedicated read stream, while writes go through the mutex. For TLS, reads also go through the mutex. This adds some complexity but optimizes the plain TCP path.

### Decision 2: Heartbeat Ping Payload Format

RFC 6455 specifies that Pong must echo the Ping payload exactly. Options:
- **4 random bytes:** Simple, collision-resistant. `rand::random::<[u8; 4]>()` is cheap.
- **Monotonic counter (u32):** Simpler, deterministic, testable. But predictable.
- **Timestamp bytes:** Encodes when the Ping was sent. Useful for RTT measurement.

**Recommendation:** 4 random bytes. Simple, satisfies BEAT-03 (payload validation), and avoids any timing side channels.

### Decision 3: build_server_config Sharing

The `build_server_config` function in `http/server.rs` (lines 78-93) is exactly what `ws/server.rs` needs. Options:
- **Make it `pub(crate)`:** Minimal change, direct import.
- **Extract to `crates/snow-rt/src/tls.rs`:** Cleaner architecture, but adds a new module.
- **Duplicate it:** Simplest, but maintenance burden.

**Recommendation:** Make it `pub(crate)` in `http/server.rs`. One-line change. Import as `use crate::http::server::build_server_config;`.

### Decision 4: MAX_PAYLOAD_SIZE Change (from 64 MiB to 16 MiB)

Prior decision [59-01] established 64 MiB as a safety cap with the note "Phase 61 will tighten to 16 MiB." The constant `MAX_PAYLOAD_SIZE` in `frame.rs` line 14 should be reduced to `16 * 1024 * 1024`. This affects ALL frames (not just fragmented messages), which is correct -- a single unfragmented frame exceeding 16 MiB should also be rejected.

### Decision 5: Pong Handling in process_frame

The existing `process_frame` in `close.rs` silently ignores Pong frames (line 111-112: `Ok(None)`). For heartbeat, the reader thread needs to inspect Pong payloads BEFORE calling `process_frame`. The recommended approach:

1. Reader thread checks `frame.opcode == WsOpcode::Pong` BEFORE calling `process_frame`
2. If Pong: validate payload against pending Ping payload, update heartbeat state
3. If not Pong: call `process_frame` as before

This avoids modifying `process_frame` (which is a pure protocol-level function) and keeps heartbeat logic in the reader thread.

### Decision 6: WsConnectionArgs for TLS

The existing `WsConnectionArgs` passes a `stream_ptr: usize` that points to a `TcpStream`. For TLS, it must point to a `WsStream` (either `WsStream::Plain(TcpStream)` or `WsStream::Tls(StreamOwned<...>)`). The actor entry function (`ws_connection_entry`) would change from:
```rust
let mut stream = unsafe { *Box::from_raw(args.stream_ptr as *mut TcpStream) };
```
to:
```rust
let mut stream = unsafe { *Box::from_raw(args.stream_ptr as *mut WsStream) };
```

This is a small but important change to unify the two paths.

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Plain TCP WebSocket only | TLS via rustls StreamOwned | Phase 61 (this phase) | Enables wss:// connections |
| No dead connection detection | Ping/pong heartbeat with timeout | Phase 61 (this phase) | Detects and cleans up dead connections |
| 64 MiB payload safety cap | 16 MiB production limit | Phase 61 (this phase) | Matches FRAG-03 requirement |
| Continuation frames passed through | Full reassembly with size limits | Phase 61 (this phase) | Supports fragmented messages |

**Deprecated/outdated:**
- The 64 MiB `MAX_PAYLOAD_SIZE` (Phase 59) is superseded by the 16 MiB production limit.
- The "Continuation opcode passed through" behavior (Phase 59 decision [59-02]) is superseded by full reassembly.

## Open Questions

1. **Mutex contention severity for TLS write path**
   - What we know: The reader thread holds `Arc<Mutex<WsStream>>` during `read_frame`, which blocks for up to the read timeout (5s). During this time, `Ws.send()` from the actor side is blocked.
   - What's unclear: Whether this 5-second worst-case latency matters for real WebSocket applications. Most sends happen in response to received messages (low contention), but broadcast scenarios (server pushing to many clients) could be affected.
   - Recommendation: Accept the 5-second worst case for now. It matches the existing read timeout. If this becomes a problem, the timeout can be reduced to 1 second, or the TLS stream can be split using a channel-based architecture (read thread reads into a channel, write thread writes from a channel, both access the stream exclusively).

2. **Whether to expose heartbeat configuration to Snow programs**
   - What we know: Requirements specify "configurable interval (default 30s)" and "configurable missed Pong threshold (default 10s)." The current API is `Ws.serve(handler, port)`.
   - What's unclear: Whether configurability means runtime-configurable by the Snow program or compile-time constants.
   - Recommendation: For Phase 61, use compile-time defaults (30s ping interval, 10s pong timeout). The constants can be made configurable in a future phase by adding optional parameters to `Ws.serve` or a configuration function.

3. **Whether snow_ws_serve should also get heartbeat and fragmentation (not just snow_ws_serve_tls)**
   - What we know: The requirements list heartbeat (BEAT-*) and fragmentation (FRAG-*) without specifying TLS-only. These are production-hardening features that apply to both plain and TLS connections.
   - What's unclear: Whether refactoring `snow_ws_serve` to use the unified `WsStream` / heartbeat / fragmentation is in scope.
   - Recommendation: Yes, apply all hardening to both code paths. Using the `WsStream` enum unifies both. This is the primary motivation for Decision 1 (unification).

## Sources

### Primary (HIGH confidence)
- **RFC 6455** (The WebSocket Protocol) -- https://datatracker.ietf.org/doc/html/rfc6455
  - Section 5.4: Fragmentation -- first fragment (FIN=0, data opcode), continuations (FIN=0, opcode 0x0), final (FIN=1, opcode 0x0), interleaved control frames allowed
  - Section 5.5.2: Ping frames -- MAY be sent at any time after connection established
  - Section 5.5.3: Pong frames -- MUST echo Ping payload, unsolicited Pongs allowed, only need to respond to most recent Ping
  - Section 7.4.1: Close code 1009 -- Message Too Big

- **Snow codebase** (direct reading, HIGH confidence):
  - `crates/snow-rt/src/ws/server.rs` -- Current WebSocket server with reader thread bridge, `Arc<Mutex<TcpStream>>` for writes, 5s read timeout
  - `crates/snow-rt/src/ws/frame.rs` -- Frame codec, `MAX_PAYLOAD_SIZE = 64 MiB`, `WsOpcode::Continuation` recognized
  - `crates/snow-rt/src/ws/close.rs` -- `process_frame` passes continuation through, Pong silently ignored
  - `crates/snow-rt/src/http/server.rs` -- `HttpStream` enum pattern, `build_server_config`, `snow_http_serve_tls` accept loop
  - `crates/snow-rt/Cargo.toml` -- rustls 0.23, rustls-pki-types 1, rand 0.9 already dependencies
  - `crates/snow-codegen/src/mir/lower.rs` -- `known_functions`, `STDLIB_MODULES` includes "Ws", `map_builtin_name`
  - `crates/snow-codegen/src/codegen/intrinsics.rs` -- LLVM function declarations for WS functions

- **rustls docs** -- https://docs.rs/rustls/latest/rustls/struct.StreamOwned.html
  - `StreamOwned` implements Read + Write, has `into_parts()`, `get_ref()`, `get_mut()`, no Clone

### Secondary (MEDIUM confidence)
- **rustls/tokio-rustls Issue #84** -- Confirmed StreamOwned cannot be split into read/write halves like TcpStream. The Arc<Mutex> pattern is the standard approach for synchronous rustls.
- **RFC 6455 Pong handling** -- Multiple sources confirm only most recent Ping needs Pong response, unsolicited Pongs are valid

### Tertiary (LOW confidence)
- None -- all critical claims verified against RFC spec, crate docs, or existing codebase.

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- all components exist in the codebase, no new dependencies needed. TLS pattern is directly lifted from HTTP TLS server.
- Architecture: HIGH -- WsStream enum mirrors HttpStream. Heartbeat inline with read timeout is a straightforward state machine. Fragment reassembly is fully specified by RFC 6455.
- Pitfalls: HIGH -- TLS stream splitting limitation verified against rustls docs. Mutex contention analyzed with concrete timing. Fragment/control interleaving rules from RFC spec.
- Codegen wiring: HIGH -- exact precedent from `snow_http_serve_tls` in intrinsics.rs, lower.rs, builtins.rs

**Research date:** 2026-02-12
**Valid until:** 2026-03-12 (extremely stable domain -- RFC 6455 is from 2011, rustls 0.23 API is stable)
