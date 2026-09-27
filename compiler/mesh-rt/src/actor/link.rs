//! Bidirectional process linking and exit signal propagation.
//!
//! Links create bidirectional connections between actors: when one crashes,
//! linked partners receive exit signals. Normal exits are delivered as
//! informational messages but do not cause the linked process to crash.
//!
//! ## Exit Signal Propagation Rules
//!
//! - **Normal exit**: Linked processes receive `{:exit, pid, :normal}` as a
//!   regular message. They do NOT crash.
//! - **Error/Killed exit**: Linked processes receive `{:exit, pid, reason}`.
//!   If `trap_exit` is false (default), this causes the linked process to
//!   crash with `Linked(pid, reason)`. If `trap_exit` is true, the signal
//!   is delivered as a regular message.

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;

use super::heap::MessageBuffer;
use super::process::{ExitReason, Message, Process, ProcessId, ProcessState};

/// Special type_tag used for exit signal messages.
///
/// u64::MAX is reserved as the exit signal sentinel -- no regular message
/// should use this tag. The data payload encodes the exiting PID and reason.
pub const EXIT_SIGNAL_TAG: u64 = u64::MAX;

/// Generate a globally unique monitor reference.
///
/// Each call returns a new u64, guaranteed unique across all threads.
pub fn next_monitor_ref() -> u64 {
    static MONITOR_REF_COUNTER: AtomicU64 = AtomicU64::new(1);
    MONITOR_REF_COUNTER.fetch_add(1, Ordering::Relaxed)
}

/// Create a bidirectional link between two processes.
///
/// After linking, if either process exits, the other receives an exit signal.
/// If the link already exists, this is a no-op (idempotent).
pub fn link(
    proc_a: &Arc<Mutex<Process>>,
    proc_b: &Arc<Mutex<Process>>,
    pid_a: ProcessId,
    pid_b: ProcessId,
) {
    proc_a.lock().links.insert(pid_b);
    proc_b.lock().links.insert(pid_a);
}

/// Remove a bidirectional link between two processes.
pub fn unlink(
    proc_a: &Arc<Mutex<Process>>,
    proc_b: &Arc<Mutex<Process>>,
    pid_a: ProcessId,
    pid_b: ProcessId,
) {
    proc_a.lock().links.remove(&pid_b);
    proc_b.lock().links.remove(&pid_a);
}

/// Encode an exit signal message for delivery to a linked process.
///
/// Layout: `[u64 exiting_pid, u8 reason_tag, ...reason_data]`
/// - reason_tag 0 = Normal
/// - reason_tag 1 = Error (followed by UTF-8 error string)
/// - reason_tag 2 = Killed
/// - reason_tag 3 = Linked (followed by u64 originator_pid + nested reason)
pub fn encode_exit_signal(exiting_pid: ProcessId, reason: &ExitReason) -> Vec<u8> {
    let mut data = Vec::new();
    // Write the exiting PID (8 bytes).
    data.extend_from_slice(&exiting_pid.0.to_le_bytes());
    // Write the reason.
    encode_reason(&mut data, reason);
    data
}

pub(crate) fn encode_reason(data: &mut Vec<u8>, reason: &ExitReason) {
    data.push(reason.tag());
    match reason {
        ExitReason::Error(msg) | ExitReason::Custom(msg) => {
            data.extend_from_slice(&(msg.len() as u64).to_le_bytes());
            data.extend_from_slice(msg.as_bytes());
        }
        ExitReason::Linked(pid, inner) => {
            data.extend_from_slice(&pid.0.to_le_bytes());
            encode_reason(data, inner);
        }
        ExitReason::Normal
        | ExitReason::Killed
        | ExitReason::Shutdown
        | ExitReason::Noconnection => {}
    }
}

/// How deep `Linked` reasons nest at most: far deeper than a chain of linked
/// actors that ended one after another, and shallow enough that one from
/// another node cannot exhaust a stack.
pub(crate) const MAX_LINK_DEPTH: usize = 1024;

