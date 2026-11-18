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
    scalar_head: Option<ScalarMessage>,
    queue: VecDeque<Message>,
    bytes: usize,
    depth_sampler: DepthSampler,
}

#[derive(Clone, Copy)]
pub(super) enum ScalarMessage {
    Byte(u8),
    Word(u64),
}

impl ScalarMessage {
    pub(super) fn from_bytes(data: &[u8]) -> Option<Self> {
        match data {
            [byte] => Some(Self::Byte(*byte)),
            [_, _, _, _, _, _, _, _] => {
                Some(Self::Word(u64::from_ne_bytes(data.try_into().unwrap())))
            }
            _ => None,
        }
    }

    fn len(self) -> usize {
        match self {
            Self::Byte(_) => 1,
            Self::Word(_) => 8,
        }
    }

    fn bits(self) -> u64 {
        match self {
            Self::Byte(byte) => u64::from_ne_bytes([byte, 0, 0, 0, 0, 0, 0, 0]),
            Self::Word(word) => word,
        }
    }

    fn into_message(self) -> Message {
        use super::{MessageBuffer, PROGRAM_MESSAGE_TAG};
        let bytes = self.bits().to_ne_bytes();
        Message {
            buffer: MessageBuffer::from_slice(&bytes[..self.len()], PROGRAM_MESSAGE_TAG),
        }
    }
}

pub(super) enum ScalarDelivery {
    Bits(u64),
    Message(Message),
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
                scalar_head: None,
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
        let mut state = self.state.lock();
        let depth = self
            .limits()
            .push(&mut state, msg.buffer.data().len(), control, |state| {
                state.queue.push_back(msg);
            })?;
        drop(state);
        Self::record_depth(depth);
        Ok(())
    }

    pub(super) fn try_push_owned(&mut self, message: Message) -> Result<(), MailboxPushError> {
        let size = message.buffer.data().len();
        self.push_owned(size, |state| state.queue.push_back(message))
    }

    pub(super) fn try_push_scalar(
        &mut self,
        message: ScalarMessage,
    ) -> Result<(), MailboxPushError> {
        self.push_owned(message.len(), |state| {
            if state.is_empty() {
                state.scalar_head = Some(message);
            } else {
                state.queue.push_back(message.into_message());
            }
        })
    }

    fn push_owned(
        &mut self,
        bytes: usize,
        append: impl FnOnce(&mut MailboxState),
    ) -> Result<(), MailboxPushError> {
        let depth = self
            .limits()
            .push(self.state.get_mut(), bytes, false, append)?;
        Self::record_depth(depth);
        Ok(())
    }

    fn limits(&self) -> MailboxLimits {
        MailboxLimits {
            items: self.max_items,
            bytes: self.max_bytes,
            quota_shard: self.quota_shard,
        }
    }

    fn record_depth(depth: Option<usize>) {
        if let Some(depth) = depth {
            runtime_telemetry().record_mailbox_depth_sample(depth);
        }
    }

    /// Remove and return the front message (FIFO dequeue).
    ///
    /// Returns `None` if the mailbox is empty.
    pub fn pop(&self) -> Option<Message> {
        let mut state = self.state.lock();
        if let Some(scalar) = state.take_scalar_head(self.quota_shard) {
            return Some(scalar.into_message());
        }
        if state.queue.is_empty() {
            return None;
        }
        state.dequeue(self.quota_shard, VecDeque::pop_front)
    }

    /// Check if the mailbox is empty.
    pub fn is_empty(&self) -> bool {
        self.state.lock().is_empty()
    }

    /// Return the number of messages in the mailbox.
    pub fn len(&self) -> usize {
        self.state.lock().len()
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
        state.remove_first(self.quota_shard, predicate)
    }

    pub(super) fn remove_first_owned(
        &mut self,
        predicate: impl Fn(&Message) -> bool,
    ) -> Option<Message> {
        self.state
            .get_mut()
            .remove_first(self.quota_shard, predicate)
    }

    pub(super) fn receive_scalar(&mut self, width: u64) -> Option<ScalarDelivery> {
        let state = self.state.get_mut();
        if let Some(scalar) = state.take_scalar_head(self.quota_shard) {
            return if scalar.len() as u64 == width {
                Some(ScalarDelivery::Bits(scalar.bits()))
            } else {
                Some(ScalarDelivery::Message(scalar.into_message()))
            };
        }
        state
            .remove_first(self.quota_shard, |msg| {
                msg.buffer.type_tag == super::PROGRAM_MESSAGE_TAG
            })
            .map(ScalarDelivery::Message)
    }
}

