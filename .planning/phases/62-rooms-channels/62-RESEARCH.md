# Phase 62: Rooms & Channels - Research

**Researched:** 2026-02-12
**Domain:** WebSocket pub/sub room registry, concurrent membership management, actor exit cleanup
**Confidence:** HIGH

## Summary

Phase 62 adds a room-based pub/sub system on top of the WebSocket actor-per-connection model built in Phases 59-61. Connections can join named rooms, broadcast messages to all members of a room, and are automatically removed from all rooms when they disconnect. The central data structure is a **room registry** -- a global concurrent map from room names to sets of connection handles -- that closely mirrors the existing `ProcessRegistry` in `crates/snow-rt/src/actor/registry.rs`.

The existing codebase provides all the patterns needed. The `ProcessRegistry` maps `String -> ProcessId` with automatic cleanup on process exit. The room registry maps `String -> HashSet<connection_handle>` with identical cleanup semantics. The `WsConnection` struct in `ws/server.rs` already holds an `Arc<Mutex<WsStream>>` for writing frames, which is exactly what broadcast needs to iterate over room members and write to each. The `handle_process_exit` function in `scheduler.rs` already calls `registry::global_registry().cleanup_process(pid)` -- the room registry cleanup hooks into this same exit path.

The API surface consists of four new runtime functions (`snow_ws_join`, `snow_ws_leave`, `snow_ws_broadcast`, `snow_ws_broadcast_except`) plus corresponding codegen wiring (intrinsic declarations, known_functions entries, map_builtin_name mappings). No new Cargo dependencies are needed. The implementation is entirely within `crates/snow-rt/src/ws/` (runtime) and `crates/snow-codegen/src/` (codegen).

**Primary recommendation:** Build a `RoomRegistry` struct modeled on `ProcessRegistry`, using `RwLock<FxHashMap<String, HashSet<usize>>>` for room->connections and a reverse index `RwLock<FxHashMap<usize, HashSet<String>>>` for connection->rooms. The connection handle (`WsConnection` pointer as `usize`) is used as the key. Broadcast iterates the room's connection set, locks each connection's `Arc<Mutex<WsStream>>`, and writes a text frame. Cleanup on disconnect calls `room_registry.cleanup_connection(conn_ptr)` from the actor entry's cleanup path.

## Standard Stack

### Core (already in codebase -- no new dependencies)
| Component | Location | Purpose | Why Standard |
|-----------|----------|---------|--------------|
| `ProcessRegistry` pattern | `crates/snow-rt/src/actor/registry.rs` | Template for RoomRegistry design | Proven concurrent registry with RwLock, reverse index, cleanup |
| `WsConnection` + `Arc<Mutex<WsStream>>` | `crates/snow-rt/src/ws/server.rs` | Connection handle with write access | Already holds the write stream for `Ws.send` |
| `write_frame` | `crates/snow-rt/src/ws/frame.rs` | Frame writing for broadcast | Already used by `snow_ws_send` |
| `parking_lot::RwLock` | dependency | Concurrent room registry access | Already used by `ProcessRegistry` |
| `rustc_hash::FxHashMap` | dependency | Fast hash maps | Already used by `ProcessRegistry` |
| `std::collections::HashSet` | stdlib | Room membership sets | Already used for process links |

### No New Dependencies Required

All required functionality is available through existing crate dependencies and the standard library. The room registry is a pure Rust data structure using `parking_lot::RwLock`, `rustc_hash::FxHashMap`, and `std::collections::HashSet`, all already present.

## Architecture Patterns

### Recommended Module Structure
```
crates/snow-rt/src/ws/
    mod.rs          # existing -- add re-exports for room functions
    frame.rs        # existing (Phase 59) -- unchanged
    handshake.rs    # existing (Phase 59) -- unchanged
    close.rs        # existing (Phase 59) -- unchanged
    server.rs       # existing (Phase 60/61) -- add room runtime functions, cleanup hook
    rooms.rs        # NEW -- RoomRegistry, global instance, cleanup logic
```

### Pattern 1: RoomRegistry (modeled on ProcessRegistry)
**What:** A global concurrent registry mapping room names to sets of connection handles, with a reverse index for efficient cleanup.
**When to use:** All room join/leave/broadcast/cleanup operations.
**Source pattern:** `crates/snow-rt/src/actor/registry.rs`

The `ProcessRegistry` uses:
- `names: RwLock<FxHashMap<String, ProcessId>>` -- forward map
- `pid_names: RwLock<FxHashMap<ProcessId, Vec<String>>>` -- reverse index for cleanup

