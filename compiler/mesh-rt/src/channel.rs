//! Bounded in-process channels for replaceable and lossless work queues.
//!
//! A channel carries values of any one type, each as one uniform slot word
//! (the way a collection holds its elements). A value that references heap
//! objects comes with its shape, as a message does: the objects are copied
//! out of the sender's heap as it is sent, since the sender's collector may
//! free them as soon as `try_send` returns, and rebuilt in the receiver's
//! heap as it is received.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

use parking_lot::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::actor::heap::MessageBuffer;
use crate::actor::process::{ProcessId, ProcessState};
use crate::actor::{self, stack, GLOBAL_SCHEDULER};
use crate::gc::mesh_gc_alloc_actor;
use crate::io::{alloc_result, err_result, MeshResult};
use crate::string::MeshString;

#[derive(Clone, Copy)]
enum OverflowPolicy {
    RejectNewest,
    DropOldest,
    LatestOnly,
}

/// A queued value: its slot word in `buffer.data`, with whatever it
/// references detached from the sender's heap.
struct Entry {
    buffer: MessageBuffer,
    /// Whether the word is a reference (the value had a shape) rather than
    /// the value's own bits.
    is_ref: bool,
    /// The word and the objects it references.
    bytes: usize,
}

/// Who waits in `recv` for a value: an actor, suspended, or a thread that is
/// not one (`main`), parked.
enum Waiter {
    Actor(ProcessId),
    Thread(std::thread::Thread),
}

impl Waiter {
    fn is(&self, other: &Waiter) -> bool {
        match (self, other) {
            (Waiter::Actor(a), Waiter::Actor(b)) => a == b,
            (Waiter::Thread(a), Waiter::Thread(b)) => a.id() == b.id(),
            _ => false,
        }
    }

    fn wake(self) {
        match self {
            Waiter::Actor(pid) => {
                // An actor waits only once the scheduler runs.
                let scheduler = actor::global_scheduler();
                if let Some(process) = scheduler.get_process(pid) {
                    scheduler.wake_if_waiting(pid, process.lock());
                }
            }
            Waiter::Thread(thread) => thread.unpark(),
        }
    }
}

struct Channel {
    capacity: usize,
    byte_capacity: usize,
    policy: OverflowPolicy,
    values: VecDeque<Entry>,
    bytes: usize,
    dropped: u64,
    /// Receivers waiting in `recv`, woken by the next value.
    waiters: Vec<Waiter>,
}

impl Channel {
    fn new(capacity: usize, byte_capacity: usize, policy: OverflowPolicy) -> Self {
        Channel {
            capacity,
            byte_capacity,
            policy,
            values: VecDeque::new(),
            bytes: 0,
            dropped: 0,
            waiters: Vec::new(),
        }
    }

    fn fits(&self, bytes: usize) -> bool {
        self.values.len() < self.capacity && self.bytes + bytes <= self.byte_capacity
    }

    /// Queue `entry` as the overflow policy says, or say why not.
    fn push(&mut self, entry: Entry) -> Result<(), &'static str> {
        if entry.bytes > self.byte_capacity {
            self.dropped += 1;
            return Err("value exceeds the channel byte capacity");
        }
        match self.policy {
            OverflowPolicy::LatestOnly => {
                self.dropped += self.values.len() as u64;
                self.values.clear();
                self.bytes = 0;
            }
            OverflowPolicy::RejectNewest if !self.fits(entry.bytes) => {
                self.dropped += 1;
                return Err("channel full");
            }
            OverflowPolicy::RejectNewest => {}
            OverflowPolicy::DropOldest => {
                while !self.fits(entry.bytes) {
                    self.pop();
                    self.dropped += 1;
                }
            }
        }
        self.bytes += entry.bytes;
        self.values.push_back(entry);
        Ok(())
    }

    fn pop(&mut self) -> Option<Entry> {
        let entry = self.values.pop_front()?;
        self.bytes -= entry.bytes;
        Some(entry)
    }
}

const SLOT_BYTES: usize = size_of::<i64>();
static NEXT_HANDLE: AtomicU64 = AtomicU64::new(1);
// ponytail: one global lock; shard by channel if contention is measurable.
static CHANNELS: OnceLock<Mutex<HashMap<u64, Channel>>> = OnceLock::new();

