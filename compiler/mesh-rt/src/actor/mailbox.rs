//! FIFO mailbox for Mesh actor message passing.
//!
//! Each actor has a mailbox that delivers messages in strict FIFO order.
//! The mailbox is thread-safe (protected by a Mutex) since messages can
//! be sent from any actor on any worker thread.

use std::collections::VecDeque;

use parking_lot::Mutex;

use super::process::Message;
use crate::dist::telemetry::runtime_telemetry;

mod quota;
pub(crate) use quota::MailboxUsage;

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
    depth_sampler: DepthSampler,
}

struct DepthSampler {
    random_state: u32,
    skip: u8,
}

impl DepthSampler {
    fn new(pid: u64) -> Self {
        Self {
            random_state: (pid as u32 ^ (pid >> 32) as u32).wrapping_mul(0x9E37_79B9),
            skip: 0,
        }
    }

    fn sample(&mut self) -> bool {
        if self.skip != 0 {
            self.skip -= 1;
            return false;
        }
        // Vary the interval so periodic mailbox traffic cannot always hit
        // the same point in its cycle. Only advance the generator on samples.
        self.random_state = self
            .random_state
            .wrapping_mul(1_664_525)
            .wrapping_add(1_013_904_223);
        self.skip = (self.random_state >> 27) as u8;
        true
    }
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
    quota_shard: usize,
}

impl Mailbox {
    /// Create a new empty mailbox.
    pub fn new() -> Self {
        Self::bounded(DEFAULT_MAILBOX_MAX_ITEMS, DEFAULT_MAILBOX_MAX_BYTES)
    }

    pub fn bounded(max_items: usize, max_bytes: usize) -> Self {
        Self::with_shard(max_items, max_bytes, 0, 0)
    }

    pub(super) fn for_process(local_pid: u64, quota_key: u64) -> Self {
        Self::with_shard(
            DEFAULT_MAILBOX_MAX_ITEMS,
            DEFAULT_MAILBOX_MAX_BYTES,
            MailboxUsage::shard_for(quota_key),
            local_pid,
        )
    }

    fn with_shard(max_items: usize, max_bytes: usize, quota_shard: usize, local_pid: u64) -> Self {
        Mailbox {
            state: Mutex::new(MailboxState {
                queue: VecDeque::new(),
                bytes: 0,
                depth_sampler: DepthSampler::new(local_pid),
            }),
            max_items,
            max_bytes,
            quota_shard,
        }
    }

    /// Append a message to the back of the mailbox (FIFO enqueue).
    pub fn push(&self, msg: Message) {
        // Legacy fire-and-forget sends intentionally discard backpressure; callers
        // that need delivery feedback use Process.try_send.
        let _ = self.try_push(msg);
    }

    pub fn try_push(&self, msg: Message) -> Result<(), MailboxPushError> {
        self.try_push_inner(msg, false)
    }

    pub(crate) fn try_push_control(&self, msg: Message) -> Result<(), MailboxPushError> {
        self.try_push_inner(msg, true)
    }

    fn try_push_inner(&self, msg: Message, control: bool) -> Result<(), MailboxPushError> {
        let message_bytes = msg.buffer.data().len();
        let mut state = self.state.lock();
        if message_bytes > self.max_bytes {
            return Err(MailboxPushError::MessageTooLarge);
        }
        let max_items = self.max_items.saturating_add(if control { 1 } else { 0 });
        let max_bytes = self
            .max_bytes
            .saturating_add(if control { message_bytes } else { 0 });
        if state.queue.len() >= max_items || state.bytes.saturating_add(message_bytes) > max_bytes {
            return Err(MailboxPushError::Full);
        }
        let telemetry = runtime_telemetry();
        let Some(reservation) =
            telemetry
                .mailbox_usage
                .reserve(self.quota_shard, message_bytes, control)
        else {
            return Err(MailboxPushError::Full);
        };
        state.bytes += message_bytes;
        state.queue.push_back(msg);
        let depth = state.depth_sampler.sample().then_some(state.queue.len());
        drop(reservation);
        drop(state);
        if let Some(depth) = depth {
            telemetry.record_mailbox_depth_sample(depth);
        }
        Ok(())
    }