The `RoomRegistry` mirrors this:
- `rooms: RwLock<FxHashMap<String, HashSet<usize>>>` -- room -> connection handles
- `conn_rooms: RwLock<FxHashMap<usize, HashSet<String>>>` -- connection -> rooms (reverse index)

Key difference: rooms are many-to-many (a connection can join multiple rooms, a room has multiple connections), whereas process names are one-to-one.

```rust
use parking_lot::RwLock;
use rustc_hash::FxHashMap;
use std::collections::HashSet;
use std::sync::OnceLock;

pub struct RoomRegistry {
    /// room_name -> set of connection handles (as usize from *mut WsConnection)
    rooms: RwLock<FxHashMap<String, HashSet<usize>>>,
    /// connection_handle -> set of room names (reverse index for cleanup)
    conn_rooms: RwLock<FxHashMap<usize, HashSet<String>>>,
}

impl RoomRegistry {
    pub fn new() -> Self {
        RoomRegistry {
            rooms: RwLock::new(FxHashMap::default()),
            conn_rooms: RwLock::new(FxHashMap::default()),
        }
    }

    pub fn join(&self, conn: usize, room: String) {
        self.rooms.write()
            .entry(room.clone())
            .or_default()
            .insert(conn);
        self.conn_rooms.write()
            .entry(conn)
            .or_default()
            .insert(room);
    }

    pub fn leave(&self, conn: usize, room: &str) {
        if let Some(members) = self.rooms.write().get_mut(room) {
            members.remove(&conn);
            if members.is_empty() {
                self.rooms.write().remove(room);
            }
        }
        if let Some(rooms) = self.conn_rooms.write().get_mut(&conn) {
            rooms.remove(room);
            if rooms.is_empty() {
                self.conn_rooms.write().remove(&conn);
            }
        }
    }

    /// Remove a connection from all rooms. Called on disconnect.
    pub fn cleanup_connection(&self, conn: usize) {
        let rooms_to_leave = {
            self.conn_rooms.write().remove(&conn).unwrap_or_default()
        };
        if !rooms_to_leave.is_empty() {
            let mut rooms = self.rooms.write();
            for room_name in &rooms_to_leave {
                if let Some(members) = rooms.get_mut(room_name) {
                    members.remove(&conn);
                    if members.is_empty() {
                        rooms.remove(room_name);
                    }
                }
            }
        }
    }

    /// Get all connection handles in a room (snapshot).
    pub fn members(&self, room: &str) -> Vec<usize> {
        self.rooms.read()
            .get(room)
            .map(|s| s.iter().copied().collect())
            .unwrap_or_default()
    }
}

static GLOBAL_ROOM_REGISTRY: OnceLock<RoomRegistry> = OnceLock::new();

pub fn global_room_registry() -> &'static RoomRegistry {
    GLOBAL_ROOM_REGISTRY.get_or_init(RoomRegistry::new)
}
```

### Pattern 2: Connection Handle as Room Key
**What:** Use the `WsConnection` raw pointer (as `usize`) as the key in the room registry, not `ProcessId`.
**Why:** The `WsConnection` pointer is what `Ws.join(conn, room)` receives from Snow code. It's the opaque handle passed to all `Ws.*` functions. Using it directly avoids a lookup from conn_ptr to PID. The conn_ptr is unique per connection (Box::into_raw guarantees unique allocation) and stable for the connection's lifetime.

**Important safety consideration:** The `WsConnection` is allocated via `Box::into_raw` in `ws_connection_entry` and freed via `Box::from_raw` in the same function's cleanup path. Between allocation and freeing, the pointer is stable and unique. The room registry must not dereference stale pointers -- cleanup must happen BEFORE the WsConnection is freed.

### Pattern 3: Broadcast via Connection Handle Dereference
**What:** `snow_ws_broadcast` gets the list of connection handles for a room, then for each handle, dereferences it to get the `WsConnection`, locks its `write_stream`, and writes a text frame.
**Why:** This reuses the exact same write path as `snow_ws_send`, just applied to multiple connections.

```rust
#[no_mangle]
pub extern "C" fn snow_ws_broadcast(
    room_name: *const SnowString,
    msg: *const SnowString,
) -> i64 {
    if room_name.is_null() || msg.is_null() { return -1; }
    let room = unsafe { (*room_name).as_str() };
    let text = unsafe { (*msg).as_str() };

    let members = global_room_registry().members(room);
    let mut failures = 0i64;
    for conn_usize in members {
        let conn = unsafe { &*(conn_usize as *const WsConnection) };
        let mut stream = conn.write_stream.lock();
        if write_frame(&mut *stream, WsOpcode::Text, text.as_bytes(), true).is_err() {
            failures += 1;
        }
    }
    failures
}
```

