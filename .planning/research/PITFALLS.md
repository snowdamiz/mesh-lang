# Domain Pitfalls: Connection Pooling, TLS/SSL, Transactions, and Struct-to-Row Mapping

**Domain:** Adding production backend features (connection pooling, TLS/SSL, database transactions, struct-to-row mapping) to an actor-based compiled language with cooperative scheduling, per-actor GC, and opaque u64 connection handles
**Researched:** 2026-02-12
**Confidence:** HIGH (based on direct Snow codebase analysis of pg.rs, sqlite.rs, scheduler.rs, heap.rs, stack.rs, server.rs, gc.rs, actor/mod.rs; PostgreSQL wire protocol documentation; TLS library comparison research; Erlang/BEAM transaction patterns; connection pooling literature)

**Scope:** This document covers pitfalls specific to adding 4 production features to the existing Snow runtime. Each pitfall is analyzed against Snow's current architecture: corosensei coroutines (!Send, thread-pinned) on M:N work-stealing scheduler, per-actor mark-sweep GC with conservative stack scanning, opaque u64 connection handles via Box::into_raw, extern "C" runtime ABI, deriving infrastructure for code generation, and the existing PgConn/SqliteConn wrapper pattern.

**Relationship to prior research:** The v2.0 PITFALLS.md (2026-02-10) covered foundational pitfalls for SQLite FFI, PostgreSQL wire protocol basics, deriving(Json), and HTTP features. This document covers SUBSEQUENT milestone pitfalls that arise when adding production-grade features ON TOP of the v2.0 foundation. There is intentional minimal overlap; references to v2.0 pitfalls are noted where relevant.

---

## Critical Pitfalls

Mistakes that cause data corruption, resource exhaustion, deadlocks, or require architectural rewrites.

---

### Pitfall 1: Connection Pool Checkout Blocks Worker Thread, Causing Cascading Starvation

**What goes wrong:**
Snow's M:N scheduler uses corosensei coroutines that are `!Send` and thread-pinned (scheduler.rs line 9: "yielded coroutines stay in the worker's local suspended list and are resumed on the same thread"). When a connection pool has no available connections, the requesting actor must wait. If this wait is implemented as a blocking spin-loop or mutex wait, the entire OS worker thread blocks. Every other actor pinned to that worker thread -- including actors that HOLD pool connections and are trying to return them -- cannot make progress. This creates a deadlock: actor A waits for a connection, blocking the thread where actor B (which holds a connection and needs to run to return it) is suspended.

With N worker threads and a pool of M connections where M < N, only M worker threads can have active connections. If all M connections are held by actors on different worker threads, and those actors yield (e.g., waiting for a DB response), the remaining N-M workers are fine. But if multiple actors on the SAME worker thread request connections, and the pool is exhausted, the thread deadlocks because the actor holding the connection on that thread cannot be resumed.

**Why it happens:**
- Corosensei coroutines are `!Send` -- they cannot migrate between threads (scheduler.rs line 389: "suspended: Vec<(ProcessId, CoroutineHandle)>")
- The worker loop (scheduler.rs lines 393-551) resumes suspended coroutines in Phase 1, then picks up new spawn requests in Phase 2. If a coroutine blocks (not yields), Phase 1 never completes for remaining coroutines on that thread
- Snow has no mechanism to detect "this actor is blocked on a resource held by another actor on the same thread"
- The existing pattern for blocking I/O (HTTP server, DB queries) accepts blocking because each operation is short-lived. Pool checkout waits can be indefinite

**Consequences:**
- Complete scheduler deadlock when pool is exhausted and connections are held across yield points
- Under moderate load: intermittent hangs where some HTTP requests never complete
- Hard to diagnose: looks like a "slow query" but is actually a scheduling deadlock
- Cannot be reproduced with low concurrency; manifests only under production load

**Prevention:**
1. **Implement pool checkout as actor message passing, not blocking wait.** The pool should be a dedicated service actor. Requesting actors send a `{checkout, self()}` message and block on `receive` (which yields to the scheduler via ProcessState::Waiting). The pool actor sends back a connection handle when one becomes available. This converts a blocking wait into a cooperative yield, allowing other actors on the same worker thread to continue.
2. **Never hold a pool connection across a yield point without careful consideration.** If an actor checks out a connection, executes a query (which blocks the thread during TCP I/O), and then yields for another reason before checking in, the connection is held while the actor is suspended. Keep connection hold times minimal: checkout, execute, checkin in a tight sequence.
3. **Set a maximum checkout timeout.** If no connection is available within N milliseconds, return an error to the calling actor rather than waiting indefinitely. This prevents cascading starvation.
4. **Size the pool to match worker thread count.** A pool of N connections for N worker threads ensures that at least one connection is always available per thread. Over-provisioning prevents contention.

**Detection:**
- Application hangs under load with all worker threads in Phase 1 of the worker loop
- `strace` shows threads sleeping in futex/read (blocked on pool checkout or DB I/O)
- Reducing pool size reliably triggers the deadlock
- Adding more worker threads temporarily alleviates the issue (until those threads also exhaust the pool)

**Phase:** Connection pooling architecture. Must be designed as message-passing from the start. A blocking checkout API cannot be retrofitted to cooperative scheduling.

---

### Pitfall 2: TLS Handshake Blocks Worker Thread for 100-500ms, Starving Co-located Actors

**What goes wrong:**
A TLS handshake involves multiple network round-trips (ClientHello -> ServerHello -> Certificate -> KeyExchange -> Finished) plus expensive cryptographic operations (RSA/ECDSA signature verification, key derivation). On a typical network, this takes 50-200ms; with certificate chain validation and OCSP stapling, it can reach 500ms. During this entire time, the OS worker thread is blocked in synchronous I/O (the rustls or native-tls handshake calls `TcpStream::read`/`write` which are blocking).

Snow's PostgreSQL driver currently uses blocking `TcpStream` (pg.rs line 19: `use std::net::TcpStream`). Adding TLS wraps this stream, but the handshake is a single blocking operation that takes orders of magnitude longer than a typical TCP connect (which is ~1-5ms for local connections). The HTTP server accept loop (server.rs line 224: `for request in server.incoming_requests()`) already blocks, but HTTP request handling is fast. A TLS accept for HTTPS adds the handshake cost to every new connection.

With the M:N scheduler, a 200ms TLS handshake on one worker thread means all actors pinned to that thread are starved for 200ms. If the server handles 100 connections/second across 4 worker threads, each worker handles ~25 connections/second. A single 200ms handshake blocks 5 connection slots on that thread.

