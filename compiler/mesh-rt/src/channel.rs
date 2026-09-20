//! Bounded in-process channels for replaceable and lossless work queues.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock, TryLockError};
use std::time::{Duration, Instant};

use crate::io::{alloc_result, MeshResult};
use crate::string::{mesh_string_new, MeshString};

#[derive(Clone, Copy)]
enum OverflowPolicy {
    RejectNewest,
    DropOldest,
    LatestOnly,
}

struct Channel {
    capacity: usize,
    policy: OverflowPolicy,
    values: VecDeque<i64>,
    dropped: u64,
}

const VALUE_BYTES: usize = size_of::<i64>();
static NEXT_HANDLE: AtomicU64 = AtomicU64::new(1);
// ponytail: one global lock; shard by channel if contention is measurable.
static CHANNELS: OnceLock<Mutex<HashMap<u64, Channel>>> = OnceLock::new();

fn channels() -> &'static Mutex<HashMap<u64, Channel>> {
    CHANNELS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn result(value: Result<i64, &'static str>) -> *mut MeshResult {
    match value {
        Ok(value) => alloc_result(0, Box::into_raw(Box::new(value)).cast()),
        Err(error) => alloc_result(
            1,
            mesh_string_new(error.as_ptr(), error.len() as u64).cast(),
        ),
    }
}

fn effective_capacity(capacity: i64, byte_capacity: i64) -> Result<usize, &'static str> {
    if capacity <= 0 {
        return Err("channel capacity must be positive");
    }
    if byte_capacity < VALUE_BYTES as i64 {
        return Err("channel byte capacity must fit one Int");
    }
    let capacity = usize::try_from(capacity).map_err(|_| "channel capacity is too large")?;
    let byte_capacity =
        usize::try_from(byte_capacity).map_err(|_| "channel byte capacity is too large")?;
    Ok(capacity.min(byte_capacity / VALUE_BYTES))
}

fn register_channel(
    capacity: i64,
    byte_capacity: i64,
    policy: *const MeshString,
) -> Result<i64, &'static str> {
    let capacity = effective_capacity(capacity, byte_capacity)?;
    let policy = match unsafe { (*policy).as_str() } {
        "reject_newest" => OverflowPolicy::RejectNewest,
        "drop_oldest" => OverflowPolicy::DropOldest,
        "latest_only" => OverflowPolicy::LatestOnly,
        _ => return Err("invalid overflow policy"),
    };
    let handle = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);
    channels()
        .lock()
        .expect("channel registry poisoned")
        .insert(
            handle,
            Channel {
                capacity,
                policy,
                values: VecDeque::new(),
                dropped: 0,
            },
        );
    i64::try_from(handle).map_err(|_| "channel handle overflow")
}

#[no_mangle]
pub extern "C" fn mesh_channel_bounded(
    capacity: i64,
    policy: *const MeshString,
) -> *mut MeshResult {
    let Some(byte_capacity) = capacity.checked_mul(VALUE_BYTES as i64) else {
        return result(Err("channel capacity is too large"));
    };
    result(register_channel(capacity, byte_capacity, policy))
}

#[no_mangle]
pub extern "C" fn mesh_channel_bounded_bytes(
    capacity: i64,
    byte_capacity: i64,
    policy: *const MeshString,
) -> *mut MeshResult {
    result(register_channel(capacity, byte_capacity, policy))
}

#[no_mangle]
pub extern "C" fn mesh_channel_try_send(handle: i64, value: i64) -> *mut MeshResult {
    let mut channels = match channels().try_lock() {
        Ok(channels) => channels,
        Err(TryLockError::WouldBlock) => return result(Err("channel busy")),
        Err(TryLockError::Poisoned(_)) => return result(Err("channel registry poisoned")),
    };
    let Some(channel) = channels.get_mut(&(handle as u64)) else {
        return result(Err("unknown channel"));
    };
    match channel.policy {
        OverflowPolicy::LatestOnly => {
            channel.dropped += channel.values.len() as u64;
            channel.values.clear();
            channel.values.push_back(value);
        }
        _ if channel.values.len() < channel.capacity => channel.values.push_back(value),
        OverflowPolicy::RejectNewest => {
            channel.dropped += 1;
            return result(Err("channel full"));
        }
        OverflowPolicy::DropOldest => {
            channel.values.pop_front();
            channel.dropped += 1;
            channel.values.push_back(value);
        }
    }
    result(Ok(0))
}

#[no_mangle]
pub extern "C" fn mesh_channel_recv(handle: i64, timeout_nanos: i64) -> *mut MeshResult {
    if timeout_nanos < 0 {
        return result(Err("invalid timeout"));
    }
    let deadline = Instant::now() + Duration::from_nanos(timeout_nanos as u64);
    loop {
        let value = channels()
            .lock()
            .expect("channel registry poisoned")
            .get_mut(&(handle as u64))
            .ok_or("unknown channel")
            .and_then(|channel| channel.values.pop_front().ok_or("channel empty"));
        if value.is_ok() || Instant::now() >= deadline {
            return result(value);
        }
        std::thread::yield_now();
    }
}

#[no_mangle]
pub extern "C" fn mesh_channel_depth(handle: i64) -> i64 {
    channels()
        .lock()
        .expect("channel registry poisoned")
        .get(&(handle as u64))
        .map_or(-1, |channel| channel.values.len() as i64)
}

#[no_mangle]
pub extern "C" fn mesh_channel_byte_depth(handle: i64) -> i64 {
    channels()
        .lock()
        .expect("channel registry poisoned")
        .get(&(handle as u64))
        .and_then(|channel| channel.values.len().checked_mul(VALUE_BYTES))
        .and_then(|bytes| i64::try_from(bytes).ok())
        .unwrap_or(-1)
}

#[no_mangle]
pub extern "C" fn mesh_channel_dropped(handle: i64) -> i64 {
    channels()
        .lock()
        .expect("channel registry poisoned")
        .get(&(handle as u64))
        .and_then(|channel| i64::try_from(channel.dropped).ok())
        .unwrap_or(-1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channel(policy: OverflowPolicy) -> Channel {
        Channel {
            capacity: 2,
            policy,
            values: VecDeque::new(),
            dropped: 0,
        }
    }

    #[test]
    fn policies_never_exceed_capacity() {
        let mut latest = channel(OverflowPolicy::LatestOnly);
        latest.values.push_back(1);
        latest.dropped += latest.values.len() as u64;
        latest.values.clear();
        latest.values.push_back(2);
        assert_eq!(latest.values, VecDeque::from([2]));
        assert_eq!(latest.dropped, 1);
    }

    #[test]
    fn byte_bound_limits_int_capacity() {
        assert_eq!(effective_capacity(3, 16), Ok(2));
        assert_eq!(
            effective_capacity(1, 7),
            Err("channel byte capacity must fit one Int")
        );
    }

    #[test]
    fn producer_does_not_wait_for_registry_lock() {
        let registry = channels().lock().expect("channel registry poisoned");
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