### Pattern 4: Disconnect Cleanup in Actor Entry
**What:** When a WebSocket connection actor exits (normal or crash), remove the connection from all rooms BEFORE freeing the `WsConnection`.
**When to use:** In the cleanup path of `ws_connection_entry` (server.rs).
**Source:** The cleanup currently does `shutdown.store(true, ...)`, sends close frame, calls `on_close`, and frees the connection handle. The room cleanup must happen BEFORE `drop(Box::from_raw(conn))`.

```rust
// In ws_connection_entry cleanup (server.rs):
// ... existing cleanup ...

// Room cleanup (ROOM-05): remove from all rooms BEFORE freeing conn handle
rooms::global_room_registry().cleanup_connection(conn as usize);

// Clean up connection handle (existing)
unsafe { drop(Box::from_raw(conn)); }
```

### Pattern 5: Codegen Wiring (four new functions)
**What:** Wire `Ws.join`, `Ws.leave`, `Ws.broadcast`, `Ws.broadcast_except` through the codegen pipeline.
**Source pattern:** Existing `ws_serve` / `ws_send` wiring in `lower.rs` and `intrinsics.rs`.

The "Ws" module is already in `STDLIB_MODULES`. Adding new functions requires:

1. **`map_builtin_name`** (lower.rs): Map `ws_join` -> `snow_ws_join`, etc.
2. **`known_functions`** (lower.rs): Type signatures for MIR lowering.
3. **`declare_intrinsics`** (intrinsics.rs): LLVM function declarations.

```rust
// map_builtin_name:
"ws_join" => "snow_ws_join".to_string(),
"ws_leave" => "snow_ws_leave".to_string(),
"ws_broadcast" => "snow_ws_broadcast".to_string(),
"ws_broadcast_except" => "snow_ws_broadcast_except".to_string(),

// known_functions:
// snow_ws_join(conn: ptr, room: ptr) -> i64
self.known_functions.insert("snow_ws_join".to_string(),
    MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Int)));
// snow_ws_leave(conn: ptr, room: ptr) -> i64
self.known_functions.insert("snow_ws_leave".to_string(),
    MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Int)));
// snow_ws_broadcast(room: ptr, msg: ptr) -> i64
self.known_functions.insert("snow_ws_broadcast".to_string(),
    MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr], Box::new(MirType::Int)));
// snow_ws_broadcast_except(room: ptr, msg: ptr, conn: ptr) -> i64
self.known_functions.insert("snow_ws_broadcast_except".to_string(),
    MirType::FnPtr(vec![MirType::Ptr, MirType::Ptr, MirType::Ptr], Box::new(MirType::Int)));

// intrinsics.rs:
// snow_ws_join(conn: ptr, room: ptr) -> i64
module.add_function("snow_ws_join", i64_type.fn_type(
    &[ptr_type.into(), ptr_type.into()], false),
    Some(Linkage::External));
// snow_ws_leave(conn: ptr, room: ptr) -> i64
module.add_function("snow_ws_leave", i64_type.fn_type(
    &[ptr_type.into(), ptr_type.into()], false),
    Some(Linkage::External));
// snow_ws_broadcast(room: ptr, msg: ptr) -> i64
module.add_function("snow_ws_broadcast", i64_type.fn_type(
    &[ptr_type.into(), ptr_type.into()], false),
    Some(Linkage::External));
// snow_ws_broadcast_except(room: ptr, msg: ptr, conn: ptr) -> i64
module.add_function("snow_ws_broadcast_except", i64_type.fn_type(
    &[ptr_type.into(), ptr_type.into(), ptr_type.into()], false),
    Some(Linkage::External));
```

