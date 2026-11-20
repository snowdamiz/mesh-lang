use std::io::{self, Write};
use std::mem::size_of;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crossbeam_channel::{bounded, Receiver, Sender, TrySendError};
use serde::{Deserialize, Serialize};

use super::store::{Claim, Settlement};

pub(super) const MAX_BUFFER: usize = 1024;
pub(super) const MAX_READ: usize = 128;
const LOG_BUFFER: usize = 128;
const MAX_LOGGERS: usize = 16;
// PostgreSQL emits job UUIDs as 36-byte text in every claim path.
const JOB_ID_BYTES: usize = 36;
const ATTEMPT_ID_BYTES: usize = 32;
static LOGGERS: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, Serialize)]
pub(super) struct Identity {
    job_id: Box<str>,
    attempt_id: Box<str>,
    attempt_number: Option<u32>,
    scheduled_at_ms: Option<u64>,
    claim_observed_at_ms: Option<u64>,
    queue_wait_ms: Option<u64>,
}

impl Identity {
    pub(super) fn new(claim: &Claim) -> Arc<Self> {
        #[derive(Deserialize)]
        struct Timing {
            attempt: u32,
            scheduled_at_ms: u64,
        }

        let context = serde_json::from_str::<Timing>(&claim.context).ok();
        let scheduled_at_ms = context.as_ref().map(|timing| timing.scheduled_at_ms);
        let claim_observed_at_ms = claim.observed_at_ms;
        Arc::new(Self {
            job_id: claim.id.as_str().into(),
            attempt_id: format!("{:0ATTEMPT_ID_BYTES$x}", rand::random::<u128>()).into_boxed_str(),
            attempt_number: context.map(|timing| timing.attempt),
            scheduled_at_ms,
            claim_observed_at_ms,
            queue_wait_ms: claim_observed_at_ms
                .zip(scheduled_at_ms)
                .and_then(|(observed, scheduled)| observed.checked_sub(scheduled)),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum FailureKind {
    HandlerFailure,
    InvalidOutcome,
    WorkerFailure,
    Timeout,
    Shutdown,
    LeaseLost,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Operation {
    Claim,
    Renew,
    Acknowledge,
    DrainCheck,
    Recover,
    Cron,
    Prune,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum LeaseLoss {
    LocalDeadline,
    Database,
}

#[derive(Clone, Copy, Serialize)]
pub(super) struct Acknowledgement {
    pub(super) submission: u32,
    pub(super) handler_duration_ms: Option<u64>,
    pub(super) failure_kind: Option<FailureKind>,
}

#[derive(Clone, Copy, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub(super) enum Kind {
    RunnerStarted,
    RunnerStopped,
    AttemptStarted,
    SettlementCommitted {
        settlement: Settlement,
        #[serde(flatten)]
        acknowledgement: Acknowledgement,
    },
    LeaseLost {
        source: LeaseLoss,
    },
    AckStale {
        #[serde(flatten)]
        acknowledgement: Acknowledgement,
    },
    AckUncertain {
        #[serde(flatten)]
        acknowledgement: Acknowledgement,
    },
    StorageFailure {
        operation: Operation,
    },
    Maintenance {
        operation: Operation,
        count: u64,
    },
}

fn serialize_identity<S: serde::Serializer>(
    identity: &Option<Arc<Identity>>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    identity.as_deref().serialize(serializer)
}

#[derive(Clone, Serialize)]
struct Event {
    version: u8,
    runner_id: u64,
    observed_at_ms: Option<u64>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_identity"
    )]
    attempt: Option<Arc<Identity>>,
    #[serde(flatten)]
    kind: Kind,
}

// Charge each occupied slot for the event and a full identity, even when the
// identity is absent or shared. Box<str> gives the identifiers no spare capacity.
const RECORD_BYTES: usize =
    size_of::<Event>() + size_of::<Identity>() + JOB_ID_BYTES + ATTEMPT_ID_BYTES;

#[derive(Serialize)]
struct BufferBytes {
    sampled_peak_bytes: usize,
    upper_bound_bytes: usize,
}

struct Buffer {
    sender: Sender<Event>,
    capacity: usize,
    sampled_peak_slots: AtomicUsize,
}

impl Buffer {
    fn new(capacity: usize) -> (Self, Receiver<Event>) {
        let (sender, receiver) = bounded(capacity);
        let buffer = Self {
            sender,
            capacity,
            sampled_peak_slots: AtomicUsize::new(0),
        };
        (buffer, receiver)
    }

    fn publish(&self, event: Event, dropped: &AtomicU64) {
        match self.sender.try_send(event) {
            Ok(()) => {
                if self.sampled_peak_slots.load(Ordering::Relaxed) < self.capacity {
                    self.sampled_peak_slots
                        .fetch_max(self.sender.len(), Ordering::Relaxed);
                }
            }
            Err(error) => {
                if matches!(error, TrySendError::Full(_)) {
                    self.sampled_peak_slots
                        .store(self.capacity, Ordering::Relaxed);
                }
                dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    fn bytes(&self) -> BufferBytes {
        BufferBytes {
            sampled_peak_bytes: self.sampled_peak_slots.load(Ordering::Relaxed) * RECORD_BYTES,
            upper_bound_bytes: self.capacity * RECORD_BYTES,
        }
    }
}

struct Reader {
    buffer: Buffer,
    receiver: Receiver<Event>,
    dropped: AtomicU64,
}

pub(super) struct Events {
    reader: Option<Reader>,
    logger: Option<Logger>,
    closed: AtomicBool,
}

impl Events {
    pub(super) fn new(capacity: usize, log: bool) -> Result<Option<Self>, String> {
        if capacity == 0 && !log {
            return Ok(None);
        }
        let reader = (capacity != 0).then(|| {
            let (buffer, receiver) = Buffer::new(capacity);
            Reader {
                buffer,
                receiver,
                dropped: AtomicU64::new(0),
            }
        });
        Ok(Some(Self {
            reader,
            logger: if log { Some(Logger::start()?) } else { None },
            closed: AtomicBool::new(false),
        }))
    }

    pub(super) fn publish(&self, runner_id: u64, kind: Kind, attempt: Option<&Arc<Identity>>) {
        let observed_at_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|time| time.as_millis().try_into().ok());
        let event = Event {
            version: 1,
            runner_id,
            observed_at_ms,
            attempt: attempt.cloned(),
            kind,
        };
        if let Some(reader) = &self.reader {
            reader.buffer.publish(event.clone(), &reader.dropped);
        }
        if let Some(logger) = &self.logger {
            logger.buffer.publish(event, &logger.dropped);
        }
    }

    pub(super) fn close(&self) {
        self.closed.store(true, Ordering::Release);
        if let Some(logger) = &self.logger {
            let _ = logger.stop.try_send(());
        }
    }

    pub(super) fn read(&self, limit: usize) -> Result<String, String> {
        let reader = self
            .reader
            .as_ref()
            .ok_or("Jobs event reader is disabled")?;
        let events: Vec<_> = reader.receiver.try_iter().take(limit).collect();
        let closed = self.closed.load(Ordering::Acquire) && reader.receiver.is_empty();
        Ok(serde_json::json!({
            "events": events,
            "closed": closed,
            "dropped": reader.dropped.load(Ordering::Relaxed),
        })
        .to_string())
    }

    pub(super) fn dropped(&self) -> u64 {
        self.reader
            .as_ref()
            .map_or(0, |reader| reader.dropped.load(Ordering::Relaxed))
    }

    pub(super) fn logs_dropped(&self) -> u64 {
        self.logger
            .as_ref()
            .map_or(0, |logger| logger.dropped.load(Ordering::Relaxed))
    }

    pub(super) fn buffer_bytes(&self) -> serde_json::Value {
        serde_json::json!({
            "reader": self.reader.as_ref().map(|reader| reader.buffer.bytes()),
            "logger": self.logger.as_ref().map(|logger| logger.buffer.bytes()),
        })
    }
}

struct LoggerSlot;

impl LoggerSlot {
    fn acquire() -> Result<Self, String> {
        let previous = LOGGERS.fetch_add(1, Ordering::Relaxed);
        let slot = Self;
        if previous >= MAX_LOGGERS {
            return Err("Jobs event logger limit reached".into());
        }
        Ok(slot)
    }
}

impl Drop for LoggerSlot {
    fn drop(&mut self) {
        LOGGERS.fetch_sub(1, Ordering::Relaxed);
    }
}

struct Logger {
    buffer: Buffer,
    stop: Sender<()>,
    dropped: Arc<AtomicU64>,
}

impl Logger {
    fn start() -> Result<Self, String> {
        #[cfg(unix)]
        use std::os::fd::AsFd;
        #[cfg(windows)]
        use std::os::windows::io::AsHandle;

        // A blocked logger must never hold the process-wide stderr lock.
        #[cfg(unix)]
        let handle = io::stderr().as_fd().try_clone_to_owned();
        #[cfg(windows)]
        let handle = io::stderr().as_handle().try_clone_to_owned();
        let mut output = std::fs::File::from(
            handle.map_err(|error| format!("could not open Jobs event logger: {error}"))?,
        );
        Self::spawn(move |line| output.write_all(line))
    }

    fn spawn(
        mut write: impl FnMut(&[u8]) -> io::Result<()> + Send + 'static,
    ) -> Result<Self, String> {
        // A blocked write can outlive its runner, so keep its slot until the thread exits.
        let slot = LoggerSlot::acquire()?;
        let (buffer, receiver) = Buffer::new(LOG_BUFFER);
        let (stop, stopping) = bounded(1);
        let dropped = Arc::new(AtomicU64::new(0));
        let write_dropped = Arc::clone(&dropped);
        std::thread::Builder::new()
            .name("mesh-jobs-json-log".into())
            .spawn(move || {
                let _slot = slot;
                let mut failed = false;
                let mut log = |event: Event| {
                    if !failed {
                        let mut line = serde_json::to_vec(&event).expect("serializable Jobs event");
                        line.push(b'\n');
                        failed = write(&line).is_err();
                    }
                    if failed {
                        write_dropped.fetch_add(1, Ordering::Relaxed);
                    }
                };
                loop {
                    crossbeam_channel::select_biased! {
                        recv(stopping) -> _ => {
                            for event in receiver.try_iter().take(LOG_BUFFER) {
                                log(event);
                            }
                            break;
                        },
                        recv(receiver) -> event => match event {
                            Ok(event) => log(event),
                            Err(_) => break,
                        },
                    }
                }
            })
            .map_err(|error| format!("could not start Jobs event logger: {error}"))?;
        Ok(Self {
            buffer,
            stop,
            dropped,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use std::time::Duration;

    fn charged_bytes(sampled_slots: usize, capacity: usize) -> Value {
        json!({
            "sampled_peak_bytes": sampled_slots * RECORD_BYTES,
            "upper_bound_bytes": capacity * RECORD_BYTES,
        })
    }

    #[test]
    fn reader_returns_versioned_payload_free_events_and_counts_overflow() {
        let events = Events::new(2, false).unwrap().unwrap();
        let identity = Identity::new(&Claim {
            id: "9f1ae775-5c0d-407e-a322-f04d33db8608".into(),
            token: "secret-lease-token".into(),
            context: r#"{"attempt":2,"scheduled_at_ms":100,"args":["secret-credential"],
                "metadata":{"account":"secret-account"},"queue":"secret-queue"}"#
                .into(),
            timeout_ms: 500,
            observed_at_ms: Some(140),
        });
        assert!(Events::new(0, false).unwrap().is_none());
        assert_eq!(events.buffer_bytes()["reader"], charged_bytes(0, 2));
        events.publish(7, Kind::RunnerStarted, None);
        assert_eq!(events.buffer_bytes()["reader"], charged_bytes(1, 2));
        events.publish(7, Kind::AttemptStarted, Some(&identity));
        events.publish(7, Kind::RunnerStopped, None);
        events.close();

        let first: Value = serde_json::from_str(&events.read(1).unwrap()).unwrap();
        assert_eq!(first["closed"], false);
        assert_eq!(first["dropped"], 1);
        assert_eq!(first["events"][0]["event"], "runner_started");
        assert!(first["events"][0].get("attempt").is_none());
        let batch = events.read(1).unwrap();
        assert!(!batch.contains("secret-"), "{batch}");
        let last: Value = serde_json::from_str(&batch).unwrap();
        assert_eq!(last["closed"], true);
        assert_eq!(last["dropped"], 1);
        let event = &last["events"][0];
        assert_eq!(event["runner_id"], 7);
        assert_eq!(event["version"], 1);
        assert_eq!(event["event"], "attempt_started");
        assert!(event["observed_at_ms"].is_number());
        assert_eq!(
            event["attempt"],
            json!({
                "job_id": "9f1ae775-5c0d-407e-a322-f04d33db8608",
                "attempt_id": identity.attempt_id,
                "attempt_number": 2,
                "scheduled_at_ms": 100,
                "claim_observed_at_ms": 140,
                "queue_wait_ms": 40
            })
        );
        assert_eq!(identity.attempt_id.len(), 32);
        assert_eq!(
            serde_json::from_str::<Value>(&events.read(1).unwrap()).unwrap(),
            json!({"closed": true, "dropped": 1, "events": []})
        );
        assert_eq!(events.buffer_bytes()["reader"], charged_bytes(2, 2));
        assert_eq!(events.buffer_bytes()["logger"], Value::Null);
    }

    #[test]
    fn full_send_records_saturation_and_disconnected_send_does_not() {
        let dropped = AtomicU64::new(0);
        let event = Event {
            version: 1,
            runner_id: 7,
            observed_at_ms: None,
            attempt: None,
            kind: Kind::RunnerStarted,
        };
        let (full, _receiver) = Buffer::new(1);
        // Fill the slot without a successful publication sample.
        assert!(full.sender.try_send(event.clone()).is_ok());
        full.publish(event.clone(), &dropped);
        assert_eq!(
            serde_json::to_value(full.bytes()).unwrap(),
            charged_bytes(1, 1)
        );
        assert_eq!(dropped.load(Ordering::Relaxed), 1);

        let (disconnected, receiver) = Buffer::new(MAX_BUFFER);
        drop(receiver);
        disconnected.publish(event, &dropped);
        assert_eq!(
            serde_json::to_value(disconnected.bytes()).unwrap(),
            charged_bytes(0, MAX_BUFFER)
        );
        assert_eq!(dropped.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn a_blocked_then_failed_logger_cannot_block_publish_or_close() {
        let timeout = Duration::from_secs(5);
        let (writing, line) = bounded(1);
        let (release, waiting) = bounded(1);
        let logger = Logger::spawn(move |bytes| {
            writing.send(bytes.to_vec()).unwrap();
            waiting.recv_timeout(timeout).unwrap();
            Err(io::ErrorKind::BrokenPipe.into())
        })
        .unwrap();
        let mut events = Events::new(1, false).unwrap().unwrap();
        events.logger = Some(logger);
        events.publish(2, Kind::RunnerStarted, None);
        let bytes = line.recv_timeout(timeout).unwrap();
        assert_eq!(bytes.last(), Some(&b'\n'));
        let event: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(event["event"], "runner_started");

        for _ in 0..=LOG_BUFFER {
            events.publish(2, Kind::RunnerStopped, None);
        }
        assert_eq!(events.logs_dropped(), 1);
        assert_eq!(events.dropped(), LOG_BUFFER as u64 + 1);
        assert_eq!(events.buffer_bytes()["reader"], charged_bytes(1, 1));
        assert_eq!(
            events.buffer_bytes()["logger"],
            charged_bytes(LOG_BUFFER, LOG_BUFFER)
        );
        events.close();
        assert_eq!(
            serde_json::from_str::<Value>(&events.read(1).unwrap()).unwrap()["closed"],
            true
        );
        release.send(()).unwrap();
        // The test sink is dropped when the logger exits after draining.
        assert_eq!(
            line.recv_timeout(timeout),
            Err(crossbeam_channel::RecvTimeoutError::Disconnected)
        );
        assert_eq!(events.logs_dropped(), LOG_BUFFER as u64 + 2);
    }
}
