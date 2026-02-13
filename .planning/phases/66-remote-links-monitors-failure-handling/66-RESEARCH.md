# Phase 66: Remote Links, Monitors & Failure Handling - Research

**Researched:** 2026-02-12
**Domain:** Distributed fault tolerance -- remote process monitors, node monitors, connection loss propagation, and cross-node exit signal forwarding
**Confidence:** HIGH

## Summary

Phase 66 builds the distributed fault tolerance layer on top of Phase 65's message routing infrastructure. The phase has four concrete deliverables: (1) remote process monitoring via `Process.monitor(remote_pid)` that delivers `:down` messages when the remote process crashes or its node disconnects; (2) node monitoring via `Node.monitor(node)` that delivers `:nodedown` / `:nodeup` events; (3) connection loss handling that fires `:noconnection` exit signals for all remote links and `:down` messages for all remote monitors when a node connection drops; and (4) bidirectional remote link exit signal propagation -- a crash on node A terminates linked processes on node B via a wire-protocol `DIST_EXIT` message.

The critical architectural insight is that Phase 64 and 65 left explicit hooks for this phase throughout the codebase. The `cleanup_session` function in `dist/node.rs` has a comment "Phase 66 will add `:nodedown` notification here." The `dist_send` function silently drops on write errors with a comment "Phase 66 adds :nodedown." The heartbeat timeout detection in `heartbeat_loop_session` has a comment "Phase 66 will fire `:nodedown` signals here." These three insertion points are the exact locations where connection-loss propagation must be wired in.

The existing local link system (`link.rs`) provides `propagate_exit()` which handles exit signal encoding, trap_exit semantics, and process state transitions -- but it only works with local processes (requires `Arc<Mutex<Process>>` access). For remote links, the exiting node must send a `DIST_EXIT` wire message to the remote node, and the remote node's reader thread must decode it and call `propagate_exit` locally. For connection loss, the local node must synthesize `:noconnection` exit signals for every remote PID that has local links, without any wire message (since the connection is dead).

**Primary recommendation:** Structure as three plans: (1) Remote process monitoring infrastructure -- add `monitors` and `monitored_by` sets to the `Process` struct, new `DIST_MONITOR`/`DIST_DEMONITOR`/`DIST_MONITOR_EXIT` wire messages, `snow_process_monitor` and `snow_process_demonitor` extern "C" APIs, and `:down` message delivery on process exit; (2) Node monitoring and connection loss propagation -- add node monitor registry to `NodeState`, `snow_node_monitor` extern "C" API, `:nodedown`/`:nodeup` message delivery, and the critical `handle_node_disconnect` function that fires `:noconnection` exit signals for all remote links and `:down` for all remote monitors when `cleanup_session` runs; (3) Remote link exit propagation -- add `DIST_EXIT` wire message for cross-node exit signal forwarding, modify `handle_process_exit` in the scheduler to send `DIST_EXIT` for remote-linked PIDs, and add reader-loop handling to deliver remote exit signals locally.

## Standard Stack

### Core (already in Cargo.toml -- zero new dependencies)
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| parking_lot | 0.12 | RwLock for node monitor registry, Mutex for Process | Already used throughout codebase |
| rustc-hash | 2 | FxHashMap/FxHashSet for monitor/link tracking | Already used in node state and scheduler |
| rand | 0.9 | Monitor reference generation | Already used in handshake and heartbeat |

### Supporting (from existing crate modules)
| Module | Purpose | When to Use |
|--------|---------|-------------|
| `actor::link` | `propagate_exit()`, `encode_exit_signal()`, `EXIT_SIGNAL_TAG` | Delivering remote exit signals to local linked processes |
| `actor::process` | `ProcessId`, `Process`, `ExitReason`, `ProcessState` | Adding monitors/monitored_by fields, new exit reason variant |
| `actor::scheduler` | `handle_process_exit()`, `get_process()`, process table | Hooking remote exit propagation into the exit path |
| `dist::node` | `NodeState`, `NodeSession`, `cleanup_session()`, `write_msg`, `read_dist_msg` | Sending/receiving monitor and exit wire messages |
| `actor::mod` | `local_send()`, `global_scheduler()` | Delivering monitor DOWN messages to actor mailboxes |

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| Per-process monitor sets | Global monitor registry (FxHashMap<ProcessId, Vec<MonitorEntry>>) | Per-process is simpler and follows Erlang's model; global registry adds a lock contention point |
| Monotonic u64 monitor refs | UUID or random refs | Monotonic is simpler, no collision risk within a single node lifetime, matches Erlang's integer refs |
| Scanning all processes on disconnect | Per-node tracking set of remote-linked PIDs | Tracking set is O(1) per link add/remove vs O(N) scan; but N is typically small. Tracking set recommended for clean design |
| Inline disconnect handling in cleanup_session | Separate `handle_node_disconnect` function | Separate function keeps cleanup_session simple and makes the notification logic testable in isolation |

**Installation:** No new dependencies. All work is within existing `snow-rt/src/dist/` and `snow-rt/src/actor/`.

## Architecture Patterns

