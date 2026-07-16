//! Load-report registry and capacity-weighted power-of-two routing.

use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, RwLock};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

use super::telemetry::{runtime_telemetry, NodeLifecycleState, NodeRoles, PressureSnapshot};

const MAX_NODE_ID_BYTES: usize = 512;
const MAX_BOOT_ID_BYTES: usize = 128;
const MAX_FAILURE_DOMAIN_BYTES: usize = 256;
const MAX_HANDLER_COUNT: usize = 1_024;
const MAX_HANDLER_BYTES: usize = 512;
const LOAD_REPORT_VERSION: u16 = 3;

#[derive(Clone, Debug, PartialEq)]
pub struct NodeLoadReport {
    pub protocol_version: u16,
    pub node_id: String,
    pub boot_id: String,
    pub roles: NodeRoles,
    pub state: NodeLifecycleState,
    pub capacity_units: u16,
    pub active_workers: u16,
    pub runnable_actors: u64,
    pub inflight: u32,
    pub queued_items: u32,
    pub queued_bytes: u64,
    pub outstanding_reservations: u32,
    pub p95_queue_wait: Duration,
    pub memory_pressure: f64,
    /// EWMA used by capacity decisions; instantaneous pressure remains
    /// derivable from the bounded raw fields for hard overload protection.
    pub decision_pressure_ewma: f64,
    pub sequence: u64,
    pub control_term: u64,
    pub membership_generation: u64,
    pub failure_domain: String,
    pub handlers: BTreeSet<String>,
}

impl NodeLoadReport {
    pub fn validate(&self) -> Result<(), String> {
        validate_bounded_string(&self.node_id, MAX_NODE_ID_BYTES, "node_id")?;
        validate_bounded_string(&self.boot_id, MAX_BOOT_ID_BYTES, "boot_id")?;
        if self.capacity_units == 0 {
            return Err("load_report_capacity_zero".to_string());
        }
        if !self.memory_pressure.is_finite() || self.memory_pressure < 0.0 {
            return Err("load_report_memory_pressure_invalid".to_string());
        }
        if !self.decision_pressure_ewma.is_finite() || self.decision_pressure_ewma < 0.0 {
            return Err("load_report_pressure_ewma_invalid".to_string());
        }
        if self.handlers.len() > MAX_HANDLER_COUNT {
            return Err("load_report_handler_count_exceeded".to_string());
        }
        if self
            .handlers
            .iter()
            .any(|handler| handler.is_empty() || handler.len() > MAX_HANDLER_BYTES)
        {
            return Err("load_report_handler_invalid".to_string());
        }
        if self.failure_domain.len() > MAX_FAILURE_DOMAIN_BYTES {
            return Err("load_report_failure_domain_too_long".to_string());
        }
        Ok(())
    }

    pub fn pressure(&self, target_inflight: u32, target_queue_wait: Duration) -> PressureSnapshot {
        PressureSnapshot::calculate(
            self.inflight,
            target_inflight,
            self.p95_queue_wait,
            target_queue_wait,
            self.runnable_actors,
            self.active_workers,
            self.memory_pressure,
        )
    }

    pub fn encode(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        let mut output = Vec::with_capacity(256);
        output.extend_from_slice(&self.protocol_version.to_le_bytes());
        encode_string(&mut output, &self.node_id)?;
        encode_string(&mut output, &self.boot_id)?;
        output.push(self.roles.bits());
        output.push(self.state.as_u8());
        output.extend_from_slice(&self.capacity_units.to_le_bytes());
        output.extend_from_slice(&self.active_workers.to_le_bytes());
        output.extend_from_slice(&self.runnable_actors.to_le_bytes());
        output.extend_from_slice(&self.inflight.to_le_bytes());
        output.extend_from_slice(&self.queued_items.to_le_bytes());
        output.extend_from_slice(&self.queued_bytes.to_le_bytes());
        output.extend_from_slice(&self.outstanding_reservations.to_le_bytes());
        let queue_wait_micros: u64 = self
            .p95_queue_wait
            .as_micros()
            .try_into()
            .unwrap_or(u64::MAX);
        output.extend_from_slice(&queue_wait_micros.to_le_bytes());
        output.extend_from_slice(&self.memory_pressure.to_bits().to_le_bytes());
        output.extend_from_slice(&self.sequence.to_le_bytes());
        output.extend_from_slice(&self.control_term.to_le_bytes());
        output.extend_from_slice(&self.membership_generation.to_le_bytes());
        encode_string(&mut output, &self.failure_domain)?;
        let handler_count = u16::try_from(self.handlers.len())
            .map_err(|_| "load_report_handler_count_exceeded".to_string())?;
        output.extend_from_slice(&handler_count.to_le_bytes());
        for handler in &self.handlers {
            encode_string(&mut output, handler)?;
        }
        if self.protocol_version >= 3 {
            output.extend_from_slice(&self.decision_pressure_ewma.to_bits().to_le_bytes());
        }
        Ok(output)
    }