/// Decode an exit signal message back into `(ProcessId, ExitReason)`.
///
/// This is the inverse of `encode_exit_signal`. The supervisor uses this to
/// parse exit signals received in its mailbox.
///
/// Layout: `[u64 exiting_pid, u8 reason_tag, ...reason_data]`
pub fn decode_exit_signal(data: &[u8]) -> Option<(ProcessId, ExitReason)> {
    let pid = ProcessId(u64::from_le_bytes(data.get(..8)?.try_into().unwrap()));
    let (reason, _consumed) = decode_reason(&data[8..])?;
    Some((pid, reason))
}

/// Decode an ExitReason from raw bytes.
///
/// Returns `(ExitReason, bytes_consumed)` or `None` if the data is malformed:
/// cut short, a text that is not UTF-8, or `Linked` nested past
/// `MAX_LINK_DEPTH`. The bytes may come from another node, so the nesting
/// is read without recursing.
pub(crate) fn decode_reason(data: &[u8]) -> Option<(ExitReason, usize)> {
    let word = |at: usize| -> Option<u64> {
        let bytes = data.get(at..at.checked_add(8)?)?;
        Some(u64::from_le_bytes(bytes.try_into().unwrap()))
    };
    // A text of `len` bytes after its length word at `at`, and where it ends.
    let text = |at: usize| -> Option<(String, usize)> {
        let start = at + 8;
        let end = start.checked_add(usize::try_from(word(at)?).ok()?)?;
        let text = std::str::from_utf8(data.get(start..end)?).ok()?;
        Some((text.to_string(), end))
    };
    let mut links = Vec::new();
    let mut at = 0;
    let (leaf, end) = loop {
        let tag = *data.get(at)?;
        at += 1;
        match tag {
            0 => break (ExitReason::Normal, at),
            1 => break text(at).map(|(text, end)| (ExitReason::Error(text), end))?,
            2 => break (ExitReason::Killed, at),
            3 if links.len() < MAX_LINK_DEPTH => {
                links.push(ProcessId(word(at)?));
                at += 8;
            }
            4 => break (ExitReason::Shutdown, at),
            5 => break text(at).map(|(text, end)| (ExitReason::Custom(text), end))?,
            6 => break (ExitReason::Noconnection, at),
            _ => return None,
        }
    };
    let reason = links
        .into_iter()
        .rev()
        .fold(leaf, |inner, pid| ExitReason::Linked(pid, Box::new(inner)));
    Some((reason, end))
}

