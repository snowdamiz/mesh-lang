---
status: resolved
trigger: "SIGBUS (Bus error: 10) crash when concurrent HTTP requests hit mesher dashboard endpoints"
created: 2026-02-15
updated: 2026-02-15
resolved: 2026-02-15
resolution: "Coroutine stack overflow. meshc linker preferred stale debug libmesh_rt.a (64KB stacks) over release (512KB). Fixed linker profile preference order."
---

# Mesher Concurrent SIGBUS Crash - Debug Document

## 1. Problem Statement

The mesher binary crashes with `Bus error: 10` (SIGBUS, exit code 138) when the
frontend dashboard page is loaded. The dashboard page sends multiple concurrent
HTTP requests to various `/api/v1/projects/:project_id/dashboard/*` endpoints.
The crash is **100% reproducible** with concurrent requests and **never occurs**
with sequential requests.

### Reproduction Steps

1. Start the mesher: `./mesher/mesher`
2. Wait for `[Mesher] Foundation ready` and HTTP server listening
3. Send concurrent curl requests:
   ```bash
   curl http://localhost:8080/api/v1/projects/default/dashboard/health &
   curl http://localhost:8080/api/v1/projects/default/dashboard/volume &
   curl http://localhost:8080/api/v1/projects/default/dashboard/levels &
   curl http://localhost:8080/api/v1/projects/default/dashboard/top-issues &
   wait
   ```
4. The mesher process dies with exit code 138 (SIGBUS)

### Symptom Details

- Signal: SIGBUS (Bus error: 10, exit code 138)
- Platform: macOS ARM64 (Apple Silicon), Darwin 25.2.0
- When: Only when **multiple** dashboard endpoint requests arrive concurrently
- The frontend's `npm run dev` dashboard page triggers this naturally

---

## 2. Architecture Context

### The Mesh Language Runtime

The mesher is a compiled application written in the custom "Mesh" programming
language (`.mpl` files). The Mesh compiler (`meshc`) compiles `.mpl` source to
LLVM IR, which is then compiled to native ARM64 machine code.

Key runtime components (all in Rust, `crates/mesh-rt/`):

- **Actor System**: M:N scheduler with corosensei stackful coroutines
- **HTTP Server**: Actor-per-connection model, hand-rolled HTTP/1.1 parser
- **PG Pool**: Bounded PostgreSQL connection pool with Mutex+Condvar
- **GC**: Per-actor bump allocator heaps with mark-sweep collection

### HTTP Request Lifecycle

```
TCP accept (main thread / accept loop)
  -> spawn actor (coroutine) via scheduler
    -> connection_handler_entry() on coroutine stack
      -> parse_request() -- blocking I/O on TCP stream
      -> process_request() -- builds MeshHttpRequest, matches router
        -> call_handler() -- calls compiled Mesh handler function
          -> Mesh handler code:
            1. mesh_service_call(registry_pid, ...) -- YIELDS coroutine
               ... scheduler processes other actors ...
               ... service actor processes request, sends reply ...
               ... scheduler resumes this coroutine ...
            2. mesh_pool_query(pool, sql, params)
               -> mesh_pool_checkout(pool)
                  -> may create_connection() -- BLOCKS OS thread
                     -> mesh_pg_connect() -- TCP + SCRAM auth + rand::rng()
               -> mesh_pg_query(conn, sql, params) -- BLOCKS OS thread
               -> mesh_pool_checkin(pool, conn)
            3. Construct JSON response
        -> write_response() -- send HTTP response back
```

### Scheduler Design (crates/mesh-rt/src/actor/scheduler.rs)

- Uses `std::thread::available_parallelism()` worker threads (typically 8-12)
- Coroutines are `!Send` -- they are **thread-pinned** after creation
- Work-stealing operates only on spawn requests, not running coroutines
- Each worker has a local `suspended: Vec<(ProcessId, CoroutineHandle)>` list
- Workers poll suspended coroutines checking for Ready state (Waiting = skip)
- Backoff: first 100 idle iterations spin, then 100us sleep, then 1ms sleep
- At idle, scheduler workers consume ~300% CPU (spinning on Waiting actors)

### Thread-Local State (crates/mesh-rt/src/actor/stack.rs)