    pub fn decode(input: &[u8]) -> Result<Self, String> {
        let mut cursor = Cursor::new(input);
        let protocol_version = cursor.u16()?;
        let mut report = Self {
            protocol_version,
            node_id: cursor.string(MAX_NODE_ID_BYTES)?,
            boot_id: cursor.string(MAX_BOOT_ID_BYTES)?,
            roles: NodeRoles::from_bits(cursor.u8()?),
            state: NodeLifecycleState::from_u8(cursor.u8()?)?,
            capacity_units: cursor.u16()?,
            active_workers: cursor.u16()?,
            runnable_actors: cursor.u64()?,
            inflight: cursor.u32()?,
            queued_items: cursor.u32()?,
            queued_bytes: cursor.u64()?,
            outstanding_reservations: cursor.u32()?,
            p95_queue_wait: Duration::from_micros(cursor.u64()?),
            memory_pressure: f64::from_bits(cursor.u64()?),
            decision_pressure_ewma: 0.0,
            sequence: cursor.u64()?,
            control_term: cursor.u64()?,
            membership_generation: cursor.u64()?,
            failure_domain: cursor.string(MAX_FAILURE_DOMAIN_BYTES)?,
            handlers: {
                let count = cursor.u16()? as usize;
                if count > MAX_HANDLER_COUNT {
                    return Err("load_report_handler_count_exceeded".to_string());
                }
                let mut handlers = BTreeSet::new();
                for _ in 0..count {
                    handlers.insert(cursor.string(MAX_HANDLER_BYTES)?);
                }
                handlers
            },
        };
        report.decision_pressure_ewma = if protocol_version >= 3 {
            f64::from_bits(cursor.u64()?)
        } else {
            report.pressure(128, Duration::from_millis(25)).score
        };
        if !cursor.finished() {
            return Err("load_report_trailing_bytes".to_string());
        }
        report.validate()?;
        Ok(report)
    }
}

impl NodeRoles {
    fn from_bits(bits: u8) -> Self {
        Self::new(
            bits & Self::CONTROLLER != 0,
            bits & Self::GATEWAY != 0,
            bits & Self::WORKER != 0,
        )
    }
}

fn validate_bounded_string(value: &str, maximum: usize, label: &str) -> Result<(), String> {
    if value.is_empty() {
        Err(format!("load_report_{label}_missing"))
    } else if value.len() > maximum {
        Err(format!("load_report_{label}_too_long"))
    } else {
        Ok(())
    }
}

fn encode_string(output: &mut Vec<u8>, value: &str) -> Result<(), String> {
    let length =
        u16::try_from(value.len()).map_err(|_| "load_report_string_too_long".to_string())?;
    output.extend_from_slice(&length.to_le_bytes());
    output.extend_from_slice(value.as_bytes());
    Ok(())
}

struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], String> {
        let end = self
            .position
            .checked_add(length)
            .ok_or_else(|| "load_report_length_overflow".to_string())?;
        if end > self.bytes.len() {
            return Err("load_report_truncated".to_string());
        }
        let result = &self.bytes[self.position..end];
        self.position = end;
        Ok(result)
    }

    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }

    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }

    fn string(&mut self, maximum: usize) -> Result<String, String> {
        let length = self.u16()? as usize;
        if length > maximum {
            return Err("load_report_string_bound_exceeded".to_string());
        }
        std::str::from_utf8(self.take(length)?)
            .map(str::to_string)
            .map_err(|_| "load_report_invalid_utf8".to_string())
    }

    fn finished(&self) -> bool {
        self.position == self.bytes.len()
    }
}

