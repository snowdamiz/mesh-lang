//! WebSocket room registry for pub/sub broadcast messaging.
//!
//! Provides a global concurrent registry mapping room names to sets of
//! connection handles. Connections can join named rooms, leave rooms, and
//! broadcast text frames to all members of a room.
//!
//! ## Architecture
//!
//! Modeled on `ProcessRegistry` (`crates/snow-rt/src/actor/registry.rs`):
//! - `rooms: RwLock<FxHashMap<String, HashSet<usize>>>` -- room to connections
//! - `conn_rooms: RwLock<FxHashMap<usize, HashSet<String>>>` -- reverse index
//!
//! Lock ordering: always acquire `rooms` first, then `conn_rooms` (nested,
//! consistent order to prevent deadlock).
//!
//! ## Runtime Functions
//!
//! - `snow_ws_join(conn, room)` -- subscribe connection to room
//! - `snow_ws_leave(conn, room)` -- unsubscribe connection from room
//! - `snow_ws_broadcast(room, msg)` -- send text frame to all in room
//! - `snow_ws_broadcast_except(room, msg, except)` -- send to all except one

use parking_lot::RwLock;
use rustc_hash::FxHashMap;
use std::collections::HashSet;
use std::sync::OnceLock;
use std::sync::atomic::Ordering;

use crate::string::SnowString;
use super::server::WsConnection;
use super::{write_frame, WsOpcode};

// ---------------------------------------------------------------------------
// RoomRegistry
// ---------------------------------------------------------------------------

/// Global room registry for WebSocket pub/sub.
///
/// Maps room names to sets of connection handles (WsConnection pointer as
/// usize), with a reverse index for O(rooms_per_conn) cleanup on disconnect.
pub struct RoomRegistry {
    /// room_name -> set of connection handles
    rooms: RwLock<FxHashMap<String, HashSet<usize>>>,
    /// connection_handle -> set of room names (reverse index for cleanup)
    conn_rooms: RwLock<FxHashMap<usize, HashSet<String>>>,
}

impl RoomRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        RoomRegistry {
            rooms: RwLock::new(FxHashMap::default()),
            conn_rooms: RwLock::new(FxHashMap::default()),
        }
    }

    /// Subscribe a connection to a named room.
    ///
    /// Lock ordering: rooms write, then conn_rooms write.
    pub fn join(&self, conn: usize, room: String) {
        let mut rooms = self.rooms.write();
        let mut conn_rooms = self.conn_rooms.write();
        rooms.entry(room.clone()).or_default().insert(conn);
        conn_rooms.entry(conn).or_default().insert(room);
    }

    /// Unsubscribe a connection from a room.
    ///
    /// Removes the connection from the room's member set, and removes the
    /// room from the connection's room set. Empty entries are cleaned up
    /// to prevent memory leaks.
    ///
    /// Lock ordering: rooms write, then conn_rooms write.
    pub fn leave(&self, conn: usize, room: &str) {
        let mut rooms = self.rooms.write();
        let mut conn_rooms = self.conn_rooms.write();

        if let Some(members) = rooms.get_mut(room) {
            members.remove(&conn);
            if members.is_empty() {
                rooms.remove(room);
            }
        }

        if let Some(room_set) = conn_rooms.get_mut(&conn) {
            room_set.remove(room);
            if room_set.is_empty() {
                conn_rooms.remove(&conn);
            }
        }
    }

    /// Remove a connection from all rooms. Called on disconnect.
    ///
    /// Removes the connection from the reverse index, then removes it from
    /// each room's member set. Empty rooms are cleaned up.
    ///
    /// Lock ordering: rooms write, then conn_rooms write.
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

    /// Get a snapshot of all connection handles in a room.
    ///
    /// Acquires only the rooms read lock, released before the caller iterates.
    pub fn members(&self, room: &str) -> Vec<usize> {
        self.rooms
            .read()
            .get(room)
            .map(|s| s.iter().copied().collect())
            .unwrap_or_default()
    }
}

// ---------------------------------------------------------------------------
// Global instance
// ---------------------------------------------------------------------------