fn channels() -> &'static Mutex<HashMap<u64, Channel>> {
    CHANNELS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Tries at the registry lock before a producer gives up on it: a few
/// microseconds, far longer than anyone holds it (to take or put one value).
const PRODUCER_SPINS: u32 = 1_000;

/// The registry for a producer, which never waits: `"channel busy"` when it
/// stays taken.
fn producer_registry() -> Result<MutexGuard<'static, HashMap<u64, Channel>>, &'static str> {
    for _ in 0..PRODUCER_SPINS {
        if let Some(channels) = channels().try_lock() {
            return Ok(channels);
        }
        std::hint::spin_loop();
    }
    Err("channel busy")
}

/// `Ok(value)` for a scalar: a `Result` payload is a pointer, which pattern
/// lowering loads the concrete `T` through.
fn ok_scalar(value: i64) -> *mut MeshResult {
    let payload = mesh_gc_alloc_actor(SLOT_BYTES as u64, 8) as *mut i64;
    unsafe { payload.write(value) };
    alloc_result(0, payload.cast())
}

fn created(value: Result<i64, &'static str>) -> *mut MeshResult {
    value.map_or_else(err_result, ok_scalar)
}

fn register_channel(
    capacity: i64,
    byte_capacity: Option<i64>,
    policy: *const MeshString,
) -> Result<i64, &'static str> {
    if capacity <= 0 {
        return Err("channel capacity must be positive");
    }
    let capacity = usize::try_from(capacity).map_err(|_| "channel capacity is too large")?;
    let byte_capacity = match byte_capacity {
        None => usize::MAX,
        Some(bytes) if bytes < SLOT_BYTES as i64 => {
            return Err("channel byte capacity must fit one Int")
        }
        Some(bytes) => usize::try_from(bytes).map_err(|_| "channel byte capacity is too large")?,
    };
    let policy = match unsafe { (*policy).as_str() } {
        "reject_newest" => OverflowPolicy::RejectNewest,
        "drop_oldest" => OverflowPolicy::DropOldest,
        "latest_only" => OverflowPolicy::LatestOnly,
        _ => return Err("invalid overflow policy"),
    };
    let handle = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);
    channels()
        .lock()
        .insert(handle, Channel::new(capacity, byte_capacity, policy));
    i64::try_from(handle).map_err(|_| "channel handle overflow")
}

#[no_mangle]
pub extern "C" fn mesh_channel_bounded(
    capacity: i64,
    policy: *const MeshString,
) -> *mut MeshResult {
    created(register_channel(capacity, None, policy))
}

#[no_mangle]
pub extern "C" fn mesh_channel_bounded_bytes(
    capacity: i64,
    byte_capacity: i64,
    policy: *const MeshString,
) -> *mut MeshResult {
    created(register_channel(capacity, Some(byte_capacity), policy))
}

/// Send a value that is its slot's own bits.
#[no_mangle]
pub extern "C" fn mesh_channel_try_send(handle: i64, value: i64) -> *mut MeshResult {
    mesh_channel_try_send_shaped(handle, value, std::ptr::null())
}

/// Send a slot word that references heap objects `shape` describes (see
/// `actor::msg_shape`); a null `shape` means the word is the value itself.
/// A producer never waits: not for space, and not for the registry lock.
#[no_mangle]
pub extern "C" fn mesh_channel_try_send_shaped(
    handle: i64,
    value: i64,
    shape: *const u32,
) -> *mut MeshResult {
    let mut buffer = MessageBuffer::new(value.to_le_bytes().to_vec(), 0);
    if let Some(scheduler) = GLOBAL_SCHEDULER.get() {
        actor::detach_from_sender(scheduler, &mut buffer, 0, shape);
    }
    let bytes = SLOT_BYTES
        + buffer
            .captured
            .objects
            .iter()
            .map(|object| object.bytes.len())
            .sum::<usize>();
    let entry = Entry {
        buffer,
        is_ref: !shape.is_null(),
        bytes,
    };
    let waiters = {
        let mut channels = match producer_registry() {
            Ok(channels) => channels,
            Err(error) => return err_result(error),
        };
        let Some(channel) = channels.get_mut(&(handle as u64)) else {
            return err_result("unknown channel");
        };
        if let Err(error) = channel.push(entry) {
            return err_result(error);
        }
        std::mem::take(&mut channel.waiters)
    };
    waiters.into_iter().for_each(Waiter::wake);
    ok_scalar(0)
}