#[derive(Clone, Debug)]
struct ObservedReport {
    report: NodeLoadReport,
    received_at: Instant,
}

#[derive(Debug, Default)]
pub struct LoadReportRegistry {
    reports: RwLock<HashMap<String, ObservedReport>>,
}

impl LoadReportRegistry {
    pub fn apply(&self, report: NodeLoadReport, now: Instant) -> Result<(), String> {
        report.validate()?;
        let mut reports = self.reports.write().unwrap();
        if let Some(existing) = reports.get(&report.node_id) {
            if report.control_term < existing.report.control_term {
                return Err("load_report_stale_control_term".to_string());
            }
            if report.boot_id == existing.report.boot_id
                && report.sequence <= existing.report.sequence
            {
                return Err("load_report_out_of_order".to_string());
            }
        }
        reports.insert(
            report.node_id.clone(),
            ObservedReport {
                report,
                received_at: now,
            },
        );
        Ok(())
    }

    pub fn report(&self, node_id: &str, now: Instant, ttl: Duration) -> Option<NodeLoadReport> {
        self.reports
            .read()
            .unwrap()
            .get(node_id)
            .filter(|observed| now.saturating_duration_since(observed.received_at) <= ttl)
            .map(|observed| observed.report.clone())
    }

    pub fn snapshot(&self, now: Instant, ttl: Duration) -> Vec<NodeLoadReport> {
        self.reports
            .read()
            .unwrap()
            .values()
            .filter(|observed| now.saturating_duration_since(observed.received_at) <= ttl)
            .map(|observed| observed.report.clone())
            .collect()
    }

    #[cfg(test)]
    fn clear(&self) {
        self.reports.write().unwrap().clear();
    }
}

static LOAD_REPORTS: OnceLock<LoadReportRegistry> = OnceLock::new();

pub fn load_report_registry() -> &'static LoadReportRegistry {
    LOAD_REPORTS.get_or_init(LoadReportRegistry::default)
}

static LOCAL_BOOT_ID: OnceLock<String> = OnceLock::new();
static LOCAL_REPORT_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static LOCAL_PRESSURE_EWMA: OnceLock<std::sync::Mutex<Option<f64>>> = OnceLock::new();

pub fn local_load_report(node_id: &str, handlers: BTreeSet<String>) -> NodeLoadReport {
    let runtime = runtime_telemetry();
    let telemetry = runtime.routing_snapshot();
    let roles = roles_from_env();
    let state = state_from_env(node_id);
    let mut report = NodeLoadReport {
        protocol_version: LOAD_REPORT_VERSION,
        node_id: node_id.to_string(),
        boot_id: LOCAL_BOOT_ID
            .get_or_init(|| hex(&rand::random::<[u8; 16]>()))
            .clone(),
        roles,
        state,
        capacity_units: env_u16("MESH_CAPACITY_UNITS", telemetry.active_workers.max(1)),
        active_workers: telemetry.active_workers,
        runnable_actors: telemetry.runnable_actors,
        inflight: telemetry.inflight_requests,
        queued_items: telemetry
            .queued_requests
            .saturating_add(telemetry.remote_dispatch_queued_items),
        queued_bytes: telemetry
            .queued_bytes
            .saturating_add(telemetry.remote_dispatch_queued_bytes),
        outstanding_reservations: telemetry.outstanding_reservations,
        p95_queue_wait: telemetry.p95_queue_wait,
        memory_pressure: env_f64("MESH_MEMORY_PRESSURE", 0.0),
        decision_pressure_ewma: 0.0,
        sequence: LOCAL_REPORT_SEQUENCE.fetch_add(1, Ordering::Relaxed),
        control_term: env_u64("MESH_CONTROL_TERM", 0),
        membership_generation: env_u64("MESH_MEMBERSHIP_GENERATION", 0),
        failure_domain: std::env::var("MESH_FAILURE_DOMAIN").unwrap_or_default(),
        handlers,
    };
    let instantaneous = report
        .pressure(
            env_u32(
                "MESH_ROUTING_TARGET_INFLIGHT",
                configured_routing().target_inflight,
            ),
            Duration::from_millis(env_u64(
                "MESH_ROUTING_TARGET_QUEUE_WAIT_MS",
                configured_routing().target_queue_wait_millis,
            )),
        )
        .score;
    let alpha = env_f64("MESH_PRESSURE_EWMA_ALPHA", 0.2).clamp(0.01, 1.0);
    let mut ewma = LOCAL_PRESSURE_EWMA
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .unwrap();
    let updated = ewma.map_or(instantaneous, |previous| {
        alpha * instantaneous + (1.0 - alpha) * previous
    });
    *ewma = Some(updated);
    report.decision_pressure_ewma = updated;
    report
}