    /// Remove and return the front message (FIFO dequeue).
    ///
    /// Returns `None` if the mailbox is empty.
    pub fn pop(&self) -> Option<Message> {
        let mut state = self.state.lock();
        if state.queue.is_empty() {
            return None;
        }
        state.dequeue(self.quota_shard, VecDeque::pop_front)
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
                return state.dequeue(self.quota_shard, |queue| queue.remove(i));
            }
        }
        None
    }
}

impl Drop for Mailbox {
    fn drop(&mut self) {
        let state = self.state.get_mut();
        if !state.queue.is_empty() {
            runtime_telemetry()
                .mailbox_usage
                .lock(self.quota_shard)
                .release(state.queue.len(), state.bytes);
        }
    }
}

impl MailboxState {
    fn dequeue(
        &mut self,
        shard: usize,
        remove: impl FnOnce(&mut VecDeque<Message>) -> Option<Message>,
    ) -> Option<Message> {
        let mut quota = runtime_telemetry().mailbox_usage.lock(shard);
        let message = remove(&mut self.queue)?;
        let bytes = message.buffer.data().len();
        self.bytes -= bytes;
        quota.release(1, bytes);
        Some(message)
    }
}

impl Default for Mailbox {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actor::heap::MessageBuffer;

    #[test]
    fn depth_sampling_starts_immediately_and_uses_bounded_variable_intervals() {
        let sample_positions = |pid| {
            let mut sampler = DepthSampler::new(pid);
            (0..2048).filter(|_| sampler.sample()).collect::<Vec<_>>()
        };
        let samples = sample_positions(1);
        assert_eq!(samples[0], 0);
        assert_ne!(samples, sample_positions(2));
        let intervals: Vec<_> = samples.windows(2).map(|pair| pair[1] - pair[0]).collect();
        assert!(intervals.iter().all(|gap| (1..=32).contains(gap)));
        assert!(intervals.iter().any(|gap| *gap != intervals[0]));
    }

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
        assert_eq!(m1.buffer.data(), vec![1]);

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
        assert_eq!(mb.pop().unwrap().buffer.data(), [1, 2]);
        assert_eq!(mb.byte_len(), 0);
        assert_eq!(
            mb.try_push(make_msg(&[1, 2, 3, 4], 3)),
            Err(MailboxPushError::MessageTooLarge)
        );
    }

    #[test]
    fn selective_receive_restores_capacity_without_reordering_the_queue() {
        let mb = Mailbox::bounded(3, 4);
        mb.try_push(make_msg(&[1], 1)).unwrap();
        mb.try_push(make_msg(&[2, 2], 2)).unwrap();
        mb.try_push(make_msg(&[3], 3)).unwrap();

        let removed = mb.remove_first(|msg| msg.buffer.type_tag == 2).unwrap();
        assert_eq!(removed.buffer.data(), [2, 2]);
        assert_eq!(mb.byte_len(), 2);
        mb.try_push(make_msg(&[4, 4], 4)).unwrap();
        for tag in [1, 3, 4] {
            assert_eq!(mb.pop().unwrap().buffer.type_tag, tag);
        }
    }

    #[test]
    fn control_message_uses_one_reserved_slot_without_dropping_fifo_data() {
        let mb = Mailbox::bounded(2, 2);

        assert_eq!(mb.try_push(make_msg(&[1], 1)), Ok(()));
        assert_eq!(mb.try_push(make_msg(&[2], 2)), Ok(()));
        assert_eq!(mb.try_push_control(make_msg(&[3], 3)), Ok(()));

        assert_eq!(mb.pop().unwrap().buffer.type_tag, 1);
        assert_eq!(mb.pop().unwrap().buffer.type_tag, 2);
        assert_eq!(mb.pop().unwrap().buffer.type_tag, 3);
        assert!(mb.pop().is_none());
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