Three thread-locals track execution context:
- `CURRENT_YIELDER`: pointer to active coroutine's Yielder (for yield)
- `CURRENT_PID`: PID of currently running actor (for mesh_actor_self, GC alloc)
- `STACK_BASE`: base address of coroutine stack (for GC scanning)

These are set by the scheduler before `handle.resume()` and cleared after.

### Coroutine Stack Size (crates/mesh-rt/src/actor/process.rs)

```rust
pub const DEFAULT_STACK_SIZE: usize = 64 * 1024;  // 64 KiB
```

Corosensei's `DefaultStack::new(size)` allocates via mmap with a guard page
(PROT_NONE) at the bottom. Usable stack is approximately `size - 4KB`.

---

## 3. What Has Been Confirmed

### 3.1 Sequential Requests Work (CONFIRMED)

Single requests to any dashboard endpoint return correct JSON and the mesher
stays alive. Tested multiple times across sessions.

```
curl http://localhost:8080/api/v1/projects/default/dashboard/health
# Returns: {"unresolved_count":0,"events_24h":0,"new_today":0}
# Status: 200
# Mesher: ALIVE
```

### 3.2 Concurrent Requests Crash (CONFIRMED)

4+ concurrent requests to dashboard endpoints cause the mesher to die.
Reproduced consistently across many test runs.

```
# All 4 sent concurrently:
curl .../dashboard/health &
curl .../dashboard/volume &
curl .../dashboard/levels &
curl .../dashboard/top-issues &
# Result: 1 request gets 200, others get 000 (connection reset), mesher DEAD
```

### 3.3 Concurrent 404s Work (CONFIRMED)

Concurrent requests to non-existent routes do NOT crash the mesher:

```
# All 4 sent concurrently to non-existent route:
curl http://localhost:8080/nonexistent &  (x4)
# Result: All return 404, mesher ALIVE
```

This is key: 404 responses are generated entirely in Rust code
(`process_request` returns `(404, "Not Found")` at server.rs:815 without ever
calling compiled Mesh handler code). The crash only occurs when compiled Mesh
handler code executes.

### 3.4 Crash Location: ChaCha12Core::generate (from lldb)

Running under lldb revealed the crash point:

```
Thread #6:
  ChaCha12Core::generate(self=0x0, r=0x0)
  EXC_BAD_ACCESS (code=2, address=0x105d37c90)
```

- **code=2**: KERN_PROTECTION_FAILURE (accessing memory with wrong permissions)
- **address**: 0x105d37c90 (non-null, could be a guard page or read-only region)
- **ChaCha12Core::generate**: Part of the `rand` crate, called during PG SCRAM-SHA-256 auth nonce generation via `rand::rng()` in `scram_client_first()` (pg.rs:438)

### 3.5 Single Worker Thread Prevents Crash But Causes Deadlock (CONFIRMED)

Setting `num_threads = 1` in the scheduler (forcing single-threaded execution):
- The SIGBUS crash does NOT occur
- Instead, the system deadlocks (handlers block the single worker thread during
  pool checkout, preventing the scheduler from processing service actor replies)

This confirms the issue is specific to **multi-threaded concurrent execution**.

### 3.6 CPU at Idle: ~300% (OBSERVED)

Even with no requests, the mesher consumes ~308% CPU. This is the scheduler
workers spinning while checking Waiting actors. Each iteration reads the process
table (RwLock read) and checks process state (parking_lot Mutex lock). This high
contention may exacerbate concurrency issues.

---

## 4. What Has Been Eliminated

### 4.1 Stack Overflow (TESTED - NOT THE CAUSE)

| Stack Size | Result |
|-----------|--------|
| 64 KiB (default) | CRASH |
| 512 KiB | CRASH |
| 8 MiB | CRASH |

The crash persists even with 8MB stacks, ruling out coroutine stack overflow.
Note: The doc comment on `DEFAULT_STACK_SIZE` was updated to say "512 KiB" but
the value remained `64 * 1024`. This session changed it to `512 * 1024` and
confirmed the crash still occurs. (The 512KB change is still in the code as it's
a reasonable improvement regardless.)

### 4.2 Handler Serialization via Mutex (TESTED - INCONCLUSIVE)

A `static HANDLER_LOCK: std::sync::Mutex<()>` was added around `call_handler()`
to serialize all Mesh handler execution. Result: mesher still dies.