/// The global room registry, lazily initialized.
static GLOBAL_ROOM_REGISTRY: OnceLock<RoomRegistry> = OnceLock::new();

/// Get a reference to the global room registry.
pub fn global_room_registry() -> &'static RoomRegistry {
    GLOBAL_ROOM_REGISTRY.get_or_init(RoomRegistry::new)
}

// ---------------------------------------------------------------------------
// Runtime functions (extern "C" for Snow codegen)
// ---------------------------------------------------------------------------

/// Subscribe a WebSocket connection to a named room.
///
/// `conn` is a pointer to a `WsConnection`. `room_name` is a pointer to a
/// `SnowString` containing the room name.
///
/// Returns 0 on success, -1 on null arguments.
#[no_mangle]
pub extern "C" fn snow_ws_join(conn: *mut u8, room_name: *const SnowString) -> i64 {
    if conn.is_null() || room_name.is_null() {
        return -1;
    }
    // Extract room name as owned String to prevent GC dangling reference (Pitfall 4)
    let room = unsafe { (*room_name).as_str().to_string() };
    global_room_registry().join(conn as usize, room);
    0
}

/// Unsubscribe a WebSocket connection from a named room.
///
/// `conn` is a pointer to a `WsConnection`. `room_name` is a pointer to a
/// `SnowString` containing the room name.
///
/// Returns 0 on success, -1 on null arguments.
#[no_mangle]
pub extern "C" fn snow_ws_leave(conn: *mut u8, room_name: *const SnowString) -> i64 {
    if conn.is_null() || room_name.is_null() {
        return -1;
    }
    let room = unsafe { (*room_name).as_str() };
    global_room_registry().leave(conn as usize, room);
    0
}

/// Broadcast a text frame to all connections in a named room.
///
/// `room_name` and `msg` are pointers to `SnowString` values. Snapshots the
/// member list (releases read lock), then iterates and writes to each
/// connection. Connections with the `shutdown` flag set are skipped.
///
/// Returns the number of write failures (0 = all succeeded), or -1 on null
/// arguments.
#[no_mangle]
pub extern "C" fn snow_ws_broadcast(
    room_name: *const SnowString,
    msg: *const SnowString,
) -> i64 {
    if room_name.is_null() || msg.is_null() {
        return -1;
    }
    let room = unsafe { (*room_name).as_str() };
    let text = unsafe { (*msg).as_str() };
    let payload = text.as_bytes();

    // Snapshot member list (read lock, released immediately)
    let members = global_room_registry().members(room);

    let mut failures = 0i64;
    for conn_usize in members {
        let conn = unsafe { &*(conn_usize as *const WsConnection) };
        // Check shutdown flag to avoid writing to closing connections
        if conn.shutdown.load(Ordering::SeqCst) {
            continue;
        }
        let mut stream = conn.write_stream.lock();
        if write_frame(&mut *stream, WsOpcode::Text, payload, true).is_err() {
            failures += 1;
        }
    }
    failures
}

