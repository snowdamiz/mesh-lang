//! FIFO mailbox for Mesh actor message passing.
//!
//! Each actor has a mailbox that delivers messages in strict FIFO order.
//! The mailbox is thread-safe (protected by a Mutex) since messages can
//! be sent from any actor on any worker thread.

use std::collections::VecDeque;

use parking_lot::Mutex;

use super::process::Message;

pub const DEFAULT_MAILBOX_MAX_ITEMS: usize = 1024;
pub const DEFAULT_MAILBOX_MAX_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MailboxPushError {
    Full,
    MessageTooLarge,
}

struct MailboxState {
    queue: VecDeque<Message>,
    bytes: usize,
    rejected: u64,
}

/// A thread-safe FIFO mailbox for an actor.
///
/// Messages are appended to the back (`push`) and removed from the front
/// (`pop`), ensuring strict FIFO delivery order. The internal queue is
/// protected by a `parking_lot::Mutex` for efficient cross-thread access.
pub struct Mailbox {
    state: Mutex<MailboxState>,
    max_items: usize,
    max_bytes: usize,
}

impl Mailbox {
    /// Create a new empty mailbox.
    pub fn new() -> Self {
        Self::bounded(DEFAULT_MAILBOX_MAX_ITEMS, DEFAULT_MAILBOX_MAX_BYTES)
    }

    pub fn bounded(max_items: usize, max_bytes: usize) -> Self {
        Mailbox {
            state: Mutex::new(MailboxState {
                queue: VecDeque::new(),
                bytes: 0,
                rejected: 0,
            }),
            max_items,
            max_bytes,
        }
    }

    /// Append a message to the back of the mailbox (FIFO enqueue).
    pub fn push(&self, msg: Message) {
        // Legacy fire-and-forget sends intentionally discard backpressure; callers
        // that need delivery feedback use Process.try_send.
        let _ = self.try_push(msg);
    }

    pub fn try_push(&self, msg: Message) -> Result<(), MailboxPushError> {
        let message_bytes = msg.buffer.data.len();
        let depth = {
            let mut state = self.state.lock();
            if message_bytes > self.max_bytes {
                state.rejected = state.rejected.saturating_add(1);
                return Err(MailboxPushError::MessageTooLarge);
            }
            if state.queue.len() >= self.max_items
                || state.bytes.saturating_add(message_bytes) > self.max_bytes
            {
                state.rejected = state.rejected.saturating_add(1);
                return Err(MailboxPushError::Full);
            }
            state.bytes += message_bytes;
            state.queue.push_back(msg);
            state.queue.len()
        };
        crate::dist::telemetry::runtime_telemetry().record_mailbox_enqueue(depth);
        Ok(())
    }

    /// Remove and return the front message (FIFO dequeue).
    ///
    /// Returns `None` if the mailbox is empty.
    pub fn pop(&self) -> Option<Message> {
        let message = {
            let mut state = self.state.lock();
            let message = state.queue.pop_front();
            if let Some(message) = &message {
                state.bytes -= message.buffer.data.len();
            }
            message
        };
        if message.is_some() {
            crate::dist::telemetry::runtime_telemetry().record_mailbox_dequeue(1);
        }
        message
    }

    /// Check if the mailbox is empty.
    pub fn is_empty(&self) -> bool {
        self.state.lock().queue.is_empty()
    }

    /// Return the number of messages in the mailbox.
    pub fn len(&self) -> usize {
        self.state.lock().queue.len()
    }

    pub fn byte_len(&self) -> usize {
        self.state.lock().bytes
    }

    pub fn rejected(&self) -> u64 {
        self.state.lock().rejected
    }

    /// Selectively remove the first message matching a predicate.
    ///
    /// Scans the mailbox from front to back and removes the first message
    /// for which `predicate` returns `true`. All other messages remain in
    /// their original order. Returns `None` if no message matches.
    ///
    /// This implements Erlang-style selective receive: the caller can wait
    /// for a specific message while leaving unrelated messages queued.
    pub fn remove_first<F>(&self, predicate: F) -> Option<Message>
    where
        F: Fn(&Message) -> bool,
    {
        let mut state = self.state.lock();
        for i in 0..state.queue.len() {
            if predicate(&state.queue[i]) {
                let message = state.queue.remove(i);
                if let Some(message) = &message {
                    state.bytes -= message.buffer.data.len();
                }
                drop(state);
                if message.is_some() {
                    crate::dist::telemetry::runtime_telemetry().record_mailbox_dequeue(1);
                }
                return message;
            }
        }
        None
    }
}