**However**, this test was **flawed and inconclusive**:
- The mutex blocks the OS worker thread
- Mesh handlers call `mesh_service_call()` which yields the coroutine
- The Mutex guard lives on the coroutine stack -- it remains "locked" while
  the coroutine is suspended
- Other coroutines on other worker threads trying to call handlers will block
  their worker threads on the mutex
- This creates deadlocks: service actors can't be processed because workers
  are blocked on the mutex, and the original handler can't resume because
  its service call never gets a reply
- The "DEAD" result was likely a deadlock timeout, not the SIGBUS

The HANDLER_LOCK has been removed from the codebase.

### 4.3 TargetData Codegen Bug (PREVIOUSLY FIXED)

Three earlier debug sessions found and fixed a codegen bug where
`TargetData::create("")` used LLVM's default x86-32 data layout instead of
arm64-apple-darwin. This caused struct sizes to be computed incorrectly,
truncating service state during tuple encoding. This was fixed by replacing
all instances with `self.target_machine.get_target_data()`.

**This fix resolved the ORIGINAL SIGBUS** that occurred during idle (timer
actors crashing). The current SIGBUS is a DIFFERENT bug that only manifests
under concurrent HTTP request load.

### 4.4 MIR Type Resolution (PREVIOUSLY FIXED)

A previous session also fixed `Ty::Var` mapping to `Unit` in MIR lowering,
which caused return values to be discarded. This was also committed.

### 4.5 Slug Resolution (PREVIOUSLY FIXED)

The frontend sends `"default"` as project_id (a slug, not a UUID). A previous
session added `resolve_project_id()` to map slugs to UUIDs before SQL queries.
This fixed the functional bug but NOT the crash.

### 4.6 Mutable Global State in Generated Code (CHECKED - NONE)

The generated LLVM IR (`mesher/mesher.ll`, 24K+ lines) was checked for mutable
global state. Only `unnamed_addr constant` string literals were found -- no
mutable globals. The generated code does not have shared mutable state.

---

## 5. Current State of the Code

### Files Modified (Uncommitted)

- `crates/mesh-rt/src/actor/process.rs`: `DEFAULT_STACK_SIZE` changed from
  `64 * 1024` to `512 * 1024` (doc comment updated to match)

### Files Modified (Already Committed by Previous Sessions)

- `crates/mesh-codegen/src/codegen/expr.rs`: TargetData fix (5 instances)
- `crates/mesh-codegen/src/codegen/mod.rs`: set_data_layout + TargetData fix
- `crates/mesh-codegen/src/mir/lower.rs`: Unit type resolution fallback
- `mesher/storage/schema.mpl`: slug column added
- `mesher/storage/queries.mpl`: get_project_id_by_slug added
- `mesher/api/helpers.mpl`: resolve_project_id added
- `mesher/api/dashboard.mpl`, `search.mpl`, `settings.mpl`, `team.mpl`,
  `alerts.mpl`: Updated handlers to use resolve_project_id

### HANDLER_LOCK Status

The `HANDLER_LOCK` static mutex was added to `server.rs` during debugging and
has been removed. The `call_handler()` function now calls compiled Mesh code
directly without any serialization wrapper. The current `server.rs` does NOT
have any HANDLER_LOCK.

---

## 6. Analysis of the Crash Mechanism

### What We Know

1. The crash is `EXC_BAD_ACCESS code=2` (protection fault) at a non-null address
2. It occurs in `rand_chacha::ChaCha12Core::generate`
3. `ChaCha12Core::generate` is called from `rand::rng()` during SCRAM auth
4. SCRAM auth happens in `mesh_pg_connect()` when creating new pool connections
5. New connections are created when `mesh_pool_checkout` finds no idle connections
6. Multiple handlers checking out connections simultaneously triggers this

### The Call Chain That Crashes

On worker thread N, inside a coroutine:
```
connection_handler_entry
  parse_request                     ~1-2KB stack
  process_request                   ~2-4KB stack
    call_handler
      [compiled Mesh handler]       ~???KB stack
        mesh_service_call           ~1KB stack
          mesh_actor_receive
            yield_current           SUSPENDED HERE
        [... resumed ...]
        mesh_pool_query
          mesh_pool_checkout        ~1KB stack
            [drops pool mutex]
            create_connection       ~2KB stack
              mesh_pg_connect
                negotiate_tls       ~1KB stack
                scram_client_first
                  rand::rng()
                    ChaCha12Core::generate  CRASH
```