/// Dequeue a value, waiting for one until `timeout_nanos` have passed. The
/// wait holds nothing: an actor is suspended, so its worker runs other
/// actors meanwhile, and another thread (`main`) is parked; the next
/// `try_send` wakes either.
#[no_mangle]
pub extern "C-unwind" fn mesh_channel_recv(handle: i64, timeout_nanos: i64) -> *mut MeshResult {
    if timeout_nanos < 0 {
        return err_result("invalid timeout");
    }
    let deadline = Instant::now() + Duration::from_nanos(timeout_nanos as u64);
    let in_actor = stack::CURRENT_YIELDER.with(|c| c.yielder.get().is_some());
    let actor = stack::get_current_pid()
        .filter(|_| in_actor)
        .zip(GLOBAL_SCHEDULER.get());
    let me = match actor {
        Some((pid, _)) => Waiter::Actor(pid),
        None => Waiter::Thread(std::thread::current()),
    };
    let set_state = |state: ProcessState| {
        if let Some((pid, scheduler)) = actor {
            if let Some(process) = scheduler.get_process(pid) {
                process.lock().set_live_state(state);
            }
        }
    };
    if let Some((pid, _)) = actor {
        actor::wake_at(pid, deadline);
    }
    loop {
        // Waiting before looking: a value sent from here on finds the actor
        // Waiting (or already registered) and wakes it.
        set_state(ProcessState::Waiting);
        {
            let mut channels = channels().lock();
            let Some(channel) = channels.get_mut(&(handle as u64)) else {
                drop(channels);
                set_state(ProcessState::Ready);
                return err_result("unknown channel");
            };
            let popped = channel.pop();
            if popped.is_some() || Instant::now() >= deadline {
                channel.waiters.retain(|waiter| !waiter.is(&me));
                drop(channels);
                set_state(ProcessState::Ready);
                return popped.map_or_else(|| err_result("channel empty"), received);
            }
            if !channel.waiters.iter().any(|waiter| waiter.is(&me)) {
                channel.waiters.push(match &me {
                    Waiter::Actor(pid) => Waiter::Actor(*pid),
                    Waiter::Thread(thread) => Waiter::Thread(thread.clone()),
                });
            }
        }
        match actor {
            Some(_) => stack::yield_current(),
            None => std::thread::park_timeout(deadline.saturating_duration_since(Instant::now())),
        }
    }
}

/// `Ok(value)` for a dequeued entry, its objects rebuilt in the current
/// process's heap.
fn received(mut entry: Entry) -> *mut MeshResult {
    if let Some(receiver) = actor::current_process() {
        entry.buffer.addressed_to(&receiver);
        let mut process = receiver.lock();
        unsafe {
            entry
                .buffer
                .captured
                .materialize(&mut process.heap, entry.buffer.data.as_mut_ptr());
        }
        process.heap_borrows.append(&mut entry.buffer.borrows);
    }
    let word = i64::from_le_bytes(
        entry.buffer.data[..SLOT_BYTES]
            .try_into()
            .expect("a channel entry holds one slot"),
    );
    if entry.is_ref {
        alloc_result(0, word as usize as *mut u8)
    } else {
        ok_scalar(word)
    }
}

/// `f` of channel `handle`, or `-1` for an unknown handle.
fn inspect(handle: i64, f: impl FnOnce(&Channel) -> u64) -> i64 {
    channels()
        .lock()
        .get(&(handle as u64))
        .and_then(|channel| i64::try_from(f(channel)).ok())
        .unwrap_or(-1)
}

#[no_mangle]
pub extern "C" fn mesh_channel_depth(handle: i64) -> i64 {
    inspect(handle, |channel| channel.values.len() as u64)
}

