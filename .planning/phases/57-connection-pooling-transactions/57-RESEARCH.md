# Phase 57: Connection Pooling & Transactions - Research

**Researched:** 2026-02-12
**Domain:** Connection pooling (Mutex+Condvar), PostgreSQL/SQLite transaction management, runtime intrinsics
**Confidence:** HIGH

## Summary

Phase 57 adds two foundational database features to Snow: connection pooling for PostgreSQL and transaction management for both PostgreSQL and SQLite. These are implemented as Rust runtime intrinsics following the exact pattern established in Phases 53-56.

The connection pool is a Rust-side `Mutex<PoolInner>` + `Condvar` structure, NOT an actor-based pool. The FEATURES.md suggested an actor-based pool, but the requirements (POOL-07) specify "Pool handles are opaque u64 values (GC-safe, same pattern as DB connections)." This means the pool lives in Rust-side heap memory, accessed via `Box::into_raw` handles, with `Mutex` for thread safety. This is simpler, has no message-passing overhead, and follows the exact same pattern as PgConn/SqliteConn handles. Multiple actors on the scheduler's worker threads call into the pool through extern "C" intrinsics -- the `Mutex` serializes access. The `Condvar` handles blocking when all connections are checked out (with configurable timeout).

Transaction support requires two changes: (1) adding a `txn_status: u8` field to `PgConn` that tracks the ReadyForQuery transaction status byte (`I`=idle, `T`=in-transaction, `E`=error), and (2) new intrinsics for `BEGIN`/`COMMIT`/`ROLLBACK` for both PG and SQLite. The `Pg.transaction(conn, fn)` block-based API calls a Snow closure wrapped in `catch_unwind` for panic-safe rollback. The closure calling convention is well-established: `fn_ptr(env_ptr, conn_handle) -> result_ptr` with null env_ptr for bare functions.

**Primary recommendation:** Implement the pool as a `Mutex<PoolInner>` + `Condvar` in `snow-rt/src/db/pool.rs`. Implement transaction intrinsics as simple SQL commands (`BEGIN`/`COMMIT`/`ROLLBACK`) sent via the existing wire protocol helpers. Track PG transaction status in PgConn. Follow the established compiler pipeline pattern (runtime -> intrinsics -> MIR lower -> type checker) for all new functions.

## Standard Stack

### Core (Rust std only -- no new crate dependencies)
| Component | Source | Purpose | Why Standard |
|-----------|--------|---------|--------------|
| `std::sync::Mutex` | Rust std | Pool state serialization | Standard thread-safe interior mutability. parking_lot::Mutex is already in the project and would work too. |
| `std::sync::Condvar` | Rust std | Blocking checkout with timeout | Standard condition variable for waiting on pool availability. `wait_timeout` provides checkout timeout. |
| `std::time::Instant` | Rust std | Idle connection age tracking | Standard monotonic clock for connection health timing. |
| `std::panic::catch_unwind` | Rust std | Transaction rollback on panic | Already used throughout the runtime (actor scheduler, HTTP handlers). |

### No New Crate Dependencies
This phase requires zero new Cargo dependencies. All pooling and transaction logic uses Rust standard library primitives and the existing PG wire protocol / SQLite FFI code from Phases 53-55.

## Architecture Patterns

### Recommended File Structure
```
crates/snow-rt/src/
  db/
    mod.rs              # pub mod sqlite; pub mod pg; pub mod pool;
    sqlite.rs           # (existing) + new: snow_sqlite_begin/commit/rollback
    pg.rs               # (existing) + modify PgConn to track txn_status
                        # + new: snow_pg_begin/commit/rollback/transaction
    pool.rs             # NEW: connection pool (Mutex<PoolInner> + Condvar)

crates/snow-codegen/src/
  codegen/intrinsics.rs  # Add: snow_pool_* and snow_pg_begin/commit/rollback/transaction
                         # and snow_sqlite_begin/commit/rollback declarations
  mir/lower.rs           # Add: known_functions + map_builtin_name for Pool/Pg.transaction/Sqlite.begin etc.
  mir/types.rs           # Add: "PoolHandle" => MirType::Int

crates/snow-typeck/src/
  infer.rs               # Add: Pool module in stdlib_modules() + STDLIB_MODULE_NAMES
  builtins.rs            # Add: PoolHandle opaque type + pool_*/pg_begin/etc function signatures
```