### Why Only Under Concurrency?

When 4 concurrent requests arrive:
- The pool has min=2 pre-created connections
- 2 handlers get idle connections (no new creation needed)
- 2 handlers need NEW connections -> call `create_connection` -> SCRAM auth

With sequential requests, the pool always has an idle connection available
(min=2, and the previous request already checked in its connection). New
connections are rarely created.

### What Doesn't Make Sense

- `rand::rng()` uses thread-local storage, each thread has independent RNG state
- Coroutines are thread-pinned (never migrate between workers)
- The pool mutex correctly serializes shared state
- GC allocation uses per-actor heaps (mutex-protected, no cross-actor sharing)
- No mutable globals in generated code

### Possible Root Causes (Not Yet Tested)

#### A. Memory Corruption from Generated Code

The compiled Mesh handler code might corrupt memory in a way that only manifests
when multiple handlers run simultaneously on different threads. Possibilities:
- Buffer overwrite in codegen'd struct/tuple operations
- Incorrect pointer arithmetic in generated code
- Stack-allocated temporaries with incorrect sizes in generated LLVM IR

**How to test**: Replace all dashboard handlers with trivial "return 200"
handlers and check if crash persists.

#### B. Race Condition in ActorHeap / GC

`mesh_gc_alloc_actor` acquires a read lock on the process table, then locks
the specific process to allocate from its heap. Under high contention from
scheduler workers (which also read-lock the process table every iteration),
there might be a subtle ordering issue.

The GC's `mesh_gc_collect()` scans the coroutine stack for roots. If GC runs
on one actor while another actor's heap is being allocated from... but each
actor has its own heap and GC only collects the current actor's heap.

**How to test**: Disable GC collection entirely (make `mesh_gc_collect` a no-op)
and check if crash persists.

#### C. Use-After-Free in Pool Connection Management

`mesh_pool_checkout` drops the pool mutex before calling `create_connection`.
During connection creation, the coroutine's OS thread is blocked doing I/O.
If the pool's internal state is corrupted between the unlock and the connection
creation return... but the pool uses parking_lot::Mutex which is well-tested.

**How to test**: Set pool min=10, max=10 so no connections are ever created
during handling (all pre-created at startup).

#### D. Thread-Safety Issue in `mesh_pg_connect` During Concurrent Use

Multiple threads calling `mesh_pg_connect` simultaneously. Each does:
1. Parse URL (local variables, safe)
2. TCP connect (independent sockets, safe)
3. TLS negotiation via rustls (Arc<ServerConfig> shared, should be safe)
4. SCRAM auth using `rand::rng()` (thread-local, should be safe)
5. Read PG messages (independent streams, safe)
6. **Allocate MeshStrings via `mesh_gc_alloc_actor`** -- this accesses TLS
   (CURRENT_PID) and the process table (RwLock)

The MeshString allocations inside `mesh_pg_connect` (via `err_result`,
`rust_str_to_mesh`) go through `mesh_gc_alloc_actor` which reads CURRENT_PID
from TLS. If TLS is somehow corrupted or the wrong PID is set... but the
scheduler correctly sets CURRENT_PID before resume and the coroutine is
thread-pinned.

**How to test**: Run with `MallocScribble=1` or `MallocGuardEdges=1` to
detect use-after-free or buffer overflows.

#### E. Codegen Calling Convention Mismatch

The `call_handler` function transmutes a `*mut u8` function pointer to
`fn(*mut u8) -> *mut u8` or `fn(*mut u8, *mut u8) -> *mut u8`. If the
compiled Mesh function's actual ABI doesn't match (e.g., it expects
different register conventions), this could corrupt registers/stack.

This wouldn't be concurrency-specific per se, but might only crash under
certain memory layouts that occur more frequently with concurrent execution.

**How to test**: Inspect the generated LLVM IR function signatures for
dashboard handlers and verify they match the calling convention.

---

## 7. Recommended Next Steps (Priority Order)

### Step 1: Pre-Create All Pool Connections

Change `mesh_pool_open` to use min=10 (same as max) so ALL connections are
created at startup. This eliminates `create_connection` during handling and
removes the `rand::rng()` crash site from the hot path. If the crash goes
away, the issue is in concurrent connection creation. If it persists, the
issue is elsewhere in the handler execution.

