//! Global process name registry, replicated across all cluster nodes.
//!
//! Unlike the local `ProcessRegistry` (node-scoped), the global registry
//! provides cluster-wide name registration. Every node holds a complete
//! replica of the name table; lookups are always local (no network).
//!
//! ## Replication Strategy
//!
//! Fully replicated with asynchronous broadcast:
//! - `register()` stores locally and broadcasts `DIST_GLOBAL_REGISTER`
//! - `unregister()` removes locally and broadcasts `DIST_GLOBAL_UNREGISTER`
//! - On node connect, a `DIST_GLOBAL_SYNC` snapshot is exchanged
//! - On node disconnect, all names owned by that node are cleaned up
//! - On process exit, all global names for that PID are cleaned up
//!
//! ## Lock Design
//!
//! All three maps are wrapped in a single `RwLock<GlobalRegistryInner>` to
//! avoid deadlocks and ensure consistency between the forward and reverse
//! indexes.

use parking_lot::RwLock;
use rustc_hash::FxHashMap;
use std::sync::OnceLock;

use crate::actor::process::ProcessId;

// ---------------------------------------------------------------------------
// GlobalRegistryInner -- the three maps under a single lock
// ---------------------------------------------------------------------------

/// Inner state of the global registry, protected by a single RwLock.
struct GlobalRegistryInner {
    /// name -> (PID, owning_node_name) mapping
    names: FxHashMap<String, (ProcessId, String)>,
    /// PID -> names reverse index for efficient cleanup on process exit
    pid_names: FxHashMap<ProcessId, Vec<String>>,
    /// node_name -> names reverse index for efficient cleanup on node disconnect
    node_names: FxHashMap<String, Vec<String>>,
}