struct MailboxLimits {
    items: usize,
    bytes: usize,
    quota_shard: usize,
}

impl MailboxLimits {
    fn push(
        self,
        state: &mut MailboxState,
        message_bytes: usize,
        control: bool,
        append: impl FnOnce(&mut MailboxState),
    ) -> Result<Option<usize>, MailboxPushError> {
        if message_bytes > self.bytes {
            return Err(MailboxPushError::MessageTooLarge);
        }
        let max_items = self.items.saturating_add(if control { 1 } else { 0 });
        let max_bytes = self
            .bytes
            .saturating_add(if control { message_bytes } else { 0 });
        if state.len() >= max_items || state.bytes.saturating_add(message_bytes) > max_bytes {
            return Err(MailboxPushError::Full);
        }
        let Some(reservation) =
            runtime_telemetry()
                .mailbox_usage
                .reserve(self.quota_shard, message_bytes, control)
        else {
            return Err(MailboxPushError::Full);
        };
        state.bytes += message_bytes;
        append(state);
        let depth = state.depth_sampler.sample().then_some(state.len());
        drop(reservation);
        Ok(depth)
    }
}

impl MailboxState {
    fn len(&self) -> usize {
        self.queue.len() + usize::from(self.scalar_head.is_some())
    }

    fn is_empty(&self) -> bool {
        self.scalar_head.is_none() && self.queue.is_empty()
    }

    fn take_scalar_head(&mut self, shard: usize) -> Option<ScalarMessage> {
        let scalar = self.scalar_head?;
        let mut quota = runtime_telemetry().mailbox_usage.lock(shard);
        self.scalar_head = None;
        self.bytes -= scalar.len();
        quota.release(1, scalar.len());
        Some(scalar)
    }

    fn remove_first(
        &mut self,
        shard: usize,
        predicate: impl Fn(&Message) -> bool,
    ) -> Option<Message> {
        if let Some(scalar) = self.scalar_head {
            let message = scalar.into_message();
            if predicate(&message) {
                self.take_scalar_head(shard);
                return Some(message);
            }
        }
        if predicate(self.queue.front()?) {
            return self.dequeue(shard, VecDeque::pop_front);
        }
        let index = self.queue.iter().skip(1).position(predicate)? + 1;
        self.dequeue(shard, |queue| queue.remove(index))
    }
}

impl Drop for Mailbox {
    fn drop(&mut self) {
        let state = self.state.get_mut();
        if !state.is_empty() {
            runtime_telemetry()
                .mailbox_usage
                .lock(self.quota_shard)
                .release(state.len(), state.bytes);
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
    fn a_scalar_head_participates_in_fifo_selection_and_admission() {
        let mut mailbox = Mailbox::bounded(3, 9);
        let word = u64::MAX - 1;
        assert_eq!(mailbox.try_push_scalar(ScalarMessage::Word(word)), Ok(()));
        assert_eq!(mailbox.try_push(make_msg(&[47], 17)), Ok(()));
        assert_eq!(
            mailbox.try_push_scalar(ScalarMessage::Byte(9)),
            Err(MailboxPushError::Full)
        );

        let control = mailbox.remove_first(|msg| msg.buffer.type_tag == 17);
        assert_eq!(control.unwrap().buffer.data(), &[47]);
        assert_eq!(mailbox.try_push_scalar(ScalarMessage::Byte(9)), Ok(()));
        assert_eq!(mailbox.len(), 2);
        assert_eq!(mailbox.byte_len(), 9);
        assert!(
            matches!(mailbox.receive_scalar(8), Some(ScalarDelivery::Bits(bits)) if bits == word)
        );
        let Some(ScalarDelivery::Message(byte)) = mailbox.receive_scalar(1) else {
            panic!("the second scalar was queued behind the first");
        };
        assert_eq!(byte.buffer.data(), &[9]);
        assert!(mailbox.is_empty());
        assert_eq!(mailbox.byte_len(), 0);
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