### Anti-Patterns to Avoid
- **Using ProcessId as the room key:** The Snow-level API passes `conn` (a `WsConnection*`), not a PID. Using PID would require a lookup table from PID to conn, adding unnecessary indirection. Use the conn pointer directly.
- **Holding write locks during broadcast iteration:** Do NOT hold the room registry's write lock while iterating and writing frames. Take a read lock to snapshot the member list, drop the lock, then iterate. This prevents deadlock and allows other connections to join/leave during broadcast.
- **Cleaning up rooms after freeing WsConnection:** The cleanup must happen BEFORE `drop(Box::from_raw(conn))` because `cleanup_connection` does not dereference the pointer -- it only uses it as a key. But broadcast DOES dereference pointers. If a broadcast races with cleanup, the broadcast could dereference a freed pointer. The cleanup must remove from the registry first, THEN free the connection.
- **Acquiring RwLock write during read:** In `leave()`, do not hold a read lock on `rooms` while trying to write-lock `conn_rooms` or vice versa. Use separate lock acquisitions to prevent deadlock. The `ProcessRegistry` avoids this by using separate locks for forward and reverse maps.
- **Empty room accumulation:** When all connections leave a room, remove the room entry from the map. Otherwise, room names accumulate in memory forever. The `cleanup_connection` and `leave` methods must check `members.is_empty()` and remove the entry.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Concurrent map | Custom concurrent hash map | `parking_lot::RwLock<FxHashMap<...>>` | Already proven in ProcessRegistry, handles concurrent reads efficiently |
| Frame writing | Custom broadcast writer | `write_frame` from `frame.rs` | Already handles all frame encodings, masking, and length formats |
| Connection handle | Custom handle type | Existing `WsConnection*` as `usize` | Already allocated per connection, unique, stable for connection lifetime |
| Reverse index for cleanup | Linear scan of all rooms | `conn_rooms: RwLock<FxHashMap<usize, HashSet<String>>>` | O(rooms_per_conn) cleanup instead of O(total_rooms), same pattern as ProcessRegistry's `pid_names` |
| Process exit hook | Custom exit monitoring | Inline cleanup in `ws_connection_entry` | The actor entry function already has a cleanup section that runs on both normal exit and crash |

**Key insight:** The room registry is structurally identical to the process registry, just with different key/value types. The broadcast operation is just `snow_ws_send` in a loop. The cleanup hook is a one-line call in the existing actor cleanup path. There is no novel complexity in this phase.

## Common Pitfalls

### Pitfall 1: Use-After-Free in Broadcast During Disconnect
**What goes wrong:** Connection A is disconnecting. The room cleanup removes A from the registry, but a concurrent broadcast on another thread already has A's pointer in its snapshot and tries to dereference it after it's been freed.
**Why it happens:** The broadcast snapshots the member list (read lock), then iterates and dereferences pointers (no lock). Between snapshotting and dereferencing, a connection could be freed.
**How to avoid:** Ensure the disconnect cleanup path is: (1) remove from room registry, (2) signal reader thread shutdown, (3) send close frame, (4) call on_close, (5) THEN free WsConnection. Step (1) must happen first. For broadcast, after snapshotting, check if each connection's `shutdown` flag is set before writing. Better yet: the `write_frame` call will simply fail (return Err) if the stream is closed, which is already handled by counting failures. The real protection is that `cleanup_connection` removes the conn from the registry BEFORE `Box::from_raw(conn)` frees it. Any broadcast that snapshots before the removal will see the conn; any broadcast that snapshots after will not. A broadcast that snapshots before but dereferences after the free would be a UAF. To prevent this: move room cleanup to the VERY FIRST thing in the cleanup path, before shutdown is signaled. Since the WsConnection is not freed until the end, the pointer remains valid throughout.
**Warning signs:** Intermittent crashes (SIGSEGV) during high-load broadcast with connections disconnecting.

**Recommended cleanup order:**
```
1. rooms::global_room_registry().cleanup_connection(conn_ptr as usize)  // remove from rooms
2. shutdown.store(true, ...)                                             // signal reader thread
3. send close frame (if crash: 1011, if normal: 1000)
4. call on_close callback
5. drop(Box::from_raw(conn))                                            // free connection
```

### Pitfall 2: Deadlock from Nested Lock Acquisition
**What goes wrong:** `join()` acquires write lock on `rooms`, then tries to acquire write lock on `conn_rooms`. If another thread is in `cleanup_connection` holding `conn_rooms` write lock and waiting for `rooms` write lock, deadlock occurs.
**Why it happens:** The `ProcessRegistry` also acquires two locks, but in `register()` it acquires `names` write then `pid_names` write -- always in the same order. If `cleanup_connection` acquires `conn_rooms` then `rooms`, the order is reversed.
**How to avoid:** Always acquire locks in a consistent order: `rooms` first, then `conn_rooms`. Alternatively, use separate lock scopes (acquire, release, acquire) like `ProcessRegistry.cleanup_process()` does -- it acquires `pid_names` write, extracts names, releases, then acquires `names` write.
**Warning signs:** Server hangs under load, especially during concurrent joins and disconnects.

