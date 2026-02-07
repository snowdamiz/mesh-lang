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
use std::sync::Arc;

use parking_lot::Mutex;

use super::heap::MessageBuffer;
use super::process::{ExitReason, Message, Process, ProcessId, ProcessState};

/// Special type_tag used for exit signal messages.
///
/// u64::MAX is reserved as the exit signal sentinel -- no regular message
/// should use this tag. The data payload encodes the exiting PID and reason.
pub const EXIT_SIGNAL_TAG: u64 = u64::MAX;

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
fn encode_exit_signal(exiting_pid: ProcessId, reason: &ExitReason) -> Vec<u8> {
    let mut data = Vec::new();
    // Write the exiting PID (8 bytes).
    data.extend_from_slice(&exiting_pid.0.to_le_bytes());
    // Write the reason.
    encode_reason(&mut data, reason);
    data
}

fn encode_reason(data: &mut Vec<u8>, reason: &ExitReason) {
    match reason {
        ExitReason::Normal => {
            data.push(0);
        }
        ExitReason::Error(msg) => {
            data.push(1);
            let bytes = msg.as_bytes();
            data.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
            data.extend_from_slice(bytes);
        }
        ExitReason::Killed => {
            data.push(2);
        }
        ExitReason::Linked(pid, inner) => {
            data.push(3);
            data.extend_from_slice(&pid.0.to_le_bytes());
            encode_reason(data, inner);
        }
    }
}

/// Propagate exit signals to all linked processes.
///
/// For each linked PID:
/// - Normal exit: deliver exit signal as a regular message (no crash).
/// - Error/Killed exit: if the linked process has `trap_exit = true`, deliver
///   as a regular message. Otherwise, mark the linked process as
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

            let is_normal = matches!(reason, ExitReason::Normal);

            if is_normal || proc.trap_exit {
                // Deliver as a regular message -- the process does not crash.
                let buffer = MessageBuffer::new(signal_data.clone(), EXIT_SIGNAL_TAG);
                proc.mailbox.push(Message { buffer });

                // Wake if Waiting.
                if matches!(proc.state, ProcessState::Waiting) {
                    proc.state = ProcessState::Ready;
                    woken.push(*linked_pid);
                }
            } else {
                // Crash the linked process with a Linked exit reason.
                proc.state = ProcessState::Exited(ExitReason::Linked(
                    exiting_pid,
                    Box::new(reason.clone()),
                ));
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
    fn test_normal_exit_delivers_message_no_crash() {
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

        // Process B should NOT have crashed.
        let b = proc_b.lock();
        assert!(
            !matches!(b.state, ProcessState::Exited(_)),
            "Normal exit should not crash linked process"
        );

        // But it should have received an exit signal message.
        let msg = b.mailbox.pop().unwrap();
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

        // Set B to Waiting state.
        proc_b.lock().state = ProcessState::Waiting;

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
        propagate_exit(pid_a, &ExitReason::Error("crash".to_string()), linked, |pid| {
            if pid == pid_b {
                Some(Arc::clone(&proc_b_clone))
            } else {
                None
            }
        });

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
