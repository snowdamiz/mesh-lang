//! Bounded runtime-native telemetry and hierarchical admission control.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

const HISTOGRAM_SAMPLES: usize = 512;
const MAX_REPORTED_WORKER_QUEUES: usize = 256;
const CAPACITY_DRIVER_OPERATION_KINDS: usize = 6;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NodeRoles(u8);

impl NodeRoles {
    pub const CONTROLLER: u8 = 1;
    pub const GATEWAY: u8 = 2;
    pub const WORKER: u8 = 4;

    pub const fn new(controller: bool, gateway: bool, worker: bool) -> Self {
        Self(
            ((controller as u8) * Self::CONTROLLER)
                | ((gateway as u8) * Self::GATEWAY)
                | ((worker as u8) * Self::WORKER),
        )
    }

    pub const fn bits(self) -> u8 {
        self.0
    }

    pub const fn contains(self, role: u8) -> bool {
        self.0 & role != 0
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NodeLifecycleState {
    Provisioning,
    Joining,
    Warming,
    #[default]
    Ready,
    Draining,
    Terminating,
    Removed,
    Failed,
}

impl NodeLifecycleState {
    pub const fn routing_eligible(self) -> bool {
        matches!(self, Self::Ready)
    }

    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Provisioning => 0,
            Self::Joining => 1,
            Self::Warming => 2,
            Self::Ready => 3,
            Self::Draining => 4,
            Self::Terminating => 5,
            Self::Removed => 6,
            Self::Failed => 7,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Provisioning => "provisioning",
            Self::Joining => "joining",
            Self::Warming => "warming",
            Self::Ready => "ready",
            Self::Draining => "draining",
            Self::Terminating => "terminating",
            Self::Removed => "removed",
            Self::Failed => "failed",
        }
    }