### Pitfall 3: Broadcast Blocks on Slow Connections
**What goes wrong:** During broadcast, one connection's `write_stream.lock()` takes a long time (e.g., contending with the reader thread's 100ms read timeout), blocking the broadcast for all subsequent connections.
**Why it happens:** The broadcast iterates connections sequentially, locking each one's mutex. The reader thread holds this same mutex during `read_frame` (up to 100ms due to the read timeout).
**How to avoid:** This is an acceptable trade-off for Phase 62. The worst-case broadcast time is O(N * 100ms) where N is the number of connections in the room. For small rooms (< 100 connections), this is under 10 seconds, which is fine. For future optimization, broadcast could be parallelized (spawn tasks per connection) or use a dedicated write queue per connection. For now, document this limitation.
**Warning signs:** Broadcast latency increases linearly with room size.

### Pitfall 4: Room Name as SnowString Pointer Lifetime
**What goes wrong:** The room name is passed as a `*const SnowString`. If the caller's SnowString is freed before the room registry copies it, the registry holds a dangling reference.
**Why it happens:** The room name is a Snow-level string allocated on the actor's heap. If `Ws.join(conn, room)` is followed by a GC cycle that collects the room string, the registry's reference is invalid.
**How to avoid:** In `snow_ws_join`, immediately extract the room name as a Rust `String` (via `(*room_name).as_str().to_string()`) before passing it to the registry. The registry stores owned `String` values, not SnowString pointers. This is exactly what `snow_actor_register` does (mod.rs lines 631-637).
**Warning signs:** Garbled room names, crashes in room lookup.

### Pitfall 5: Forgetting to Export from ws/mod.rs
**What goes wrong:** New room functions compile but are not visible to the linker because they're not exported from the `ws` module.
**Why it happens:** Rust's visibility rules require `pub` items in submodules to be re-exported from the parent module.
**How to avoid:** Add `pub mod rooms;` to `ws/mod.rs` and re-export the public functions. The `#[no_mangle] pub extern "C"` functions in `rooms.rs` or `server.rs` will be visible to the linker regardless, but re-exporting keeps the module structure clean and allows Rust-side testing.
**Warning signs:** Linker errors during `cargo test`.