**Why it happens:**
- `std::net::TcpStream` is synchronous/blocking. rustls wraps it with `rustls::StreamOwned<ClientConnection, TcpStream>` which inherits the blocking behavior
- Snow has no non-blocking I/O integration with the scheduler. The reduction check (mod.rs lines 160-191) only triggers yields at Snow-code yield points, not during Rust runtime I/O
- TLS handshakes cannot be split into yield-compatible chunks without a full async I/O redesign
- Unlike database queries (which are rare), HTTPS accept-loop TLS handshakes occur on EVERY new connection

**Consequences:**
- HTTPS server throughput drops dramatically compared to HTTP (not just from crypto overhead, but from scheduler starvation)
- Actors unrelated to HTTP (e.g., timer-based workers, database actors) experience 200ms latency spikes
- Under connection storms (client reconnect after network blip), all worker threads simultaneously block on TLS handshakes, freezing the entire runtime

**Prevention:**
1. **Perform TLS handshakes on dedicated OS threads, outside the actor scheduler.** The HTTPS accept loop should run on a separate thread pool (similar to how `snow_timer_send_after` in mod.rs line 463 spawns OS threads for timers). After the handshake completes, the established TLS stream is passed to an actor for request handling. This isolates handshake latency from the scheduler.
2. **For PostgreSQL TLS (client-side), accept the blocking cost but document it.** PG connections are long-lived and pooled, so the TLS handshake happens once per connection, not per query. The one-time ~200ms cost at pool initialization is acceptable.
3. **For HTTPS (server-side), use a tiered architecture:** OS thread pool accepts TCP connections and performs TLS handshakes; completed connections are dispatched to actor-per-request on the Snow scheduler. This mirrors BEAM's approach where NIF-level operations run on dirty schedulers.
4. **Set aggressive TLS handshake timeouts (5-10 seconds).** A slow or malicious client performing a deliberately slow handshake (Slowloris-style) should be disconnected before it starves a worker thread for too long.

**Detection:**
- HTTPS throughput is 5-10x lower than HTTP on the same hardware
- Unrelated actors experience periodic 100-500ms latency spikes correlated with HTTPS connection rate
- `perf` or profiling shows worker threads spending significant time in `rustls::conn::ConnectionCommon::complete_io`

**Phase:** TLS implementation. The thread architecture (where handshakes run) must be decided before writing any TLS code.

---

### Pitfall 3: Transaction Left Open After Actor Crash Poisons Pooled Connection

**What goes wrong:**
An actor calls `BEGIN` on a PostgreSQL connection, executes some queries, but crashes (panic, linked actor death, supervisor kill) before calling `COMMIT` or `ROLLBACK`. The connection is returned to the pool in an in-transaction state. The PostgreSQL server reports this via ReadyForQuery status byte `T` (in transaction) instead of `I` (idle). The next actor that checks out this connection unknowingly inherits the open transaction. Its queries execute within the crashed actor's transaction context. If the previous transaction had an error, PostgreSQL puts the connection in the `E` (failed transaction) state, and ALL subsequent queries fail with "current transaction is aborted, commands ignored until end of transaction block."

The existing PG driver (pg.rs lines 770-799, snow_pg_execute) reads messages until ReadyForQuery but does NOT check the transaction status byte. It silently discards ReadyForQuery's body content. This means the runtime has no way to detect that a connection is in a dirty transaction state.

**Why it happens:**
- Snow actors follow the "let it crash" philosophy (Erlang-style). Crashes are expected and handled by supervisors. But database connections are stateful external resources that do NOT reset on actor restart.
- The current pg.rs implementation ignores the ReadyForQuery status byte (pg.rs line 789: `b'Z' => break`), providing no mechanism to check transaction state.
- Connection pooling amplifies the problem: without pooling, crashed connections are simply dropped (the TCP connection closes, PostgreSQL auto-rollbacks). With pooling, the connection survives the actor crash.
- There is no terminate_callback equivalent for "return this connection to the pool with cleanup." The terminate_callback (mod.rs lines 557-575) can be set, but the connection handle (an opaque u64) must be explicitly passed to it.

**Consequences:**
- Silent data corruption: next actor's queries run in wrong transaction context
- Mysterious "current transaction is aborted" errors on seemingly valid queries
- Debugging nightmare: the error appears in actor B but was caused by actor A's crash
- Pool connections become permanently poisoned (every subsequent user gets the error) until the connection is dropped and replaced
- Under high crash rates (expected in development), pool becomes entirely poisoned within minutes

**Prevention:**
1. **Pool must validate connection state on checkin AND checkout.** On every checkin (connection return), send a `ROLLBACK` if ReadyForQuery status is `T` or `E`. On checkout, verify status is `I` (idle). If not, send `ROLLBACK` and re-verify. If still not idle, discard the connection and create a new one.
2. **Parse the ReadyForQuery status byte.** Modify the message-reading loop in pg.rs to extract and store the transaction status from ReadyForQuery messages. The status byte is body[0]: `I` = idle, `T` = in transaction, `E` = failed transaction. Store this on PgConn for pool management.
3. **Wrap connection checkout/checkin in actor lifecycle.** Use Snow's terminate_callback to ensure connections are always returned to the pool, even on crash. The pool's checkin handler then performs the ROLLBACK cleanup.
4. **Consider a "connection wrapper" that automatically issues ROLLBACK on Drop.** In the Rust runtime, wrap the pool checkout in an RAII guard. When the guard is dropped (including on unwind from catch_unwind in the actor entry), it returns the connection to the pool with cleanup. This matches the connection_handler_entry pattern in server.rs line 185 where catch_unwind wraps the handler.

**Detection:**
- "current transaction is aborted" errors that appear without any preceding error in the same actor
- Queries silently execute in a transaction context when no BEGIN was issued
- Pool connections accumulate in `T` or `E` state over time
- Restarting the application (which drops all connections) temporarily fixes the issue

**Phase:** Connection pooling + transaction support. These two features are deeply coupled and MUST be designed together.

---

### Pitfall 4: Opaque u64 Connection Handle Becomes Dangling Pointer After Pool Reclaims Connection

**What goes wrong:**
Snow currently represents connections as opaque u64 handles created via `Box::into_raw(conn) as u64` (pg.rs line 711, sqlite.rs line 162). The Snow program stores this u64 as a regular integer. With connection pooling, the lifecycle changes: the pool owns the connection, and the u64 handle is a loan, not ownership. If the pool reclaims a connection (timeout, validation failure, pool resize) while an actor still holds the u64 handle, the actor has a dangling pointer. Calling `snow_pg_execute` with this handle dereferences freed memory (`let conn = &mut *(conn_handle as *mut PgConn)` at pg.rs line 751).