### Recommended Project Structure
```
crates/snow-rt/src/
├── dist/
│   ├── mod.rs           # MODIFIED: re-export new message types
│   └── node.rs          # MODIFIED: new wire messages, cleanup_session triggers nodedown,
│                        #           handle_node_disconnect, node monitor registry
├── actor/
│   ├── mod.rs           # MODIFIED: snow_process_monitor, snow_process_demonitor,
│   │                    #           snow_node_monitor extern "C" APIs
│   ├── process.rs       # MODIFIED: add monitors/monitored_by fields, Noconnection exit reason
│   ├── link.rs          # MODIFIED: add remote exit propagation helper
│   └── scheduler.rs     # MODIFIED: handle_process_exit sends DIST_EXIT for remote links,
│                        #           DIST_MONITOR_EXIT for remote monitors
└── lib.rs               # MODIFIED: re-export new extern "C" functions
```

### Pattern 1: Process Monitor Infrastructure

**What:** Add process-level monitor tracking via `monitors` (processes I'm monitoring) and `monitored_by` (processes monitoring me) sets on the `Process` struct. When a monitored process exits, deliver a `:down` message to all monitoring processes.

**When to use:** Whenever `Process.monitor(remote_pid)` or local monitoring is called.

**Rationale:** Mirrors Erlang's `erlang:monitor(process, Pid)` semantics. Monitors are unidirectional (unlike links). The monitored process does not crash when the monitoring process exits. The monitoring process receives `{:down, ref, :process, pid, reason}`.

```rust
// Additions to Process struct in process.rs:

/// Processes being monitored by this process.
/// Maps monitor_ref -> monitored_pid
pub monitors: FxHashMap<u64, ProcessId>,

/// Processes monitoring this process.
/// Maps monitor_ref -> monitoring_pid
pub monitored_by: FxHashMap<u64, ProcessId>,
```

**Monitor reference:** Use a global `AtomicU64` counter to generate unique monitor refs. The ref is returned to the caller and used to match `:down` messages. This follows Erlang's model where `monitor/2` returns a reference.

```rust
/// Generate a globally unique monitor reference.
pub fn next_monitor_ref() -> u64 {
    static MONITOR_REF_COUNTER: AtomicU64 = AtomicU64::new(1);
    MONITOR_REF_COUNTER.fetch_add(1, Ordering::Relaxed)
}
```

### Pattern 2: DOWN Message Format

**What:** When a monitored process exits, deliver a message with a special type tag (distinct from `EXIT_SIGNAL_TAG`) encoding the monitor ref, the monitored PID, and the exit reason.

**When to use:** On process exit (in `handle_process_exit`), iterate `monitored_by` and send DOWN messages.

**Rationale:** Erlang's DOWN message is `{'DOWN', Ref, process, Pid, Reason}`. Snow's binary message format already uses type tags. A dedicated `DOWN_SIGNAL_TAG` distinguishes monitor notifications from exit signal messages.

```rust
/// Special type_tag for monitor DOWN messages.
/// Distinguished from EXIT_SIGNAL_TAG (u64::MAX) to allow pattern matching.
pub const DOWN_SIGNAL_TAG: u64 = u64::MAX - 1;

/// Encode a DOWN message.
/// Layout: [u64 monitor_ref][u64 monitored_pid][u8 reason_tag][...reason_data]
pub fn encode_down_signal(monitor_ref: u64, monitored_pid: ProcessId, reason: &ExitReason) -> Vec<u8> {
    let mut data = Vec::new();
    data.extend_from_slice(&monitor_ref.to_le_bytes());
    data.extend_from_slice(&monitored_pid.0.to_le_bytes());
    encode_reason(&mut data, reason);
    data
}
```

### Pattern 3: Distribution Wire Messages for Monitors and Exit Signals

**What:** Define new wire message tags for the distribution protocol layer, continuing the 0x10-0x1F range established in Phase 65.

**When to use:** All cross-node monitor and exit signal operations.

**Rationale:** Mirrors Erlang's distribution protocol messages MONITOR_P (19), DEMONITOR_P (20), MONITOR_P_EXIT (21), EXIT (3), and EXIT2 (8), but uses Snow's simpler single-byte tag format.

```rust
// New distribution message tags (continuing from 0x12 = DIST_PEER_LIST):
const DIST_LINK: u8 = 0x13;          // [tag][u64 from_pid][u64 to_pid]
const DIST_UNLINK: u8 = 0x14;        // [tag][u64 from_pid][u64 to_pid]
const DIST_EXIT: u8 = 0x15;          // [tag][u64 from_pid][u64 to_pid][exit_reason_bytes]
const DIST_MONITOR: u8 = 0x16;       // [tag][u64 from_pid][u64 to_pid][u64 ref]
const DIST_DEMONITOR: u8 = 0x17;     // [tag][u64 from_pid][u64 to_pid][u64 ref]
const DIST_MONITOR_EXIT: u8 = 0x18;  // [tag][u64 from_pid][u64 to_pid][u64 ref][exit_reason_bytes]
```

### Pattern 4: Remote Process Monitor Flow

**What:** When `Process.monitor(remote_pid)` is called: (1) generate a local monitor ref, (2) add `{ref -> remote_pid}` to the local process's `monitors` map, (3) send a `DIST_MONITOR` wire message to the remote node, (4) the remote node adds `{ref -> monitoring_pid}` to the target process's `monitored_by` map. When the remote process exits: (5) the remote node sends `DIST_MONITOR_EXIT` back, (6) the local reader thread delivers a DOWN message to the monitoring process.

**When to use:** Cross-node process monitoring.

**Rationale:** This is the standard two-phase monitor protocol (Erlang uses the same MONITOR_P/MONITOR_P_EXIT pair). The monitor ref ensures that each monitor is uniquely identifiable, enabling `demonitor(ref)`.

```rust
// In snow_process_monitor:
#[no_mangle]
pub extern "C" fn snow_process_monitor(target_pid: u64) -> u64 {
    let my_pid = match stack::get_current_pid() {
        Some(pid) => pid,
        None => return 0,
    };
    let monitor_ref = next_monitor_ref();
    let target = ProcessId(target_pid);

    if target.is_local() {
        // Local monitor: add to both processes' monitor sets
        // Also check if target is already dead (deliver DOWN immediately)
        local_monitor(my_pid, target, monitor_ref);
    } else {
        // Remote monitor: record locally, send DIST_MONITOR to remote node
        remote_monitor(my_pid, target, monitor_ref);
    }
    monitor_ref
}
```

### Pattern 5: Node Monitor Registry

**What:** A registry in `NodeState` that tracks which local processes have called `Node.monitor(node_name)`. When a node disconnects, all registered monitors receive a `:nodedown` message. When a node reconnects, they receive `:nodeup`.

**When to use:** For `Node.monitor(node)` API.

**Rationale:** Erlang's `monitor_node/2` provides exactly this functionality. The registry is a `RwLock<FxHashMap<String, Vec<ProcessId>>>` mapping node names to the list of monitoring process PIDs.

```rust
// Addition to NodeState:
/// Processes monitoring specific nodes.
/// node_name -> list of (monitoring_pid, is_once) pairs.
/// "Once" monitors auto-remove after first nodedown (Erlang's default behavior).
pub node_monitors: RwLock<FxHashMap<String, Vec<(ProcessId, bool)>>>,
```

### Pattern 6: Connection Loss Propagation (handle_node_disconnect)

**What:** When `cleanup_session` runs (triggered by heartbeat timeout or reader error), call a new `handle_node_disconnect` function that: (1) iterates ALL local processes, (2) for each process with links to remote PIDs on the disconnected node, synthesizes `:noconnection` exit signals, (3) for each process with monitors on remote PIDs on the disconnected node, synthesizes `:down` messages with reason `:noconnection`, (4) delivers `:nodedown` messages to all node monitors. This is the CORE of Phase 66.

**When to use:** Whenever a node connection is lost (heartbeat timeout, read error, explicit disconnect).

**Rationale:** When a node goes down, the TCP/TLS connection is dead. No wire messages can be sent. The local node must synthesize all necessary failure signals locally, without any cooperation from the dead node. This matches Erlang's behavior: "from the perspective of either process, there is no difference between a network failure and a process crash."

```rust
/// Handle node disconnection: fire all failure signals locally.
///
/// This is the central failure handler for distributed fault tolerance.
/// Called from cleanup_session after removing the session from NodeState.
fn handle_node_disconnect(node_name: &str, node_id: u16) {
    let sched = match crate::actor::GLOBAL_SCHEDULER.get() {
        Some(s) => s,
        None => return,
    };

    let noconnection = ExitReason::Error("noconnection".to_string());

    // Step 1: Scan all processes for remote links to the disconnected node.
    let process_table = sched.process_table();
    let table = process_table.read();
    for (pid, proc_arc) in table.iter() {
        let mut proc = proc_arc.lock();

        // Find links to remote PIDs on the disconnected node.
        let remote_links: Vec<ProcessId> = proc.links.iter()
            .filter(|linked_pid| linked_pid.node_id() == node_id)
            .cloned()
            .collect();

        if !remote_links.is_empty() {
            // Remove the remote links.
            for remote_pid in &remote_links {
                proc.links.remove(remote_pid);
            }

            // Deliver :noconnection exit signal.
            // (Same semantics as propagate_exit, but the "from" is the remote pid)
            for remote_pid in &remote_links {
                if proc.trap_exit {
                    let signal_data = link::encode_exit_signal(*remote_pid, &noconnection);
                    let buffer = MessageBuffer::new(signal_data, link::EXIT_SIGNAL_TAG);
                    proc.mailbox.push(Message { buffer });
                    if matches!(proc.state, ProcessState::Waiting) {
                        proc.state = ProcessState::Ready;
                        // wake_process after dropping lock
                    }
                } else {
                    proc.state = ProcessState::Exited(ExitReason::Linked(
                        *remote_pid,
                        Box::new(noconnection.clone()),
                    ));
                    break; // Process is dead, no need to check more links
                }
            }
        }

        // Find monitors on remote PIDs on the disconnected node.
        let remote_monitors: Vec<(u64, ProcessId)> = proc.monitors.iter()
            .filter(|(_, monitored_pid)| monitored_pid.node_id() == node_id)
            .map(|(ref_id, pid)| (*ref_id, *pid))
            .collect();

        for (monitor_ref, monitored_pid) in &remote_monitors {
            proc.monitors.remove(monitor_ref);
            let down_data = encode_down_signal(*monitor_ref, *monitored_pid, &noconnection);
            let buffer = MessageBuffer::new(down_data, DOWN_SIGNAL_TAG);
            proc.mailbox.push(Message { buffer });
            if matches!(proc.state, ProcessState::Waiting) {
                proc.state = ProcessState::Ready;
            }
        }
    }

    // Step 2: Deliver :nodedown to all node monitors.
    if let Some(state) = node_state() {
        let monitors = state.node_monitors.read();
        if let Some(watchers) = monitors.get(node_name) {
            for (watcher_pid, _once) in watchers {
                // Deliver {:nodedown, node_name} message
                deliver_nodedown_message(*watcher_pid, node_name, sched);
            }
        }
        drop(monitors);

        // Remove "once" monitors
        let mut monitors = state.node_monitors.write();
        if let Some(watchers) = monitors.get_mut(node_name) {
            watchers.retain(|(_, once)| !once);
        }
    }
}
```

### Pattern 7: Remote Exit Signal Propagation via DIST_EXIT

**What:** When a process exits on node A and has links to processes on node B, node A sends a `DIST_EXIT` wire message to node B containing the exiting PID, the linked PID, and the exit reason. Node B's reader thread receives this and applies the exit signal to the local linked process using the existing `propagate_exit` logic.

**When to use:** In `handle_process_exit` (scheduler.rs), when iterating linked PIDs, check if any are remote. For remote PIDs, send `DIST_EXIT` instead of calling `propagate_exit` directly.

**Rationale:** The local `propagate_exit` requires `Arc<Mutex<Process>>` access, which only exists for local processes. Remote processes live on another node's process table. The wire message is the distribution equivalent.

```rust
// Modified handle_process_exit in scheduler.rs:
// After extracting linked_pids from the exiting process...

let (local_links, remote_links): (HashSet<ProcessId>, HashSet<ProcessId>) =
    linked_pids.into_iter().partition(|pid| pid.is_local());

// Propagate to local links (existing behavior).
let woken = link::propagate_exit(pid, &reason, local_links, |linked_pid| {
    process_table.read().get(&linked_pid).cloned()
});

// Propagate to remote links via DIST_EXIT wire messages.
for remote_pid in &remote_links {
    send_dist_exit(pid, *remote_pid, &reason);
}
```

### Pattern 8: Noconnection Exit Reason

**What:** Add a `Noconnection` variant to `ExitReason` to clearly distinguish connection-loss-induced exits from regular error exits.

**When to use:** When synthesizing exit signals during `handle_node_disconnect`.

**Rationale:** Erlang uses the atom `noconnection` as the exit reason when a linked process's node disconnects. Having a dedicated variant makes pattern matching cleaner and avoids relying on magic strings in `ExitReason::Error("noconnection")`.

```rust
// Addition to ExitReason enum in process.rs:
/// Node connection lost -- the remote process may still be alive.
///
/// Delivered to locally linked processes when the remote node disconnects.
/// Distinguished from Error because the remote process did not necessarily
/// crash -- the network partition or node shutdown caused the signal.
Noconnection,
```

The encode/decode functions in `link.rs` need a new reason tag (e.g., tag 6 for Noconnection).

### Anti-Patterns to Avoid
- **Holding process table lock while sending wire messages:** The `handle_node_disconnect` function must NOT hold the process table read lock while writing to TLS streams. Extract the data needed, drop the lock, then send. However, since disconnect means the connection is dead, this mainly applies to DIST_EXIT sent during normal process exit (not during disconnect).
- **Blocking the reader thread with monitor scans:** The reader thread receives `DIST_MONITOR_EXIT` and must deliver a DOWN message. This should be a quick `local_send` call, not a scan of the process table.
- **Creating remote Process entries in the local table:** Remote processes should NOT have entries in the local process table. Monitors and links to remote processes are tracked by their PID values in the local process's monitor/link sets, NOT by creating phantom Process objects.
- **Attempting to send wire messages during disconnect:** When `handle_node_disconnect` fires, the connection is already dead. All signals must be synthesized locally. Do NOT try to send DIST_EXIT/DIST_MONITOR_EXIT to a disconnected node.
- **Using a single lock for both process scanning and message delivery:** During disconnect, iterating all processes while holding a write lock on the process table will deadlock if any message delivery tries to read the table. Use a read lock, collect actions, drop the lock, then execute actions.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Exit signal encoding | New serialization format | Existing `encode_exit_signal` / `decode_exit_signal` in `link.rs` | Already proven, handles all ExitReason variants |
| Local exit propagation | Duplicate propagation logic | Existing `propagate_exit()` in `link.rs` | Handles trap_exit, state transitions, waking |
| Wire message framing | Custom framing | Existing `write_msg` / `read_dist_msg` length-prefixed protocol | Already used by all distribution messages |
| Session/node lookup | Linear scan | Existing `sessions` and `node_id_map` in `NodeState` | FxHashMap already provides O(1) lookup |
| Connection loss detection | Polling/checking | Existing heartbeat timeout and reader error paths | Already trigger `cleanup_session` |
| Process table iteration | Custom collection | Existing `sched.process_table().read()` | Already provides concurrent-safe iteration |

## Common Pitfalls

### Pitfall 1: Process Table Lock Ordering Deadlock
**What goes wrong:** `handle_node_disconnect` holds the process table read lock while trying to wake processes via `sched.wake_process()`, which also acquires locks on scheduler internals. If another thread holds a scheduler lock and tries to read the process table, deadlock occurs.
**Why it happens:** The disconnect handler must scan all processes AND modify their state AND wake them.
**How to avoid:** Two-phase approach: (1) Under the process table read lock, collect all actions to take (which PIDs to wake, which messages to deliver). (2) Drop the read lock. (3) Execute the collected actions. This is the same pattern used by the garbage collector.
**Warning signs:** Deadlock during node disconnect, especially under concurrent send/receive load.

### Pitfall 2: Remote Monitor on Already-Dead Process
**What goes wrong:** `Process.monitor(remote_pid)` sends `DIST_MONITOR` to the remote node, but by the time it arrives, the remote process has already exited. The monitor is registered on a dead process and no DOWN message is ever sent.
**Why it happens:** Race between monitor setup and process exit.
**How to avoid:** On the remote side, when processing `DIST_MONITOR`, check if the target process is already in `Exited` state. If so, immediately send `DIST_MONITOR_EXIT` back. This matches Erlang's behavior where `monitor/2` on a dead process immediately delivers `{'DOWN', Ref, process, Pid, noproc}`.
**Warning signs:** Monitor never fires for processes that exited right before or during monitor setup.

### Pitfall 3: Stale Remote PID After Node Restart
**What goes wrong:** Node B restarts. Node A reconnects to the new incarnation of B. Local processes on A still hold old PIDs with B's old creation counter. They try to send to or monitor processes using stale PIDs. The new node B doesn't recognize these PIDs.
**Why it happens:** The creation counter in the PID (bits 47..40) distinguishes incarnations, but existing links/monitors store the old creation value.
**How to avoid:** On reconnection, the new node B advertises its new creation counter in the handshake. Node A's `handle_node_disconnect` (already fired when the old connection dropped) would have already cleaned up all old links and monitors with `:noconnection` signals. New links/monitors use the new creation counter. As long as disconnect cleanup is thorough, this is not an issue.
**Warning signs:** Silent message drops or "process not found" errors after node restart.

### Pitfall 4: DIST_EXIT Sent to Disconnected Node
**What goes wrong:** A process on node A crashes. Its links include a process on node B. The scheduler tries to send `DIST_EXIT` to node B, but B just disconnected (heartbeat timeout detected by a different thread). The session is already removed from `NodeState.sessions`.
**Why it happens:** Race between process exit and node disconnect. `handle_process_exit` and `cleanup_session` run on different threads.
**How to avoid:** In `send_dist_exit`, look up the session the same way `dist_send` does -- if the session is gone, silently drop. The `handle_node_disconnect` on the other thread will have already fired `:noconnection` signals to the same linked processes. The remote side (if it's still alive) will eventually detect the connection loss via its own heartbeat and fire its own cleanup. Double-delivery is prevented because link cleanup removes the link from the process, so neither path can fire twice.
**Warning signs:** Panics on failed session lookups during exit propagation.

### Pitfall 5: Monitor/Link Cleanup Must Be Atomic With Process Exit
**What goes wrong:** Process A exits on node X. Between the time A's `monitored_by` set is read and the time `DIST_MONITOR_EXIT` messages are sent, a new `DIST_MONITOR` arrives for process A from node Y. The monitor is added to A's `monitored_by` set AFTER the exit scan, so the new monitor never fires.
**Why it happens:** `handle_process_exit` reads `monitored_by`, but the reader thread concurrently processes `DIST_MONITOR`.
**How to avoid:** When processing `DIST_MONITOR` on the reader thread, check if the target process is already `Exited`. If so, immediately send back `DIST_MONITOR_EXIT`. The `handle_process_exit` path takes the process lock when extracting `monitored_by`, which prevents concurrent modification.
**Warning signs:** Intermittent monitor failures under concurrent exit/monitor race conditions.

### Pitfall 6: Nodedown Flooding
**What goes wrong:** A node with 1000 linked/monitored processes disconnects. `handle_node_disconnect` must deliver 1000+ messages atomically. If each delivery acquires and releases the process lock individually, the total time is significant, and the heartbeat thread is blocked.
**Why it happens:** Large number of cross-node links/monitors.
**How to avoid:** Accept this as inherent to the design (Erlang has the same behavior -- node disconnect is a heavyweight event). The two-phase approach (collect under read lock, deliver without lock) minimizes lock contention. For the heartbeat thread, `cleanup_session` + `handle_node_disconnect` runs after the session is already removed, so no heartbeat messages are sent during this time.
**Warning signs:** Slow node disconnect handling under heavy cross-node linking. Acceptable for the target cluster size (3-20 nodes).

### Pitfall 7: Multiple Monitors Between Same Process Pair
**What goes wrong:** Process A calls `monitor(B)` twice. Two separate monitor refs are created. When B exits, A should receive TWO separate DOWN messages. If the implementation uses a set instead of a map, the second monitor replaces the first.
**Why it happens:** Erlang explicitly supports multiple independent monitors between the same process pair, each with its own ref.
**How to avoid:** Use `FxHashMap<u64, ProcessId>` (ref -> pid) for both `monitors` and `monitored_by`, NOT `FxHashSet<ProcessId>`. The ref is the key, allowing multiple monitors to the same PID.
**Warning signs:** Only one DOWN message received when two monitors were set up.

## Code Examples

### Remote Process Monitor API
```rust
// Source: New extern "C" API in actor/mod.rs

/// Monitor a process (local or remote).
///
/// Returns a monitor reference (u64) that uniquely identifies this monitor.
/// When the monitored process exits, the caller receives a DOWN message:
///   type_tag = DOWN_SIGNAL_TAG
///   data = [u64 monitor_ref][u64 monitored_pid][u8 reason_tag][...reason_data]
///
/// Returns 0 if called outside an actor context.
#[no_mangle]
pub extern "C" fn snow_process_monitor(target_pid: u64) -> u64 {
    let my_pid = match stack::get_current_pid() {
        Some(pid) => pid,
        None => return 0,
    };

    let sched = global_scheduler();
    let target = ProcessId(target_pid);
    let monitor_ref = link::next_monitor_ref();

    if target.is_local() {
        // Local monitor.
        if let Some(target_proc) = sched.get_process(target) {
            let target_state = {
                let t = target_proc.lock();
                t.state.clone()
            };

            // Check if target is already dead.
            if let ProcessState::Exited(ref reason) = target_state {
                // Deliver DOWN immediately (like Erlang's noproc).
                if let Some(my_proc) = sched.get_process(my_pid) {
                    let down_data = link::encode_down_signal(monitor_ref, target, reason);
                    let buffer = MessageBuffer::new(down_data, link::DOWN_SIGNAL_TAG);
                    my_proc.lock().mailbox.push(Message { buffer });
                }
                return monitor_ref;
            }

            // Register the monitor on both sides.
            if let Some(my_proc) = sched.get_process(my_pid) {
                my_proc.lock().monitors.insert(monitor_ref, target);
            }
            target_proc.lock().monitored_by.insert(monitor_ref, my_pid);
        } else {
            // Target doesn't exist -- deliver DOWN(noproc) immediately.
            if let Some(my_proc) = sched.get_process(my_pid) {
                let noproc = ExitReason::Error("noproc".to_string());
                let down_data = link::encode_down_signal(monitor_ref, target, &noproc);
                let buffer = MessageBuffer::new(down_data, link::DOWN_SIGNAL_TAG);
                my_proc.lock().mailbox.push(Message { buffer });
            }
        }
    } else {
        // Remote monitor: record locally and send DIST_MONITOR.
        if let Some(my_proc) = sched.get_process(my_pid) {
            my_proc.lock().monitors.insert(monitor_ref, target);
        }
        send_dist_monitor(my_pid, target, monitor_ref);
    }

    monitor_ref
}
```

### Node Monitor API
```rust
// Source: New extern "C" API in actor/mod.rs

/// Monitor a node for connection/disconnection events.
///
/// The calling process will receive:
///   {:nodedown, node_name} when the connection is lost
///   {:nodeup, node_name}   when the connection is (re-)established
///
/// Returns 0 on success, 1 on failure.
#[no_mangle]
pub extern "C" fn snow_node_monitor(
    node_ptr: *const u8,
    node_len: u64,
) -> u64 {
    let my_pid = match stack::get_current_pid() {
        Some(pid) => pid,
        None => return 1,
    };

    let node_name = unsafe {
        match std::str::from_utf8(std::slice::from_raw_parts(node_ptr, node_len as usize)) {
            Ok(s) => s.to_string(),
            Err(_) => return 1,
        }
    };

    let state = match crate::dist::node::node_state() {
        Some(s) => s,
        None => return 1,
    };

    let mut monitors = state.node_monitors.write();
    monitors
        .entry(node_name)
        .or_insert_with(Vec::new)
        .push((my_pid, false)); // false = persistent monitor

    0
}
```

### DIST_EXIT Wire Message (Remote Exit Propagation)
```rust
// Source: New function in dist/node.rs

/// Send a DIST_EXIT wire message to propagate an exit signal to a remote node.
///
/// Wire format: [DIST_EXIT][u64 from_pid][u64 to_pid][exit_reason_bytes]
///
/// Called from handle_process_exit when the exiting process has links
/// to remote processes. Silently drops if the session is unavailable
/// (the remote node may have already disconnected).
pub(crate) fn send_dist_exit(from_pid: ProcessId, to_pid: ProcessId, reason: &ExitReason) {
    let state = match node_state() {
        Some(s) => s,
        None => return,
    };

    let node_id = to_pid.node_id();
    let node_name = {
        let map = state.node_id_map.read();
        match map.get(&node_id) {
            Some(name) => name.clone(),
            None => return,
        }
    };

    let session = {
        let sessions = state.sessions.read();
        match sessions.get(&node_name) {
            Some(s) => Arc::clone(s),
            None => return, // Node already disconnected; cleanup_session handles it
        }
    };

    // Encode the exit reason using the existing link.rs encoder.
    let reason_bytes = link::encode_exit_signal(from_pid, reason);
    // Note: encode_exit_signal includes from_pid; for DIST_EXIT we send
    // from_pid and to_pid explicitly, so we use a custom encoding:
    let mut payload = Vec::with_capacity(1 + 8 + 8 + reason_bytes.len());
    payload.push(DIST_EXIT);
    payload.extend_from_slice(&from_pid.as_u64().to_le_bytes());
    payload.extend_from_slice(&to_pid.as_u64().to_le_bytes());
    // Encode just the reason (without redundant from_pid)
    link::encode_reason_to(&mut payload, reason);

    let mut stream = session.stream.lock().unwrap();
    let _ = write_msg(&mut *stream, &payload);
}
```

### Reader Loop DIST_EXIT Handler
```rust
// Source: Extension of reader_loop_session in dist/node.rs

DIST_EXIT => {
    // Wire format: [tag][u64 from_pid][u64 to_pid][exit_reason_bytes]
    if msg.len() >= 17 { // 1 + 8 + 8
        let from_pid = ProcessId(u64::from_le_bytes(msg[1..9].try_into().unwrap()));
        let to_pid = ProcessId(u64::from_le_bytes(msg[9..17].try_into().unwrap()));
        let reason_bytes = &msg[17..];

        if let Some(reason) = link::decode_reason_from(reason_bytes) {
            // Deliver the exit signal to the local process.
            let sched = crate::actor::global_scheduler();
            if let Some(proc_arc) = sched.get_process(to_pid) {
                let mut proc = proc_arc.lock();

                // Remove the link to the remote process.
                proc.links.remove(&from_pid);

                let is_non_crashing = matches!(reason, ExitReason::Normal | ExitReason::Shutdown);

                if is_non_crashing || proc.trap_exit {
                    let signal_data = link::encode_exit_signal(from_pid, &reason);
                    let buffer = MessageBuffer::new(signal_data, link::EXIT_SIGNAL_TAG);
                    proc.mailbox.push(Message { buffer });

                    if matches!(proc.state, ProcessState::Waiting) {
                        proc.state = ProcessState::Ready;
                        drop(proc);
                        sched.wake_process(to_pid);
                    }
                } else {
                    proc.state = ProcessState::Exited(ExitReason::Linked(
                        from_pid,
                        Box::new(reason),
                    ));
                }
            }
        }
    }
}
```

### Connection Loss Handler (cleanup_session modification)
```rust
// Source: Modified cleanup_session in dist/node.rs

fn cleanup_session(remote_name: &str) {
    if let Some(state) = NODE_STATE.get() {
        let removed = {
            let mut sessions = state.sessions.write();
            sessions.remove(remote_name)
        };
        if let Some(session) = removed {
            let node_id = session.node_id;
            let mut id_map = state.node_id_map.write();
            id_map.remove(&node_id);
            drop(id_map);

            // Phase 66: Fire all failure signals for this disconnected node.
            handle_node_disconnect(remote_name, node_id);
        }
    }
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| No remote monitors | `Process.monitor(remote_pid)` with DOWN messages | Phase 66 | Supervisors can detect remote crashes |
| No node monitors | `Node.monitor(node)` with nodedown/nodeup events | Phase 66 | Application code can react to node disconnections |
| Silent drop on disconnect | `:noconnection` exit signals for links, `:down` for monitors | Phase 66 | Processes fail fast when node dies |
| Links only work locally | DIST_EXIT propagates exit signals across nodes | Phase 66 | Remote-linked processes crash together as expected |
| `cleanup_session` only removes session | `cleanup_session` triggers `handle_node_disconnect` | Phase 66 | Connection loss is a fully-handled event |

**Deprecated/outdated:**
- "Phase 66 will add :nodedown" comments throughout `dist_send`, `cleanup_session`, heartbeat loop: replaced by actual implementation
- Silent-drop on disconnected send: still silently drops (at-most-once delivery) but the sender's node now also fires nodedown/noconnection signals to monitoring processes

## Open Questions

1. **Noconnection as Dedicated ExitReason Variant vs String**
   - What we know: Erlang uses the atom `noconnection` as the exit reason. Snow currently has `Normal`, `Shutdown`, `Error(String)`, `Killed`, `Linked(pid, reason)`, `Custom(String)`.
   - What's unclear: Should we add a dedicated `Noconnection` variant to `ExitReason`, or use `ExitReason::Error("noconnection".to_string())`? A dedicated variant is cleaner for pattern matching but requires updating all encode/decode paths.
   - Recommendation: Add a dedicated `Noconnection` variant with reason tag 6. It makes the code self-documenting and enables precise pattern matching in Snow user code. The encode/decode change is mechanical (one new match arm each).

2. **Remote Link Creation Wire Protocol**
   - What we know: Currently `snow_actor_link(target_pid)` only works for local PIDs. Both processes must be in the same process table for `link::link()` to insert the bidirectional entries.
   - What's unclear: Should Phase 66 extend `snow_actor_link` to support remote PIDs via `DIST_LINK`/`DIST_UNLINK` wire messages? This requires the local node to record the link AND notify the remote node to record the reverse link.
   - Recommendation: Yes, remote links are required by FT-04 ("Remote links propagate exit signals across nodes"). The flow is: (1) local process adds `remote_pid` to its links set, (2) sends `DIST_LINK(local_pid, remote_pid)` to the remote node, (3) remote reader thread adds `local_pid` to the target process's links set. On exit, `DIST_EXIT` propagates the signal. On disconnect, `handle_node_disconnect` synthesizes `:noconnection` for all remote links.

3. **Monitor Ref Encoding in Wire Protocol**
   - What we know: Monitor refs are locally-generated `u64` values. The wire protocol needs to transmit them for `DIST_MONITOR`, `DIST_DEMONITOR`, and `DIST_MONITOR_EXIT`.
   - What's unclear: Are locally-generated refs unique enough across nodes? Two nodes could independently generate the same ref value.
   - Recommendation: Prefix the ref with the monitoring node's node_id to ensure global uniqueness. Alternatively, since monitors are always between a specific pair of processes, the `(monitoring_pid, ref)` tuple is globally unique because PIDs are globally unique. The ref alone is sufficient for the wire protocol since the monitoring_pid is always included in the message.

4. **Nodeup Delivery on Reconnection**
   - What we know: FT-02 requires `:nodeup` events when a node reconnects. Currently, node connections are established via `snow_node_connect` (explicit) or mesh formation (automatic).
   - What's unclear: Where should `:nodeup` be delivered? After `register_session` + `spawn_session_threads` completes in `snow_node_connect`? Or in `accept_loop` for incoming connections?
   - Recommendation: Deliver `:nodeup` at the end of `register_session` (or in a new `handle_node_connect` function called from both `accept_loop` and `snow_node_connect`). Check the node monitor registry for the connecting node's name and deliver `:nodeup` to all registered monitors.

5. **Process Table Scan Efficiency on Disconnect**
   - What we know: `handle_node_disconnect` must find ALL local processes with links or monitors to the disconnected node. Currently this requires iterating the entire process table.
   - What's unclear: Is a full table scan acceptable, or should we maintain a per-node tracking set?
   - Recommendation: For the target cluster size (3-20 nodes, hundreds to thousands of processes), a full scan is acceptable. The scan is O(N) where N is total processes, and each process's link set check is O(1) amortized (HashSet lookup by node_id). A per-node tracking set (e.g., `node_remote_links: RwLock<FxHashMap<u16, FxHashSet<ProcessId>>>` in NodeState) would be O(1) on disconnect but adds bookkeeping overhead on every link/monitor creation. Start with the scan; optimize if profiling shows it's a bottleneck.

## Sources

### Primary (HIGH confidence)
- **Codebase analysis** (direct file reads):
  - `crates/snow-rt/src/dist/node.rs` -- NodeState, NodeSession, cleanup_session (line 539), handle_node_disconnect insertion point, reader_loop_session (line 378), heartbeat_loop_session (line 492), wire message tags (DIST_SEND 0x10, DIST_REG_SEND 0x11, DIST_PEER_LIST 0x12), HeartbeatState, session lifecycle
  - `crates/snow-rt/src/actor/link.rs` -- propagate_exit (line 178), encode_exit_signal/decode_exit_signal, EXIT_SIGNAL_TAG (u64::MAX), exit reason encoding format
  - `crates/snow-rt/src/actor/process.rs` -- Process struct (line 231) with links/trap_exit/mailbox, ExitReason enum (line 132) with Normal/Shutdown/Error/Killed/Linked/Custom variants, ProcessId bit-packing (node_id/creation/local_id)
  - `crates/snow-rt/src/actor/scheduler.rs` -- handle_process_exit (line 607), propagate_exit call (line 626), process table type
  - `crates/snow-rt/src/actor/mod.rs` -- dist_send (line 321) with "Phase 66 adds :nodedown" comment, snow_actor_link (line 657), snow_actor_exit (line 1016), local_send (line 275)
  - `crates/snow-rt/src/lib.rs` -- current extern "C" re-exports
  - `.planning/phases/65-remote-send-distribution-router/65-RESEARCH.md` -- Phase 65 architecture and design decisions
  - `.planning/REQUIREMENTS.md` -- FT-01 through FT-04 definitions
  - `.planning/ROADMAP.md` -- Phase 66 success criteria and Phase 67 dependency

### Secondary (MEDIUM confidence)
- [Erlang Distribution Protocol](https://erlang.org/~rickard/OTP-15251/erts-10.5.6/doc/html/erl_dist_protocol.html) -- MONITOR_P (tag 19), DEMONITOR_P (tag 20), MONITOR_P_EXIT (tag 21), EXIT (tag 3), LINK (tag 1) wire message formats
- [Erlang Processes Reference](https://www.erlang.org/doc/system/ref_man_processes.html) -- Monitor/link semantics: DOWN message format `{'DOWN', Ref, process, Pid, Reason}`, unidirectional monitors, noconnection exit reason
- [Distributed Erlang](https://www.erlang.org/doc/system/distributed.html) -- monitor_node/2, nodedown message format, "connections are by default transitive"
- [Distributed Process Monitoring in Erlang](https://softwarepatternslexicon.com/patterns-erlang/5/6/) -- Patterns for distributed monitoring, node monitor best practices

### Tertiary (LOW confidence)
- [Erlang Message Passing Blog](https://www.erlang.org/blog/message-passing/) -- TCP connection-based failure detection model, "either process becoming unavailable for whatever reason is a failure"

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- zero new dependencies; all existing infrastructure reused
- Architecture (remote process monitors): HIGH -- well-established pattern from Erlang, clear insertion points in codebase
- Architecture (node monitors): HIGH -- simple registry pattern, clear cleanup_session hook
- Architecture (connection loss propagation): HIGH -- insertion points explicitly documented in prior phases' code comments
- Architecture (remote exit propagation): HIGH -- straightforward extension of existing propagate_exit + wire protocol
- Pitfalls: HIGH -- derived from analyzing actual code paths and known distributed systems race conditions
- Open questions: MEDIUM -- Noconnection variant and process table scan are design choices, not unknowns

**Research date:** 2026-02-12
**Valid until:** 2026-03-14 (stable domain; all dependencies frozen)