/// Broadcast a text frame to all connections in a room except one.
///
/// Same as `snow_ws_broadcast` but skips the connection at `except_conn`.
/// `except_conn` can be null (treated as no exclusion).
///
/// Returns the number of write failures (0 = all succeeded), or -1 on null
/// room_name or msg.
#[no_mangle]
pub extern "C" fn snow_ws_broadcast_except(
    room_name: *const SnowString,
    msg: *const SnowString,
    except_conn: *mut u8,
) -> i64 {
    if room_name.is_null() || msg.is_null() {
        return -1;
    }
    let room = unsafe { (*room_name).as_str() };
    let text = unsafe { (*msg).as_str() };
    let payload = text.as_bytes();
    let except = except_conn as usize;

    // Snapshot member list (read lock, released immediately)
    let members = global_room_registry().members(room);

    let mut failures = 0i64;
    for conn_usize in members {
        if conn_usize == except {
            continue; // skip excluded connection
        }
        let conn = unsafe { &*(conn_usize as *const WsConnection) };
        // Check shutdown flag to avoid writing to closing connections
        if conn.shutdown.load(Ordering::SeqCst) {
            continue;
        }
        let mut stream = conn.write_stream.lock();
        if write_frame(&mut *stream, WsOpcode::Text, payload, true).is_err() {
            failures += 1;
        }
    }
    failures
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Create a fresh registry for testing (avoids global state interference).
    fn fresh_registry() -> RoomRegistry {
        RoomRegistry::new()
    }

    #[test]
    fn test_join_and_members() {
        let reg = fresh_registry();
        reg.join(100, "lobby".to_string());
        reg.join(200, "lobby".to_string());
        reg.join(100, "chat".to_string());

        let mut members = reg.members("lobby");
        members.sort();
        assert_eq!(members, vec![100, 200]);

        let members = reg.members("chat");
        assert_eq!(members, vec![100]);

        assert!(reg.members("nonexistent").is_empty());
    }

    #[test]
    fn test_leave() {
        let reg = fresh_registry();
        reg.join(100, "lobby".to_string());
        reg.join(200, "lobby".to_string());

        reg.leave(100, "lobby");

        let members = reg.members("lobby");
        assert_eq!(members, vec![200]);
    }

    #[test]
    fn test_leave_removes_empty_room() {
        let reg = fresh_registry();
        reg.join(100, "temp".to_string());
        reg.leave(100, "temp");

        // Room should be removed entirely
        assert!(reg.members("temp").is_empty());
        assert!(reg.rooms.read().get("temp").is_none());
    }

    #[test]
    fn test_cleanup_connection() {
        let reg = fresh_registry();
        reg.join(100, "lobby".to_string());
        reg.join(100, "chat".to_string());
        reg.join(100, "game".to_string());
        reg.join(200, "lobby".to_string());

        reg.cleanup_connection(100);

        // 100 should be gone from all rooms
        assert_eq!(reg.members("lobby"), vec![200]);
        assert!(reg.members("chat").is_empty());
        assert!(reg.members("game").is_empty());

        // Empty rooms should be cleaned up
        assert!(reg.rooms.read().get("chat").is_none());
        assert!(reg.rooms.read().get("game").is_none());

        // conn_rooms reverse index should be cleaned up
        assert!(reg.conn_rooms.read().get(&100).is_none());
    }

    #[test]
    fn test_cleanup_nonexistent_connection_is_noop() {
        let reg = fresh_registry();
        // Should not panic
        reg.cleanup_connection(999);
    }

    #[test]
    fn test_join_same_room_twice_is_idempotent() {
        let reg = fresh_registry();
        reg.join(100, "lobby".to_string());
        reg.join(100, "lobby".to_string());

        let members = reg.members("lobby");
        assert_eq!(members, vec![100]);
    }

    #[test]
    fn test_leave_nonexistent_room_is_noop() {
        let reg = fresh_registry();
        reg.join(100, "lobby".to_string());
        // Should not panic
        reg.leave(100, "nonexistent");
        // Original membership unchanged
        assert_eq!(reg.members("lobby"), vec![100]);
    }

    #[test]
    fn test_concurrent_join_leave() {
        use std::sync::Arc;

        let reg = Arc::new(fresh_registry());
        let num_threads = 8;

        let handles: Vec<_> = (0..num_threads)
            .map(|t| {
                let reg = Arc::clone(&reg);
                std::thread::spawn(move || {
                    let conn = (t + 1) * 100;
                    reg.join(conn, "shared".to_string());
                    // Verify own membership
                    let members = reg.members("shared");
                    assert!(members.contains(&conn));
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }

        let members = reg.members("shared");
        assert_eq!(members.len(), num_threads);
    }

    #[test]
    fn test_null_args_return_negative_one() {
        assert_eq!(snow_ws_join(std::ptr::null_mut(), std::ptr::null()), -1);
        assert_eq!(snow_ws_leave(std::ptr::null_mut(), std::ptr::null()), -1);
        assert_eq!(snow_ws_broadcast(std::ptr::null(), std::ptr::null()), -1);
        assert_eq!(
            snow_ws_broadcast_except(std::ptr::null(), std::ptr::null(), std::ptr::null_mut()),
            -1
        );
    }
}