### Pitfall 6: Empty Broadcast Returns Silently
**What goes wrong:** `Ws.broadcast("nonexistent_room", msg)` succeeds without error, but no messages are sent.
**Why it happens:** The room has no members (or doesn't exist), so the broadcast loop runs zero iterations.
**How to avoid:** This is actually correct behavior -- broadcasting to an empty room is a no-op. Document this in the API. Return 0 (zero failures) for empty rooms.
**Warning signs:** None -- this is expected behavior.

## Code Examples

### Example 1: snow_ws_join Runtime Function
```rust
// Source: follows pattern from snow_ws_send in ws/server.rs
#[no_mangle]
pub extern "C" fn snow_ws_join(conn: *mut u8, room_name: *const SnowString) -> i64 {
    if conn.is_null() || room_name.is_null() { return -1; }
    let room = unsafe { (*room_name).as_str().to_string() };
    global_room_registry().join(conn as usize, room);
    0 // success
}
```

### Example 2: snow_ws_leave Runtime Function
```rust
#[no_mangle]
pub extern "C" fn snow_ws_leave(conn: *mut u8, room_name: *const SnowString) -> i64 {
    if conn.is_null() || room_name.is_null() { return -1; }
    let room = unsafe { (*room_name).as_str() };
    global_room_registry().leave(conn as usize, room);
    0 // success
}
```

### Example 3: snow_ws_broadcast Runtime Function
```rust
#[no_mangle]
pub extern "C" fn snow_ws_broadcast(
    room_name: *const SnowString,
    msg: *const SnowString,
) -> i64 {
    if room_name.is_null() || msg.is_null() { return -1; }
    let room = unsafe { (*room_name).as_str() };
    let text = unsafe { (*msg).as_str() };
    let payload = text.as_bytes();

    // Snapshot member list (read lock, released immediately)
    let members = global_room_registry().members(room);

    let mut failures = 0i64;
    for conn_usize in members {
        let conn = unsafe { &*(conn_usize as *const WsConnection) };
        // Check shutdown flag to avoid writing to closing connections
        if conn.shutdown.load(std::sync::atomic::Ordering::SeqCst) {
            continue;
        }
        let mut stream = conn.write_stream.lock();
        if write_frame(&mut *stream, WsOpcode::Text, payload, true).is_err() {
            failures += 1;
        }
    }
    failures
}
```

### Example 4: snow_ws_broadcast_except Runtime Function
```rust
#[no_mangle]
pub extern "C" fn snow_ws_broadcast_except(
    room_name: *const SnowString,
    msg: *const SnowString,
    except_conn: *mut u8,
) -> i64 {
    if room_name.is_null() || msg.is_null() { return -1; }
    let room = unsafe { (*room_name).as_str() };
    let text = unsafe { (*msg).as_str() };
    let payload = text.as_bytes();
    let except = except_conn as usize;

    let members = global_room_registry().members(room);

    let mut failures = 0i64;
    for conn_usize in members {
        if conn_usize == except { continue; } // skip excluded connection
        let conn = unsafe { &*(conn_usize as *const WsConnection) };
        if conn.shutdown.load(std::sync::atomic::Ordering::SeqCst) {
            continue;
        }
        let mut stream = conn.write_stream.lock();
        if write_frame(&mut *stream, WsOpcode::Text, payload, true).is_err() {
            failures += 1;
        }
    }
    failures
}
```

### Example 5: Disconnect Cleanup Hook (in ws_connection_entry)
```rust
// In ws_connection_entry cleanup path (server.rs), BEFORE freeing conn:
// Existing:
//   shutdown.store(true, Ordering::SeqCst);
//   send close frame
//   call on_close

// NEW: Room cleanup (ROOM-05)
crate::ws::rooms::global_room_registry().cleanup_connection(conn as usize);

// Existing:
//   unsafe { drop(Box::from_raw(conn)); }
```

### Example 6: Snow-Level Usage (expected API)
```snow
Ws.serve(%{
  on_connect: fn(conn, path, headers) ->
    Ws.join(conn, "lobby")
    :ok
  end,

  on_message: fn(conn, msg) ->
    # Broadcast to all in the room except sender
    Ws.broadcast_except("lobby", msg, conn)
    :ok
  end,

  on_close: fn(conn, code, reason) ->
    # Ws.leave is not strictly needed -- cleanup on disconnect
    # handles it automatically (ROOM-05). But explicit leave
    # is useful for switching rooms.
    :ok
  end
}, 8080)
```

## Detailed Design Decisions

### Decision 1: Connection Handle (usize) vs ProcessId as Room Key

**Connection handle (recommended):** Use `conn_ptr as usize` -- the raw pointer to `WsConnection`.
- PRO: Direct -- `Ws.join(conn, room)` passes `conn` which IS the pointer. No lookup needed.
- PRO: Unique per connection (Box::into_raw guarantees unique allocation).
- PRO: Stable for connection lifetime (freed only in cleanup path).
- CON: Not human-readable in debug output.

**ProcessId alternative:** Use the actor's PID as the room key.
- PRO: More semantic, easier to debug.
- CON: Requires mapping conn_ptr -> PID (additional lookup).
- CON: PID is not directly available from conn_ptr without storing it in WsConnection.
- CON: Broadcast still needs conn_ptr to write frames (would need PID -> conn_ptr reverse map).

**Recommendation:** Use `conn_ptr as usize`. The registry never dereferences this pointer (it's just a key). Only broadcast dereferences it, and broadcast already has the pointer. This avoids adding any fields to `WsConnection` and avoids a PID-to-conn lookup table.

### Decision 2: RoomRegistry Location -- New File vs server.rs

**New file `rooms.rs` (recommended):**
- PRO: Clean separation of concerns. Room registry is conceptually distinct from the WebSocket server.
- PRO: `server.rs` is already 1200+ lines. Adding 150+ lines of room code would make it harder to navigate.
- PRO: Tests for room logic can be self-contained.
- CON: One more file to manage.

**In server.rs:**
- PRO: Everything in one place.
- CON: server.rs becomes very large and mixes connection lifecycle with room management.

**Recommendation:** Create `rooms.rs` for the `RoomRegistry` struct, global instance, and runtime functions. The cleanup hook call is a one-liner in `server.rs`. This matches the pattern where `registry.rs` is separate from `scheduler.rs`.

### Decision 3: Lock Granularity for Broadcast

**Single RwLock on the whole registry (recommended for Phase 62):**
- PRO: Simple implementation. Read lock during broadcast allows concurrent broadcasts.
- PRO: Write lock for join/leave/cleanup is brief (hash map insert/remove).
- CON: All rooms share one lock -- broadcast to room A blocks join to room B.

**Per-room lock (future optimization):**
- PRO: Broadcast to room A doesn't block join to room B.
- CON: More complex -- need a concurrent map of room name to `Arc<RwLock<HashSet<usize>>>`.
- CON: Cleanup becomes harder (need to iterate all rooms the connection belongs to).

**Recommendation:** Use the simple `RwLock<FxHashMap>` approach for Phase 62. The `ProcessRegistry` uses this pattern and it works well. If room contention becomes an issue, per-room locks can be added later.

### Decision 4: Broadcast Return Value

**Return failure count (recommended):**
- `0`: all sends succeeded
- `N > 0`: N connections failed (write errors)
- `-1`: invalid arguments (null pointers)

This gives the caller actionable information without requiring a complex result type. Failed connections likely have closed streams and will be cleaned up when their actor exits.

### Decision 5: Ws.broadcast Argument Order

Following the phase requirements:
- `Ws.broadcast(room, message)` -- room first, then message
- `Ws.broadcast_except(room, message, conn)` -- room, message, then excluded connection

At the runtime level:
- `snow_ws_broadcast(room_name: ptr, msg: ptr) -> i64`
- `snow_ws_broadcast_except(room_name: ptr, msg: ptr, except_conn: ptr) -> i64`

Note: `Ws.join(conn, room)` and `Ws.leave(conn, room)` take conn first (matches `Ws.send(conn, msg)` convention), while `Ws.broadcast(room, msg)` takes room first (the room is the "target" like PID in `send`).

### Decision 6: RwLock Acquisition Order (Deadlock Prevention)

To prevent deadlock, all operations must acquire locks in the same order. Consistent order: `rooms` lock first, then `conn_rooms` lock.

- `join()`: write `rooms`, then write `conn_rooms` (correct order)
- `leave()`: write `rooms`, then write `conn_rooms` (correct order)
- `cleanup_connection()`: Following `ProcessRegistry.cleanup_process()` pattern -- acquire `conn_rooms` write to extract room names, release it, then acquire `rooms` write to remove entries. This reverses the order but is safe because there's no interleaving -- each lock is acquired and released independently.

Actually, `ProcessRegistry.cleanup_process()` acquires `pid_names` first, then `names`. If `register()` acquires `names` first, then `pid_names`, this could deadlock. Looking at the actual code: `cleanup_process()` acquires `pid_names` write, extracts names, drops it, THEN acquires `names` write. Since the first lock is dropped before the second is acquired, there is no nesting and no deadlock risk.

**Recommendation:** Use the same non-nested pattern:
- `join()`: acquire `rooms` write, insert, drop. Acquire `conn_rooms` write, insert, drop.
- `leave()`: acquire `rooms` write, remove, drop. Acquire `conn_rooms` write, remove, drop.
- `cleanup_connection()`: acquire `conn_rooms` write, extract room names, drop. Acquire `rooms` write, remove entries, drop.

This means there's a tiny window between the two operations where the state is inconsistent (e.g., in `join`, the connection is in `rooms` but not yet in `conn_rooms`). This is harmless -- if cleanup happens in this window, `cleanup_connection` won't find the connection in `conn_rooms`, so it won't try to remove it from `rooms`. But it's already in `rooms` -- this is a leak. Solution: `cleanup_connection` should also do a brute-force sweep of `rooms` to catch this edge case. OR, avoid the window by nesting locks (acquire both in the same order always).

**Revised recommendation:** Use nested locks in a consistent order. Always acquire `rooms` write first, then `conn_rooms` write:

```rust
pub fn join(&self, conn: usize, room: String) {
    let mut rooms = self.rooms.write();
    let mut conn_rooms = self.conn_rooms.write();
    rooms.entry(room.clone()).or_default().insert(conn);
    conn_rooms.entry(conn).or_default().insert(room);
}

pub fn cleanup_connection(&self, conn: usize) {
    let mut rooms = self.rooms.write();
    let mut conn_rooms = self.conn_rooms.write();
    if let Some(room_names) = conn_rooms.remove(&conn) {
        for room_name in room_names {
            if let Some(members) = rooms.get_mut(&room_name) {
                members.remove(&conn);
                if members.is_empty() {
                    rooms.remove(&room_name);
                }
            }
        }
    }
}
```

This is simpler and deadlock-free because both locks are always acquired in the same order. The downside is that concurrent operations on different rooms are serialized through the write lock, but this is acceptable for Phase 62.

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Direct `Ws.send(conn, msg)` only | `Ws.broadcast(room, msg)` for group messaging | Phase 62 (this phase) | Enables chat rooms, multiplayer, pub/sub |
| Manual connection tracking | Automatic room registry with cleanup | Phase 62 (this phase) | No leaked connections on disconnect |
| N/A | ProcessRegistry pattern reuse | Phase 62 (this phase) | Proven concurrency pattern |

## Open Questions

1. **Should empty rooms be removed from the registry?**
   - What we know: When all connections leave a room, the `HashSet` is empty. Keeping it wastes memory; removing it means the room "doesn't exist" until someone joins again.
   - What's unclear: Whether Snow programs will check if a room exists (there's no `Ws.room_exists` API in the requirements).
   - Recommendation: Remove empty rooms immediately. They can be recreated implicitly when the first connection joins. This prevents memory leaks from creating many transient rooms.

2. **Should broadcast send binary frames in addition to text?**
   - What we know: ROOM-03 specifies "sends a text frame." There's no ROOM requirement for binary broadcast.
   - What's unclear: Whether future phases will need binary broadcast.
   - Recommendation: Only implement text broadcast for Phase 62 (matching the requirements). Binary broadcast can be added later following the exact same pattern with `WsOpcode::Binary`.

3. **Thread safety of connection pointer dereference during broadcast**
   - What we know: The broadcast snapshots member pointers, then dereferences each. A concurrent disconnect could free the WsConnection between snapshot and dereference.
   - What's unclear: The exact ordering guarantee between room cleanup and WsConnection freeing.
   - Recommendation: The cleanup order in `ws_connection_entry` must be: (1) room cleanup, (2) reader thread shutdown, (3) close frame, (4) on_close, (5) free WsConnection. Additionally, check the `shutdown` flag before writing in broadcast -- if set, skip that connection. This provides a cheap guard against writing to a connection that's in the process of closing.

## Sources

### Primary (HIGH confidence)
- **`crates/snow-rt/src/actor/registry.rs`** -- ProcessRegistry: RwLock<FxHashMap>, reverse index, cleanup_process, OnceLock global instance. Exact structural template for RoomRegistry.
- **`crates/snow-rt/src/ws/server.rs`** -- WsConnection struct (Arc<Mutex<WsStream>>, shutdown flag), ws_connection_entry cleanup path, snow_ws_send/snow_ws_send_binary (frame writing pattern), WsHandler struct, reserved type tags.
- **`crates/snow-rt/src/actor/scheduler.rs`** -- handle_process_exit: calls registry::global_registry().cleanup_process(pid). Shows where process-exit cleanup hooks are invoked.
- **`crates/snow-rt/src/actor/mod.rs`** -- snow_actor_send (message delivery + wake pattern), snow_actor_register/whereis (string argument handling pattern from Snow level).
- **`crates/snow-rt/src/ws/frame.rs`** -- write_frame function signature and usage.
- **`crates/snow-rt/src/ws/mod.rs`** -- Module re-exports pattern.
- **`crates/snow-codegen/src/codegen/intrinsics.rs`** -- LLVM function declarations for snow_ws_* functions. Pattern for adding new intrinsics.
- **`crates/snow-codegen/src/mir/lower.rs`** -- known_functions, STDLIB_MODULES (includes "Ws"), map_builtin_name for ws_serve/ws_send/etc.
- **`.planning/phases/60-actor-integration/60-RESEARCH.md`** -- Actor-per-connection architecture, reader thread bridge, reserved type tags, connection handle design.
- **`.planning/phases/61-production-hardening/61-RESEARCH.md`** -- Unified WsStream, Arc<Mutex<WsStream>> pattern, heartbeat, fragmentation.

### Secondary (MEDIUM confidence)
- **`parking_lot::RwLock` documentation** -- Confirms reader-writer semantics: multiple concurrent readers, exclusive writers. Used extensively in the codebase.

### Tertiary (LOW confidence)
- None -- all critical claims verified against existing codebase code.

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- all components exist in the codebase, no new dependencies. RoomRegistry is a direct mirror of ProcessRegistry.
- Architecture: HIGH -- room registry follows proven ProcessRegistry pattern. Broadcast uses existing write_frame. Cleanup hooks into existing actor exit path. Codegen wiring follows exact precedent from Phase 60/61 ws_serve/ws_send.
- Pitfalls: HIGH -- use-after-free risk identified from direct code analysis of ws_connection_entry lifecycle. Deadlock risk analyzed from lock ordering in ProcessRegistry. Broadcast latency analyzed from 100ms reader thread timeout.
- Codegen wiring: HIGH -- exact precedent from existing ws_serve/ws_send/ws_serve_tls in intrinsics.rs, lower.rs, and map_builtin_name.

**Research date:** 2026-02-12
**Valid until:** Indefinite (codebase-internal research, no external dependencies)