### Pattern 1: Mutex+Condvar Connection Pool
**What:** Pool state lives in a `PoolInner` struct protected by `Mutex`. Callers that find no idle connections block on a `Condvar` with timeout.
**When to use:** When multiple OS threads (Snow's scheduler workers) need to share a fixed set of resources.
**Why not an actor:** Pool handles must be opaque u64 values (POOL-07). An actor-based pool would require message passing for every checkout/checkin, adding latency. A Mutex pool is simpler and the lock is held for microseconds (just pop/push a VecDeque).

```rust
// Pool internal state
struct PoolInner {
    url: String,
    idle: VecDeque<PooledConn>,   // idle connections ready for checkout
    active_count: usize,           // connections currently checked out
    total_created: usize,          // total connections ever created (for max tracking)
    min_conns: usize,
    max_conns: usize,
    checkout_timeout_ms: u64,
    closed: bool,                  // set by Pool.close
}

struct PooledConn {
    handle: u64,                   // PgConn handle (Box::into_raw)
    created_at: Instant,           // for age-based health checks
    last_used: Instant,            // for idle timeout
}

struct PgPool {
    inner: Mutex<PoolInner>,
    available: Condvar,            // notified on checkin
}
```

### Pattern 2: PgConn Transaction Status Tracking (TXN-03)
**What:** Add a `txn_status: u8` field to `PgConn` that is updated every time a ReadyForQuery message is received. The byte is `b'I'` (idle), `b'T'` (in transaction), or `b'E'` (failed transaction).
**When to use:** Required for POOL-05 (connection cleanup on checkin) and transaction status queries.
**Why:** The PostgreSQL wire protocol sends ReadyForQuery after every command cycle. The status byte tells us if a connection is in a dirty state (leftover transaction). On pool checkin, if `txn_status != b'I'`, the pool sends `ROLLBACK` before returning the connection to idle.

```rust
// Modified PgConn struct
struct PgConn {
    stream: PgStream,
    txn_status: u8,  // b'I' (idle), b'T' (in-txn), b'E' (failed-txn)
}
```

Every place that reads ReadyForQuery (the `b'Z'` match arms in `snow_pg_connect`, `snow_pg_execute`, `snow_pg_query`) must be updated to extract and store the status byte:
```rust
b'Z' => {
    if body.len() >= 1 {
        conn.txn_status = body[0];
    }
    break;
}
```

### Pattern 3: Closure Calling Convention for Pg.transaction
**What:** Snow closures are `{fn_ptr: *mut u8, env_ptr: *mut u8}`. The runtime calls `fn_ptr(env_ptr, args...)` if env is non-null, or `fn_ptr(args...)` if null.
**When to use:** Any runtime intrinsic that accepts a callback/closure from Snow code.
**Established in:** `snow_list_map`, `snow_list_filter`, HTTP middleware chain, `snow_job_async`.

For `Pg.transaction(conn, fn(conn) do ... end)`:
```rust
// The closure receives the conn handle as i64 and returns *mut u8 (SnowResult)
// Signature: snow_pg_transaction(conn_handle: u64, fn_ptr: *mut u8, env_ptr: *mut u8) -> *mut u8

type BareTxnFn = unsafe extern "C" fn(u64) -> *mut u8;
type ClosureTxnFn = unsafe extern "C" fn(*mut u8, u64) -> *mut u8;
```

### Pattern 4: Pool Handle as Opaque u64 (POOL-07)
**What:** Same as PgConn/SqliteConn -- `Box::into_raw(pool) as u64`. The pool struct lives on the Rust heap, not GC heap. Explicit `Pool.close(pool)` is required.
**Why:** GC has no finalizer support. The pool must outlive any actor that uses it (typically lives for the entire program lifetime).

### Anti-Patterns to Avoid
- **Actor-based pool with message passing:** Adds latency (message send + receive + context switch) for every checkout/checkin. Mutex contention is negligible for the microsecond lock hold time.
- **Global singleton pool:** Requirements show `Pool.open(url, config)` returns a handle. Multiple pools must be possible.
- **GC-allocated pool state:** Pool contains `VecDeque<PooledConn>` with Rust types. Must live on Rust heap, not GC heap.
- **Blocking without timeout:** Checkout must have a configurable timeout (checkout_timeout_ms). Never block forever.
- **Skipping ROLLBACK on checkin:** A checked-in connection might be in a transaction state (user forgot to commit/rollback). Always check `txn_status` and send ROLLBACK if needed (POOL-05).

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Thread-safe pool queue | Lock-free concurrent queue | `Mutex<VecDeque>` + `Condvar` | Lock-free queues are complex. Mutex hold time is <1us (push/pop). Condvar gives timeout support for free. |
| Connection health check protocol | Custom ping mechanism | `SELECT 1` via existing `snow_pg_execute` | Already have full query capability. SELECT 1 is the universal health check. |
| Transaction SQL generation | Custom transaction protocol messages | Simple `snow_pg_execute(conn, "BEGIN", [])` | BEGIN/COMMIT/ROLLBACK are standard SQL. Use the existing execute path. |
| Closure invocation | Custom callback mechanism | Existing `fn_ptr/env_ptr` convention | Established pattern used by List.map, HTTP handlers, Job.async. |

**Key insight:** The pool is a thin synchronization wrapper around the existing PgConn infrastructure. Transactions are just SQL commands sent through the existing wire protocol. No new protocols or complex machinery needed.

## Common Pitfalls

### Pitfall 1: Connection Leak on Panic
**What goes wrong:** If an actor panics while holding a checked-out connection, the connection is never returned to the pool. The pool slowly drains to zero available connections.
**Why it happens:** `Pool.checkout` gives a connection handle. If the actor crashes before `Pool.checkin`, the pool never sees the connection again.
**How to avoid:** Two strategies:
  1. For `Pool.query`/`Pool.execute` (auto checkout): checkout and checkin happen within the same Rust intrinsic call. Panic in SQL execution still completes the Rust function.
  2. For `Pg.transaction(conn, fn)`: the runtime wraps the closure call in `catch_unwind`. On panic, ROLLBACK + checkin still execute.
  3. For manual `Pool.checkout`/`Pool.checkin`: document that connections may leak if actor crashes. This matches Elixir's model (ETS heir mechanism is out of scope for v1).
**Warning signs:** Pool gradually runs out of connections under load. `Pool.checkout` starts timing out.

### Pitfall 2: Deadlock from Nested Checkout
**What goes wrong:** An actor checks out a connection, then tries to check out another from the same pool while all connections are in use.
**Why it happens:** If max_conns=5 and 5 actors each hold one connection and each try to get a second, all block forever.
**How to avoid:** The checkout timeout prevents true deadlock (returns Err after timeout). Document that nested checkouts from the same pool should be avoided. This is the standard approach in all connection pool implementations.
**Warning signs:** Checkout timeout errors under moderate load.

### Pitfall 3: Stale Connection After Server Restart
**What goes wrong:** PostgreSQL server restarts. Pooled connections are now dead TCP sockets. Queries fail with "read tag: connection reset by peer."
**Why it happens:** TCP connections don't have a keepalive mechanism fast enough to detect server restarts.
**How to avoid:** POOL-04 requires health check on checkout. Before returning a connection from the idle queue, send `SELECT 1`. If it fails, discard the connection and try the next one (or create a new one).
**Warning signs:** Burst of query errors after DB maintenance windows.

### Pitfall 4: Transaction Status Desync
**What goes wrong:** PgConn.txn_status says `I` (idle) but the connection is actually in a transaction.
**Why it happens:** If a bug causes ReadyForQuery parsing to be skipped or the status byte to not be stored.
**How to avoid:** Update `txn_status` in EVERY code path that reads ReadyForQuery. There are currently 3 places: `snow_pg_connect` (post-auth), `snow_pg_execute` (post-execute), `snow_pg_query` (post-query). All new transaction functions must also update it.
**Warning signs:** Connections returned to pool with stale transactions. Subsequent queries on those connections fail with "current transaction is aborted."

### Pitfall 5: SQLite Threading Model
**What goes wrong:** SQLite connections are opened with `SQLITE_OPEN_READWRITE | SQLITE_OPEN_CREATE` (the default). SQLite in `serialized` mode allows multi-thread access to the same connection, but the existing Snow pattern uses one connection per handle.
**Why it happens:** SQLite has 3 threading modes: single-thread, multi-thread, serialized.
**How to avoid:** SQLite pooling is not in this phase's requirements (POOL-01 through POOL-07 all specify PostgreSQL). SQLite transactions (TXN-02) operate on individual connections, not pooled connections. No SQLite pool needed.
**Warning signs:** N/A for this phase.

### Pitfall 6: catch_unwind Requires UnwindSafe
**What goes wrong:** Wrapping the closure call in `catch_unwind` requires `AssertUnwindSafe` because raw pointers are not `UnwindSafe`.
**Why it happens:** Rust's `catch_unwind` requires `FnOnce + UnwindSafe`. Raw pointers (`*mut u8`) don't implement `UnwindSafe`.
**How to avoid:** Use `std::panic::AssertUnwindSafe(|| { ... })` as already done throughout the runtime (see `actor/scheduler.rs:658`, `http/server.rs:383`).
**Warning signs:** Compilation error: "the type `*mut u8` may not be safely transferred across an unwind boundary."

## Code Examples

### Example 1: Pool Open
```rust
// snow_pool_open(url: *const SnowString, min_conns: i64, max_conns: i64, checkout_timeout_ms: i64) -> *mut u8
// Returns SnowResult<PoolHandle, String>
//
// Creates min_conns initial connections, validates them, stores in idle queue.
// Pool handle is Box::into_raw as u64 (same as PgConn).

#[no_mangle]
pub extern "C" fn snow_pool_open(
    url: *const SnowString,
    min_conns: i64,
    max_conns: i64,
    checkout_timeout_ms: i64,
) -> *mut u8 {
    // Parse URL, create min_conns initial PG connections
    // Store them in PoolInner.idle VecDeque
    // Wrap in PgPool { inner: Mutex::new(pool_inner), available: Condvar::new() }
    // Box::into_raw(pool) as u64 -> alloc_result(0, handle)
}
```

### Example 2: Pool Checkout with Condvar Timeout
```rust
// Core checkout logic (inside Mutex lock):
fn checkout_inner(pool: &PgPool, timeout: Duration) -> Result<u64, String> {
    let mut inner = pool.inner.lock().unwrap();  // or parking_lot Mutex

    loop {
        if inner.closed {
            return Err("pool is closed".to_string());
        }

        // Try to get an idle connection
        if let Some(conn) = inner.idle.pop_front() {
            inner.active_count += 1;
            return Ok(conn.handle);
        }

        // Can we create a new one?
        if inner.total_created < inner.max_conns {
            inner.total_created += 1;
            inner.active_count += 1;
            drop(inner);  // release lock before connecting
            // Create new PG connection (may take time)
            // If creation fails, decrement total_created and active_count
            return create_new_connection(&pool);
        }

        // All connections busy, wait with timeout
        let (new_inner, timeout_result) = pool.available.wait_timeout(inner, timeout).unwrap();
        inner = new_inner;
        if timeout_result.timed_out() {
            return Err("pool checkout timeout".to_string());
        }
    }
}
```

### Example 3: Pool Checkin with Transaction Cleanup (POOL-05)
```rust
// On checkin, ensure connection is in clean state:
fn checkin_inner(pool: &PgPool, conn_handle: u64) {
    // Check transaction status
    let conn = unsafe { &mut *(conn_handle as *mut PgConn) };
    if conn.txn_status != b'I' {
        // Connection has an active/failed transaction -- rollback
        send_simple_query(conn, "ROLLBACK");
        // After ROLLBACK, ReadyForQuery should return 'I'
    }

    let mut inner = pool.inner.lock().unwrap();
    inner.active_count -= 1;
    inner.idle.push_back(PooledConn {
        handle: conn_handle,
        created_at: Instant::now(), // or preserved from creation
        last_used: Instant::now(),
    });
    pool.available.notify_one();  // wake one waiting checkout
}
```

### Example 4: Health Check on Checkout (POOL-04)
```rust
// Before returning a connection from idle queue, validate it:
fn health_check(conn_handle: u64) -> bool {
    let conn = unsafe { &mut *(conn_handle as *mut PgConn) };
    // Send a simple query: Parse("SELECT 1") + Bind + Execute + Sync
    // Read response. If any I/O error -> connection is dead.
    // If successful -> connection is alive.
    match execute_health_check(conn) {
        Ok(_) => true,
        Err(_) => {
            // Close the dead connection
            close_pg_conn(conn_handle);
            false
        }
    }
}
```

### Example 5: Pg.transaction with catch_unwind (TXN-04, TXN-05)
```rust
// snow_pg_transaction(conn_handle: u64, fn_ptr: *mut u8, env_ptr: *mut u8) -> *mut u8
// Returns SnowResult<T, String> where T is whatever the closure returns
#[no_mangle]
pub extern "C" fn snow_pg_transaction(
    conn_handle: u64,
    fn_ptr: *mut u8,
    env_ptr: *mut u8,
) -> *mut u8 {
    // 1. Send BEGIN
    let begin_result = send_simple_command(conn_handle, "BEGIN");
    if begin_result.is_err() { return err_result("BEGIN failed"); }

    // 2. Call the closure with catch_unwind
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        unsafe {
            if env_ptr.is_null() {
                let f: fn(u64) -> *mut u8 = std::mem::transmute(fn_ptr);
                f(conn_handle)
            } else {
                let f: fn(*mut u8, u64) -> *mut u8 = std::mem::transmute(fn_ptr);
                f(env_ptr, conn_handle)
            }
        }
    }));

    match result {
        Ok(result_ptr) => {
            // Check if closure returned Ok or Err
            let r = unsafe { &*(result_ptr as *const SnowResult) };
            if r.tag == 0 {
                // Success -> COMMIT
                send_simple_command(conn_handle, "COMMIT");
                result_ptr
            } else {
                // Error -> ROLLBACK
                send_simple_command(conn_handle, "ROLLBACK");
                result_ptr  // propagate the Err
            }
        }
        Err(_panic) => {
            // Panic -> ROLLBACK
            send_simple_command(conn_handle, "ROLLBACK");
            err_result("transaction panicked")
        }
    }
}
```

### Example 6: SQLite Transaction Commands (TXN-02)
```rust
// Simple wrappers that execute BEGIN/COMMIT/ROLLBACK SQL via sqlite3_exec
#[no_mangle]
pub extern "C" fn snow_sqlite_begin(conn_handle: u64) -> *mut u8 {
    execute_sql(conn_handle, "BEGIN")
}

#[no_mangle]
pub extern "C" fn snow_sqlite_commit(conn_handle: u64) -> *mut u8 {
    execute_sql(conn_handle, "COMMIT")
}

#[no_mangle]
pub extern "C" fn snow_sqlite_rollback(conn_handle: u64) -> *mut u8 {
    execute_sql(conn_handle, "ROLLBACK")
}

// Helper: execute a parameterless SQL command and return Result<Unit, String>
fn execute_sql(conn_handle: u64, sql: &str) -> *mut u8 {
    // Use sqlite3_exec (simpler than prepare/step/finalize for no-param commands)
    // or reuse the existing snow_sqlite_execute pattern with empty params
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| PgConn ignores ReadyForQuery status byte | PgConn tracks txn_status (I/T/E) | Phase 57 | Enables pool cleanup (POOL-05) and transaction status queries |
| Individual connections only | Pool.open returns reusable pool handle | Phase 57 | Production-ready connection management |
| Manual BEGIN/COMMIT/ROLLBACK via execute | Pg.transaction(conn, fn) with auto-commit/rollback | Phase 57 | Safe transaction semantics with panic protection |
| Box::into_raw for individual PgConn only | Box::into_raw for PgPool containing Mutex<PoolInner> | Phase 57 | Same GC-safe handle pattern, now for pools |

## Requirement-to-Implementation Mapping

| Requirement | Implementation | Intrinsic(s) |
|-------------|---------------|---------------|
| POOL-01: Create pool with config | `snow_pool_open(url, min, max, timeout)` | `snow_pool_open` |
| POOL-02: Manual checkout/checkin | `snow_pool_checkout(pool)` / `snow_pool_checkin(pool, conn)` | `snow_pool_checkout`, `snow_pool_checkin` |
| POOL-03: Auto checkout-use-checkin | `snow_pool_query(pool, sql, params)` / `snow_pool_execute(pool, sql, params)` | `snow_pool_query`, `snow_pool_execute` |
| POOL-04: Health check idle connections | Health check in checkout path (SELECT 1) | Internal to checkout logic |
| POOL-05: Clean state on checkin | ROLLBACK if txn_status != 'I' | Internal to checkin logic |
| POOL-06: Pool.close drains connections | `snow_pool_close(pool)` sets closed=true, closes all idle, waits for active | `snow_pool_close` |
| POOL-07: Opaque u64 handles | `Box::into_raw(PgPool) as u64` | Handle pattern |
| TXN-01: Pg.begin/commit/rollback | Send SQL commands via existing wire protocol | `snow_pg_begin`, `snow_pg_commit`, `snow_pg_rollback` |
| TXN-02: Sqlite.begin/commit/rollback | Execute SQL via existing SQLite FFI | `snow_sqlite_begin`, `snow_sqlite_commit`, `snow_sqlite_rollback` |
| TXN-03: Track ReadyForQuery status | Add `txn_status: u8` field to PgConn, update on every ReadyForQuery | Modify existing code |
| TXN-04: Pg.transaction with auto commit/rollback | Closure wrapper with BEGIN/COMMIT/ROLLBACK | `snow_pg_transaction` |
| TXN-05: Rollback on panic | `catch_unwind` + ROLLBACK in panic path | Internal to `snow_pg_transaction` |

## Compiler Pipeline Additions

### New Snow API Surface
```
Pool.open(url, min, max, timeout) -> Result<PoolHandle, String>
Pool.close(pool)                  -> Unit
Pool.checkout(pool)               -> Result<PgConn, String>
Pool.checkin(pool, conn)          -> Unit
Pool.query(pool, sql, params)     -> Result<List<Map<String, String>>, String>
Pool.execute(pool, sql, params)   -> Result<Int, String>

Pg.begin(conn)                    -> Result<Unit, String>
Pg.commit(conn)                   -> Result<Unit, String>
Pg.rollback(conn)                 -> Result<Unit, String>
Pg.transaction(conn, fn)          -> Result<T, String>

Sqlite.begin(conn)                -> Result<Unit, String>
Sqlite.commit(conn)               -> Result<Unit, String>
Sqlite.rollback(conn)             -> Result<Unit, String>
```

### Files to Modify (Compiler Pipeline)
Each new intrinsic requires entries in all 4 locations:

1. **`snow-rt/src/db/pool.rs` (NEW):** Runtime implementation of all `snow_pool_*` functions
2. **`snow-rt/src/db/pg.rs` (MODIFY):** Add `txn_status` to PgConn, add `snow_pg_begin/commit/rollback/transaction`
3. **`snow-rt/src/db/sqlite.rs` (MODIFY):** Add `snow_sqlite_begin/commit/rollback`
4. **`snow-codegen/src/codegen/intrinsics.rs`:** LLVM function declarations for all new intrinsics
5. **`snow-codegen/src/mir/lower.rs`:** `known_functions` entries + `map_builtin_name` mappings + `STDLIB_MODULES` update
6. **`snow-codegen/src/mir/types.rs`:** `"PoolHandle" => MirType::Int`
7. **`snow-typeck/src/infer.rs`:** Pool module in `stdlib_modules()` + update `STDLIB_MODULE_NAMES`
8. **`snow-typeck/src/builtins.rs`:** PoolHandle type + all function signatures

### parking_lot vs std::sync

The project already uses `parking_lot::Mutex` (see `scheduler.rs:30`). The pool could use either:
- `parking_lot::Mutex` + `parking_lot::Condvar` -- consistent with existing runtime code
- `std::sync::Mutex` + `std::sync::Condvar` -- no additional import needed

**Recommendation:** Use `parking_lot::Mutex` + `parking_lot::Condvar` for consistency with the scheduler. The `parking_lot` crate is already a dependency.

## Open Questions

1. **Pool.query/execute signature for SQLite?**
   - What we know: POOL-01 through POOL-07 all specify PostgreSQL ("PostgreSQL connection pool"). The success criteria mention `Pool.open(url, config)` and `Pool.query(pool, sql, params)`.
   - What's unclear: Should the pool also work with SQLite connections? The FEATURES.md mentioned both, but the requirements are PG-specific.
   - Recommendation: Implement PG-only pool. SQLite connections are typically file-based and don't need pooling (no TCP/TLS overhead). If needed later, the same pattern works.

2. **Pg.transaction closure return type**
   - What we know: The closure returns `Result<T, String>`. On Ok, commit. On Err, rollback.
   - What's unclear: How does the planner handle the generic `T`? At the runtime level, everything is `*mut u8`.
   - Recommendation: At runtime, the closure returns `*mut u8` (a SnowResult). The runtime checks the tag (0=Ok, 1=Err) to decide commit vs rollback. The actual `T` inside the Ok variant is opaque to the runtime.

3. **Connection creation inside pool (thread safety)**
   - What we know: Creating a new PG connection involves TCP connect + TLS handshake + auth. This takes 10-150ms.
   - What's unclear: Should the Mutex be held during connection creation?
   - Recommendation: NO. Drop the lock before creating a connection. Increment `total_created` optimistically, create connection outside the lock, then re-acquire to add to idle queue. If creation fails, decrement `total_created`.

## Sources

### Primary (HIGH confidence)
- Snow codebase: `crates/snow-rt/src/db/pg.rs` -- existing PgConn, wire protocol, Box::into_raw handle pattern
- Snow codebase: `crates/snow-rt/src/db/sqlite.rs` -- existing SqliteConn, FFI pattern
- Snow codebase: `crates/snow-rt/src/actor/scheduler.rs` -- parking_lot::Mutex usage, catch_unwind pattern
- Snow codebase: `crates/snow-rt/src/http/server.rs` -- closure calling convention (fn_ptr/env_ptr)
- Snow codebase: `crates/snow-rt/src/collections/list.rs` -- closure calling convention (BareFn/ClosureFn)
- Snow codebase: `crates/snow-codegen/src/codegen/intrinsics.rs` -- intrinsic declaration pattern
- Snow codebase: `crates/snow-codegen/src/mir/lower.rs` -- known_functions + map_builtin_name pattern
- Snow codebase: `crates/snow-typeck/src/infer.rs` -- stdlib_modules() pattern
- Snow codebase: `crates/snow-codegen/src/mir/types.rs` -- opaque type -> MirType::Int pattern
- [PostgreSQL ReadyForQuery message format](https://www.postgresql.org/docs/current/protocol-message-formats.html) -- transaction status byte (I/T/E)

### Secondary (MEDIUM confidence)
- Snow `.planning/research/FEATURES.md` -- Pool and Transaction design direction, API surface examples
- Snow `.planning/phases/54-postgresql-driver/54-RESEARCH.md` -- compiler pipeline pattern documentation

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- no new dependencies, using established Rust primitives (Mutex, Condvar, catch_unwind)
- Architecture: HIGH -- follows exact patterns from Phases 53-56 (opaque handle, compiler pipeline, closure convention)
- Pitfalls: HIGH -- well-known connection pool pitfalls, verified against codebase patterns
- Transaction semantics: HIGH -- BEGIN/COMMIT/ROLLBACK are standard SQL, ReadyForQuery status byte is documented in PG protocol spec

**Research date:** 2026-02-12
**Valid until:** 2026-03-12 (stable domain, no fast-moving dependencies)