/// Propagate exit signals to all linked processes.
///
/// For each linked PID:
/// - A process with `trap_exit = true` (a supervisor) gets the signal as a
///   message, whatever the reason; one that traps this link (a job's
///   caller) gets it for an abnormal exit.
/// - Otherwise a normal or shutdown exit is dropped: the process's own
///   `receive` would read the signal as one of its messages (a supervised
///   worker printed "got 2", its supervisor's pid, as `main` ended).
/// - Otherwise an abnormal exit marks the linked process
///   `Exited(Linked(exiting_pid, reason))`.
///
/// Returns the set of linked PIDs so the caller can wake Waiting processes.
pub fn propagate_exit<F>(
    exiting_pid: ProcessId,
    reason: &ExitReason,
    linked_pids: HashSet<ProcessId>,
    get_process: F,
) -> Vec<ProcessId>
where
    F: Fn(ProcessId) -> Option<Arc<Mutex<Process>>>,
{
    let mut woken = Vec::new();
    let signal_data = encode_exit_signal(exiting_pid, reason);

    for linked_pid in &linked_pids {
        if let Some(proc_arc) = get_process(*linked_pid) {
            let mut proc = proc_arc.lock();

            // Skip already-exited processes.
            if matches!(proc.state, ProcessState::Exited(_)) {
                continue;
            }

            // Remove the reverse link (the exiting process is gone).
            proc.links.remove(&exiting_pid);

            let is_non_crashing = matches!(reason, ExitReason::Normal | ExitReason::Shutdown);
            let trapped = proc.trapped_links.remove(&exiting_pid) && !is_non_crashing;

            if proc.trap_exit || trapped {
                // Deliver as a regular message -- the process does not crash.
                let buffer = MessageBuffer::new(signal_data.clone(), EXIT_SIGNAL_TAG);
                proc.mailbox.push(Message { buffer });

                // Wake if Waiting.
                if matches!(proc.state, ProcessState::Waiting)
                    && proc.set_live_state(ProcessState::Ready)
                {
                    woken.push(*linked_pid);
                }
            } else if !is_non_crashing {
                // Crash the linked process with a Linked exit reason.
                proc.mark_exited(ExitReason::Linked(exiting_pid, Box::new(reason.clone())));
            }
        }
    }

    woken
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actor::process::{Priority, Process, ProcessId};
    use std::sync::Arc;

    /// An exit reason from another node is read defensively: bytes cut short,
    /// a length past the end (even one that overflows) and nesting deeper
    /// than any chain of links decode to nothing, never a panic.
    #[test]
    fn a_malformed_exit_reason_decodes_to_nothing() {
        let mut long_text = vec![1];
        long_text.extend_from_slice(&u64::MAX.to_le_bytes());
        let mut deep = Vec::new();
        for _ in 0..=MAX_LINK_DEPTH {
            deep.push(3);
            deep.extend_from_slice(&7u64.to_le_bytes());
        }
        deep.push(0);
        for data in [
            &[][..],
            &[1, 5, 0],
            &[1, 5, 0, 0, 0, 0, 0, 0, 0, b'a'],
            &[3, 1, 2],
            &[5, 1, 0],
            &[5, 3, 0, 0, 0, 0, 0, 0, 0, b'a'],
            &[1, 1, 0, 0, 0, 0, 0, 0, 0, 0xff],
            &[9],
            &long_text,
            &deep,
        ] {
            assert_eq!(decode_reason(data), None, "{data:?}");
        }
        assert_eq!(decode_exit_signal(&[0; 8]), None);
    }

    /// A linked process that has already left the table is passed over.
    #[test]
    fn propagate_exit_passes_over_a_process_that_is_gone() {
        let (gone, _) = make_process();
        let linked = [gone].into_iter().collect();
        let woken = propagate_exit(ProcessId(1), &ExitReason::Killed, linked, |_| None);
        assert!(woken.is_empty());
    }

    /// A chain of links as deep as any real one decodes, and says how many
    /// bytes it took.
    #[test]
    fn a_deep_chain_of_linked_reasons_round_trips() {
        let reason = (0..MAX_LINK_DEPTH).fold(ExitReason::Custom("root".into()), |inner, pid| {
            ExitReason::Linked(ProcessId(pid as u64), Box::new(inner))
        });
        let mut data = Vec::new();
        encode_reason(&mut data, &reason);
        data.push(0xAA);
        assert_eq!(decode_reason(&data), Some((reason, data.len() - 1)));
    }

    fn make_process() -> (ProcessId, Arc<Mutex<Process>>) {
        let pid = ProcessId::next();
        let proc = Arc::new(Mutex::new(Process::new(pid, Priority::Normal)));
        (pid, proc)
    }

    #[test]
    fn test_link_creates_bidirectional_link() {
        let (pid_a, proc_a) = make_process();
        let (pid_b, proc_b) = make_process();

        link(&proc_a, &proc_b, pid_a, pid_b);

        assert!(proc_a.lock().links.contains(&pid_b));
        assert!(proc_b.lock().links.contains(&pid_a));
    }

    #[test]
    fn test_link_idempotent() {
        let (pid_a, proc_a) = make_process();
        let (pid_b, proc_b) = make_process();

        link(&proc_a, &proc_b, pid_a, pid_b);
        link(&proc_a, &proc_b, pid_a, pid_b);

        // HashSet ensures no duplicates.
        assert_eq!(proc_a.lock().links.len(), 1);
        assert_eq!(proc_b.lock().links.len(), 1);
    }

    #[test]
    fn test_unlink_removes_bidirectional_link() {
        let (pid_a, proc_a) = make_process();
        let (pid_b, proc_b) = make_process();

        link(&proc_a, &proc_b, pid_a, pid_b);
        unlink(&proc_a, &proc_b, pid_a, pid_b);

        assert!(proc_a.lock().links.is_empty());
        assert!(proc_b.lock().links.is_empty());
    }

    #[test]
    fn test_normal_exit_is_ignored_by_a_process_not_trapping_exits() {
        let (pid_a, _proc_a) = make_process();
        let (pid_b, proc_b) = make_process();

        let linked = {
            let mut s = HashSet::new();
            s.insert(pid_b);
            s
        };

        let proc_b_clone = Arc::clone(&proc_b);
        propagate_exit(pid_a, &ExitReason::Normal, linked, |pid| {
            if pid == pid_b {
                Some(Arc::clone(&proc_b_clone))
            } else {
                None
            }
        });

        // Process B neither crashes nor gets a message its receive would
        // misread; only the link is gone.
        let b = proc_b.lock();
        assert!(
            !matches!(b.state, ProcessState::Exited(_)),
            "Normal exit should not crash linked process"
        );
        assert!(b.mailbox.pop().is_none());
        assert!(!b.links.contains(&pid_a));
    }

    #[test]
    fn test_normal_exit_reaches_a_process_trapping_exits() {
        let (pid_a, _proc_a) = make_process();
        let (pid_b, proc_b) = make_process();
        proc_b.lock().trap_exit = true;

        let proc_b_clone = Arc::clone(&proc_b);
        propagate_exit(pid_a, &ExitReason::Normal, HashSet::from([pid_b]), |pid| {
            (pid == pid_b).then(|| Arc::clone(&proc_b_clone))
        });

        let msg = proc_b.lock().mailbox.pop().unwrap();
        assert_eq!(msg.buffer.type_tag, EXIT_SIGNAL_TAG);
    }

    #[test]
    fn test_error_exit_crashes_linked_process() {
        let (pid_a, _proc_a) = make_process();
        let (pid_b, proc_b) = make_process();

        let linked = {
            let mut s = HashSet::new();
            s.insert(pid_b);
            s
        };

        let proc_b_clone = Arc::clone(&proc_b);
        propagate_exit(
            pid_a,
            &ExitReason::Error("division by zero".to_string()),
            linked,
            |pid| {
                if pid == pid_b {
                    Some(Arc::clone(&proc_b_clone))
                } else {
                    None
                }
            },
        );

        // Process B should have crashed with Linked reason.
        let b = proc_b.lock();
        match &b.state {
            ProcessState::Exited(ExitReason::Linked(from_pid, inner)) => {
                assert_eq!(*from_pid, pid_a);
                match inner.as_ref() {
                    ExitReason::Error(msg) => assert_eq!(msg, "division by zero"),
                    other => panic!("Expected Error, got {:?}", other),
                }
            }
            other => panic!("Expected Exited(Linked(...)), got {:?}", other),
        }
    }

    #[test]
    fn linked_crash_destroys_owned_secrets() {
        let (exiting_pid, _exiting_process) = make_process();
        let (linked_pid, linked_process) = make_process();
        crate::secret::insert_test_secret(linked_pid);
        let linked_pids = HashSet::from([linked_pid]);

        propagate_exit(
            exiting_pid,
            &ExitReason::Error("crash".to_string()),
            linked_pids,
            |_| Some(Arc::clone(&linked_process)),
        );

        let remaining = crate::secret::owned_secret_count_for_test(linked_pid);
        crate::secret::destroy_owned(linked_pid);
        assert_eq!(remaining, 0);
    }

    #[test]
    fn test_error_exit_with_trap_exit_delivers_message() {
        let (pid_a, _proc_a) = make_process();
        let (pid_b, proc_b) = make_process();

        // Enable trap_exit on process B.
        proc_b.lock().trap_exit = true;

        let linked = {
            let mut s = HashSet::new();
            s.insert(pid_b);
            s
        };

        let proc_b_clone = Arc::clone(&proc_b);
        propagate_exit(
            pid_a,
            &ExitReason::Error("crash".to_string()),
            linked,
            |pid| {
                if pid == pid_b {
                    Some(Arc::clone(&proc_b_clone))
                } else {
                    None
                }
            },
        );

        // Process B should NOT have crashed (trap_exit = true).
        let b = proc_b.lock();
        assert!(
            !matches!(b.state, ProcessState::Exited(_)),
            "trap_exit should prevent crash"
        );

        // Should have received exit signal as message.
        let msg = b.mailbox.pop().unwrap();
        assert_eq!(msg.buffer.type_tag, EXIT_SIGNAL_TAG);
    }

    #[test]
    fn test_killed_exit_crashes_linked_process() {
        let (pid_a, _proc_a) = make_process();
        let (pid_b, proc_b) = make_process();

        let linked = {
            let mut s = HashSet::new();
            s.insert(pid_b);
            s
        };

        let proc_b_clone = Arc::clone(&proc_b);
        propagate_exit(pid_a, &ExitReason::Killed, linked, |pid| {
            if pid == pid_b {
                Some(Arc::clone(&proc_b_clone))
            } else {
                None
            }
        });

        let b = proc_b.lock();
        match &b.state {
            ProcessState::Exited(ExitReason::Linked(from_pid, inner)) => {
                assert_eq!(*from_pid, pid_a);
                assert!(matches!(inner.as_ref(), ExitReason::Killed));
            }
            other => panic!("Expected Exited(Linked(..., Killed)), got {:?}", other),
        }
    }

    #[test]
    fn test_propagation_removes_reverse_link() {
        let (pid_a, _proc_a) = make_process();
        let (pid_b, proc_b) = make_process();

        // Manually add reverse link.
        proc_b.lock().links.insert(pid_a);

        let linked = {
            let mut s = HashSet::new();
            s.insert(pid_b);
            s
        };

        let proc_b_clone = Arc::clone(&proc_b);
        propagate_exit(pid_a, &ExitReason::Normal, linked, |pid| {
            if pid == pid_b {
                Some(Arc::clone(&proc_b_clone))
            } else {
                None
            }
        });

        // Reverse link should be removed.
        assert!(!proc_b.lock().links.contains(&pid_a));
    }

    #[test]
    fn test_propagation_wakes_waiting_process() {
        let (pid_a, _proc_a) = make_process();
        let (pid_b, proc_b) = make_process();

        // Set B to Waiting state; it traps exits, so the signal wakes it.
        proc_b.lock().state = ProcessState::Waiting;
        proc_b.lock().trap_exit = true;

        let linked = {
            let mut s = HashSet::new();
            s.insert(pid_b);
            s
        };

        let proc_b_clone = Arc::clone(&proc_b);
        let woken = propagate_exit(pid_a, &ExitReason::Normal, linked, |pid| {
            if pid == pid_b {
                Some(Arc::clone(&proc_b_clone))
            } else {
                None
            }
        });

        assert!(woken.contains(&pid_b));
        assert!(matches!(proc_b.lock().state, ProcessState::Ready));
    }

    #[test]
    fn test_propagation_skips_exited_process() {
        let (pid_a, _proc_a) = make_process();
        let (pid_b, proc_b) = make_process();

        // Already exited.
        proc_b.lock().state = ProcessState::Exited(ExitReason::Normal);

        let linked = {
            let mut s = HashSet::new();
            s.insert(pid_b);
            s
        };

        let proc_b_clone = Arc::clone(&proc_b);
        propagate_exit(
            pid_a,
            &ExitReason::Error("crash".to_string()),
            linked,
            |pid| {
                if pid == pid_b {
                    Some(Arc::clone(&proc_b_clone))
                } else {
                    None
                }
            },
        );

        // Should still be Normal exited, not overwritten.
        assert!(matches!(
            proc_b.lock().state,
            ProcessState::Exited(ExitReason::Normal)
        ));
    }

    #[test]
    fn test_encode_exit_signal_normal() {
        let pid = ProcessId(42);
        let data = encode_exit_signal(pid, &ExitReason::Normal);
        // 8 bytes PID + 1 byte reason_tag(0)
        assert_eq!(data.len(), 9);
        let read_pid = u64::from_le_bytes(data[0..8].try_into().unwrap());
        assert_eq!(read_pid, 42);
        assert_eq!(data[8], 0); // Normal
    }

    #[test]
    fn test_encode_exit_signal_error() {
        let pid = ProcessId(7);
        let data = encode_exit_signal(pid, &ExitReason::Error("oops".to_string()));
        // 8 bytes PID + 1 byte tag(1) + 8 bytes len + 4 bytes "oops"
        assert_eq!(data.len(), 21);
        assert_eq!(data[8], 1); // Error
        let msg_len = u64::from_le_bytes(data[9..17].try_into().unwrap());
        assert_eq!(msg_len, 4);
        assert_eq!(&data[17..21], b"oops");
    }

    #[test]
    fn test_shutdown_exit_is_ignored_by_a_process_not_trapping_exits() {
        let (pid_a, _proc_a) = make_process();
        let (pid_b, proc_b) = make_process();

        let linked = {
            let mut s = HashSet::new();
            s.insert(pid_b);
            s
        };

        let proc_b_clone = Arc::clone(&proc_b);
        propagate_exit(pid_a, &ExitReason::Shutdown, linked, |pid| {
            if pid == pid_b {
                Some(Arc::clone(&proc_b_clone))
            } else {
                None
            }
        });

        // Process B should NOT have crashed (Shutdown is non-crashing like
        // Normal), nor get a message.
        let b = proc_b.lock();
        assert!(
            !matches!(b.state, ProcessState::Exited(_)),
            "Shutdown exit should not crash linked process"
        );
        assert!(b.mailbox.pop().is_none());
    }

    #[test]
    fn test_custom_exit_crashes_linked_process() {
        let (pid_a, _proc_a) = make_process();
        let (pid_b, proc_b) = make_process();

        let linked = {
            let mut s = HashSet::new();
            s.insert(pid_b);
            s
        };

        let proc_b_clone = Arc::clone(&proc_b);
        propagate_exit(
            pid_a,
            &ExitReason::Custom("user_reason".to_string()),
            linked,
            |pid| {
                if pid == pid_b {
                    Some(Arc::clone(&proc_b_clone))
                } else {
                    None
                }
            },
        );

        // Process B should have crashed (Custom is crashing like Error).
        let b = proc_b.lock();
        match &b.state {
            ProcessState::Exited(ExitReason::Linked(from_pid, inner)) => {
                assert_eq!(*from_pid, pid_a);
                match inner.as_ref() {
                    ExitReason::Custom(msg) => assert_eq!(msg, "user_reason"),
                    other => panic!("Expected Custom, got {:?}", other),
                }
            }
            other => panic!("Expected Exited(Linked(...)), got {:?}", other),
        }
    }

    #[test]
    fn test_encode_decode_roundtrip_normal() {
        let pid = ProcessId(100);
        let reason = ExitReason::Normal;
        let data = encode_exit_signal(pid, &reason);
        let (decoded_pid, decoded_reason) = decode_exit_signal(&data).unwrap();
        assert_eq!(decoded_pid, pid);
        assert!(matches!(decoded_reason, ExitReason::Normal));
    }

    #[test]
    fn test_encode_decode_roundtrip_shutdown() {
        let pid = ProcessId(200);
        let reason = ExitReason::Shutdown;
        let data = encode_exit_signal(pid, &reason);
        let (decoded_pid, decoded_reason) = decode_exit_signal(&data).unwrap();
        assert_eq!(decoded_pid, pid);
        assert!(matches!(decoded_reason, ExitReason::Shutdown));
    }

    #[test]
    fn test_encode_decode_roundtrip_error() {
        let pid = ProcessId(300);
        let reason = ExitReason::Error("division by zero".to_string());
        let data = encode_exit_signal(pid, &reason);
        let (decoded_pid, decoded_reason) = decode_exit_signal(&data).unwrap();
        assert_eq!(decoded_pid, pid);
        match decoded_reason {
            ExitReason::Error(msg) => assert_eq!(msg, "division by zero"),
            other => panic!("Expected Error, got {:?}", other),
        }
    }

    #[test]
    fn test_encode_decode_roundtrip_killed() {
        let pid = ProcessId(400);
        let reason = ExitReason::Killed;
        let data = encode_exit_signal(pid, &reason);
        let (decoded_pid, decoded_reason) = decode_exit_signal(&data).unwrap();
        assert_eq!(decoded_pid, pid);
        assert!(matches!(decoded_reason, ExitReason::Killed));
    }

    #[test]
    fn test_encode_decode_roundtrip_linked() {
        let pid = ProcessId(500);
        let inner_pid = ProcessId(501);
        let reason =
            ExitReason::Linked(inner_pid, Box::new(ExitReason::Error("crash".to_string())));
        let data = encode_exit_signal(pid, &reason);
        let (decoded_pid, decoded_reason) = decode_exit_signal(&data).unwrap();
        assert_eq!(decoded_pid, pid);
        match decoded_reason {
            ExitReason::Linked(lp, inner) => {
                assert_eq!(lp, inner_pid);
                match *inner {
                    ExitReason::Error(msg) => assert_eq!(msg, "crash"),
                    other => panic!("Expected Error, got {:?}", other),
                }
            }
            other => panic!("Expected Linked, got {:?}", other),
        }
    }

    #[test]
    fn test_encode_decode_roundtrip_custom() {
        let pid = ProcessId(600);
        let reason = ExitReason::Custom("user_shutdown".to_string());
        let data = encode_exit_signal(pid, &reason);
        let (decoded_pid, decoded_reason) = decode_exit_signal(&data).unwrap();
        assert_eq!(decoded_pid, pid);
        match decoded_reason {
            ExitReason::Custom(msg) => assert_eq!(msg, "user_shutdown"),
            other => panic!("Expected Custom, got {:?}", other),
        }
    }

    #[test]
    fn test_decode_exit_signal_too_short() {
        // Less than 9 bytes should fail
        assert!(decode_exit_signal(&[0u8; 8]).is_none());
        assert!(decode_exit_signal(&[]).is_none());
    }

    #[test]
    fn test_encode_decode_roundtrip_shutdown_signal() {
        let pid = ProcessId(42);
        let data = encode_exit_signal(pid, &ExitReason::Shutdown);
        // 8 bytes PID + 1 byte reason_tag(4)
        assert_eq!(data.len(), 9);
        assert_eq!(data[8], 4); // Shutdown tag
    }

    #[test]
    fn test_multiple_linked_processes() {
        let (pid_a, _proc_a) = make_process();
        let (pid_b, proc_b) = make_process();
        let (pid_c, proc_c) = make_process();

        let linked = {
            let mut s = HashSet::new();
            s.insert(pid_b);
            s.insert(pid_c);
            s
        };

        let proc_b_clone = Arc::clone(&proc_b);
        let proc_c_clone = Arc::clone(&proc_c);
        propagate_exit(
            pid_a,
            &ExitReason::Error("crash".to_string()),
            linked,
            |pid| {
                if pid == pid_b {
                    Some(Arc::clone(&proc_b_clone))
                } else if pid == pid_c {
                    Some(Arc::clone(&proc_c_clone))
                } else {
                    None
                }
            },
        );

        // Both B and C should have crashed.
        assert!(matches!(
            proc_b.lock().state,
            ProcessState::Exited(ExitReason::Linked(..))
        ));
        assert!(matches!(
            proc_c.lock().state,
            ProcessState::Exited(ExitReason::Linked(..))
        ));
    }
}