This requires changing `mesher/main.mpl` (Pool.open min parameter).

### Step 2: Run with MallocScribble=1

```bash
MallocScribble=1 ./mesher/mesher
```

This fills freed memory with 0x55. If there's a use-after-free, the corrupt
data will be 0x5555... which is distinctive in crash dumps.

### Step 3: Get Full lldb Backtrace with Symbols

```bash
lldb -- ./mesher/mesher
(lldb) run
# In another terminal, send concurrent curls
# After crash:
(lldb) bt all
(lldb) register read
(lldb) memory read $sp-256 $sp+256
```

The full backtrace across ALL threads will show exactly where each thread
was when the crash occurred.

### Step 4: Instrument GC Allocator

Add bounds checking and canary values to `mesh_gc_alloc_actor` to detect
heap corruption:
- Write canary bytes before and after each allocation
- Verify canaries on subsequent allocations
- If canary is corrupted, print diagnostic and abort

### Step 5: Check Generated LLVM IR

Inspect the LLVM IR for dashboard handler functions:
- Verify function signatures match calling convention
- Check alloca sizes for correctness
- Look for potential buffer overflows in struct/tuple operations
- Verify all pointer operations use correct types and alignments

---

## 8. Build/Test Reference

### Building

```bash
# Build compiler and runtime
cd /Users/sn0w/Documents/dev/snow
cargo build --release

# Build mesher (requires cc linker in PATH)
PATH=./target/release:/usr/bin:$PATH meshc build mesher/
```

### Testing

```bash
# Kill stale processes
pkill -9 -f "mesher/mesher"

# Start mesher
./mesher/mesher &

# Wait for "Foundation ready" and HTTP server listening (~4 seconds)

# Test sequential (should work)
curl -s -o /dev/null -w "%{http_code}" http://localhost:8080/api/v1/projects/default/dashboard/health

# Test concurrent (triggers crash)
curl -s http://localhost:8080/api/v1/projects/default/dashboard/health &
curl -s http://localhost:8080/api/v1/projects/default/dashboard/volume &
curl -s http://localhost:8080/api/v1/projects/default/dashboard/levels &
curl -s http://localhost:8080/api/v1/projects/default/dashboard/top-issues &
wait
```

### API Routes (from mesher/main.mpl)

Dashboard endpoints that trigger the crash:
- `GET /api/v1/projects/:project_id/dashboard/health`
- `GET /api/v1/projects/:project_id/dashboard/volume`
- `GET /api/v1/projects/:project_id/dashboard/levels`
- `GET /api/v1/projects/:project_id/dashboard/top-issues`
- `GET /api/v1/projects/:project_id/dashboard/tags`

Other endpoints that also crash when concurrent:
- `GET /api/v1/projects/:project_id/issues`
- `GET /api/v1/projects/:project_id/settings`
- `GET /api/v1/projects/:project_id/alerts`

---

## 9. Resolution (Session 9)

### Root Cause: Coroutine Stack Overflow + Stale Linker Target

The crash was a **coroutine stack overflow**, not a race condition or memory corruption.

#### The Linker Bug

`crates/mesh-codegen/src/link.rs` (`find_mesh_rt()`) searched for `libmesh_rt.a`
in this order: `["debug", "release"]`. When both existed, it **always linked the
debug runtime**, regardless of which was built more recently.

The user built with `cargo build --release` (producing an updated release runtime
with 512KB stacks), but `meshc build mesher/` silently linked the stale debug
runtime (`target/debug/libmesh_rt.a`) which still had 64KB coroutine stacks.

This means **all previous debug sessions** that modified `DEFAULT_STACK_SIZE`,
pool parameters, or added diagnostic code in the release runtime were unknowingly
testing against the unchanged debug runtime. The "8MB stack test" from Session 6
never actually used 8MB stacks.

#### The Stack Overflow

The macOS crash report (`~/Library/Logs/DiagnosticReports/mesher-*.ips`) proved it:

```
VM_ALLOCATE  10abc0000-10abd0000  [64K] rw-/rwx  ← coroutine stack
VM_ALLOCATE  10abd0000-10abd4000  [16K] ---/rwx  ← GUARD PAGE (PROT_NONE)
VM_ALLOCATE  10abd4000-10abe4000  [64K] rw-/rwx  ← next coroutine stack

sp = 0x10abd3cb0  ← IN THE GUARD PAGE
ESR: "Data Abort byte write Translation fault"
```