#[no_mangle]
pub extern "C" fn mesh_channel_byte_depth(handle: i64) -> i64 {
    inspect(handle, |channel| channel.bytes as u64)
}

#[no_mangle]
pub extern "C" fn mesh_channel_dropped(handle: i64) -> i64 {
    inspect(handle, |channel| channel.dropped)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(value: i64, bytes: usize) -> Entry {
        Entry {
            buffer: MessageBuffer::new(value.to_le_bytes().to_vec(), 0),
            is_ref: false,
            bytes,
        }
    }

    fn values(channel: &Channel) -> Vec<i64> {
        channel
            .values
            .iter()
            .map(|entry| i64::from_le_bytes(entry.buffer.data[..8].try_into().unwrap()))
            .collect()
    }

    #[test]
    fn policies_never_exceed_capacity() {
        let mut latest = Channel::new(2, usize::MAX, OverflowPolicy::LatestOnly);
        for value in 1..=3 {
            assert_eq!(latest.push(entry(value, 8)), Ok(()));
        }
        assert_eq!(values(&latest), [3]);
        assert_eq!(latest.dropped, 2);

        let mut oldest = Channel::new(2, usize::MAX, OverflowPolicy::DropOldest);
        for value in 1..=3 {
            assert_eq!(oldest.push(entry(value, 8)), Ok(()));
        }
        assert_eq!(values(&oldest), [2, 3]);
        assert_eq!(oldest.dropped, 1);

        let mut newest = Channel::new(2, usize::MAX, OverflowPolicy::RejectNewest);
        assert_eq!(newest.push(entry(1, 8)), Ok(()));
        assert_eq!(newest.push(entry(2, 8)), Ok(()));
        assert_eq!(newest.push(entry(3, 8)), Err("channel full"));
        assert_eq!(values(&newest), [1, 2]);
        assert_eq!(newest.dropped, 1);
    }

    #[test]
    fn byte_bound_counts_what_values_reference() {
        let mut channel = Channel::new(10, 40, OverflowPolicy::DropOldest);
        assert_eq!(channel.push(entry(1, 24)), Ok(()));
        assert_eq!(channel.push(entry(2, 8)), Ok(()));
        // 24 + 8 + 16 > 40: the oldest goes.
        assert_eq!(channel.push(entry(3, 16)), Ok(()));
        assert_eq!(values(&channel), [2, 3]);
        assert_eq!(channel.bytes, 24);
        assert_eq!(
            channel.push(entry(4, 41)),
            Err("value exceeds the channel byte capacity")
        );
        assert_eq!(channel.dropped, 2);
        assert_eq!(channel.pop().map(|entry| entry.bytes), Some(8));
        assert_eq!(channel.bytes, 16);
    }

    #[test]
    fn byte_capacity_must_fit_one_slot() {
        let policy = crate::string::mesh_string_new(b"drop_oldest".as_ptr(), 11);
        assert_eq!(
            register_channel(1, Some(7), policy),
            Err("channel byte capacity must fit one Int")
        );
        assert_eq!(
            register_channel(0, None, policy),
            Err("channel capacity must be positive")
        );
    }

    /// Held by a test that holds the registry lock, and by those whose sends
    /// it would make busy.
    static REGISTRY_TESTS: Mutex<()> = Mutex::new(());

    fn policy(name: &str) -> *const MeshString {
        crate::string::mesh_str(name)
    }

    /// A channel's value, or the error it gave.
    fn outcome(result: *mut MeshResult) -> Result<i64, String> {
        let result = unsafe { &*result };
        if result.tag == 0 {
            Ok(unsafe { *(result.value as *const i64) })
        } else {
            Err(unsafe { (*(result.value as *const MeshString)).as_str() }.to_string())
        }
    }

    fn new_channel(capacity: i64) -> i64 {
        crate::gc::mesh_rt_init();
        outcome(mesh_channel_bounded(capacity, policy("reject_newest"))).unwrap()
    }

    #[test]
    fn unknown_channels_policies_and_timeouts_are_errors() {
        let _serial = REGISTRY_TESTS.lock();
        crate::gc::mesh_rt_init();
        let unknown = i64::MAX;
        let error = |message: &str| Err(message.to_string());
        assert_eq!(
            outcome(mesh_channel_bounded(1, policy("bogus"))),
            error("invalid overflow policy")
        );
        assert_eq!(
            outcome(mesh_channel_try_send(unknown, 1)),
            error("unknown channel")
        );
        assert_eq!(
            outcome(mesh_channel_recv(unknown, 0)),
            error("unknown channel")
        );
        assert_eq!(
            outcome(mesh_channel_recv(unknown, -1)),
            error("invalid timeout")
        );
    }

    /// A thread (`main`) and an actor that wait on one channel are each woken
    /// by a value, and each leaves the waiters when it has one.
    #[test]
    fn a_thread_and_an_actor_wait_on_one_channel() {
        let _serial = REGISTRY_TESTS.lock();
        let channel = new_channel(4);
        let waiters = || channels().lock()[&(channel as u64)].waiters.len();
        let actor = std::thread::spawn(move || {
            crate::actor::in_actor(move || outcome(mesh_channel_recv(channel, 10_000_000_000)))
        });
        let thread =
            std::thread::spawn(move || outcome(mesh_channel_recv(channel, 10_000_000_000)));
        while waiters() < 2 {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(outcome(mesh_channel_try_send(channel, 1)), Ok(0));
        // Both were woken; one takes the value, the other waits again.
        while waiters() != 1 {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(outcome(mesh_channel_try_send(channel, 2)), Ok(0));
        let mut values = [actor.join().unwrap(), thread.join().unwrap()];
        values.sort();
        assert_eq!(values, [Ok(1), Ok(2)]);
        assert_eq!(waiters(), 0);
    }

    /// Two threads that wait on one channel each register, and each gets a
    /// value.
    #[test]
    fn two_threads_wait_on_one_channel() {
        let _serial = REGISTRY_TESTS.lock();
        let channel = new_channel(4);
        let waiters = || channels().lock()[&(channel as u64)].waiters.len();
        let receive = move || outcome(mesh_channel_recv(channel, 10_000_000_000));
        let threads = [std::thread::spawn(receive), std::thread::spawn(receive)];
        while waiters() < 2 {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(outcome(mesh_channel_try_send(channel, 1)), Ok(0));
        assert_eq!(outcome(mesh_channel_try_send(channel, 2)), Ok(0));
        let mut values = threads.map(|thread| thread.join().unwrap());
        values.sort();
        assert_eq!(values, [Ok(1), Ok(2)]);
    }

    /// An actor woken by something else (a message) while it waits on a
    /// channel waits on, still registered once.
    #[test]
    fn an_actor_woken_by_a_message_waits_on() {
        let _serial = REGISTRY_TESTS.lock();
        let channel = new_channel(1);
        let (pid_sender, pid) = std::sync::mpsc::channel();
        let receiver = std::thread::spawn(move || {
            crate::actor::in_actor(move || {
                let (me, _) = crate::actor::running_process();
                pid_sender.send(me).unwrap();
                outcome(mesh_channel_recv(channel, 10_000_000_000))
            })
        });
        let me = pid.recv().unwrap();
        let waiting = || channels().lock()[&(channel as u64)].waiters.len() == 1;
        while !waiting() {
            std::thread::sleep(Duration::from_millis(1));
        }
        crate::actor::local_send(me.as_u64(), std::ptr::null(), 0);
        std::thread::sleep(Duration::from_millis(20));
        assert!(waiting(), "registered once, still waiting");
        assert_eq!(outcome(mesh_channel_try_send(channel, 5)), Ok(0));
        assert_eq!(receiver.join().unwrap(), Ok(5));
    }

    #[test]
    fn producer_does_not_wait_for_registry_lock() {
        let _serial = REGISTRY_TESTS.lock();
        let registry = channels().lock();
        let (sender, receiver) = std::sync::mpsc::channel();
        let producer = std::thread::spawn(move || {
            let response = mesh_channel_try_send(1, 1);
            sender
                .send(unsafe { (*response).tag })
                .expect("test receiver dropped");
        });
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(1)),
            Ok(1),
            "producer blocked on the channel registry"
        );
        drop(registry);
        producer.join().expect("producer panicked");
    }
}