/// Refresh telemetry that requires walking scheduler or peer-session state.
/// Routing itself consumes only the cached atomic snapshot produced here.
pub(crate) fn refresh_local_routing_telemetry() {
    if let Some(scheduler) = crate::actor::GLOBAL_SCHEDULER.get() {
        scheduler.refresh_telemetry();
    }
    crate::dist::node::refresh_peer_session_telemetry();
    runtime_telemetry().refresh_routing_cache();
}

fn roles_from_env() -> NodeRoles {
    let roles = std::env::var("MESH_ROLES").unwrap_or_else(|_| "gateway,worker".to_string());
    NodeRoles::new(
        roles.split(',').any(|role| role.trim() == "controller"),
        roles.split(',').any(|role| role.trim() == "gateway"),
        roles.split(',').any(|role| role.trim() == "worker"),
    )
}

fn state_from_env(_node_id: &str) -> NodeLifecycleState {
    crate::dist::readiness::local_lifecycle_state()
}

fn env_u16(name: &str, default: u16) -> u16 {
    std::env::var(name)
        .ok()
        .and_then(|raw| raw.parse().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|raw| raw.parse().ok())
        .unwrap_or(default)
}

fn env_u32(name: &str, default: u32) -> u32 {
    std::env::var(name)
        .ok()
        .and_then(|raw| raw.parse().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

fn env_f64(name: &str, default: f64) -> f64 {
    std::env::var(name)
        .ok()
        .and_then(|raw| raw.parse().ok())
        .filter(|value: &f64| value.is_finite() && *value >= 0.0)
        .unwrap_or(default)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[derive(Clone, Debug, PartialEq)]
pub struct RoutingPolicy {
    pub load_report_ttl: Duration,
    pub target_inflight: u32,
    pub target_queue_wait: Duration,
    pub max_inflight: u32,
    pub max_queued_items: u32,
    pub max_queued_bytes: u64,
}

impl Default for RoutingPolicy {
    fn default() -> Self {
        Self {
            load_report_ttl: Duration::from_secs(2),
            target_inflight: 128,
            target_queue_wait: Duration::from_millis(25),
            max_inflight: 256,
            max_queued_items: 512,
            max_queued_bytes: 64 * 1024 * 1024,
        }
    }
}

fn configured_routing() -> super::autonomous::RuntimeRoutingConfig {
    super::autonomous::embedded_autonomous_config()
        .map(|config| config.routing.clone())
        .unwrap_or_default()
}

/// Resolve the effective routing policy. Explicit process-level overrides take
/// precedence over embedded manifest values.
pub fn runtime_routing_policy() -> RoutingPolicy {
    let configured = configured_routing();
    RoutingPolicy {
        load_report_ttl: Duration::from_millis(env_u64(
            "MESH_LOAD_REPORT_TTL_MS",
            configured.load_report_ttl_millis,
        )),
        target_inflight: env_u32("MESH_ROUTING_TARGET_INFLIGHT", configured.target_inflight),
        target_queue_wait: Duration::from_millis(env_u64(
            "MESH_ROUTING_TARGET_QUEUE_WAIT_MS",
            configured.target_queue_wait_millis,
        )),
        max_inflight: env_u32("MESH_MAX_INFLIGHT_PER_NODE", configured.max_inflight),
        max_queued_items: env_u32("MESH_MAX_QUEUED_PER_NODE", configured.max_queued_items),
        max_queued_bytes: env_u64(
            "MESH_MAX_QUEUED_BYTES_PER_NODE",
            configured.max_queued_bytes,
        ),
    }
}

pub(crate) fn runtime_load_report_interval() -> Duration {
    let configured = configured_routing();
    Duration::from_millis(env_u64(
        "MESH_LOAD_REPORT_INTERVAL_MS",
        configured.load_report_interval_millis,
    ))
}

pub(crate) fn runtime_retry_budget_percent() -> u8 {
    std::env::var("MESH_RETRY_BUDGET_PERCENT")
        .ok()
        .and_then(|value| value.parse::<u8>().ok())
        .filter(|percent| *percent <= 100)
        .unwrap_or_else(|| configured_routing().retry_budget_percent)
}

pub(crate) fn runtime_adaptive_routing_enabled() -> bool {
    std::env::var("MESH_ADAPTIVE_ROUTING")
        .ok()
        .and_then(|value| match value.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "on" => Some(true),
            "0" | "false" | "off" => Some(false),
            _ => None,
        })
        .unwrap_or_else(|| {
            super::autonomous::embedded_autonomous_config()
                .is_some_and(|config| config.routing.adaptive)
        })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IneligibilityReason {
    MissingReport,
    StaleReport,
    MissingWorkerRole,
    NotReady,
    DrainIntent,
    CircuitOpen,
    HandlerUnavailable,
    InflightLimit,
    QueueItemLimit,
    QueueByteLimit,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RoutingDecision {
    pub selected_node: String,
    pub sampled_nodes: Vec<String>,
    pub effective_score: f64,
    pub dominant_signal: String,
    pub membership_generation: u64,
    pub rejections: Vec<(String, IneligibilityReason)>,
}

#[derive(Debug, Default)]
struct SelectionReservationRegistry {
    counts: Mutex<HashMap<String, u32>>,
}

impl SelectionReservationRegistry {
    fn lock(&self) -> MutexGuard<'_, HashMap<String, u32>> {
        self.counts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

static SELECTION_RESERVATIONS: OnceLock<Arc<SelectionReservationRegistry>> = OnceLock::new();

#[derive(Debug)]
pub(crate) struct RoutingReservation {
    node_id: String,
    registry: Arc<SelectionReservationRegistry>,
}

impl Drop for RoutingReservation {
    fn drop(&mut self) {
        let mut reservations = self.registry.lock();
        let Some(count) = reservations.get_mut(&self.node_id) else {
            return;
        };
        *count = count.saturating_sub(1);
        if *count == 0 {
            reservations.remove(&self.node_id);
        }
    }
}

pub fn select_owner(
    request_id: &str,
    handler: &str,
    ingress_node: &str,
    membership: &[String],
    pinned_owner: Option<&str>,
    policy: &RoutingPolicy,
    now: Instant,
) -> Result<RoutingDecision, String> {
    select_owner_with_reservations(
        request_id,
        handler,
        ingress_node,
        membership,
        pinned_owner,
        policy,
        now,
        &HashMap::new(),
    )
}

pub(crate) fn select_owner_and_reserve(
    request_id: &str,
    handler: &str,
    ingress_node: &str,
    membership: &[String],
    pinned_owner: Option<&str>,
    policy: &RoutingPolicy,
    now: Instant,
) -> Result<(RoutingDecision, RoutingReservation), String> {
    let registry = Arc::clone(
        SELECTION_RESERVATIONS.get_or_init(|| Arc::new(SelectionReservationRegistry::default())),
    );
    select_owner_and_reserve_with_registry(
        request_id,
        handler,
        ingress_node,
        membership,
        pinned_owner,
        policy,
        now,
        registry,
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "routing selection keeps its explicit policy inputs auditable"
)]
fn select_owner_and_reserve_with_registry(
    request_id: &str,
    handler: &str,
    ingress_node: &str,
    membership: &[String],
    pinned_owner: Option<&str>,
    policy: &RoutingPolicy,
    now: Instant,
    registry: Arc<SelectionReservationRegistry>,
) -> Result<(RoutingDecision, RoutingReservation), String> {
    let mut reservations = registry.lock();
    let decision = select_owner_with_reservations(
        request_id,
        handler,
        ingress_node,
        membership,
        pinned_owner,
        policy,
        now,
        &reservations,
    )?;
    let selected_node = decision.selected_node.clone();
    let count = reservations.entry(selected_node.clone()).or_default();
    *count = count.saturating_add(1);
    drop(reservations);
    Ok((
        decision,
        RoutingReservation {
            node_id: selected_node,
            registry,
        },
    ))
}

#[expect(
    clippy::too_many_arguments,
    reason = "routing selection keeps its explicit policy inputs auditable"
)]
fn select_owner_with_reservations(
    request_id: &str,
    handler: &str,
    ingress_node: &str,
    membership: &[String],
    pinned_owner: Option<&str>,
    policy: &RoutingPolicy,
    now: Instant,
    selection_reservations: &HashMap<String, u32>,
) -> Result<RoutingDecision, String> {
    if membership.is_empty() {
        return Err("routing_membership_empty".to_string());
    }
    let registry = load_report_registry();
    let mut eligible = Vec::new();
    let mut rejections = Vec::new();
    for node in membership {
        let Some(report) = registry.report(node, now, policy.load_report_ttl) else {
            let reason = if registry.reports.read().unwrap().contains_key(node) {
                IneligibilityReason::StaleReport
            } else {
                IneligibilityReason::MissingReport
            };
            rejections.push((node.clone(), reason));
            continue;
        };
        if !report.roles.contains(NodeRoles::WORKER) {
            rejections.push((node.clone(), IneligibilityReason::MissingWorkerRole));
        } else if crate::dist::operator::drain_requested(node) {
            rejections.push((node.clone(), IneligibilityReason::DrainIntent));
        } else if crate::dist::node::peer_circuit_open(node, now) {
            rejections.push((node.clone(), IneligibilityReason::CircuitOpen));
        } else if !report.state.routing_eligible() {
            rejections.push((node.clone(), IneligibilityReason::NotReady));
        } else if !report.handlers.is_empty() && !report.handlers.contains(handler) {
            rejections.push((node.clone(), IneligibilityReason::HandlerUnavailable));
        } else if report.inflight >= policy.max_inflight {
            rejections.push((node.clone(), IneligibilityReason::InflightLimit));
        } else if report.queued_items >= policy.max_queued_items {
            rejections.push((node.clone(), IneligibilityReason::QueueItemLimit));
        } else if report.queued_bytes >= policy.max_queued_bytes {
            rejections.push((node.clone(), IneligibilityReason::QueueByteLimit));
        } else {
            eligible.push(report);
        }
    }

    if let Some(owner) = pinned_owner {
        if let Some(report) = eligible.iter().find(|report| report.node_id == owner) {
            return Ok(decision_for(
                report,
                ingress_node,
                vec![owner.to_string()],
                rejections,
                policy,
                selection_reservations.get(owner).copied().unwrap_or(0),
            ));
        }
    }
    if eligible.is_empty() {
        return Err(format!(
            "routing_no_eligible_nodes:handler={handler}:{rejections:?}"
        ));
    }
    let generation = eligible
        .iter()
        .map(|report| report.membership_generation)
        .max()
        .unwrap_or(0);
    let first = sample_index(request_id, generation, 0, eligible.len());
    let second = if eligible.len() == 1 {
        first
    } else {
        let candidate = sample_index(request_id, generation, 1, eligible.len() - 1);
        if candidate >= first {
            candidate + 1
        } else {
            candidate
        }
    };
    let first_reservations = selection_reservations
        .get(&eligible[first].node_id)
        .copied()
        .unwrap_or(0);
    let second_reservations = selection_reservations
        .get(&eligible[second].node_id)
        .copied()
        .unwrap_or(0);
    let first_score = effective_score(&eligible[first], ingress_node, policy, first_reservations);
    let second_score =
        effective_score(&eligible[second], ingress_node, policy, second_reservations);
    let selected = if first_score <= second_score {
        &eligible[first]
    } else {
        &eligible[second]
    };
    Ok(decision_for(
        selected,
        ingress_node,
        vec![
            eligible[first].node_id.clone(),
            eligible[second].node_id.clone(),
        ],
        rejections,
        policy,
        if selected.node_id == eligible[first].node_id {
            first_reservations
        } else {
            second_reservations
        },
    ))
}

pub fn select_record_replicas(
    owner: &str,
    total_replicas: usize,
    candidates: &[NodeLoadReport],
) -> Result<Vec<String>, String> {
    if total_replicas == 0 {
        return Err("invalid_total_replicas".to_string());
    }
    let required = total_replicas - 1;
    let owner_domain = candidates
        .iter()
        .find(|candidate| candidate.node_id == owner)
        .map(|candidate| candidate.failure_domain.as_str())
        .unwrap_or("");
    let mut choices: Vec<&NodeLoadReport> = candidates
        .iter()
        .filter(|candidate| {
            candidate.node_id != owner
                && candidate.state.routing_eligible()
                && !crate::dist::operator::drain_requested(&candidate.node_id)
        })
        .collect();
    choices.sort_by_key(|candidate| {
        (
            candidate.failure_domain == owner_domain,
            candidate.failure_domain.clone(),
            candidate.node_id.clone(),
        )
    });
    if choices.len() < required {
        return Err(format!(
            "replica_capacity_unavailable:required={required}:available={}",
            choices.len()
        ));
    }
    Ok(choices
        .into_iter()
        .take(required)
        .map(|candidate| candidate.node_id.clone())
        .collect())
}

fn decision_for(
    report: &NodeLoadReport,
    ingress_node: &str,
    sampled_nodes: Vec<String>,
    rejections: Vec<(String, IneligibilityReason)>,
    policy: &RoutingPolicy,
    selection_reservations: u32,
) -> RoutingDecision {
    let pressure = report.pressure(policy.target_inflight, policy.target_queue_wait);
    RoutingDecision {
        selected_node: report.node_id.clone(),
        sampled_nodes,
        effective_score: effective_score(report, ingress_node, policy, selection_reservations),
        dominant_signal: pressure.dominant_signal.to_string(),
        membership_generation: report.membership_generation,
        rejections,
    }
}

fn effective_score(
    report: &NodeLoadReport,
    ingress_node: &str,
    policy: &RoutingPolicy,
    selection_reservations: u32,
) -> f64 {
    let pressure = report.pressure(policy.target_inflight, policy.target_queue_wait);
    let reservation_pressure = report
        .outstanding_reservations
        .saturating_add(selection_reservations) as f64
        / policy.max_inflight as f64;
    let locality_cost = if report.node_id == ingress_node {
        0.0
    } else {
        0.025
    };
    (report.decision_pressure_ewma.max(pressure.score * 0.25)
        + reservation_pressure
        + locality_cost)
        / f64::from(report.capacity_units.max(1))
}

fn sample_index(request_id: &str, generation: u64, salt: u8, length: usize) -> usize {
    let mut hasher = Sha256::new();
    hasher.update(request_id.as_bytes());
    hasher.update(generation.to_le_bytes());
    hasher.update([salt]);
    let digest = hasher.finalize();
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&digest[..8]);
    (u64::from_le_bytes(bytes) as usize) % length
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(node: &str, pressure: u32, state: NodeLifecycleState) -> NodeLoadReport {
        NodeLoadReport {
            protocol_version: LOAD_REPORT_VERSION,
            node_id: node.to_string(),
            boot_id: format!("boot-{node}"),
            roles: NodeRoles::new(false, false, true),
            state,
            capacity_units: 1,
            active_workers: 2,
            runnable_actors: 0,
            inflight: pressure,
            queued_items: 0,
            queued_bytes: 0,
            outstanding_reservations: 0,
            p95_queue_wait: Duration::ZERO,
            memory_pressure: 0.0,
            decision_pressure_ewma: f64::from(pressure) / 128.0,
            sequence: 1,
            control_term: 1,
            membership_generation: 1,
            failure_domain: node.to_string(),
            handlers: BTreeSet::new(),
        }
    }

    #[test]
    fn load_report_round_trip_preserves_bounded_fields() {
        let original = report("worker-a", 7, NodeLifecycleState::Ready);

        assert_eq!(
            NodeLoadReport::decode(&original.encode().expect("encode report"))
                .expect("decode report"),
            original
        );
    }

    #[test]
    fn load_report_version_two_decodes_with_derived_ewma() {
        let mut original = report("worker-a", 64, NodeLifecycleState::Ready);
        original.protocol_version = 2;
        let decoded =
            NodeLoadReport::decode(&original.encode().expect("encode version-two report"))
                .expect("decode version-two report");

        assert_eq!(decoded.protocol_version, 2);
        assert_eq!(decoded.decision_pressure_ewma, 0.5);
    }

    #[test]
    fn registry_rejects_out_of_order_sequence_for_same_boot() {
        let registry = LoadReportRegistry::default();
        let now = Instant::now();
        registry
            .apply(report("worker-a", 0, NodeLifecycleState::Ready), now)
            .expect("first report");

        assert_eq!(
            registry.apply(report("worker-a", 1, NodeLifecycleState::Ready), now),
            Err("load_report_out_of_order".to_string())
        );
    }

    #[test]
    fn routing_never_selects_draining_node() {
        let registry = load_report_registry();
        registry.clear();
        let now = Instant::now();
        registry
            .apply(report("ready", 1, NodeLifecycleState::Ready), now)
            .expect("ready report");
        registry
            .apply(report("draining", 0, NodeLifecycleState::Draining), now)
            .expect("draining report");

        let decision = select_owner(
            "request-1",
            "Todos.list",
            "gateway",
            &["ready".to_string(), "draining".to_string()],
            None,
            &RoutingPolicy::default(),
            now,
        )
        .expect("routing decision");

        assert_eq!(decision.selected_node, "ready");
    }

    #[test]
    fn routing_favors_lower_normalized_pressure() {
        let registry = load_report_registry();
        registry.clear();
        let now = Instant::now();
        registry
            .apply(report("cool", 1, NodeLifecycleState::Ready), now)
            .expect("cool report");
        registry
            .apply(report("hot", 200, NodeLifecycleState::Ready), now)
            .expect("hot report");

        let decision = select_owner(
            "request-1",
            "Todos.list",
            "gateway",
            &["cool".to_string(), "hot".to_string()],
            None,
            &RoutingPolicy::default(),
            now,
        )
        .expect("routing decision");

        assert_eq!(decision.selected_node, "cool");
    }

    #[test]
    fn ingress_reservation_moves_the_next_choice_to_an_unreserved_peer() {
        let reports = load_report_registry();
        reports.clear();
        let now = Instant::now();
        reports
            .apply(report("worker-a", 0, NodeLifecycleState::Ready), now)
            .expect("worker-a report");
        reports
            .apply(report("worker-b", 0, NodeLifecycleState::Ready), now)
            .expect("worker-b report");
        let reservations = Arc::new(SelectionReservationRegistry::default());
        let membership = ["worker-a".to_string(), "worker-b".to_string()];

        let (first, _reservation) = select_owner_and_reserve_with_registry(
            "request-1",
            "Todos.list",
            "gateway",
            &membership,
            None,
            &RoutingPolicy::default(),
            now,
            Arc::clone(&reservations),
        )
        .expect("first routing reservation");
        let (second, _reservation) = select_owner_and_reserve_with_registry(
            "request-2",
            "Todos.list",
            "gateway",
            &membership,
            None,
            &RoutingPolicy::default(),
            now,
            reservations,
        )
        .expect("second routing reservation");

        assert_ne!(first.selected_node, second.selected_node);
    }

    #[test]
    fn replicas_are_distinct_from_owner() {
        let candidates = vec![
            report("owner", 0, NodeLifecycleState::Ready),
            report("replica-a", 0, NodeLifecycleState::Ready),
            report("replica-b", 0, NodeLifecycleState::Ready),
        ];

        assert_eq!(
            select_record_replicas("owner", 3, &candidates).expect("replicas"),
            vec!["replica-a".to_string(), "replica-b".to_string()]
        );
    }
}