The stack pointer grew downward through the 64KB stack into the 16KB guard page.
The deep call chain that overflows 64KB:

```
connection_handler_entry → catch_unwind → process_request
  → handle_project_health → resolve_project_id → get_project_id_by_slug
    → mesh_pool_query → mesh_pool_checkout → create_connection
      → mesh_pg_connect → scram_client_first → rand::rng()
        → ChaCha12Core::generate  ← STACK OVERFLOW HERE
```

#### Why Only Under Concurrency

Sequential requests reuse idle pool connections (min=2). No new connections are
created, so the deep SCRAM auth stack frames are never reached.

Concurrent requests exhaust idle connections, forcing `create_connection` →
`mesh_pg_connect` → SCRAM auth, which adds ~20 stack frames and overflows 64KB.

### Fix Applied

**`crates/mesh-codegen/src/link.rs`**: Changed `find_mesh_rt()` to prefer the
runtime matching meshc's own build profile using `cfg!(debug_assertions)`:
- Release meshc → prefers `target/release/libmesh_rt.a`
- Debug meshc → prefers `target/debug/libmesh_rt.a`

### Verification

After removing `target/debug/libmesh_rt.a` and rebuilding with the release
runtime (512KB stacks), 5 rounds of 4 concurrent dashboard requests all
returned HTTP 200 with the server staying alive.

Endpoints that do NOT crash (even concurrent):
- Any non-existent route (returns 404 from Rust code, no Mesh handler)

---

## 9. Key File Locations

| File | Description |
|------|-------------|
| `crates/mesh-rt/src/http/server.rs` | HTTP server, request handling, call_handler |
| `crates/mesh-rt/src/actor/scheduler.rs` | M:N scheduler, worker_loop, work stealing |
| `crates/mesh-rt/src/actor/stack.rs` | Coroutine management, TLS, yield_current |
| `crates/mesh-rt/src/actor/process.rs` | Process control block, DEFAULT_STACK_SIZE |
| `crates/mesh-rt/src/actor/service.rs` | mesh_service_call, reply mechanism |
| `crates/mesh-rt/src/actor/heap.rs` | Per-actor GC heap, GcHeader |
| `crates/mesh-rt/src/db/pool.rs` | PG connection pool, checkout/checkin |
| `crates/mesh-rt/src/db/pg.rs` | PG wire protocol, SCRAM auth, rand::rng() |
| `crates/mesh-rt/src/gc.rs` | GC allocation entry points, global arena |
| `mesher/main.mpl` | Mesher entry point, router config, pool setup |
| `mesher/api/dashboard.mpl` | Dashboard API handlers |
| `mesher/api/helpers.mpl` | resolve_project_id helper |
| `mesher/mesher.ll` | Generated LLVM IR (24K+ lines) |

---

## 10. Timeline of Debug Attempts

| # | Session | Hypothesis | Fix Applied | Result |
|---|---------|-----------|-------------|--------|
| 1 | Debugger agent 1 | TargetData::create("") wrong sizes | Replaced with target_machine.get_target_data() | Fixed idle SIGBUS, not concurrent |
| 2 | Debugger agent 2 | Missing set_data_layout | Added module.set_data_layout() | No change to concurrent crash |
| 3 | Debugger agent 3 | Ty::Var -> Unit discarding returns + missing slug resolution | Fixed MIR lowering + added resolve_project_id | No change to concurrent crash |
| 4 | Debugger agent 4 | Slug resolution needed across all handlers | Updated 16 handlers | No change to concurrent crash |
| 5 | Direct debugging | Stack overflow (64KB) | Tested 512KB, 8MB | Still crashes |
| 6 | Direct debugging | Concurrent handler execution | Added HANDLER_LOCK mutex | Inconclusive (deadlock) |
| 7 | Direct debugging | Single worker eliminates race | Set num_threads=1 | No crash but deadlock |
| 8 | This session | Stack size (doc comment mismatch) | Set 512KB properly | Still crashes |

**Key insight**: Debug attempts 1-4 (by subagents) all claimed to fix the issue
but tested with sequential requests or wrong endpoints. The concurrent crash was
never actually addressed by any of these fixes. The TargetData fix (attempt 1)
did fix a DIFFERENT SIGBUS that occurred at idle.