The GC makes this worse: the u64 handle is an integer, so the GC never traces it (by design -- pg.rs line 10: "The GC never traces integer values"). This means there is no GC-based mechanism to detect that the underlying connection was freed. The handle looks valid (it's just a number) but points to deallocated memory.

**Why it happens:**
- The current ownership model is simple: one actor creates a connection, uses it, closes it. The u64 handle has a 1:1 relationship with the PgConn Box.
- Connection pooling introduces shared ownership: the pool owns the connection, actors borrow it. But the u64 handle mechanism has no borrow tracking.
- There is no reference counting or generation counter on the handle. A recycled handle (same u64 value after Box deallocation and reallocation) could point to a completely different connection.
- The Box::into_raw/Box::from_raw pattern is fundamentally incompatible with pooling because from_raw takes ownership and deallocates on drop.

**Consequences:**
- Use-after-free crashes (SIGSEGV in snow_pg_execute or snow_pg_query)
- If memory is reallocated, the handle points to a different object -- queries go to the wrong database connection
- Intermittent: depends on allocator behavior and timing of pool reclamation vs. actor usage
- Security risk: queries intended for one database could be sent to another if handle reuse occurs

**Prevention:**
1. **Replace raw pointer handles with generation-counted slot IDs.** Instead of `Box::into_raw`, use a global `Vec<Option<PgConn>>` (or `SlotMap`) where the u64 handle encodes both the slot index and a generation counter. When a connection is reclaimed, the slot's generation is incremented. Accessing a stale handle (wrong generation) returns an error instead of undefined behavior. This is the standard pattern for entity-component systems and resource handles in game engines.
2. **Never expose raw pool connections to Snow code.** The pool should return a "checkout token" (an opaque u64 that maps to a pool slot), not the raw PgConn pointer. All DB operations go through the pool, which validates the token before accessing the connection. The Snow-level API becomes `Pool.query(pool_handle, token, sql, params)` instead of `Pg.query(conn_handle, sql, params)`.
3. **Use message-passing instead of shared handles.** The pool actor owns all connections. Snow code sends query messages to the pool actor, which executes them on an available connection and sends results back. The Snow program never has a connection handle at all. This is the safest approach but changes the API significantly.
4. **If retaining direct handles, add a `is_valid(handle)` check at every FFI boundary.** Every `snow_pg_execute`/`snow_pg_query` call should validate the handle before dereferencing. This converts SIGSEGV into a Snow-level error.

**Detection:**
- SIGSEGV in `snow_pg_execute` or `snow_pg_query` with stack trace pointing to the `*(conn_handle as *mut PgConn)` dereference
- Database operations randomly fail with "connection closed" or produce garbled results
- Problem worsens under high concurrency (more pool recycling events)
- ASAN builds catch use-after-free immediately

**Phase:** Connection pooling handle design. The handle abstraction must be redesigned BEFORE implementing pooling. Cannot be retrofitted.

---

### Pitfall 5: TLS Session State is Not GC-Visible, Causing Use-After-Free on GC-Triggered Collection

**What goes wrong:**
When TLS wraps a TCP stream, the resulting object (`rustls::StreamOwned<ClientConnection, TcpStream>` or equivalent) contains internal buffers, session keys, and cipher state allocated on the Rust/system heap. Snow stores the connection as an opaque u64 handle (Box::into_raw pattern). The GC never sees this handle as a pointer (it's an integer). This is correct and intentional for the connection handle itself.

However, the problem occurs when Snow-level values (SnowStrings, query parameters) are passed to functions that operate on the TLS stream. During a TLS write, the data must be encrypted before sending. If the encryption involves multiple steps and a GC collection happens between preparing the plaintext (from a SnowString) and encrypting it, and the SnowString was the last reference on the stack, the GC could collect the SnowString. The TLS layer then encrypts freed memory.

This is a variant of the v2.0 Pitfall 16 (SnowString lifetime during PG wire protocol send), but TLS adds an extra layer: the plaintext data passes through the TLS encrypt buffer before reaching the TCP socket, creating a longer window where the SnowString must remain alive.

**Why it happens:**
- Conservative GC scans the coroutine stack (heap.rs mark_from_roots, lines 401-448). If a SnowString pointer is only held in a Rust local variable that the compiler optimizes away before the TLS write completes, the GC cannot see it.
- TLS encryption is not atomic from the GC's perspective: prepare data -> encrypt -> send is multiple function calls
- The current pg.rs already copies data into Rust Vec<u8> buffers before sending (e.g., write_parse at pg.rs line 142), which is safe. The risk is that TLS wrapping adds the temptation to pass SnowString data directly to the TLS stream without copying

**Consequences:**
- Encrypted garbage sent over the wire, causing TLS protocol errors on the receiving end
- Intermittent: only occurs when GC pressure coincides with TLS writes
- Manifests as "TLS alert: bad_record_mac" or "TLS alert: decode_error" from the remote peer
- Extremely hard to diagnose: looks like a network/TLS bug, not a GC bug

**Prevention:**
1. **Maintain the existing copy-to-Vec<u8>-buffer pattern.** The current pg.rs already builds complete wire protocol messages in a `Vec<u8>` (system heap) before writing to the stream. When adding TLS, ensure the TLS stream receives data from these Rust-owned buffers, NEVER directly from GC-managed SnowString pointers. This is already the natural pattern; the risk is breaking it during refactoring.
2. **Encapsulate TLS writes in a single Rust function that takes &[u8].** The function should accept a byte slice (already copied from SnowString), pass it through TLS encryption, and write to the socket. No GC-managed pointer should be reachable from parameters or locals during the TLS write.
3. **Never yield (reduction_check) between copying SnowString data and completing the TLS write.** The pg.rs functions are called from actors, and reduction_check is inserted by the compiler at loop back-edges and function calls. Since the wire protocol functions are Rust extern "C" functions (not Snow code), the compiler does NOT insert reduction checks inside them. This is already safe; ensure TLS functions follow the same pattern (Rust code, no Snow callbacks).

**Detection:**
- TLS alert errors ("bad_record_mac", "decode_error") under GC pressure
- ASAN/MSAN detecting reads of freed memory in TLS encryption paths
- Works fine under low allocation pressure; fails under sustained load
- PostgreSQL server logs showing malformed messages after TLS upgrade

**Phase:** TLS stream wrapping. Follow the existing buffer-copy pattern in pg.rs. Primarily a code-review concern, not an architectural decision.

---

## Moderate Pitfalls

Mistakes that cause incorrect behavior, performance problems, or developer confusion, but are fixable without architectural changes.

---

### Pitfall 6: Transaction API Without Explicit Scoping Leads to Transaction Leaks

**What goes wrong:**
A naive transaction API exposes `Pg.begin(conn)`, `Pg.commit(conn)`, `Pg.rollback(conn)` as separate calls. Users write:

```
let conn = Pool.checkout(pool)
Pg.begin(conn)
let result = Pg.query(conn, "SELECT ...", [])
// ... forgot to commit or rollback
Pool.checkin(pool, conn)
```

The connection is returned to the pool with an open transaction. Even without a crash, simple programmer forgetfulness leaves transactions open. Every code path (including error branches) must explicitly commit or rollback. In a language without RAII or try-finally (Snow does not have either), this is extremely error-prone.

The problem is compounded by Snow's pattern matching on Result types. If a query returns `Err`, the programmer's error-handling branch might return early without committing or rolling back:

```
match Pg.query(conn, sql, []) do
  Ok(rows) -> Pg.commit(conn)
  Err(msg) -> log(msg)  // forgot Pg.rollback(conn) here!
end
```

**Why it happens:**
- Snow has no RAII, try-finally, or defer mechanism for guaranteed cleanup
- The actor model encourages message-passing patterns where connections are used across multiple message-handling iterations, making scope boundaries unclear
- Erlang/BEAM has the same problem; the standard solution is a transaction wrapper function, not separate begin/commit/rollback calls

**Consequences:**
- Transaction leaks under normal operation (not just crashes)
- Silent connection poisoning in the pool (see Pitfall 3)
- Hard to test: individual tests pass (they commit explicitly), but complex flows miss branches
- PostgreSQL eventually reports "too many idle in transaction" connections

**Prevention:**
1. **Provide a `Pg.transaction(conn, fn)` higher-order function as the PRIMARY API.** The function takes a callback, wraps it in BEGIN/COMMIT, and issues ROLLBACK if the callback returns Err or if the actor panics. The separate begin/commit/rollback calls should exist as escape hatches but be documented as advanced/unsafe.
2. **The transaction wrapper should use catch_unwind (or equivalent) at the Rust level.** Similar to connection_handler_entry in server.rs line 185, wrap the callback in panic recovery. On panic, issue ROLLBACK before re-raising. This handles actor crashes that don't kill the process but do unwind the stack (catch_unwind in scheduler.rs).
3. **The pool checkin handler should ALWAYS issue ROLLBACK if the connection is in-transaction.** This is the safety net for all the cases where the API-level wrapper doesn't catch the leak.
4. **Consider a future `with` or `defer` language feature** that provides scoped resource management. This is a compiler feature, not a runtime feature, and should be tracked as a language enhancement separate from this milestone.

**Detection:**
- PostgreSQL `pg_stat_activity` shows connections in "idle in transaction" state for long periods
- Pool health checks detect connections with ReadyForQuery status `T`
- Memory leaks from held transaction locks preventing VACUUM

**Phase:** Transaction API design. Design the `transaction(conn, fn)` wrapper before exposing any transaction functionality.

---

### Pitfall 7: struct-to-row Mapping Column Name Mismatch Due to Snake/Camel Case Conventions

**What goes wrong:**
Snow uses PascalCase for types and snake_case for fields (following Haskell/ML conventions). PostgreSQL uses lowercase identifiers by default (case-insensitive unless quoted). A Snow struct:

```
struct UserProfile do
  firstName :: String
  lastName :: String
  createdAt :: String
end deriving(Row)
```

would try to match columns named `firstName`, `lastName`, `createdAt`. But PostgreSQL columns are typically `first_name`, `last_name`, `created_at`. The struct field names don't match the column names, causing deserialization to fail silently (missing fields become empty strings in the current Map<String, String> return type) or loudly (field not found errors in a typed mapping).

The existing `deriving(Json)` infrastructure (typeck/infer.rs lines 2003-2060) maps struct field names directly to JSON keys. If `deriving(Row)` follows the same pattern, the naming mismatch between Snow fields and SQL columns is inevitable.

**Why it happens:**
- Snow and SQL have different naming conventions
- The deriving infrastructure maps field names 1:1 without transformation
- JSON keys typically match the programming language's naming convention (because the same language generates them), but SQL columns follow database conventions
- There is no attribute or annotation system in Snow to override the mapping (e.g., `@column("first_name")` does not exist)

**Consequences:**
- Every struct-to-row mapping requires column aliases in SQL: `SELECT first_name AS firstName, ...`
- Boilerplate SQL for every query defeats the purpose of automatic mapping
- Users blame the ORM/mapping layer for "not working" when it's a naming mismatch
- Inconsistency between deriving(Json) (which works with camelCase keys) and deriving(Row) (which doesn't work with snake_case columns)

**Prevention:**
1. **Auto-convert field names to snake_case for column matching.** When `deriving(Row)` looks up a column in the result set, convert the struct field name to snake_case first. `firstName` becomes `first_name`, `createdAt` becomes `created_at`. This matches the conventions of Diesel (Rust), SQLAlchemy (Python), and ActiveRecord (Ruby).
2. **Fall back to exact match if snake_case match fails.** This handles cases where the SQL column actually IS in camelCase (e.g., PostgreSQL with quoted identifiers).
3. **Support case-insensitive matching as the default.** PostgreSQL folds unquoted identifiers to lowercase. `SELECT firstName FROM ...` actually returns a column named `firstname`. Case-insensitive matching handles this transparently.
4. **Consider adding an annotation system in a future milestone.** `@column("first_name")` or `@json("firstName")` attributes on struct fields would give explicit control. This is a parser/typechecker feature and should not block the initial deriving(Row) implementation.

**Detection:**
- Struct fields are always empty/default after deserialization from database rows
- Queries work only when column aliases match struct field names exactly
- deriving(Row) works perfectly with SELECT * from tables where column names happen to match Snow conventions

**Phase:** struct-to-row mapping implementation. The naming convention decision should be made at design time.

---

### Pitfall 8: Connection Pool Size Configuration Interacts With Worker Thread Count in Non-Obvious Ways

**What goes wrong:**
Snow's scheduler defaults to one worker thread per CPU core (scheduler.rs lines 117-123). The connection pool has a configurable maximum size. If pool_size > worker_thread_count, excess connections are wasted (no thread available to use them). If pool_size < worker_thread_count, some workers will always block waiting for connections. The "right" pool size depends on the workload: CPU-bound Snow code benefits from more workers with fewer connections; I/O-bound database code benefits from more connections with potentially fewer workers.

The interaction is worse because Snow's cooperative scheduling means a single worker thread can multiplex many actors. If each actor needs a connection, pool_size must scale with actor count, not thread count. But actor count is dynamic and potentially unbounded (HTTP server spawns one actor per request).

PostgreSQL also has its own connection limit (default: 100). If pool_size exceeds PG's max_connections, connection attempts fail with "too many connections." This is a common production problem.

**Why it happens:**
- No guidance for pool sizing relative to scheduler configuration
- The relationship between Snow scheduler threads, Snow actors, and database connections is non-obvious
- Default configurations (pool_size = 5, workers = num_cpus) may work on a developer machine but fail in production
- PostgreSQL's max_connections is a shared resource across all clients, not just Snow

**Consequences:**
- Under-provisioned pool: connection checkout timeouts, application slowness
- Over-provisioned pool: wasted PostgreSQL connections, potential "too many connections" errors
- Pool size that works in development (few concurrent requests) fails in production (many concurrent requests)
- No runtime visibility into pool utilization or wait times

**Prevention:**
1. **Default pool size = 2 * num_cpus, capped at 20.** This is a reasonable starting point based on PgBouncer documentation and HikariCP's sizing recommendations. It provides headroom for concurrent requests without exhausting typical PostgreSQL limits.
2. **Expose pool metrics to Snow code.** Provide functions like `Pool.active_count(pool)`, `Pool.idle_count(pool)`, `Pool.waiting_count(pool)`. This allows users to implement health checks and auto-tuning.
3. **Document the sizing heuristic prominently.** "For web applications: pool_size = 2 * number of CPU cores. For batch processing: pool_size = number of CPU cores. Never exceed PostgreSQL's max_connections."
4. **Add a pool creation parameter for max_checkout_wait_ms.** Default to 5000ms. If a connection is not available within this time, return an error. This prevents silent hangs.

**Detection:**
- Checkout timeout errors under load
- PostgreSQL "too many connections" errors
- Application throughput plateaus despite available CPU
- Pool waiting_count consistently > 0 (under-provisioned)

**Phase:** Connection pool configuration. Should be addressed during pool implementation, not as a separate phase.

---

### Pitfall 9: TLS Certificate Verification Requires System Root Certificates, Breaking Single-Binary Philosophy

**What goes wrong:**
Snow compiles to a single static binary with no runtime dependencies. TLS client connections (to PostgreSQL) must verify the server's certificate against trusted root CAs. On Linux, root certificates live in `/etc/ssl/certs/` or `/etc/pki/tls/certs/`. On macOS, they're in the system keychain. On Alpine Linux (common in Docker), they're in `/etc/ssl/certs/ca-certificates.crt`. If none of these exist (minimal Docker image, embedded system), TLS connections fail with "certificate verify failed."

rustls (the recommended TLS library for Snow's single-binary philosophy) uses `webpki-roots` or `rustls-native-certs` for root certificate loading. `webpki-roots` bundles Mozilla's root certificates INTO the binary (~200KB), making the binary self-contained. `rustls-native-certs` reads the system store at runtime, which fails on minimal systems.

The choice between bundled and system certs affects both binary size and security model:
- Bundled certs: self-contained binary, but root CAs become stale (new CAs not recognized, revoked CAs still trusted) until the binary is recompiled
- System certs: always current, but requires system configuration and breaks the single-binary guarantee

**Why it happens:**
- TLS requires a trust anchor (root CA certificates) that the binary cannot generate
- The single-binary philosophy conflicts with the need for externally-managed trust anchors
- This is a fundamental tension in any self-contained TLS implementation

**Consequences:**
- TLS connections fail on minimal Docker images with "no trusted root certificates"
- Developers on macOS (where system certs work) don't encounter the issue; it surfaces only in deployment
- Bundled certs become a security liability if not updated regularly
- Users expect "it just works" but TLS requires explicit certificate management

**Prevention:**
1. **Default to `webpki-roots` (bundled Mozilla root CAs).** This preserves the single-binary philosophy. The ~200KB binary size increase is negligible compared to LLVM's contribution to Snow binary size.
2. **Allow runtime override via environment variable.** `SNOW_TLS_CA_FILE=/path/to/ca.crt` or `SNOW_TLS_CA_DIR=/path/to/certs/` allows users to specify custom certificates for corporate environments or self-signed CAs.
3. **Support `SNOW_TLS_INSECURE=1` for development only.** Skip certificate verification when explicitly requested. Print a loud warning to stderr. Never enable by default.
4. **For PostgreSQL `sslmode` compatibility**, support the standard connection string parameters: `sslmode=require` (encrypt but don't verify), `sslmode=verify-ca` (verify certificate), `sslmode=verify-full` (verify certificate and hostname). Default to `require` for development ergonomics, document `verify-full` for production.

**Detection:**
- "certificate verify failed" errors when connecting to cloud PostgreSQL (AWS RDS, Supabase, etc.)
- Works locally (system certs available) but fails in Docker (minimal image)
- SSL connections fail silently when `webpki-roots` doesn't include a specific CA (rare but possible with internal enterprise CAs)

**Phase:** TLS library integration. The certificate strategy must be decided when adding rustls as a dependency.

---

### Pitfall 10: struct-to-row Mapping with NULL Columns Causes Type Mismatch Panics

**What goes wrong:**
PostgreSQL columns can be NULL. Snow's type system distinguishes between `String` and `Option<String>`. If a struct has `name :: String` (not Optional) and the database returns NULL for that column, the deserializer must choose: panic, return an error, or silently substitute a default value. Each choice has consequences:

- Panic: unexpected crash on production data
- Error: forces every query to handle errors even when NULLs are "impossible" (e.g., NOT NULL column that is actually NULL during migration)
- Default value (empty string for String, 0 for Int): silent data corruption

The current pg.rs query function (line 892-899) already handles this by returning empty strings for NULL: `if col_len == -1 { String::new() }`. This works for the Map<String, String> return type but is incorrect for typed struct mapping where the field type should determine NULL handling.

**Why it happens:**
- SQL and Snow have different NULL semantics. SQL NULL means "no value." Snow Option<T> means "maybe no value." Snow String means "definitely has a value."
- The database schema may allow NULLs even when the application expects them to be impossible
- Schema changes (adding a nullable column, relaxing NOT NULL) break previously-working mappings

**Consequences:**
- Runtime panics when NULL appears in a non-Optional field
- Silent data corruption if defaults are substituted for NULLs
- Users must wrap EVERY field in Option<T> defensively, defeating the purpose of typed mapping
- Breaking change when database schema evolves to allow NULLs

**Prevention:**
1. **Non-Optional fields: return a Result::Err if the column is NULL.** `from_row` should return `Result<T, String>` where Err contains "column 'name' is NULL but field type is non-optional String". This is explicit, safe, and matches Diesel/sqlx behavior.
2. **Optional fields: NULL maps to None, non-NULL maps to Some(value).** `Option<String>` fields handle NULLs naturally.
3. **Provide a derive option for default values.** A future annotation `@default("")` or `@default(0)` on struct fields could specify what value to use for NULL. This should NOT be the default behavior.
4. **Document the NULL handling prominently.** "Use Option<T> for any column that might be NULL. Non-optional fields return an error on NULL."
5. **Consider compile-time warnings** when deriving(Row) on a struct with non-Optional fields. The warning can't know the database schema, but it can remind the user: "field 'name' is non-optional; NULL values will cause runtime errors."

**Detection:**
- Runtime errors like "column 'name' is NULL but field is non-optional"
- Tests pass with NOT NULL test data but fail with production data containing NULLs
- Users defensively wrapping all fields in Option to avoid errors

**Phase:** struct-to-row mapping deserialization. The NULL handling strategy must be decided at design time.

---

### Pitfall 11: PostgreSQL `SSLRequest` Handshake Must Precede TLS, Breaking the Existing Connection Flow

**What goes wrong:**
PostgreSQL TLS is not "connect with TLS from the start." The protocol requires:
1. Open a plain TCP connection
2. Send `SSLRequest` message (8 bytes: length=8, code=80877103)
3. Read 1 byte response: `S` (server supports SSL) or `N` (no SSL)
4. If `S`: perform TLS handshake on the existing TCP connection
5. Then send the normal `StartupMessage` over the TLS-encrypted connection

The existing `snow_pg_connect` (pg.rs lines 491-713) sends the StartupMessage immediately after TCP connect. Adding TLS requires inserting the SSLRequest exchange BEFORE the StartupMessage. This changes the connection function's control flow and the PgConn struct (which currently wraps a plain TcpStream).

If the developer naively wraps the TcpStream in TLS after the StartupMessage, the TLS handshake fails because the PostgreSQL server is already in normal protocol mode, not TLS negotiation mode.

**Why it happens:**
- PostgreSQL's TLS upgrade is an in-band protocol negotiation, not a separate port (unlike HTTPS which uses port 443)
- The SSLRequest is NOT a standard PostgreSQL message (it has no type byte, just like the StartupMessage)
- The existing code has no extension point for "do something between TCP connect and StartupMessage"
- Many tutorials and examples show PostgreSQL TLS as "just wrap the stream," omitting the SSLRequest step

**Consequences:**
- TLS connection fails with "unexpected message" errors from PostgreSQL
- If TLS handshake is attempted without SSLRequest, the server interprets TLS ClientHello as a malformed PostgreSQL message and disconnects
- Works with `sslmode=disable` but fails with any SSL mode

**Prevention:**
1. **Refactor `snow_pg_connect` to have a clear two-phase structure.** Phase 1: TCP connect + optional TLS negotiation (SSLRequest -> TLS handshake). Phase 2: PostgreSQL StartupMessage + authentication. The PgConn struct should hold either a `TcpStream` or a `TlsStream<TcpStream>`, abstracted behind a trait or enum.
2. **Use an enum for the stream type:**
   ```rust
   enum PgStream {
       Plain(TcpStream),
       Tls(rustls::StreamOwned<rustls::ClientConnection, TcpStream>),
   }
   ```
   Implement `Read + Write` for this enum so the rest of the wire protocol code is unchanged.
3. **Handle the `N` response gracefully.** If the server responds with `N` (no SSL), the connection should either fall back to plain TCP (if `sslmode=prefer`) or return an error (if `sslmode=require`).
4. **Test against PostgreSQL configured with `ssl=on`, `ssl=off`, and `ssl=prefer`.** Each configuration requires different behavior from the client.

**Detection:**
- "unexpected byte at start of message" errors from PostgreSQL server logs
- TLS handshake timeout (server doesn't respond because it's waiting for a PostgreSQL message, not a TLS ClientHello)
- Connection works with SSL disabled but fails with SSL enabled

**Phase:** PostgreSQL TLS implementation. The SSLRequest flow must be implemented before any TLS handshake code.

---

## Minor Pitfalls

Issues that cause developer friction or minor bugs, but have straightforward fixes.

---

### Pitfall 12: rustls Crypto Backend (aws-lc-rs) Build Complexity on Non-Standard Platforms

**What goes wrong:**
rustls defaults to `aws-lc-rs` as its cryptographic backend. aws-lc-rs includes C and assembly code that requires a C compiler and CMake to build. On standard platforms (Linux x86_64, macOS ARM64), this works out of the box. On non-standard platforms (musl/Alpine Linux, cross-compilation for ARM, older macOS), the build may fail.

The alternative backend `ring` is pure Rust + assembly and has fewer build dependencies, but is also not trivial to cross-compile.

Snow's build system already compiles C code (SQLite amalgamation via `cc::Build` from the v2.0 milestone), so the toolchain is present. But aws-lc-rs requires CMake in addition to a C compiler, which may not be installed.

**Prevention:**
1. **Use rustls with the `ring` crypto backend instead of the default `aws-lc-rs`.** Ring has broader platform support and simpler build requirements. The performance difference is negligible for Snow's use case (TLS is not the bottleneck; database queries and application logic are).
2. **Add CMake as a documented build dependency** if aws-lc-rs is used. Update Snow's build instructions.
3. **Test cross-compilation in CI.** Add a CI job that builds Snow for musl/Alpine to catch build issues early.

**Detection:**
- Build failures on CI with "CMake not found" or "unsupported platform" errors from aws-lc-rs
- Users reporting build failures on non-standard platforms

**Phase:** TLS dependency selection. Decide on `ring` vs `aws-lc-rs` when adding the rustls dependency.

---

### Pitfall 13: struct-to-row Mapping Code Generation Must Handle All Snow Primitive Types

**What goes wrong:**
PostgreSQL returns column values as text strings in the text protocol format (which Snow's PG driver currently uses -- pg.rs uses format code 0 = text in write_bind at line 163 and write_describe_portal). The `deriving(Row)` code generator must produce parsing code for each Snow type:

- `String`: no parsing needed (already text)
- `Int`: parse text as i64 (can fail: "abc" is not a valid Int)
- `Float`: parse text as f64 (can fail, and has precision concerns: "3.14159265358979323846" may lose precision)
- `Bool`: parse "t"/"f"/"true"/"false"/"1"/"0" (PostgreSQL uses "t"/"f")
- `Option<T>`: handle NULL (column length -1) then recursively parse T
- `List<T>`: PostgreSQL array syntax `{1,2,3}` is NOT JSON array syntax `[1,2,3]`

Missing any type or getting the parsing wrong for edge cases produces silent data corruption or runtime errors.

**Why it happens:**
- PostgreSQL's text representation varies by type and is not the same as JSON
- Boolean is "t"/"f" not "true"/"false"
- Arrays use curly braces not square brackets
- Dates, timestamps, and UUIDs are text but require specific parsing
- The deriving(Json) infrastructure handles JSON types, not PostgreSQL text types

**Consequences:**
- Bool fields always fail to parse ("true" instead of "t")
- Float fields lose precision or fail on scientific notation ("1.23e4")
- Array/List fields produce garbage when parsing `{1,2,3}` as if it were JSON
- Date/timestamp fields are returned as unparsed strings, confusing users who expected typed values

**Prevention:**
1. **Start with String-only mapping.** For the initial implementation, `deriving(Row)` only supports structs where ALL fields are `String`. This avoids all parsing issues and is still useful (users parse manually, same as the current Map<String, String> approach but with named fields).
2. **Add type-specific parsers incrementally.** After String, add Int (`str.parse::<i64>()`), Float (`str.parse::<f64>()`), Bool (`match s { "t" | "true" | "1" => true, _ => false }`). Each parser is tested in isolation.
3. **Do NOT support PostgreSQL array syntax in the initial implementation.** Arrays (`{1,2,3}`) require a dedicated parser. Defer to a future milestone. Users who need arrays can use String fields and parse manually.
4. **Return `Result<T, String>` from the generated `from_row` function.** Every parsing failure produces an Err with the column name, expected type, and actual value. Never panic on parse failure.

**Detection:**
- Parse failures at runtime for non-String fields
- Tests that only use String values pass, but Int/Float/Bool fields fail
- PostgreSQL boolean "t"/"f" not recognized as valid booleans

**Phase:** struct-to-row mapping type support. Start with String-only, expand incrementally.

---

### Pitfall 14: Pool Connection Health Check Adds Latency to Every Checkout

**What goes wrong:**
Connection pools must validate that connections are still alive (the server hasn't closed them due to timeout, network failure, or restart). The standard approach is to send a "ping" query (`SELECT 1` or PostgreSQL's `;` empty query) on checkout. But this adds a network round-trip (1-5ms) to every checkout, which adds up: 100 queries/second * 3ms/ping = 300ms/second spent on health checks.

For Snow's actor-based model, the health check also blocks the worker thread (same as any DB operation), compounding the scheduling starvation problem from Pitfall 2.

**Why it happens:**
- Connections can become stale silently (TCP keepalive doesn't detect application-level disconnects)
- Without health checks, stale connections cause query failures after checkout
- The "check on checkout" pattern is the simplest but most expensive approach

**Consequences:**
- Measurable latency increase (1-5ms) on every database operation
- Under high query volume, health checks consume significant pool capacity
- False positive failures when the network has transient issues (health check fails but the connection would have worked for the actual query)

**Prevention:**
1. **Use "test on borrow with idle timeout" strategy.** Only health-check connections that have been idle for longer than N seconds (default: 30s). Recently-used connections are assumed to be alive. This eliminates health checks for hot pools.
2. **Use TCP keepalive instead of application-level pings.** Set `TcpStream::set_keepalive(Some(Duration::from_secs(60)))` on PostgreSQL connections. The OS detects dead connections automatically. This is not sufficient for all failure modes but catches the most common (server restart, network partition).
3. **Validate lazily: catch errors and retry.** Instead of health-checking on checkout, attempt the query. If it fails with a connection error (as opposed to a SQL error), discard the connection, check out a new one, and retry. This is the approach used by HikariCP (Java) and is the most performant.
4. **For the initial implementation, skip health checks entirely.** Let stale connections fail naturally and be replaced. This is acceptable for a first version; health checks can be added later.

**Detection:**
- Checkout latency includes a consistent 1-5ms overhead
- Health check failures cause checkout errors even when the database is healthy (transient network issue)
- Pool metrics show high health-check-failure rate during network instability

**Phase:** Connection pool health check implementation. Can be deferred to after the initial pool implementation.

---

## Phase-Specific Warnings

| Phase Topic | Likely Pitfall | Severity | Mitigation |
|-------------|---------------|----------|------------|
| Connection pooling architecture | Pool checkout blocks worker thread (Pitfall 1) | CRITICAL | Message-passing checkout, not blocking wait |
| Connection pooling architecture | Dangling handle after pool reclaim (Pitfall 4) | CRITICAL | Generation-counted slot IDs, not raw pointers |
| Connection pooling architecture | Pool size vs worker thread count (Pitfall 8) | MODERATE | Default 2*num_cpus, expose pool metrics |
| Connection pooling architecture | Health check latency (Pitfall 14) | LOW | Idle-timeout strategy, lazy validation |
| PostgreSQL TLS | SSLRequest must precede TLS handshake (Pitfall 11) | CRITICAL | Two-phase connect: SSLRequest then StartupMessage |
| PostgreSQL TLS | TLS handshake blocks worker thread (Pitfall 2) | CRITICAL | Handshake on dedicated OS thread for HTTPS; accept for PG (one-time cost) |
| PostgreSQL TLS | Certificate verification vs single-binary (Pitfall 9) | MODERATE | Bundle webpki-roots, allow env var override |
| PostgreSQL TLS | GC + TLS stream interaction (Pitfall 5) | MODERATE | Maintain copy-to-buffer pattern from pg.rs |
| PostgreSQL TLS | rustls build complexity (Pitfall 12) | LOW | Use ring backend, not aws-lc-rs |
| HTTPS server TLS | TLS accept starvation (Pitfall 2) | CRITICAL | Dedicated thread pool for TLS accepts |
| Database transactions | Open transaction after actor crash (Pitfall 3) | CRITICAL | Pool validates ReadyForQuery status, ROLLBACK on checkin |
| Database transactions | Transaction leak from missing commit (Pitfall 6) | MODERATE | Provide transaction(conn, fn) wrapper as primary API |
| struct-to-row mapping | Column name mismatch (Pitfall 7) | MODERATE | Auto snake_case conversion, case-insensitive matching |
| struct-to-row mapping | NULL handling for non-Optional fields (Pitfall 10) | MODERATE | Return Result::Err on NULL for non-Optional fields |
| struct-to-row mapping | Type parsing for non-String fields (Pitfall 13) | MODERATE | Start with String-only, expand incrementally |

---

## Integration Pitfalls (Cross-Feature)

These pitfalls arise from the INTERACTION between features, not from any single feature in isolation.

### Pool + Transactions: The Deadly Combination

Connection pooling and transactions are deeply coupled. Every pool design decision affects transaction safety, and vice versa. The critical interaction: if a connection with an open transaction is returned to the pool, every subsequent user of that connection is affected. This is Pitfall 3 (transaction poisoning) plus Pitfall 4 (handle lifecycle), combined.

**The safe order of implementation:**
1. First: pool with ROLLBACK-on-checkin safety net
2. Then: transaction API that uses the pool
3. Never: transaction API without pool awareness

### TLS + Cooperative Scheduling: The Latency Tax

TLS adds blocking time to both PostgreSQL connections (one-time handshake) and HTTPS server (per-connection handshake). For PostgreSQL, the one-time cost is acceptable because connections are pooled. For HTTPS, the per-connection cost is not acceptable on the actor scheduler.

**The safe order of implementation:**
1. First: PostgreSQL TLS (client-side, pooled connections, one-time handshake cost)
2. Then: HTTPS TLS (server-side, requires dedicated thread pool for accepts)
3. Never: HTTPS TLS on the actor scheduler without a dedicated accept thread pool

### struct-to-row + Pool: Connection State Assumptions

struct-to-row mapping generates code that executes queries and parses results. If the underlying connection is in a bad state (from a poisoned pool), the generated code receives garbage data and produces confusing errors. The mapping code should not assume a clean connection; it should validate that the query succeeded before attempting to parse rows.

---

## Sources

### Official Documentation
- [PostgreSQL Wire Protocol v3: Message Flow](https://www.postgresql.org/docs/current/protocol-flow.html) -- SSLRequest, Sync, ReadyForQuery transaction status (HIGH confidence)
- [PostgreSQL Wire Protocol v3: SSL Session Encryption](https://www.postgresql.org/docs/current/protocol-flow.html#PROTOCOL-FLOW-SSL) -- SSLRequest message format and flow (HIGH confidence)
- [PostgreSQL SASL Authentication](https://www.postgresql.org/docs/current/sasl-authentication.html) -- SCRAM-SHA-256 over TLS (HIGH confidence)
- [rustls documentation](https://docs.rs/rustls/latest/rustls/) -- TLS library API, certificate handling (HIGH confidence)

### Domain Research
- [Building a Connection Pool from Scratch (2025)](https://medium.com/nerd-for-tech/building-a-connection-pool-from-scratch-internals-design-and-real-world-insights-e4f72fd7d9af) -- Pool design patterns, thread safety, sizing heuristics (MEDIUM confidence)
- [Npgsql Connection Reclamation via WeakReference](https://github.com/npgsql/npgsql/issues/2878) -- GC-safe connection pool handle pattern (MEDIUM confidence)
- [Erlang: A Veteran's Take on Concurrency and Fault Tolerance](https://medium.com/@rng/erlang-a-veterans-take-on-concurrency-fault-tolerance-and-scalability-adff3f96565b) -- Actor model + database transaction patterns (MEDIUM confidence)
- [Rustls vs NativeTls (Rust forum)](https://users.rust-lang.org/t/rustls-vs-nativetls/131051) -- TLS library comparison for static linking (MEDIUM confidence)
- [Rustls Performance Benchmarks (Prossimo)](https://www.memorysafety.org/blog/rustls-performance/) -- rustls vs OpenSSL performance data (MEDIUM confidence)
- [Pitfalls of Isolation Levels in Distributed Databases (PlanetScale)](https://planetscale.com/blog/pitfalls-of-isolation-levels-in-distributed-databases) -- Transaction isolation pitfalls (MEDIUM confidence)
- [node-postgres: Lost Connection During Transaction](https://github.com/brianc/node-postgres/issues/1454) -- Connection state after crash in transaction (MEDIUM confidence)
- [IBM Maximo: GC and Connection Leak](https://www.ibm.com/support/pages/garbage-collection-and-connection-leak) -- GC interaction with connection pools (MEDIUM confidence)
- [Cooperative Scheduling Pitfalls (Microsoft)](https://learn.microsoft.com/en-us/cpp/parallel/concrt/comparing-the-concurrency-runtime-to-other-concurrency-models) -- Cooperative scheduling starvation patterns (MEDIUM confidence)

### Codebase Analysis (PRIMARY SOURCE)
- `/Users/sn0w/Documents/dev/snow/crates/snow-rt/src/db/pg.rs` -- PostgreSQL wire protocol, PgConn struct, opaque u64 handle pattern, ReadyForQuery handling
- `/Users/sn0w/Documents/dev/snow/crates/snow-rt/src/db/sqlite.rs` -- SQLite FFI, SqliteConn struct, matching handle pattern
- `/Users/sn0w/Documents/dev/snow/crates/snow-rt/src/actor/scheduler.rs` -- M:N work-stealing scheduler, !Send coroutines, worker loop, process table
- `/Users/sn0w/Documents/dev/snow/crates/snow-rt/src/actor/mod.rs` -- Actor lifecycle, terminate_callback, reduction_check, snow_actor_receive
- `/Users/sn0w/Documents/dev/snow/crates/snow-rt/src/actor/stack.rs` -- Corosensei coroutine management, CURRENT_YIELDER, thread-pinning
- `/Users/sn0w/Documents/dev/snow/crates/snow-rt/src/actor/heap.rs` -- Per-actor GC, conservative stack scanning, mark-sweep, find_object_containing
- `/Users/sn0w/Documents/dev/snow/crates/snow-rt/src/actor/process.rs` -- PCB, ProcessState, terminate_callback type
- `/Users/sn0w/Documents/dev/snow/crates/snow-rt/src/gc.rs` -- GC allocation entry points, global arena vs actor heap
- `/Users/sn0w/Documents/dev/snow/crates/snow-rt/src/http/server.rs` -- Actor-per-connection HTTP, catch_unwind pattern, tiny_http blocking accept
- `/Users/sn0w/Documents/dev/snow/crates/snow-typeck/src/infer.rs` -- deriving infrastructure, trait resolution for deriving(Json)
- `/Users/sn0w/Documents/dev/snow/crates/snow-codegen/src/mir/lower.rs` -- MIR generation for deriving traits