    pub fn from_u8(value: u8) -> Result<Self, String> {
        match value {
            0 => Ok(Self::Provisioning),
            1 => Ok(Self::Joining),
            2 => Ok(Self::Warming),
            3 => Ok(Self::Ready),
            4 => Ok(Self::Draining),
            5 => Ok(Self::Terminating),
            6 => Ok(Self::Removed),
            7 => Ok(Self::Failed),
            _ => Err(format!("invalid_node_lifecycle_state:{value}")),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PressureSnapshot {
    pub score: f64,
    pub dominant_signal: &'static str,
    pub inflight: f64,
    pub queue_wait: f64,
    pub runnable: f64,
    pub memory: f64,
}

impl PressureSnapshot {
    pub fn calculate(
        inflight: u32,
        target_inflight: u32,
        p95_queue_wait: Duration,
        target_queue_wait: Duration,
        runnable_actors: u64,
        active_workers: u16,
        memory_pressure: f64,
    ) -> Self {
        let inflight_pressure = inflight as f64 / target_inflight.max(1) as f64;
        let queue_pressure = p95_queue_wait.as_secs_f64()
            / target_queue_wait.max(Duration::from_nanos(1)).as_secs_f64();
        let runnable_pressure = runnable_actors as f64 / active_workers.max(1) as f64;
        let memory_pressure = if memory_pressure.is_finite() {
            memory_pressure.max(0.0)
        } else {
            1.0
        };
        let components = [
            ("inflight", inflight_pressure),
            ("queue_wait", queue_pressure),
            ("runnable", runnable_pressure),
            ("memory", memory_pressure),
        ];
        let (dominant_signal, score) = components
            .iter()
            .copied()
            .max_by(|left, right| left.1.total_cmp(&right.1))
            .unwrap_or(("unknown", 0.0));
        Self {
            score,
            dominant_signal,
            inflight: inflight_pressure,
            queue_wait: queue_pressure,
            runnable: runnable_pressure,
            memory: memory_pressure,
        }
    }
}

#[derive(Debug)]
struct BoundedHistogram {
    samples: VecDeque<u64>,
}

impl BoundedHistogram {
    fn new() -> Self {
        Self {
            samples: VecDeque::with_capacity(HISTOGRAM_SAMPLES),
        }
    }

    fn record_value(&mut self, value: u64) {
        if self.samples.len() == HISTOGRAM_SAMPLES {
            self.samples.pop_front();
        }
        self.samples.push_back(value);
    }

    fn record_duration(&mut self, duration: Duration) {
        self.record_value(duration.as_micros().try_into().unwrap_or(u64::MAX));
    }

    fn percentile_value(&self, percentile: f64) -> u64 {
        if self.samples.is_empty() {
            return 0;
        }
        let mut samples: Vec<u64> = self.samples.iter().copied().collect();
        samples.sort_unstable();
        let index = ((samples.len() - 1) as f64 * percentile.clamp(0.0, 1.0)).ceil() as usize;
        samples[index]
    }

    fn percentile_duration(&self, percentile: f64) -> Duration {
        Duration::from_micros(self.percentile_value(percentile))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CapacityDriverOperationKind {
    Validate,
    Observe,
    Ensure,
    BeginDrain,
    Terminate,
    GetOperation,
}

impl CapacityDriverOperationKind {
    const ALL: [Self; CAPACITY_DRIVER_OPERATION_KINDS] = [
        Self::Validate,
        Self::Observe,
        Self::Ensure,
        Self::BeginDrain,
        Self::Terminate,
        Self::GetOperation,
    ];

    const fn index(self) -> usize {
        match self {
            Self::Validate => 0,
            Self::Observe => 1,
            Self::Ensure => 2,
            Self::BeginDrain => 3,
            Self::Terminate => 4,
            Self::GetOperation => 5,
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::Validate => "validate",
            Self::Observe => "observe",
            Self::Ensure => "ensure",
            Self::BeginDrain => "begin_drain",
            Self::Terminate => "terminate",
            Self::GetOperation => "get_operation",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapacityDriverOperationTelemetry {
    pub operation: String,
    pub count: u64,
    pub errors: u64,
    pub p95_latency: Duration,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct OutboundLaneTelemetrySnapshot {
    pub class: String,
    pub queued_items: u32,
    pub queued_bytes: u64,
    pub item_capacity: u32,
    pub byte_capacity: u64,
    pub utilization: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PeerSessionTelemetrySnapshot {
    pub peer: String,
    pub healthy: bool,
    pub connected_millis: u64,
    pub circuit_state: String,
    pub lanes: Vec<OutboundLaneTelemetrySnapshot>,
}

#[derive(Debug)]
pub struct RuntimeTelemetry {
    active_workers: AtomicU32,
    configured_workers: AtomicU32,
    runnable_actors: AtomicU64,
    global_run_queue_depth: AtomicU32,
    worker_run_queue_depths: Mutex<Vec<u32>>,
    scheduler_busy_nanos: AtomicU64,
    scheduler_idle_nanos: AtomicU64,
    mailbox_messages: AtomicU64,
    mailbox_depth: Mutex<BoundedHistogram>,
    http_connections: AtomicU32,
    inflight_requests: AtomicU32,
    queued_requests: AtomicU32,
    queued_bytes: AtomicU64,
    rejected_requests: AtomicU64,
    outstanding_reservations: AtomicU32,
    queue_wait: Mutex<BoundedHistogram>,
    routing_p95_queue_wait_micros: AtomicU64,
    service_time: Mutex<BoundedHistogram>,
    end_to_end_time: Mutex<BoundedHistogram>,
    remote_dispatch_queued_items: AtomicU32,
    remote_dispatch_queued_bytes: AtomicU64,
    remote_dispatch_timeouts: AtomicU64,
    remote_dispatch_retries: AtomicU64,
    remote_dispatch_circuit_rejections: AtomicU64,
    remote_dispatch_queue_rejections: AtomicU64,
    remote_dispatch_open_circuits: AtomicU32,
    capacity_driver_operations: [AtomicU64; CAPACITY_DRIVER_OPERATION_KINDS],
    capacity_driver_errors: [AtomicU64; CAPACITY_DRIVER_OPERATION_KINDS],
    capacity_driver_latency: [Mutex<BoundedHistogram>; CAPACITY_DRIVER_OPERATION_KINDS],
}

impl RuntimeTelemetry {
    pub fn new(active_workers: u16, configured_workers: u16) -> Self {
        Self {
            active_workers: AtomicU32::new(u32::from(active_workers)),
            configured_workers: AtomicU32::new(u32::from(configured_workers)),
            runnable_actors: AtomicU64::new(0),
            global_run_queue_depth: AtomicU32::new(0),
            worker_run_queue_depths: Mutex::new(Vec::new()),
            scheduler_busy_nanos: AtomicU64::new(0),
            scheduler_idle_nanos: AtomicU64::new(0),
            mailbox_messages: AtomicU64::new(0),
            mailbox_depth: Mutex::new(BoundedHistogram::new()),
            http_connections: AtomicU32::new(0),
            inflight_requests: AtomicU32::new(0),
            queued_requests: AtomicU32::new(0),
            queued_bytes: AtomicU64::new(0),
            rejected_requests: AtomicU64::new(0),
            outstanding_reservations: AtomicU32::new(0),
            queue_wait: Mutex::new(BoundedHistogram::new()),
            routing_p95_queue_wait_micros: AtomicU64::new(0),
            service_time: Mutex::new(BoundedHistogram::new()),
            end_to_end_time: Mutex::new(BoundedHistogram::new()),
            remote_dispatch_queued_items: AtomicU32::new(0),
            remote_dispatch_queued_bytes: AtomicU64::new(0),
            remote_dispatch_timeouts: AtomicU64::new(0),
            remote_dispatch_retries: AtomicU64::new(0),
            remote_dispatch_circuit_rejections: AtomicU64::new(0),
            remote_dispatch_queue_rejections: AtomicU64::new(0),
            remote_dispatch_open_circuits: AtomicU32::new(0),
            capacity_driver_operations: std::array::from_fn(|_| AtomicU64::new(0)),
            capacity_driver_errors: std::array::from_fn(|_| AtomicU64::new(0)),
            capacity_driver_latency: std::array::from_fn(|_| Mutex::new(BoundedHistogram::new())),
        }
    }

    pub fn set_scheduler(&self, active: u16, configured: u16, runnable: u64) {
        self.active_workers
            .store(u32::from(active), Ordering::Relaxed);
        self.configured_workers
            .store(u32::from(configured), Ordering::Relaxed);
        self.runnable_actors.store(runnable, Ordering::Relaxed);
    }

    pub fn set_runnable_actors(&self, runnable: u64) {
        self.runnable_actors.store(runnable, Ordering::Relaxed);
    }

    pub fn set_scheduler_queues(&self, global_depth: usize, worker_depths: &[usize]) {
        self.global_run_queue_depth.store(
            global_depth.try_into().unwrap_or(u32::MAX),
            Ordering::Relaxed,
        );
        let mut stored = self.worker_run_queue_depths.lock();
        stored.clear();
        stored.extend(
            worker_depths
                .iter()
                .take(MAX_REPORTED_WORKER_QUEUES)
                .map(|depth| (*depth).try_into().unwrap_or(u32::MAX)),
        );
    }

    pub fn record_scheduler_cycle(&self, busy: bool, duration: Duration) {
        let nanos = duration.as_nanos().try_into().unwrap_or(u64::MAX);
        if busy {
            self.scheduler_busy_nanos
                .fetch_add(nanos, Ordering::Relaxed);
        } else {
            self.scheduler_idle_nanos
                .fetch_add(nanos, Ordering::Relaxed);
        }
    }

    pub(crate) fn record_mailbox_enqueue(&self, depth: usize) {
        self.mailbox_messages.fetch_add(1, Ordering::Relaxed);
        self.mailbox_depth
            .lock()
            .record_value(depth.try_into().unwrap_or(u64::MAX));
    }

    pub(crate) fn record_mailbox_dequeue(&self, count: usize) {
        let count = count.try_into().unwrap_or(u64::MAX);
        let _ =
            self.mailbox_messages
                .fetch_update(Ordering::AcqRel, Ordering::Relaxed, |current| {
                    Some(current.saturating_sub(count))
                });
    }

    pub fn begin_http_connection(&'static self) -> HttpConnectionPermit {
        self.http_connections.fetch_add(1, Ordering::Relaxed);
        HttpConnectionPermit {
            telemetry: self,
            accepted_at: Instant::now(),
            service_started_at: None,
        }
    }

    pub fn record_queue_wait(&self, duration: Duration) {
        self.queue_wait.lock().record_duration(duration);
    }

    pub(crate) fn refresh_routing_cache(&self) {
        let p95_queue_wait_micros = self.queue_wait.lock().percentile_value(0.95);
        self.routing_p95_queue_wait_micros
            .store(p95_queue_wait_micros, Ordering::Relaxed);
    }

    pub(crate) fn routing_snapshot(&self) -> RoutingTelemetrySnapshot {
        RoutingTelemetrySnapshot {
            active_workers: self.active_workers.load(Ordering::Relaxed) as u16,
            runnable_actors: self.runnable_actors.load(Ordering::Relaxed),
            inflight_requests: self.inflight_requests.load(Ordering::Relaxed),
            queued_requests: self.queued_requests.load(Ordering::Relaxed),
            queued_bytes: self.queued_bytes.load(Ordering::Relaxed),
            outstanding_reservations: self.outstanding_reservations.load(Ordering::Relaxed),
            p95_queue_wait: Duration::from_micros(
                self.routing_p95_queue_wait_micros.load(Ordering::Relaxed),
            ),
            remote_dispatch_queued_items: self.remote_dispatch_queued_items.load(Ordering::Relaxed),
            remote_dispatch_queued_bytes: self.remote_dispatch_queued_bytes.load(Ordering::Relaxed),
        }
    }

    pub fn record_service_time(&self, duration: Duration) {
        self.service_time.lock().record_duration(duration);
    }

    pub fn record_end_to_end_time(&self, duration: Duration) {
        self.end_to_end_time.lock().record_duration(duration);
    }

    pub fn set_remote_dispatch_queue(&self, items: u32, bytes: u64, open_circuits: u32) {
        self.remote_dispatch_queued_items
            .store(items, Ordering::Relaxed);
        self.remote_dispatch_queued_bytes
            .store(bytes, Ordering::Relaxed);
        self.remote_dispatch_open_circuits
            .store(open_circuits, Ordering::Relaxed);
    }

    pub fn record_remote_dispatch_timeout(&self) {
        self.remote_dispatch_timeouts
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_remote_dispatch_retry(&self) {
        self.remote_dispatch_retries.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_remote_dispatch_circuit_rejection(&self) {
        self.remote_dispatch_circuit_rejections
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_remote_dispatch_queue_rejection(&self) {
        self.remote_dispatch_queue_rejections
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_capacity_driver_operation(
        &self,
        operation: CapacityDriverOperationKind,
        duration: Duration,
        failed: bool,
    ) {
        let index = operation.index();
        self.capacity_driver_operations[index].fetch_add(1, Ordering::Relaxed);
        if failed {
            self.capacity_driver_errors[index].fetch_add(1, Ordering::Relaxed);
        }
        self.capacity_driver_latency[index]
            .lock()
            .record_duration(duration);
    }

    pub fn snapshot(&self) -> LocalTelemetrySnapshot {
        let mailbox_depth = self.mailbox_depth.lock();
        LocalTelemetrySnapshot {
            active_workers: self.active_workers.load(Ordering::Relaxed) as u16,
            configured_workers: self.configured_workers.load(Ordering::Relaxed) as u16,
            runnable_actors: self.runnable_actors.load(Ordering::Relaxed),
            global_run_queue_depth: self.global_run_queue_depth.load(Ordering::Relaxed),
            worker_run_queue_depths: self.worker_run_queue_depths.lock().clone(),
            scheduler_busy_time: Duration::from_nanos(
                self.scheduler_busy_nanos.load(Ordering::Relaxed),
            ),
            scheduler_idle_time: Duration::from_nanos(
                self.scheduler_idle_nanos.load(Ordering::Relaxed),
            ),
            mailbox_messages: self.mailbox_messages.load(Ordering::Relaxed),
            mailbox_depth_p50: mailbox_depth.percentile_value(0.50),
            mailbox_depth_p95: mailbox_depth.percentile_value(0.95),
            mailbox_depth_max: mailbox_depth.percentile_value(1.0),
            http_connections: self.http_connections.load(Ordering::Relaxed),
            inflight_requests: self.inflight_requests.load(Ordering::Relaxed),
            queued_requests: self.queued_requests.load(Ordering::Relaxed),
            queued_bytes: self.queued_bytes.load(Ordering::Relaxed),
            rejected_requests: self.rejected_requests.load(Ordering::Relaxed),
            outstanding_reservations: self.outstanding_reservations.load(Ordering::Relaxed),
            p95_queue_wait: self.queue_wait.lock().percentile_duration(0.95),
            p95_service_time: self.service_time.lock().percentile_duration(0.95),
            p95_end_to_end_time: self.end_to_end_time.lock().percentile_duration(0.95),
            remote_dispatch_queued_items: self.remote_dispatch_queued_items.load(Ordering::Relaxed),
            remote_dispatch_queued_bytes: self.remote_dispatch_queued_bytes.load(Ordering::Relaxed),
            remote_dispatch_timeouts: self.remote_dispatch_timeouts.load(Ordering::Relaxed),
            remote_dispatch_retries: self.remote_dispatch_retries.load(Ordering::Relaxed),
            remote_dispatch_circuit_rejections: self
                .remote_dispatch_circuit_rejections
                .load(Ordering::Relaxed),
            remote_dispatch_queue_rejections: self
                .remote_dispatch_queue_rejections
                .load(Ordering::Relaxed),
            remote_dispatch_open_circuits: self
                .remote_dispatch_open_circuits
                .load(Ordering::Relaxed),
            process_resident_memory_bytes: process_resident_memory_bytes(),
            cpu_available_parallelism: std::thread::available_parallelism()
                .map(|parallelism| parallelism.get().try_into().unwrap_or(u16::MAX))
                .unwrap_or(1),
            capacity_driver_operations: CapacityDriverOperationKind::ALL
                .iter()
                .map(|operation| {
                    let index = operation.index();
                    CapacityDriverOperationTelemetry {
                        operation: operation.as_str().to_string(),
                        count: self.capacity_driver_operations[index].load(Ordering::Relaxed),
                        errors: self.capacity_driver_errors[index].load(Ordering::Relaxed),
                        p95_latency: self.capacity_driver_latency[index]
                            .lock()
                            .percentile_duration(0.95),
                    }
                })
                .collect(),
        }
    }

    fn try_increment_bounded(counter: &AtomicU32, maximum: u32) -> bool {
        counter
            .fetch_update(Ordering::AcqRel, Ordering::Relaxed, |current| {
                (current < maximum).then_some(current + 1)
            })
            .is_ok()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalTelemetrySnapshot {
    pub active_workers: u16,
    pub configured_workers: u16,
    pub runnable_actors: u64,
    pub global_run_queue_depth: u32,
    pub worker_run_queue_depths: Vec<u32>,
    pub scheduler_busy_time: Duration,
    pub scheduler_idle_time: Duration,
    pub mailbox_messages: u64,
    pub mailbox_depth_p50: u64,
    pub mailbox_depth_p95: u64,
    pub mailbox_depth_max: u64,
    pub http_connections: u32,
    pub inflight_requests: u32,
    pub queued_requests: u32,
    pub queued_bytes: u64,
    pub rejected_requests: u64,
    pub outstanding_reservations: u32,
    pub p95_queue_wait: Duration,
    pub p95_service_time: Duration,
    pub p95_end_to_end_time: Duration,
    pub remote_dispatch_queued_items: u32,
    pub remote_dispatch_queued_bytes: u64,
    pub remote_dispatch_timeouts: u64,
    pub remote_dispatch_retries: u64,
    pub remote_dispatch_circuit_rejections: u64,
    pub remote_dispatch_queue_rejections: u64,
    pub remote_dispatch_open_circuits: u32,
    pub process_resident_memory_bytes: Option<u64>,
    pub cpu_available_parallelism: u16,
    pub capacity_driver_operations: Vec<CapacityDriverOperationTelemetry>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct RoutingTelemetrySnapshot {
    pub active_workers: u16,
    pub runnable_actors: u64,
    pub inflight_requests: u32,
    pub queued_requests: u32,
    pub queued_bytes: u64,
    pub outstanding_reservations: u32,
    pub p95_queue_wait: Duration,
    pub remote_dispatch_queued_items: u32,
    pub remote_dispatch_queued_bytes: u64,
}

#[derive(Debug)]
pub struct HttpConnectionPermit {
    telemetry: &'static RuntimeTelemetry,
    accepted_at: Instant,
    service_started_at: Option<Instant>,
}

impl HttpConnectionPermit {
    pub fn begin_service(&mut self) {
        self.service_started_at.get_or_insert_with(Instant::now);
    }
}

impl Drop for HttpConnectionPermit {
    fn drop(&mut self) {
        self.telemetry
            .http_connections
            .fetch_sub(1, Ordering::AcqRel);
        if let Some(service_started_at) = self.service_started_at {
            self.telemetry
                .record_service_time(service_started_at.elapsed());
        }
        self.telemetry
            .record_end_to_end_time(self.accepted_at.elapsed());
    }
}

#[cfg(target_os = "linux")]
fn process_resident_memory_bytes() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let kibibytes = status
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))?
        .split_whitespace()
        .next()?
        .parse::<u64>()
        .ok()?;
    kibibytes.checked_mul(1024)
}

#[cfg(not(target_os = "linux"))]
fn process_resident_memory_bytes() -> Option<u64> {
    None
}

static RUNTIME_TELEMETRY: OnceLock<RuntimeTelemetry> = OnceLock::new();

pub fn runtime_telemetry() -> &'static RuntimeTelemetry {
    RUNTIME_TELEMETRY.get_or_init(|| RuntimeTelemetry::new(1, 1))
}

#[derive(Clone, Copy, Debug)]
pub struct AdmissionLimits {
    pub max_inflight: u32,
    pub max_queued_items: u32,
    pub max_queued_bytes: u64,
    pub max_control_inflight: u32,
}

impl Default for AdmissionLimits {
    fn default() -> Self {
        Self {
            max_inflight: 256,
            max_queued_items: 512,
            max_queued_bytes: 64 * 1024 * 1024,
            max_control_inflight: 32,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AdmissionRejection {
    InflightLimit,
    QueueItemLimit,
    QueueByteLimit,
    ControlLimit,
    Draining,
}

#[derive(Debug)]
pub struct AdmissionController {
    telemetry: &'static RuntimeTelemetry,
    limits: AdmissionLimits,
    control_inflight: AtomicU32,
    accepting: std::sync::atomic::AtomicBool,
}

impl AdmissionController {
    pub fn new(limits: AdmissionLimits) -> Self {
        Self::with_telemetry(limits, runtime_telemetry())
    }

    fn with_telemetry(limits: AdmissionLimits, telemetry: &'static RuntimeTelemetry) -> Self {
        Self {
            telemetry,
            limits,
            control_inflight: AtomicU32::new(0),
            accepting: std::sync::atomic::AtomicBool::new(true),
        }
    }

    pub fn set_draining(&self, draining: bool) {
        self.accepting.store(!draining, Ordering::Release);
    }

    pub fn reserve_application(self: &Arc<Self>) -> Result<AdmissionPermit, AdmissionRejection> {
        if !self.accepting.load(Ordering::Acquire) {
            self.telemetry
                .rejected_requests
                .fetch_add(1, Ordering::Relaxed);
            return Err(AdmissionRejection::Draining);
        }
        if RuntimeTelemetry::try_increment_bounded(
            &self.telemetry.inflight_requests,
            self.limits.max_inflight,
        ) {
            self.telemetry
                .outstanding_reservations
                .fetch_add(1, Ordering::Relaxed);
            Ok(AdmissionPermit {
                controller: Arc::clone(self),
                class: AdmissionClass::Application,
                started_at: Instant::now(),
            })
        } else {
            self.telemetry
                .rejected_requests
                .fetch_add(1, Ordering::Relaxed);
            Err(AdmissionRejection::InflightLimit)
        }
    }

    pub fn enqueue(self: &Arc<Self>, bytes: u64) -> Result<QueuePermit, AdmissionRejection> {
        if !self.accepting.load(Ordering::Acquire) {
            self.telemetry
                .rejected_requests
                .fetch_add(1, Ordering::Relaxed);
            return Err(AdmissionRejection::Draining);
        }
        if !RuntimeTelemetry::try_increment_bounded(
            &self.telemetry.queued_requests,
            self.limits.max_queued_items,
        ) {
            self.telemetry
                .rejected_requests
                .fetch_add(1, Ordering::Relaxed);
            return Err(AdmissionRejection::QueueItemLimit);
        }
        let bytes_reserved = self
            .telemetry
            .queued_bytes
            .fetch_update(Ordering::AcqRel, Ordering::Relaxed, |current| {
                current
                    .checked_add(bytes)
                    .filter(|next| *next <= self.limits.max_queued_bytes)
            })
            .is_ok();
        if !bytes_reserved {
            self.telemetry
                .queued_requests
                .fetch_sub(1, Ordering::AcqRel);
            self.telemetry
                .rejected_requests
                .fetch_add(1, Ordering::Relaxed);
            return Err(AdmissionRejection::QueueByteLimit);
        }
        Ok(QueuePermit {
            controller: Arc::clone(self),
            bytes,
            enqueued_at: Instant::now(),
        })
    }

    pub fn reserve_control(self: &Arc<Self>) -> Result<AdmissionPermit, AdmissionRejection> {
        if RuntimeTelemetry::try_increment_bounded(
            &self.control_inflight,
            self.limits.max_control_inflight,
        ) {
            Ok(AdmissionPermit {
                controller: Arc::clone(self),
                class: AdmissionClass::Control,
                started_at: Instant::now(),
            })
        } else {
            self.telemetry
                .rejected_requests
                .fetch_add(1, Ordering::Relaxed);
            Err(AdmissionRejection::ControlLimit)
        }
    }
}

static GLOBAL_ADMISSION_CONTROLLER: OnceLock<Arc<AdmissionController>> = OnceLock::new();

pub fn global_admission_controller() -> &'static Arc<AdmissionController> {
    GLOBAL_ADMISSION_CONTROLLER.get_or_init(|| {
        let configured = super::autonomous::embedded_autonomous_config()
            .map(|config| config.routing.clone())
            .unwrap_or_default();
        let parse_u32 = |name: &str, default| {
            std::env::var(name)
                .ok()
                .and_then(|value| value.parse().ok())
                .filter(|value| *value > 0)
                .unwrap_or(default)
        };
        let parse_u64 = |name: &str, default| {
            std::env::var(name)
                .ok()
                .and_then(|value| value.parse().ok())
                .filter(|value| *value > 0)
                .unwrap_or(default)
        };
        Arc::new(AdmissionController::new(AdmissionLimits {
            max_inflight: parse_u32("MESH_MAX_INFLIGHT_PER_NODE", configured.max_inflight),
            max_queued_items: parse_u32("MESH_MAX_QUEUED_PER_NODE", configured.max_queued_items),
            max_queued_bytes: parse_u64(
                "MESH_MAX_QUEUED_BYTES_PER_NODE",
                configured.max_queued_bytes,
            ),
            max_control_inflight: parse_u32("MESH_MAX_CONTROL_INFLIGHT", 32),
        }))
    })
}

#[derive(Clone, Copy, Debug)]
enum AdmissionClass {
    Application,
    Control,
}

#[derive(Debug)]
pub struct AdmissionPermit {
    controller: Arc<AdmissionController>,
    class: AdmissionClass,
    started_at: Instant,
}

impl Drop for AdmissionPermit {
    fn drop(&mut self) {
        match self.class {
            AdmissionClass::Application => {
                self.controller
                    .telemetry
                    .inflight_requests
                    .fetch_sub(1, Ordering::AcqRel);
                self.controller
                    .telemetry
                    .outstanding_reservations
                    .fetch_sub(1, Ordering::AcqRel);
                self.controller
                    .telemetry
                    .record_service_time(self.started_at.elapsed());
            }
            AdmissionClass::Control => {
                self.controller
                    .control_inflight
                    .fetch_sub(1, Ordering::AcqRel);
            }
        }
    }
}

#[derive(Debug)]
pub struct QueuePermit {
    controller: Arc<AdmissionController>,
    bytes: u64,
    enqueued_at: Instant,
}

impl QueuePermit {
    pub fn begin(self) {
        let controller = Arc::clone(&self.controller);
        let wait = self.enqueued_at.elapsed();
        drop(self);
        controller.telemetry.record_queue_wait(wait);
    }

    pub fn promote(self) -> Result<AdmissionPermit, AdmissionRejection> {
        let controller = Arc::clone(&self.controller);
        let wait = self.enqueued_at.elapsed();
        drop(self);
        let permit = controller.reserve_application()?;
        controller.telemetry.record_queue_wait(wait);
        Ok(permit)
    }
}

impl Drop for QueuePermit {
    fn drop(&mut self) {
        self.controller
            .telemetry
            .queued_requests
            .fetch_sub(1, Ordering::AcqRel);
        self.controller
            .telemetry
            .queued_bytes
            .fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits() -> AdmissionLimits {
        AdmissionLimits {
            max_inflight: 1,
            max_queued_items: 1,
            max_queued_bytes: 8,
            max_control_inflight: 1,
        }
    }

    #[test]
    fn pressure_uses_dominant_queue_wait_signal() {
        let pressure = PressureSnapshot::calculate(
            1,
            10,
            Duration::from_millis(100),
            Duration::from_millis(25),
            1,
            2,
            0.2,
        );

        assert_eq!(pressure.dominant_signal, "queue_wait");
    }

    #[test]
    fn admission_rejects_above_hard_inflight_limit() {
        let controller = Arc::new(AdmissionController::new(limits()));
        let _first = controller.reserve_application().expect("first reservation");

        assert_eq!(
            controller.reserve_application().unwrap_err(),
            AdmissionRejection::InflightLimit
        );
    }

    #[test]
    fn control_budget_remains_available_when_application_is_full() {
        let controller = Arc::new(AdmissionController::new(limits()));
        let _application = controller
            .reserve_application()
            .expect("application permit");

        assert!(controller.reserve_control().is_ok());
    }

    #[test]
    fn draining_rejects_new_application_but_not_control_work() {
        let controller = Arc::new(AdmissionController::new(limits()));
        controller.set_draining(true);

        assert_eq!(
            controller.reserve_application().unwrap_err(),
            AdmissionRejection::Draining
        );
        assert!(controller.reserve_control().is_ok());
    }

    #[test]
    fn scheduler_snapshot_reports_queues_and_busy_idle_time() {
        let telemetry = RuntimeTelemetry::new(2, 4);
        telemetry.set_scheduler_queues(7, &[1, 2, 3, 4]);
        telemetry.record_scheduler_cycle(true, Duration::from_millis(3));
        telemetry.record_scheduler_cycle(false, Duration::from_millis(5));

        let snapshot = telemetry.snapshot();

        assert_eq!(snapshot.global_run_queue_depth, 7);
        assert_eq!(snapshot.worker_run_queue_depths, vec![1, 2, 3, 4]);
        assert_eq!(snapshot.scheduler_busy_time, Duration::from_millis(3));
        assert_eq!(snapshot.scheduler_idle_time, Duration::from_millis(5));
    }

    #[test]
    fn mailbox_snapshot_is_bounded_and_tracks_current_messages() {
        let telemetry = RuntimeTelemetry::new(1, 1);
        for depth in 1..=(HISTOGRAM_SAMPLES + 10) {
            telemetry.record_mailbox_enqueue(depth);
        }
        telemetry.record_mailbox_dequeue(HISTOGRAM_SAMPLES + 9);

        let snapshot = telemetry.snapshot();

        assert_eq!(snapshot.mailbox_messages, 1);
        assert_eq!(snapshot.mailbox_depth_max, (HISTOGRAM_SAMPLES + 10) as u64);
        assert!(snapshot.mailbox_depth_p95 > 0);
    }

    #[test]
    fn http_connection_permit_records_connection_service_and_end_to_end_signals() {
        let telemetry = Box::leak(Box::new(RuntimeTelemetry::new(1, 1)));
        let mut permit = telemetry.begin_http_connection();
        assert_eq!(telemetry.snapshot().http_connections, 1);
        permit.begin_service();
        std::thread::sleep(Duration::from_millis(1));
        drop(permit);

        let snapshot = telemetry.snapshot();

        assert_eq!(snapshot.http_connections, 0);
        assert!(snapshot.p95_service_time > Duration::ZERO);
        assert!(snapshot.p95_end_to_end_time >= snapshot.p95_service_time);
    }

    #[test]
    fn remote_dispatch_and_capacity_driver_counters_are_exposed() {
        let telemetry = RuntimeTelemetry::new(1, 1);
        telemetry.set_remote_dispatch_queue(4, 1_024, 1);
        telemetry.record_remote_dispatch_timeout();
        telemetry.record_remote_dispatch_retry();
        telemetry.record_remote_dispatch_circuit_rejection();
        telemetry.record_remote_dispatch_queue_rejection();
        telemetry.record_capacity_driver_operation(
            CapacityDriverOperationKind::Ensure,
            Duration::from_millis(7),
            true,
        );

        let snapshot = telemetry.snapshot();
        let ensure = snapshot
            .capacity_driver_operations
            .iter()
            .find(|operation| operation.operation == "ensure")
            .expect("ensure operation telemetry");

        assert_eq!(snapshot.remote_dispatch_queued_items, 4);
        assert_eq!(snapshot.remote_dispatch_queued_bytes, 1_024);
        assert_eq!(snapshot.remote_dispatch_timeouts, 1);
        assert_eq!(snapshot.remote_dispatch_retries, 1);
        assert_eq!(snapshot.remote_dispatch_circuit_rejections, 1);
        assert_eq!(snapshot.remote_dispatch_queue_rejections, 1);
        assert_eq!(snapshot.remote_dispatch_open_circuits, 1);
        assert_eq!(ensure.count, 1);
        assert_eq!(ensure.errors, 1);
        assert_eq!(ensure.p95_latency, Duration::from_millis(7));
    }

    #[test]
    fn every_admission_rejection_updates_the_rejection_counter() {
        let telemetry = Box::leak(Box::new(RuntimeTelemetry::new(1, 1)));
        let controller = Arc::new(AdmissionController::with_telemetry(
            AdmissionLimits {
                max_queued_items: 2,
                ..limits()
            },
            telemetry,
        ));
        let _inflight = controller.reserve_application().expect("first reservation");
        let _ = controller.reserve_application();
        let _queue = controller.enqueue(8).expect("first queue reservation");
        let _ = controller.enqueue(1);
        let _zero_queue = controller.enqueue(0).expect("zero-byte queue reservation");
        let _ = controller.enqueue(0);
        let _control = controller
            .reserve_control()
            .expect("first control reservation");
        let _ = controller.reserve_control();
        controller.set_draining(true);
        let _ = controller.enqueue(1);

        assert_eq!(telemetry.snapshot().rejected_requests, 5);
    }

    #[test]
    fn routing_snapshot_remains_available_while_full_histogram_is_contended() {
        let telemetry = Arc::new(RuntimeTelemetry::new(2, 4));
        telemetry.record_queue_wait(Duration::from_millis(17));
        telemetry.refresh_routing_cache();
        let histogram_guard = telemetry.queue_wait.lock();
        let (sender, receiver) = std::sync::mpsc::channel();
        let reader = Arc::clone(&telemetry);
        let handle = std::thread::spawn(move || {
            let _ = sender.send(reader.routing_snapshot());
        });

        let snapshot = receiver.recv_timeout(Duration::from_millis(100));
        drop(histogram_guard);
        handle.join().expect("routing snapshot reader");

        assert_eq!(
            snapshot
                .expect("routing snapshot must not wait for full telemetry locks")
                .p95_queue_wait,
            Duration::from_millis(17)
        );
    }
}