impl Drop for Mailbox {
    fn drop(&mut self) {
        let remaining = self.state.get_mut().queue.len();
        if remaining > 0 {
            crate::dist::telemetry::runtime_telemetry().record_mailbox_dequeue(remaining);
        }
    }
}

impl Default for Mailbox {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for Mailbox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.state.lock();
        f.debug_struct("Mailbox")
            .field("len", &state.queue.len())
            .field("bytes", &state.bytes)
            .field("rejected", &state.rejected)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actor::heap::MessageBuffer;

    fn make_msg(data: &[u8], tag: u64) -> Message {
        Message {
            buffer: MessageBuffer::new(data.to_vec(), tag),
        }
    }

    #[test]
    fn test_mailbox_push_pop_fifo() {
        let mb = Mailbox::new();

        mb.push(make_msg(&[1], 1));
        mb.push(make_msg(&[2], 2));
        mb.push(make_msg(&[3], 3));

        // Messages should come out in FIFO order.
        let m1 = mb.pop().unwrap();
        assert_eq!(m1.buffer.type_tag, 1);
        assert_eq!(m1.buffer.data, vec![1]);

        let m2 = mb.pop().unwrap();
        assert_eq!(m2.buffer.type_tag, 2);

        let m3 = mb.pop().unwrap();
        assert_eq!(m3.buffer.type_tag, 3);

        assert!(mb.pop().is_none());
    }

    #[test]
    fn test_mailbox_empty() {
        let mb = Mailbox::new();
        assert!(mb.is_empty());
        assert_eq!(mb.len(), 0);
        assert!(mb.pop().is_none());
    }

    #[test]
    fn test_mailbox_len() {
        let mb = Mailbox::new();
        assert_eq!(mb.len(), 0);

        mb.push(make_msg(&[1], 1));
        assert_eq!(mb.len(), 1);

        mb.push(make_msg(&[2], 2));
        assert_eq!(mb.len(), 2);

        mb.pop();
        assert_eq!(mb.len(), 1);
    }

    #[test]
    fn bounded_mailbox_rejects_new_messages_at_item_limit() {
        let mb = Mailbox::bounded(2, 1024);

        assert_eq!(mb.try_push(make_msg(&[1], 1)), Ok(()));
        assert_eq!(mb.try_push(make_msg(&[2], 2)), Ok(()));
        assert_eq!(mb.try_push(make_msg(&[3], 3)), Err(MailboxPushError::Full));
        assert_eq!(mb.len(), 2);
    }

    #[test]
    fn bounded_mailbox_tracks_and_enforces_payload_bytes() {
        let mb = Mailbox::bounded(4, 3);

        assert_eq!(mb.try_push(make_msg(&[1, 2], 1)), Ok(()));
        assert_eq!(mb.byte_len(), 2);
        assert_eq!(
            mb.try_push(make_msg(&[3, 4], 2)),
            Err(MailboxPushError::Full)
        );
        assert_eq!(mb.pop().unwrap().buffer.data, [1, 2]);
        assert_eq!(mb.byte_len(), 0);
        assert_eq!(
            mb.try_push(make_msg(&[1, 2, 3, 4], 3)),
            Err(MailboxPushError::MessageTooLarge)
        );
    }

    #[test]
    fn test_mailbox_concurrent_push() {
        use std::sync::Arc;

        let mb = Arc::new(Mailbox::new());
        let num_threads = 8;
        let msgs_per_thread = 100;

        let handles: Vec<_> = (0..num_threads)
            .map(|t| {
                let mb = Arc::clone(&mb);
                std::thread::spawn(move || {
                    for i in 0..msgs_per_thread {
                        let tag = (t * msgs_per_thread + i) as u64;
                        mb.push(make_msg(&[tag as u8], tag));
                    }
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(mb.len(), num_threads * msgs_per_thread);

        // Drain all messages -- should get exactly num_threads * msgs_per_thread.
        let mut count = 0;
        while mb.pop().is_some() {
            count += 1;
        }
        assert_eq!(count, num_threads * msgs_per_thread);
    }
}