impl GlobalRegistryInner {
    fn new() -> Self {
        GlobalRegistryInner {
            names: FxHashMap::default(),
            pid_names: FxHashMap::default(),
            node_names: FxHashMap::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// GlobalRegistry
// ---------------------------------------------------------------------------

/// Global process name registry, replicated across all cluster nodes.
///
/// Unlike the local `ProcessRegistry`, this tracks the owning node name
/// for each registration to enable cleanup when a node disconnects.
pub struct GlobalRegistry {
    inner: RwLock<GlobalRegistryInner>,
}

impl GlobalRegistry {
    /// Create a new empty global registry.
    pub fn new() -> Self {
        GlobalRegistry {
            inner: RwLock::new(GlobalRegistryInner::new()),
        }
    }

    /// Register a name globally.
    ///
    /// Returns `Ok(())` if the name was successfully registered, or
    /// `Err(message)` if the name is already taken.
    pub fn register(&self, name: String, pid: ProcessId, node_name: String) -> Result<(), String> {
        let mut inner = self.inner.write();

        if let Some((existing_pid, _)) = inner.names.get(&name) {
            return Err(format!(
                "name '{}' already globally registered to {}",
                name, existing_pid
            ));
        }

        inner.names.insert(name.clone(), (pid, node_name.clone()));
        inner.pid_names.entry(pid).or_default().push(name.clone());
        inner.node_names.entry(node_name).or_default().push(name);

        Ok(())
    }

    /// Look up a globally registered name.
    ///
    /// Always local -- no network call. Returns `Some(pid)` if registered,
    /// `None` otherwise.
    pub fn whereis(&self, name: &str) -> Option<ProcessId> {
        let inner = self.inner.read();
        inner.names.get(name).map(|(pid, _)| *pid)
    }

    /// Unregister a globally registered name.
    ///
    /// Removes from all three maps. Returns `true` if the name was found
    /// and removed, `false` if not found.
    pub fn unregister(&self, name: &str) -> bool {
        let mut inner = self.inner.write();

        if let Some((pid, node_name)) = inner.names.remove(name) {
            // Remove from pid_names reverse index.
            if let Some(list) = inner.pid_names.get_mut(&pid) {
                list.retain(|n| n != name);
                if list.is_empty() {
                    inner.pid_names.remove(&pid);
                }
            }
            // Remove from node_names reverse index.
            if let Some(list) = inner.node_names.get_mut(&node_name) {
                list.retain(|n| n != name);
                if list.is_empty() {
                    inner.node_names.remove(&node_name);
                }
            }
            true
        } else {
            false
        }
    }

    /// Remove all registrations owned by a specific node.
    ///
    /// Called when a node disconnects. Returns the list of removed names
    /// (for broadcasting unregister messages to remaining nodes).
    pub fn cleanup_node(&self, node_name: &str) -> Vec<String> {
        let mut inner = self.inner.write();

        let names_to_remove = inner.node_names.remove(node_name).unwrap_or_default();

        if !names_to_remove.is_empty() {
            for name in &names_to_remove {
                if let Some((pid, _)) = inner.names.remove(name) {
                    if let Some(list) = inner.pid_names.get_mut(&pid) {
                        list.retain(|n| n != name);
                        if list.is_empty() {
                            inner.pid_names.remove(&pid);
                        }
                    }
                }
            }
        }

        names_to_remove
    }

    /// Remove all registrations for a specific PID.
    ///
    /// Called when a local process exits. Returns the list of removed names
    /// (for broadcasting unregister messages to other nodes).
    pub fn cleanup_process(&self, pid: ProcessId) -> Vec<String> {
        let mut inner = self.inner.write();

        let names_to_remove = inner.pid_names.remove(&pid).unwrap_or_default();

        if !names_to_remove.is_empty() {
            for name in &names_to_remove {
                if let Some((_, node_name)) = inner.names.remove(name) {
                    if let Some(list) = inner.node_names.get_mut(&node_name) {
                        list.retain(|n| n != name);
                        if list.is_empty() {
                            inner.node_names.remove(&node_name);
                        }
                    }
                }
            }
        }

        names_to_remove
    }

    /// Get all current registrations as a snapshot for syncing to a newly
    /// connected node.
    ///
    /// Returns `(name, pid, owning_node_name)` tuples.
    pub fn snapshot(&self) -> Vec<(String, ProcessId, String)> {
        let inner = self.inner.read();
        inner
            .names
            .iter()
            .map(|(name, (pid, node))| (name.clone(), *pid, node.clone()))
            .collect()
    }

    /// Bulk-insert registrations from a remote node's sync snapshot.
    ///
    /// Idempotent: skips names that are already registered (first-writer wins).
    pub fn merge_snapshot(&self, entries: Vec<(String, ProcessId, String)>) {
        let mut inner = self.inner.write();

        for (name, pid, node_name) in entries {
            // Skip if already registered (idempotent merge).
            if inner.names.contains_key(&name) {
                continue;
            }
            inner.names.insert(name.clone(), (pid, node_name.clone()));
            inner.pid_names.entry(pid).or_default().push(name.clone());
            inner.node_names.entry(node_name).or_default().push(name);
        }
    }
}

impl Default for GlobalRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Global singleton
// ---------------------------------------------------------------------------

/// The global name registry, lazily initialized.
static GLOBAL_NAME_REGISTRY: OnceLock<GlobalRegistry> = OnceLock::new();

/// Get a reference to the global name registry.
pub fn global_name_registry() -> &'static GlobalRegistry {
    GLOBAL_NAME_REGISTRY.get_or_init(GlobalRegistry::new)
}

// ---------------------------------------------------------------------------
// Broadcast functions (pub(crate) for use from actor/mod.rs)
// ---------------------------------------------------------------------------

/// Broadcast a global register event to all connected nodes.
///
/// Follows the `send_peer_list` pattern: collect session references under
/// read lock, drop lock, then iterate and write to each stream.
pub(crate) fn broadcast_global_register(name: &str, pid: ProcessId, node_name: &str) {
    let state = match super::node::node_state() {
        Some(s) => s,
        None => return,
    };

    let mut payload = vec![super::node::DIST_GLOBAL_REGISTER];
    encode_entry(&mut payload, name, pid, node_name);

    // Collect session references, then drop sessions lock before writing.
    let sessions: Vec<std::sync::Arc<super::node::NodeSession>> = {
        let map = state.sessions.read();
        map.values().map(std::sync::Arc::clone).collect()
    };

    for session in &sessions {
        let _ = session.send(super::node::OutboundClass::Control, payload.clone());
    }
}

/// Broadcast a global unregister event to all connected nodes.
///
/// Follows the same broadcast pattern as `broadcast_global_register`.
pub(crate) fn broadcast_global_unregister(name: &str) {
    let state = match super::node::node_state() {
        Some(s) => s,
        None => return,
    };

    let mut payload = vec![super::node::DIST_GLOBAL_UNREGISTER];
    encode_str(&mut payload, name);

    // Collect session references, then drop sessions lock before writing.
    let sessions: Vec<std::sync::Arc<super::node::NodeSession>> = {
        let map = state.sessions.read();
        map.values().map(std::sync::Arc::clone).collect()
    };

    for session in &sessions {
        let _ = session.send(super::node::OutboundClass::Control, payload.clone());
    }
}

// ---------------------------------------------------------------------------
// Sync-on-connect (Phase 68, Plan 03)
// ---------------------------------------------------------------------------

/// Send our global registry snapshot to a newly connected node.
///
/// Called right after `send_peer_list` on both server (accept) and client
/// (connect) sides so that both nodes converge to the union of all known
/// global names. An empty snapshot is sent too: it tells a connecting peer
/// that it now knows every name there is.
pub(crate) fn send_global_sync(session: &std::sync::Arc<super::node::NodeSession>) {
    let registry = global_name_registry();
    let snapshot = registry.snapshot();

    let mut payload = vec![super::node::DIST_GLOBAL_SYNC];
    payload.extend_from_slice(&(snapshot.len() as u32).to_le_bytes());
    for (name, pid, node_name) in &snapshot {
        encode_entry(&mut payload, name, *pid, node_name);
    }

    let _ = session.send(super::node::OutboundClass::Snapshot, payload);
}

// ---------------------------------------------------------------------------
// Wire format: a string is `[u16 len][bytes]`; a name's entry is its name,
// `[u64 pid]` and its node's name.
// ---------------------------------------------------------------------------

fn encode_str(payload: &mut Vec<u8>, text: &str) {
    payload.extend_from_slice(&(text.len() as u16).to_le_bytes());
    payload.extend_from_slice(text.as_bytes());
}

fn encode_entry(payload: &mut Vec<u8>, name: &str, pid: ProcessId, node_name: &str) {
    encode_str(payload, name);
    payload.extend_from_slice(&pid.as_u64().to_le_bytes());
    encode_str(payload, node_name);
}

/// The string at `bytes[*pos..]`, moving `pos` past it; `None` when it is
/// cut short or not UTF-8.
pub(crate) fn decode_str(bytes: &[u8], pos: &mut usize) -> Option<String> {
    let len = u16::from_le_bytes(bytes.get(*pos..*pos + 2)?.try_into().ok()?) as usize;
    let text = std::str::from_utf8(bytes.get(*pos + 2..*pos + 2 + len)?).ok()?;
    *pos += 2 + len;
    Some(text.to_string())
}

/// The entry at `bytes[*pos..]`, with its pid as the sender wrote it.
pub(crate) fn decode_entry(bytes: &[u8], pos: &mut usize) -> Option<(String, u64, String)> {
    let name = decode_str(bytes, pos)?;
    let pid = u64::from_le_bytes(bytes.get(*pos..*pos + 8)?.try_into().ok()?);
    *pos += 8;
    Some((name, pid, decode_str(bytes, pos)?))
}

/// A `DIST_GLOBAL_SYNC` frame's entries, as many as decode. Its count is the
/// peer's word; the frame's length is what bounds them.
pub(crate) fn decode_sync(msg: &[u8]) -> Vec<(String, u64, String)> {
    let Some(count) = msg.get(1..5) else {
        return Vec::new();
    };
    let count = u32::from_le_bytes(count.try_into().unwrap());
    let mut pos = 5;
    (0..count)
        .map_while(|_| decode_entry(msg, &mut pos))
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Create a fresh registry for testing (avoids global state interference).
    fn fresh_registry() -> GlobalRegistry {
        GlobalRegistry::new()
    }

    #[test]
    fn test_register_and_whereis() {
        let reg = fresh_registry();
        let pid = ProcessId::next();

        reg.register("db_service".to_string(), pid, "node1@host".to_string())
            .unwrap();

        assert_eq!(reg.whereis("db_service"), Some(pid));
        assert_eq!(reg.whereis("nonexistent"), None);
    }

    #[test]
    fn test_register_duplicate_name_fails() {
        let reg = fresh_registry();
        let pid1 = ProcessId::next();
        let pid2 = ProcessId::next();

        reg.register("server".to_string(), pid1, "node1@host".to_string())
            .unwrap();
        let err = reg
            .register("server".to_string(), pid2, "node2@host".to_string())
            .unwrap_err();

        assert!(err.contains("already globally registered"));
    }

    #[test]
    fn test_unregister() {
        let reg = fresh_registry();
        let pid = ProcessId::next();

        reg.register("temp".to_string(), pid, "node1@host".to_string())
            .unwrap();
        assert!(reg.whereis("temp").is_some());

        let removed = reg.unregister("temp");
        assert!(removed);
        assert_eq!(reg.whereis("temp"), None);

        // Unregistering again returns false.
        let removed_again = reg.unregister("temp");
        assert!(!removed_again);
    }

    #[test]
    fn test_cleanup_node_removes_all_names() {
        let reg = fresh_registry();
        let pid1 = ProcessId::next();
        let pid2 = ProcessId::next();

        reg.register("svc1".to_string(), pid1, "node_a@host".to_string())
            .unwrap();
        reg.register("svc2".to_string(), pid2, "node_a@host".to_string())
            .unwrap();
        reg.register(
            "svc3".to_string(),
            ProcessId::next(),
            "node_b@host".to_string(),
        )
        .unwrap();

        let removed = reg.cleanup_node("node_a@host");
        assert_eq!(removed.len(), 2);
        assert!(removed.contains(&"svc1".to_string()));
        assert!(removed.contains(&"svc2".to_string()));

        assert_eq!(reg.whereis("svc1"), None);
        assert_eq!(reg.whereis("svc2"), None);
        // svc3 on node_b should still exist.
        assert!(reg.whereis("svc3").is_some());
    }

    #[test]
    fn test_cleanup_process_removes_all_names() {
        let reg = fresh_registry();
        let pid = ProcessId::next();

        reg.register("name1".to_string(), pid, "node1@host".to_string())
            .unwrap();
        reg.register("name2".to_string(), pid, "node1@host".to_string())
            .unwrap();

        let removed = reg.cleanup_process(pid);
        assert_eq!(removed.len(), 2);

        assert_eq!(reg.whereis("name1"), None);
        assert_eq!(reg.whereis("name2"), None);
    }

    #[test]
    fn test_cleanup_nonexistent_is_noop() {
        let reg = fresh_registry();
        let pid = ProcessId::next();

        let removed = reg.cleanup_process(pid);
        assert!(removed.is_empty());

        let removed = reg.cleanup_node("ghost@host");
        assert!(removed.is_empty());
    }

    #[test]
    fn test_snapshot_and_merge() {
        let reg1 = fresh_registry();
        let pid1 = ProcessId::next();
        let pid2 = ProcessId::next();

        reg1.register("svc_a".to_string(), pid1, "node1@host".to_string())
            .unwrap();
        reg1.register("svc_b".to_string(), pid2, "node1@host".to_string())
            .unwrap();

        let snap = reg1.snapshot();
        assert_eq!(snap.len(), 2);

        // Merge into a fresh registry.
        let reg2 = fresh_registry();
        reg2.merge_snapshot(snap);

        assert_eq!(reg2.whereis("svc_a"), Some(pid1));
        assert_eq!(reg2.whereis("svc_b"), Some(pid2));
    }

    #[test]
    fn test_merge_snapshot_idempotent() {
        let reg = fresh_registry();
        let pid1 = ProcessId::next();
        let pid2 = ProcessId::next();

        reg.register("existing".to_string(), pid1, "node1@host".to_string())
            .unwrap();

        // Try to merge a snapshot that includes the same name with a different PID.
        reg.merge_snapshot(vec![(
            "existing".to_string(),
            pid2,
            "node2@host".to_string(),
        )]);

        // The original registration should be preserved (first-writer wins).
        assert_eq!(reg.whereis("existing"), Some(pid1));
    }

    #[test]
    fn test_register_after_cleanup_succeeds() {
        let reg = fresh_registry();
        let pid1 = ProcessId::next();
        let pid2 = ProcessId::next();

        reg.register("server".to_string(), pid1, "node1@host".to_string())
            .unwrap();
        reg.cleanup_process(pid1);

        // Name should now be available for re-registration.
        reg.register("server".to_string(), pid2, "node2@host".to_string())
            .unwrap();
        assert_eq!(reg.whereis("server"), Some(pid2));
    }

    #[test]
    fn test_register_two_names_both_resolve() {
        let reg = fresh_registry();
        let pid1 = ProcessId::next();
        let pid2 = ProcessId::next();

        reg.register("service_a".to_string(), pid1, "node1@host".to_string())
            .unwrap();
        reg.register("service_b".to_string(), pid2, "node2@host".to_string())
            .unwrap();

        assert_eq!(reg.whereis("service_a"), Some(pid1));
        assert_eq!(reg.whereis("service_b"), Some(pid2));
    }

    #[test]
    fn test_register_duplicate_preserves_original() {
        let reg = fresh_registry();
        let pid1 = ProcessId::next();
        let pid2 = ProcessId::next();

        reg.register("lock_mgr".to_string(), pid1, "node1@host".to_string())
            .unwrap();
        let _ = reg.register("lock_mgr".to_string(), pid2, "node2@host".to_string());

        // Original mapping must be preserved.
        assert_eq!(reg.whereis("lock_mgr"), Some(pid1));
    }

    #[test]
    fn test_cleanup_process_preserves_other_pids() {
        let reg = fresh_registry();
        let pid_a = ProcessId::next();
        let pid_b = ProcessId::next();

        reg.register("x".to_string(), pid_a, "node1@host".to_string())
            .unwrap();
        reg.register("y".to_string(), pid_a, "node1@host".to_string())
            .unwrap();
        reg.register("z".to_string(), pid_b, "node1@host".to_string())
            .unwrap();

        let removed = reg.cleanup_process(pid_a);
        assert_eq!(removed.len(), 2);
        assert!(removed.contains(&"x".to_string()));
        assert!(removed.contains(&"y".to_string()));

        // pid_b's name must still be present.
        assert_eq!(reg.whereis("z"), Some(pid_b));
    }

    #[test]
    fn test_merge_snapshot_skips_existing_names() {
        let reg = fresh_registry();
        let pid1 = ProcessId::next();
        let pid2 = ProcessId::next();

        reg.register("a".to_string(), pid1, "node1@host".to_string())
            .unwrap();

        // Merge a snapshot containing name "a" with a different PID.
        reg.merge_snapshot(vec![("a".to_string(), pid2, "node2@host".to_string())]);

        // Existing registration wins -- "a" still maps to pid1.
        assert_eq!(reg.whereis("a"), Some(pid1));
    }

    #[test]
    fn test_concurrent_register_whereis() {
        use std::sync::Arc;

        let reg = Arc::new(fresh_registry());
        let num_threads = 8;

        let handles: Vec<_> = (0..num_threads)
            .map(|t| {
                let reg = Arc::clone(&reg);
                std::thread::spawn(move || {
                    let pid = ProcessId::next();
                    let name = format!("global_worker_{}", t);
                    reg.register(name.clone(), pid, "node@host".to_string())
                        .unwrap();
                    assert_eq!(reg.whereis(&name), Some(pid));
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }

        for t in 0..num_threads {
            let name = format!("global_worker_{}", t);
            assert!(
                reg.whereis(&name).is_some(),
                "global_worker_{} should be registered",
                t
            );
        }
    }

    // -----------------------------------------------------------------------
    // Wire format
    // -----------------------------------------------------------------------

    #[test]
    fn entries_round_trip_and_stop_where_the_frame_ends() {
        let mut payload = vec![super::super::node::DIST_GLOBAL_SYNC];
        payload.extend_from_slice(&3u32.to_le_bytes());
        encode_entry(&mut payload, "svc_alpha", ProcessId(100), "node_a@host");
        encode_entry(&mut payload, "svc_beta", ProcessId(200), "node_b@host");
        assert_eq!(
            decode_sync(&payload),
            [
                ("svc_alpha".to_string(), 100, "node_a@host".to_string()),
                ("svc_beta".to_string(), 200, "node_b@host".to_string()),
            ],
            "a third entry the count promised is not there"
        );
        // A count no frame could hold allocates nothing up front.
        payload[1..5].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(decode_sync(&payload).len(), 2);
        assert!(decode_sync(&payload[..3]).is_empty());

        let mut pos = 0;
        let mut cut = Vec::new();
        encode_entry(&mut cut, "name", ProcessId(1), "node");
        assert_eq!(decode_entry(&cut[..cut.len() - 1], &mut pos), None);
        let mut not_utf8 = vec![1, 0, 0xFF];
        not_utf8.extend_from_slice(&[0; 10]);
        assert_eq!(decode_str(&not_utf8, &mut 0), None);
    }
}
