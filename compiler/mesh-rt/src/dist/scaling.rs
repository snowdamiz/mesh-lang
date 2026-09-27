//! Fenced capacity policy, reconciliation, drain orchestration, and drivers.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

fn environment_value<'a>(environment: &'a [String], name: &str) -> Option<&'a str> {
    environment.iter().rev().find_map(|entry| {
        entry
            .split_once('=')
            .filter(|(candidate, _)| *candidate == name)
            .map(|(_, value)| value)
    })
}

fn signed_managed_identity_entries(
    operation: &DriverOperation,
    advertised_name: &str,
    roles: Vec<String>,
) -> Result<Vec<String>, String> {
    let signing_key = std::env::var(super::identity_claim::CAPACITY_IDENTITY_SIGNING_KEY_ENV)
        .map_err(|_| "capacity_identity_signing_key_missing".to_string())?;
    let now = super::identity_claim::unix_millis();
    let stable_node_id = format!(
        "{}/capacity/{}",
        operation.cluster_id, operation.operation_id
    );
    let claim = super::identity_claim::NodeIdentityClaim {
        schema_version: super::identity_claim::IDENTITY_SCHEMA_VERSION,
        cluster_id: operation.cluster_id.clone(),
        stable_node_id: stable_node_id.clone(),
        advertised_name: advertised_name.to_string(),
        roles: super::identity_claim::canonical_roles(&roles)?,
        issued_at_unix_millis: now,
        expires_at_unix_millis: now.saturating_add(30 * 24 * 60 * 60 * 1_000),
    };
    let envelope = super::identity_claim::sign_identity_claim(&claim, &signing_key)?;
    Ok(vec![
        format!("MESH_STABLE_NODE_ID={stable_node_id}"),
        format!("MESH_NODE_NAME={advertised_name}"),
        format!(
            "{}={envelope}",
            super::identity_claim::IDENTITY_ENVELOPE_ENV
        ),
    ])
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ControlTerm(pub u64);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DesiredRevision(pub u64);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesiredCapacity {
    pub revision: DesiredRevision,
    pub worker_nodes: u16,
    pub gateway_nodes: u16,
    pub template_revision: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScalingPolicy {
    pub min_nodes: u16,
    pub max_nodes: u16,
    pub target_inflight_per_node: u32,
    pub scale_up_window_millis: u64,
    pub scale_down_window_millis: u64,
    pub cooldown_millis: u64,
    pub max_scale_up_step: u16,
    pub max_scale_down_step: u16,
    pub max_unavailable: u16,
}

impl ScalingPolicy {
    pub fn validate(&self) -> Result<(), String> {
        if self.min_nodes == 0 || self.min_nodes > self.max_nodes {
            return Err("scaling_node_bounds_invalid".to_string());
        }
        if self.target_inflight_per_node == 0 {
            return Err("scaling_target_inflight_zero".to_string());
        }
        if self.scale_up_window_millis == 0
            || self.scale_down_window_millis <= self.scale_up_window_millis
        {
            return Err("scaling_stabilization_windows_invalid".to_string());
        }
        if self.max_scale_up_step == 0 || self.max_scale_down_step == 0 {
            return Err("scaling_step_bound_zero".to_string());
        }
        if self.max_unavailable >= self.min_nodes {
            return Err("scaling_max_unavailable_invalid".to_string());
        }
        Ok(())
    }

    fn scale_up_window(&self) -> Duration {
        Duration::from_millis(self.scale_up_window_millis)
    }

    fn scale_down_window(&self) -> Duration {
        Duration::from_millis(self.scale_down_window_millis)
    }

    fn cooldown(&self) -> Duration {
        Duration::from_millis(self.cooldown_millis)
    }
}

impl Default for ScalingPolicy {
    fn default() -> Self {
        Self {
            min_nodes: 2,
            max_nodes: 20,
            target_inflight_per_node: 128,
            scale_up_window_millis: 30_000,
            scale_down_window_millis: 600_000,
            cooldown_millis: 120_000,
            max_scale_up_step: 4,
            max_scale_down_step: 1,
            max_unavailable: 1,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ScalingSample {
    pub observed_at: Instant,
    pub cluster_inflight: u64,
    pub cluster_pressure_ewma: f64,
    pub ready_nodes: u16,
    pub reports_complete: bool,
    pub driver_healthy: bool,
    pub controller_stable: bool,
    pub continuity_healthy: bool,
    pub drain_incomplete: bool,
}

impl ScalingSample {
    fn recommendation(&self, policy: &ScalingPolicy) -> u16 {
        let required_by_inflight = self
            .cluster_inflight
            .div_ceil(u64::from(policy.target_inflight_per_node));
        let required_by_pressure =
            (f64::from(self.ready_nodes) * self.cluster_pressure_ewma.max(0.0)).ceil() as u64;
        u64::from(policy.min_nodes)
            .max(required_by_inflight)
            .max(required_by_pressure)
            .clamp(u64::from(policy.min_nodes), u64::from(policy.max_nodes)) as u16
    }

    fn permits_scale_down(&self) -> bool {
        self.reports_complete
            && self.driver_healthy
            && self.controller_stable
            && self.continuity_healthy
            && !self.drain_incomplete
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScalingAction {
    Hold,
    ScaleUp,
    ScaleDown,
    Frozen,
    Paused,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScalingDecision {
    pub action: ScalingAction,
    pub current: u16,
    pub raw_desired: u16,
    pub bounded_desired: u16,
    pub dominant_signal: String,
    pub constraints: Vec<String>,
    pub decided_at_unix_millis: u64,
}

#[derive(Debug)]
pub struct Autoscaler {
    policy: ScalingPolicy,
    samples: VecDeque<ScalingSample>,
    last_change: Option<Instant>,
    paused: bool,
    scale_up_enabled: bool,
    scale_down_enabled: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LocalScalingPolicy {
    pub min_workers: usize,
    pub max_workers: usize,
    pub target_runnable_per_worker: f64,
    pub target_queue_wait: Duration,
    pub scale_up_window: Duration,
    pub scale_down_window: Duration,
    pub cooldown: Duration,
}

impl LocalScalingPolicy {
    pub fn validate(&self) -> Result<(), String> {
        if self.min_workers == 0 || self.min_workers > self.max_workers {
            return Err("local_scaling_worker_bounds_invalid".to_string());
        }
        if !self.target_runnable_per_worker.is_finite() || self.target_runnable_per_worker <= 0.0 {
            return Err("local_scaling_runnable_target_invalid".to_string());
        }
        if self.target_queue_wait.is_zero()
            || self.scale_up_window.is_zero()
            || self.scale_down_window <= self.scale_up_window
        {
            return Err("local_scaling_windows_invalid".to_string());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalScalingDecision {
    pub desired_workers: usize,
    pub reason: &'static str,
    pub changed: bool,
}

#[derive(Debug)]
pub struct LocalSchedulerAutoscaler {
    policy: LocalScalingPolicy,
    high_since: Option<Instant>,
    low_since: Option<Instant>,
    last_change: Option<Instant>,
}

impl LocalSchedulerAutoscaler {
    pub fn new(policy: LocalScalingPolicy) -> Result<Self, String> {
        policy.validate()?;
        Ok(Self {
            policy,
            high_since: None,
            low_since: None,
            last_change: None,
        })
    }

    pub fn evaluate(
        &mut self,
        current_workers: usize,
        runnable_actors: u64,
        p95_queue_wait: Duration,
        now: Instant,
    ) -> LocalScalingDecision {
        let current_workers =
            current_workers.clamp(self.policy.min_workers, self.policy.max_workers);
        let runnable_per_worker = runnable_actors as f64 / current_workers.max(1) as f64;
        let high = runnable_per_worker > self.policy.target_runnable_per_worker
            || p95_queue_wait > self.policy.target_queue_wait;
        let low = runnable_per_worker < self.policy.target_runnable_per_worker * 0.5
            && p95_queue_wait < self.policy.target_queue_wait / 2;

        if high {
            self.high_since.get_or_insert(now);
            self.low_since = None;
        } else if low {
            self.low_since.get_or_insert(now);
            self.high_since = None;
        } else {
            self.high_since = None;
            self.low_since = None;
        }

        if self
            .last_change
            .is_some_and(|changed| now.saturating_duration_since(changed) < self.policy.cooldown)
        {
            return LocalScalingDecision {
                desired_workers: current_workers,
                reason: "cooldown",
                changed: false,
            };
        }

        if current_workers < self.policy.max_workers
            && self.high_since.is_some_and(|since| {
                now.saturating_duration_since(since) >= self.policy.scale_up_window
            })
        {
            let pressure_target =
                (runnable_actors as f64 / self.policy.target_runnable_per_worker).ceil() as usize;
            let desired = current_workers
                .saturating_add(1)
                .max(pressure_target)
                .min(self.policy.max_workers);
            self.last_change = Some(now);
            self.high_since = None;
            return LocalScalingDecision {
                desired_workers: desired,
                reason: "sustained_local_pressure",
                changed: desired != current_workers,
            };
        }

        if current_workers > self.policy.min_workers
            && self.low_since.is_some_and(|since| {
                now.saturating_duration_since(since) >= self.policy.scale_down_window
            })
        {
            let desired = current_workers
                .saturating_sub(1)
                .max(self.policy.min_workers);
            self.last_change = Some(now);
            self.low_since = None;
            return LocalScalingDecision {
                desired_workers: desired,
                reason: "sustained_local_idle",
                changed: desired != current_workers,
            };
        }

        LocalScalingDecision {
            desired_workers: current_workers,
            reason: if high {
                "scale_up_stabilizing"
            } else if low {
                "scale_down_stabilizing"
            } else {
                "within_target"
            },
            changed: false,
        }
    }
}

static LOCAL_AUTOSCALER_STARTED: std::sync::Once = std::sync::Once::new();

pub(crate) fn start_local_scheduler_autoscaler(
    scheduler: &'static crate::actor::scheduler::Scheduler,
) {
    let (scheduler_min, scheduler_max) = scheduler.worker_bounds();
    if scheduler_min == scheduler_max {
        return;
    }
    if super::autonomous::embedded_autonomous_config()
        .is_some_and(|config| !config.features.local_scheduler_autoscaling)
    {
        return;
    }
    LOCAL_AUTOSCALER_STARTED.call_once(|| {
        let embedded = super::autonomous::embedded_autonomous_config()
            .map(|config| config.scheduler.clone())
            .unwrap_or_default();
        let policy = LocalScalingPolicy {
            min_workers: scheduler_min,
            max_workers: scheduler_max,
            target_runnable_per_worker: env_parse(
                "MESH_SCHEDULER_TARGET_RUNNABLE",
                embedded.target_runnable_per_worker,
            ),
            target_queue_wait: Duration::from_millis(env_parse(
                "MESH_SCHEDULER_TARGET_QUEUE_WAIT_MS",
                embedded.target_queue_wait_millis,
            )),
            scale_up_window: Duration::from_millis(env_parse(
                "MESH_SCHEDULER_SCALE_UP_WINDOW_MS",
                embedded.scale_up_window_millis,
            )),
            scale_down_window: Duration::from_millis(env_parse(
                "MESH_SCHEDULER_SCALE_DOWN_WINDOW_MS",
                embedded.scale_down_window_millis,
            )),
            cooldown: Duration::from_millis(env_parse(
                "MESH_SCHEDULER_COOLDOWN_MS",
                embedded.cooldown_millis,
            )),
        };
        let Ok(mut autoscaler) = LocalSchedulerAutoscaler::new(policy) else {
            eprintln!("mesh scheduler: local autoscaling configuration invalid; keeping minimum");
            return;
        };
        std::thread::Builder::new()
            .name("mesh-local-scheduler-autoscaler".to_string())
            .spawn(move || {
                while !scheduler.is_shutdown() {
                    if crate::dist::operator::autoscaler_paused() {
                        std::thread::park_timeout(Duration::from_millis(250));
                        continue;
                    }
                    let runtime = crate::dist::telemetry::runtime_telemetry();
                    runtime.refresh_routing_cache();
                    let telemetry = runtime.routing_snapshot();
                    let decision = autoscaler.evaluate(
                        scheduler.active_workers(),
                        scheduler.runnable_count(),
                        telemetry.p95_queue_wait,
                        Instant::now(),
                    );
                    if decision.changed {
                        if let Err(error) = scheduler.resize(decision.desired_workers) {
                            eprintln!("mesh scheduler: resize_failed reason={error}");
                        }
                    }
                    std::thread::park_timeout(Duration::from_millis(250));
                }
            })
            .expect("failed to start local scheduler autoscaler");
    });
}

fn env_parse<T>(name: &str, default: T) -> T
where
    T: std::str::FromStr,
{
    std::env::var(name)
        .ok()
        .and_then(|raw| raw.parse().ok())
        .unwrap_or(default)
}

impl Autoscaler {
    pub fn new(policy: ScalingPolicy) -> Result<Self, String> {
        policy.validate()?;
        Ok(Self {
            policy,
            samples: VecDeque::new(),
            last_change: None,
            paused: false,
            scale_up_enabled: true,
            scale_down_enabled: true,
        })
    }

    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }

    pub fn set_action_gates(&mut self, scale_up: bool, scale_down: bool) {
        self.scale_up_enabled = scale_up;
        self.scale_down_enabled = scale_down;
    }

    pub fn evaluate(&mut self, current_desired: u16, sample: ScalingSample) -> ScalingDecision {
        let now = sample.observed_at;
        self.samples.push_back(sample);
        while self.samples.len() > 1
            && self.samples.get(1).is_some_and(|next| {
                now.saturating_duration_since(next.observed_at) > self.policy.scale_down_window()
            })
        {
            self.samples.pop_front();
        }
        let latest = self.samples.back().expect("sample was just inserted");
        let latest_recommendation = latest.recommendation(&self.policy);
        let dominant_signal = if latest.cluster_inflight
            > u64::from(latest.ready_nodes) * u64::from(self.policy.target_inflight_per_node)
        {
            "inflight"
        } else {
            "pressure"
        };
        let mut decision = ScalingDecision {
            action: ScalingAction::Hold,
            current: current_desired,
            raw_desired: latest_recommendation,
            bounded_desired: current_desired,
            dominant_signal: dominant_signal.to_string(),
            constraints: Vec::new(),
            decided_at_unix_millis: unix_millis(),
        };

        if self.paused {
            decision.action = ScalingAction::Paused;
            decision.constraints.push("autoscaler_paused".to_string());
            return decision;
        }
        if self
            .last_change
            .is_some_and(|last| now.saturating_duration_since(last) < self.policy.cooldown())
        {
            decision.constraints.push("cooldown".to_string());
            return decision;
        }

        let scale_up_window = window_samples(&self.samples, now, self.policy.scale_up_window());
        let scale_up_recommendation = scale_up_window
            .iter()
            .map(|sample| sample.recommendation(&self.policy))
            .max()
            .unwrap_or(latest_recommendation);
        if scale_up_recommendation > current_desired {
            if !self.scale_up_enabled {
                decision.constraints.push("scale_up_disabled".to_string());
                return decision;
            }
            // Scale-up stabilization retains the maximum recommendation seen in
            // a complete window. Requiring every sample to be high makes a
            // sustained workload look idle whenever synchronized requests cross
            // a sampling boundary.
            let sustained = full_window(&scale_up_window, now, self.policy.scale_up_window());
            if !sustained {
                decision
                    .constraints
                    .push("scale_up_stabilizing".to_string());
                return decision;
            }
            let raw = scale_up_recommendation;
            decision.action = ScalingAction::ScaleUp;
            decision.raw_desired = raw;
            decision.bounded_desired = raw
                .min(current_desired.saturating_add(self.policy.max_scale_up_step))
                .min(self.policy.max_nodes);
            self.last_change = Some(now);
            return decision;
        }

        if latest_recommendation < current_desired {
            if !self.scale_down_enabled {
                decision.constraints.push("scale_down_disabled".to_string());
                return decision;
            }
            let window = window_samples(&self.samples, now, self.policy.scale_down_window());
            let healthy = full_window(&window, now, self.policy.scale_down_window())
                && window.iter().all(|sample| sample.permits_scale_down())
                && window
                    .iter()
                    .all(|sample| sample.recommendation(&self.policy) < current_desired);
            if !healthy {
                decision.action = ScalingAction::Frozen;
                decision
                    .constraints
                    .push("scale_down_requires_complete_healthy_window".to_string());
                if !full_window(&window, now, self.policy.scale_down_window()) {
                    decision
                        .constraints
                        .push("scale_down_window_incomplete".to_string());
                }
                if window.iter().any(|sample| !sample.reports_complete) {
                    decision
                        .constraints
                        .push("scale_down_reports_incomplete".to_string());
                }
                if window.iter().any(|sample| !sample.driver_healthy) {
                    decision
                        .constraints
                        .push("scale_down_driver_unhealthy".to_string());
                }
                if window.iter().any(|sample| !sample.controller_stable) {
                    decision
                        .constraints
                        .push("scale_down_controller_unstable".to_string());
                }
                if window.iter().any(|sample| !sample.continuity_healthy) {
                    decision
                        .constraints
                        .push("scale_down_continuity_unhealthy".to_string());
                }
                if window.iter().any(|sample| sample.drain_incomplete) {
                    decision
                        .constraints
                        .push("scale_down_drain_incomplete".to_string());
                }
                if window
                    .iter()
                    .any(|sample| sample.recommendation(&self.policy) >= current_desired)
                {
                    decision
                        .constraints
                        .push("scale_down_recommendation_not_sustained".to_string());
                }
                return decision;
            }
            let raw = window
                .iter()
                .map(|sample| sample.recommendation(&self.policy))
                .min()
                .unwrap_or(latest_recommendation);
            decision.action = ScalingAction::ScaleDown;
            decision.raw_desired = raw;
            decision.bounded_desired = raw
                .max(current_desired.saturating_sub(self.policy.max_scale_down_step))
                .max(self.policy.min_nodes);
            self.last_change = Some(now);
        }
        decision
    }
}

fn window_samples(
    samples: &VecDeque<ScalingSample>,
    now: Instant,
    window: Duration,
) -> Vec<&ScalingSample> {
    let first_inside = samples
        .iter()
        .position(|sample| now.saturating_duration_since(sample.observed_at) <= window)
        .unwrap_or(samples.len());
    // Retain the sample immediately before the boundary. Without it, polling
    // jitter makes it impossible to prove that observations cover the entire
    // stabilization window: the old sample is dropped just before the new
    // oldest one reaches the requested age.
    samples
        .iter()
        .skip(first_inside.saturating_sub(1))
        .collect()
}

fn full_window(samples: &[&ScalingSample], now: Instant, window: Duration) -> bool {
    samples
        .first()
        .is_some_and(|oldest| now.saturating_duration_since(oldest.observed_at) >= window)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DriverOperationState {
    Pending,
    Succeeded,
    RetryableFailure(String),
    PermanentFailure(String),
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DriverOperation {
    pub cluster_id: String,
    pub operation_id: String,
    pub control_term: ControlTerm,
    pub desired_revision: DesiredRevision,
    pub template_revision: String,
    pub node_id: Option<String>,
    pub state: DriverOperationState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapacityObservation {
    pub nodes: Vec<ObservedCapacityNode>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObservedCapacityNode {
    pub node_id: String,
    pub operation_id: String,
    pub control_term: ControlTerm,
    pub desired_revision: DesiredRevision,
    pub template_revision: String,
    pub lifecycle: CapacityNodeLifecycle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CapacityNodeLifecycle {
    Provisioning,
    Ready,
    Draining,
    Terminating,
    Removed,
    Failed,
}

pub trait CapacityDriver: Send + Sync {
    fn validate_configuration(&self) -> Result<(), String>;
    fn observe_capacity(&self, cluster_id: &str) -> Result<CapacityObservation, String>;
    fn ensure_node(&self, operation: &DriverOperation) -> Result<DriverOperation, String>;
    fn begin_drain(
        &self,
        operation: &DriverOperation,
        node_id: &str,
    ) -> Result<DriverOperation, String>;
    fn terminate_node(
        &self,
        operation: &DriverOperation,
        node_id: &str,
    ) -> Result<DriverOperation, String>;
    fn get_operation(&self, operation_id: &str) -> Result<Option<DriverOperation>, String>;
}

struct InstrumentedCapacityDriver {
    inner: Arc<dyn CapacityDriver>,
}

impl InstrumentedCapacityDriver {
    fn record<T>(
        operation: crate::dist::telemetry::CapacityDriverOperationKind,
        started_at: Instant,
        result: &Result<T, String>,
        operation_failed: bool,
    ) {
        crate::dist::telemetry::runtime_telemetry().record_capacity_driver_operation(
            operation,
            started_at.elapsed(),
            result.is_err() || operation_failed,
        );
    }

    fn driver_operation_failed(result: &Result<DriverOperation, String>) -> bool {
        result.as_ref().is_ok_and(|operation| {
            matches!(
                operation.state,
                DriverOperationState::RetryableFailure(_)
                    | DriverOperationState::PermanentFailure(_)
                    | DriverOperationState::Unknown
            )
        })
    }
}

impl CapacityDriver for InstrumentedCapacityDriver {
    fn validate_configuration(&self) -> Result<(), String> {
        let started_at = Instant::now();
        let result = self.inner.validate_configuration();
        Self::record(
            crate::dist::telemetry::CapacityDriverOperationKind::Validate,
            started_at,
            &result,
            false,
        );
        result
    }

    fn observe_capacity(&self, cluster_id: &str) -> Result<CapacityObservation, String> {
        let started_at = Instant::now();
        let result = self.inner.observe_capacity(cluster_id);
        Self::record(
            crate::dist::telemetry::CapacityDriverOperationKind::Observe,
            started_at,
            &result,
            false,
        );
        result
    }

    fn ensure_node(&self, operation: &DriverOperation) -> Result<DriverOperation, String> {
        let started_at = Instant::now();
        let result = self.inner.ensure_node(operation);
        let failed = Self::driver_operation_failed(&result);
        Self::record(
            crate::dist::telemetry::CapacityDriverOperationKind::Ensure,
            started_at,
            &result,
            failed,
        );
        result
    }

    fn begin_drain(
        &self,
        operation: &DriverOperation,
        node_id: &str,
    ) -> Result<DriverOperation, String> {
        let started_at = Instant::now();
        let result = self.inner.begin_drain(operation, node_id);
        let failed = Self::driver_operation_failed(&result);
        Self::record(
            crate::dist::telemetry::CapacityDriverOperationKind::BeginDrain,
            started_at,
            &result,
            failed,
        );
        result
    }

    fn terminate_node(
        &self,
        operation: &DriverOperation,
        node_id: &str,
    ) -> Result<DriverOperation, String> {
        let started_at = Instant::now();
        let result = self.inner.terminate_node(operation, node_id);
        let failed = Self::driver_operation_failed(&result);
        Self::record(
            crate::dist::telemetry::CapacityDriverOperationKind::Terminate,
            started_at,
            &result,
            failed,
        );
        result
    }

    fn get_operation(&self, operation_id: &str) -> Result<Option<DriverOperation>, String> {
        let started_at = Instant::now();
        let result = self.inner.get_operation(operation_id);
        Self::record(
            crate::dist::telemetry::CapacityDriverOperationKind::GetOperation,
            started_at,
            &result,
            false,
        );
        result
    }
}

pub(crate) fn instrument_capacity_driver(
    driver: Arc<dyn CapacityDriver>,
) -> Arc<dyn CapacityDriver> {
    Arc::new(InstrumentedCapacityDriver { inner: driver })
}

#[derive(Debug)]
pub struct FakeCapacityDriver {
    operations: Mutex<BTreeMap<String, DriverOperation>>,
    nodes: Mutex<BTreeMap<String, ObservedCapacityNode>>,
    observations: AtomicU64,
}

impl FakeCapacityDriver {
    pub fn new() -> Self {
        Self {
            operations: Mutex::new(BTreeMap::new()),
            nodes: Mutex::new(BTreeMap::new()),
            observations: AtomicU64::new(0),
        }
    }
}

impl Default for FakeCapacityDriver {
    fn default() -> Self {
        Self::new()
    }
}

impl CapacityDriver for FakeCapacityDriver {
    fn validate_configuration(&self) -> Result<(), String> {
        Ok(())
    }

    fn observe_capacity(&self, _cluster_id: &str) -> Result<CapacityObservation, String> {
        self.observations.fetch_add(1, Ordering::Relaxed);
        Ok(CapacityObservation {
            nodes: self.nodes.lock().unwrap().values().cloned().collect(),
        })
    }

    fn ensure_node(&self, operation: &DriverOperation) -> Result<DriverOperation, String> {
        if let Some(existing) = self.operations.lock().unwrap().get(&operation.operation_id) {
            return Ok(existing.clone());
        }
        let node_id = format!(
            "node-{}",
            &operation.operation_id[..12.min(operation.operation_id.len())]
        );
        let mut completed = operation.clone();
        completed.node_id = Some(node_id.clone());
        completed.state = DriverOperationState::Succeeded;
        self.nodes.lock().unwrap().insert(
            node_id.clone(),
            ObservedCapacityNode {
                node_id,
                operation_id: operation.operation_id.clone(),
                control_term: operation.control_term,
                desired_revision: operation.desired_revision,
                template_revision: operation.template_revision.clone(),
                lifecycle: CapacityNodeLifecycle::Provisioning,
            },
        );
        self.operations
            .lock()
            .unwrap()
            .insert(operation.operation_id.clone(), completed.clone());
        Ok(completed)
    }

    fn begin_drain(
        &self,
        operation: &DriverOperation,
        node_id: &str,
    ) -> Result<DriverOperation, String> {
        if let Some(existing) = self.operations.lock().unwrap().get(&operation.operation_id) {
            return Ok(existing.clone());
        }
        let mut nodes = self.nodes.lock().unwrap();
        let node = nodes
            .get_mut(node_id)
            .ok_or_else(|| "capacity_node_not_found".to_string())?;
        node.lifecycle = CapacityNodeLifecycle::Draining;
        let mut completed = operation.clone();
        completed.node_id = Some(node_id.to_string());
        completed.state = DriverOperationState::Succeeded;
        self.operations
            .lock()
            .unwrap()
            .insert(operation.operation_id.clone(), completed.clone());
        Ok(completed)
    }

    fn terminate_node(
        &self,
        operation: &DriverOperation,
        node_id: &str,
    ) -> Result<DriverOperation, String> {
        if let Some(existing) = self.operations.lock().unwrap().get(&operation.operation_id) {
            return Ok(existing.clone());
        }
        if let Some(node) = self.nodes.lock().unwrap().get_mut(node_id) {
            node.lifecycle = CapacityNodeLifecycle::Removed;
        }
        let mut completed = operation.clone();
        completed.node_id = Some(node_id.to_string());
        completed.state = DriverOperationState::Succeeded;
        self.operations
            .lock()
            .unwrap()
            .insert(operation.operation_id.clone(), completed.clone());
        Ok(completed)
    }

    fn get_operation(&self, operation_id: &str) -> Result<Option<DriverOperation>, String> {
        Ok(self.operations.lock().unwrap().get(operation_id).cloned())
    }
}

#[derive(Clone)]
pub struct ProcessDriverConfig {
    pub command: Vec<String>,
    pub working_directory: PathBuf,
    pub environment: BTreeMap<String, String>,
}

impl std::fmt::Debug for ProcessDriverConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProcessDriverConfig")
            .field("executable", &self.command.first())
            .field("argument_count", &self.command.len().saturating_sub(1))
            .field("working_directory", &self.working_directory)
            .field(
                "environment_names",
                &self.environment.keys().collect::<Vec<_>>(),
            )
            .field(
                "environment_values",
                &format_args!("[redacted; {}]", self.environment.len()),
            )
            .finish()
    }
}

#[derive(Debug)]
pub struct ProcessCapacityDriver {
    config: ProcessDriverConfig,
    operations: Mutex<BTreeMap<String, DriverOperation>>,
    children: Mutex<BTreeMap<String, Child>>,
}

impl ProcessCapacityDriver {
    pub fn new(config: ProcessDriverConfig) -> Self {
        Self {
            config,
            operations: Mutex::new(BTreeMap::new()),
            children: Mutex::new(BTreeMap::new()),
        }
    }
}

impl CapacityDriver for ProcessCapacityDriver {
    fn validate_configuration(&self) -> Result<(), String> {
        if self
            .config
            .command
            .first()
            .is_none_or(|value| value.trim().is_empty())
        {
            return Err("process_driver_command_missing".to_string());
        }
        if !self.config.working_directory.is_dir() {
            return Err("process_driver_working_directory_invalid".to_string());
        }
        Ok(())
    }

    fn observe_capacity(&self, _cluster_id: &str) -> Result<CapacityObservation, String> {
        let mut children = self.children.lock().unwrap();
        let mut nodes = Vec::new();
        for (node_id, child) in children.iter_mut() {
            let lifecycle = match child.try_wait() {
                Ok(None) => CapacityNodeLifecycle::Ready,
                Ok(Some(status)) if status.success() => CapacityNodeLifecycle::Removed,
                Ok(Some(_)) | Err(_) => CapacityNodeLifecycle::Failed,
            };
            if let Some(operation) = self
                .operations
                .lock()
                .unwrap()
                .values()
                .find(|operation| operation.node_id.as_deref() == Some(node_id))
            {
                nodes.push(ObservedCapacityNode {
                    node_id: node_id.clone(),
                    operation_id: operation.operation_id.clone(),
                    control_term: operation.control_term,
                    desired_revision: operation.desired_revision,
                    template_revision: operation.template_revision.clone(),
                    lifecycle,
                });
            }
        }
        Ok(CapacityObservation { nodes })
    }

    fn ensure_node(&self, operation: &DriverOperation) -> Result<DriverOperation, String> {
        self.validate_configuration()?;
        if let Some(existing) = self.operations.lock().unwrap().get(&operation.operation_id) {
            return Ok(existing.clone());
        }
        let executable = self.config.command.first().expect("validated command");
        let mut command = Command::new(executable);
        command.args(&self.config.command[1..]);
        command.current_dir(&self.config.working_directory);
        command.envs(&self.config.environment);
        command.env("MESH_CAPACITY_OPERATION_ID", &operation.operation_id);
        command.env("MESH_CONTROL_TERM", operation.control_term.0.to_string());
        command.stdin(Stdio::null());
        let child = command
            .spawn()
            .map_err(|error| format!("process_driver_spawn_failed:{error}"))?;
        let node_id = format!("process-{}", child.id());
        self.children.lock().unwrap().insert(node_id.clone(), child);
        let mut completed = operation.clone();
        completed.node_id = Some(node_id);
        completed.state = DriverOperationState::Succeeded;
        self.operations
            .lock()
            .unwrap()
            .insert(operation.operation_id.clone(), completed.clone());
        Ok(completed)
    }

    fn begin_drain(
        &self,
        operation: &DriverOperation,
        node_id: &str,
    ) -> Result<DriverOperation, String> {
        completed_driver_operation(&self.operations, operation, node_id)
    }

    fn terminate_node(
        &self,
        operation: &DriverOperation,
        node_id: &str,
    ) -> Result<DriverOperation, String> {
        if let Some(existing) = self.operations.lock().unwrap().get(&operation.operation_id) {
            return Ok(existing.clone());
        }
        if let Some(mut child) = self.children.lock().unwrap().remove(node_id) {
            child
                .kill()
                .map_err(|error| format!("process_driver_terminate_failed:{error}"))?;
            let _ = child.wait();
        }
        completed_driver_operation(&self.operations, operation, node_id)
    }

    fn get_operation(&self, operation_id: &str) -> Result<Option<DriverOperation>, String> {
        Ok(self.operations.lock().unwrap().get(operation_id).cloned())
    }
}

fn completed_driver_operation(
    operations: &Mutex<BTreeMap<String, DriverOperation>>,
    operation: &DriverOperation,
    node_id: &str,
) -> Result<DriverOperation, String> {
    if let Some(existing) = operations.lock().unwrap().get(&operation.operation_id) {
        return Ok(existing.clone());
    }
    let mut completed = operation.clone();
    completed.node_id = Some(node_id.to_string());
    completed.state = DriverOperationState::Succeeded;
    operations
        .lock()
        .unwrap()
        .insert(operation.operation_id.clone(), completed.clone());
    Ok(completed)
}

#[derive(Clone)]
pub struct DockerDriverConfig {
    pub binary: PathBuf,
    /// Fixed arguments inserted before each Docker Engine CLI operation.
    ///
    /// The local proof uses this to execute the real Docker client inside a
    /// dedicated socket-bearing driver container. It is typed configuration,
    /// not shell interpolation.
    pub execution_prefix: Vec<String>,
    pub image: String,
    pub pool: String,
    pub network: Option<String>,
    pub environment: Vec<String>,
    /// Optional host-to-driver mapping for private environment files.
    /// Omit this only when the Docker client runs directly on the host.
    pub environment_file_mount: Option<DockerEnvironmentFileMount>,
    pub operation_timeout: Duration,
}

#[derive(Clone, Debug)]
pub struct DockerEnvironmentFileMount {
    pub host_directory: PathBuf,
    pub driver_directory: PathBuf,
}

impl std::fmt::Debug for DockerDriverConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DockerDriverConfig")
            .field("binary", &self.binary)
            .field("execution_prefix", &self.execution_prefix)
            .field("image", &self.image)
            .field("pool", &self.pool)
            .field("network", &self.network)
            .field("environment_file_mount", &self.environment_file_mount)
            .field(
                "environment",
                &format_args!("[redacted; {}]", self.environment.len()),
            )
            .field("operation_timeout", &self.operation_timeout)
            .finish()
    }
}

#[derive(Debug)]
pub struct DockerCapacityDriver {
    config: DockerDriverConfig,
    operations: Mutex<BTreeMap<String, DriverOperation>>,
}

#[derive(Debug)]
struct DockerContainerObservation {
    node_id: String,
    cluster_id: String,
    pool: String,
    template_revision: String,
    operation_id: String,
    control_term: ControlTerm,
    desired_revision: DesiredRevision,
    lifecycle: CapacityNodeLifecycle,
}

struct DockerEnvironmentFile {
    host_path: PathBuf,
    driver_path: PathBuf,
}

impl DockerCapacityDriver {
    pub fn new(config: DockerDriverConfig) -> Self {
        Self {
            config,
            operations: Mutex::new(BTreeMap::new()),
        }
    }

    fn docker(&self, arguments: &[String]) -> Result<String, String> {
        if self.config.operation_timeout.is_zero() {
            return Err("docker_driver_operation_timeout_invalid".to_string());
        }
        let mut command = Command::new(&self.config.binary);
        command.args(&self.config.execution_prefix).args(arguments);
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("docker_driver_command_failed:{error}"))?;
        // Both are piped above; the readers only collect bytes.
        let read_all = |mut stream: Box<dyn std::io::Read + Send>| {
            std::thread::spawn(move || {
                let mut bytes = Vec::new();
                let _ = stream.read_to_end(&mut bytes);
                bytes
            })
        };
        let stdout_reader = read_all(Box::new(child.stdout.take().expect("piped stdout")));
        let stderr_reader = read_all(Box::new(child.stderr.take().expect("piped stderr")));
        let deadline = Instant::now() + self.config.operation_timeout;
        let status = loop {
            let failure = match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                }
                Ok(None) => "docker_driver_api_timeout".to_string(),
                Err(error) => format!("docker_driver_wait_failed:{error}"),
            };
            let _ = child.kill();
            let _ = child.wait();
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            return Err(failure);
        };
        let stdout = stdout_reader.join().expect("stdout reader does not panic");
        let stderr = stderr_reader.join().expect("stderr reader does not panic");
        if !status.success() {
            let stderr = String::from_utf8_lossy(&stderr);
            return Err(format!("docker_driver_api_error:{}", redact(&stderr)));
        }
        String::from_utf8(stdout)
            .map(|value| value.trim().to_string())
            .map_err(|_| "docker_driver_output_invalid_utf8".to_string())
    }

    fn environment_file(&self) -> Result<Option<DockerEnvironmentFile>, String> {
        if self.config.environment.is_empty() {
            return Ok(None);
        }
        let mut contents = String::new();
        for entry in &self.config.environment {
            if entry.contains(['\n', '\r']) || !entry.contains('=') {
                return Err("docker_driver_environment_invalid".to_string());
            }
            contents.push_str(entry);
            contents.push('\n');
        }
        let (host_directory, driver_directory) = self
            .config
            .environment_file_mount
            .as_ref()
            .map(|mount| (mount.host_directory.clone(), mount.driver_directory.clone()))
            .unwrap_or_else(|| {
                let directory = std::env::temp_dir().join("mesh-capacity-driver");
                (directory.clone(), directory)
            });
        std::fs::create_dir_all(&host_directory)
            .map_err(|error| format!("docker_driver_env_directory_failed:{error}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&host_directory, std::fs::Permissions::from_mode(0o700))
                .map_err(|error| {
                    format!("docker_driver_env_directory_permissions_failed:{error}")
                })?;
        }
        // A fresh 128-bit name, created exclusively: no other file is it.
        let file_name = format!("env-{:032x}", rand::random::<u128>());
        let host_path = host_directory.join(&file_name);
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options
            .open(&host_path)
            .and_then(|mut file| {
                file.write_all(contents.as_bytes())?;
                file.sync_all()
            })
            .map_err(|error| format!("docker_driver_env_file_write_failed:{error}"))?;
        Ok(Some(DockerEnvironmentFile {
            host_path,
            driver_path: driver_directory.join(file_name),
        }))
    }

    /// The one container `operation_id` created inside `scope`, a label
    /// filter; two are a conflict the driver refuses to resolve.
    fn operation_container(
        &self,
        scope: String,
        operation_id: &str,
    ) -> Result<Option<String>, String> {
        let output = self.docker(&[
            "ps".to_string(),
            "-aq".to_string(),
            "--filter".to_string(),
            scope,
            "--filter".to_string(),
            format!("label=mesh.pool={}", self.config.pool),
            "--filter".to_string(),
            format!("label=mesh.operation={operation_id}"),
        ])?;
        let mut ids = output.lines().filter(|line| !line.trim().is_empty());
        let first = ids.next().map(str::to_string);
        if ids.next().is_some() {
            return Err("docker_driver_duplicate_operation_containers".to_string());
        }
        Ok(first)
    }

    fn inspect_container(&self, node_id: &str) -> Result<DockerContainerObservation, String> {
        let output = self.docker(&["inspect".to_string(), node_id.to_string()])?;
        let documents: serde_json::Value = serde_json::from_str(&output)
            .map_err(|error| format!("docker_driver_inspect_decode_failed:{error}"))?;
        let document = documents
            .as_array()
            .and_then(|documents| documents.first())
            .ok_or_else(|| "docker_driver_inspect_empty".to_string())?;
        let labels = document
            .pointer("/Config/Labels")
            .and_then(serde_json::Value::as_object)
            .ok_or_else(|| "docker_driver_labels_missing".to_string())?;
        let label = |name: &str| -> Result<String, String> {
            labels
                .get(name)
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .ok_or_else(|| format!("docker_driver_label_missing:{name}"))
        };
        if label("mesh.managed")? != "true" {
            return Err("docker_driver_refuses_unmanaged_container".to_string());
        }
        let state = document
            .pointer("/State/Status")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown");
        let health = document
            .pointer("/State/Health/Status")
            .and_then(serde_json::Value::as_str);
        let lifecycle = match (state, health) {
            ("running", Some("unhealthy")) => CapacityNodeLifecycle::Failed,
            ("running", Some("starting")) => CapacityNodeLifecycle::Provisioning,
            ("running", _) => CapacityNodeLifecycle::Ready,
            ("created" | "restarting" | "paused", _) => CapacityNodeLifecycle::Provisioning,
            ("removing", _) => CapacityNodeLifecycle::Terminating,
            // exited, dead, and anything newer than this driver.
            _ => CapacityNodeLifecycle::Failed,
        };
        Ok(DockerContainerObservation {
            node_id: document
                .get("Id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(node_id)
                .to_string(),
            cluster_id: label("mesh.cluster")?,
            pool: label("mesh.pool")?,
            template_revision: label("mesh.template")?,
            operation_id: label("mesh.operation")?,
            control_term: ControlTerm(
                label("mesh.term")?
                    .parse()
                    .map_err(|_| "docker_driver_term_label_invalid".to_string())?,
            ),
            desired_revision: DesiredRevision(
                label("mesh.revision")?
                    .parse()
                    .map_err(|_| "docker_driver_revision_label_invalid".to_string())?,
            ),
            lifecycle,
        })
    }

    fn validate_adoption(
        &self,
        observed: &DockerContainerObservation,
        operation: &DriverOperation,
    ) -> Result<(), String> {
        if observed.cluster_id != operation.cluster_id
            || observed.pool != self.config.pool
            || observed.operation_id != operation.operation_id
            || observed.template_revision != operation.template_revision
            || observed.control_term != operation.control_term
            || observed.desired_revision != operation.desired_revision
        {
            return Err("docker_driver_adoption_identity_mismatch".to_string());
        }
        Ok(())
    }
}

impl CapacityDriver for DockerCapacityDriver {
    fn validate_configuration(&self) -> Result<(), String> {
        if self.config.image.trim().is_empty() || self.config.pool.trim().is_empty() {
            return Err("docker_driver_template_invalid".to_string());
        }
        self.docker(&[
            "version".to_string(),
            "--format".to_string(),
            "{{.Server.Version}}".to_string(),
        ])?;
        Ok(())
    }

    fn observe_capacity(&self, cluster_id: &str) -> Result<CapacityObservation, String> {
        let ids = self.docker(&[
            "ps".to_string(),
            "-aq".to_string(),
            "--filter".to_string(),
            format!("label=mesh.cluster={cluster_id}"),
            "--filter".to_string(),
            format!("label=mesh.pool={}", self.config.pool),
        ])?;
        let mut nodes = Vec::new();
        for node_id in ids.lines().filter(|node_id| !node_id.is_empty()) {
            let observed = self.inspect_container(node_id)?;
            if observed.cluster_id != cluster_id || observed.pool != self.config.pool {
                return Err("docker_driver_observation_scope_mismatch".to_string());
            }
            nodes.push(ObservedCapacityNode {
                node_id: observed.node_id,
                operation_id: observed.operation_id,
                control_term: observed.control_term,
                desired_revision: observed.desired_revision,
                template_revision: observed.template_revision,
                lifecycle: observed.lifecycle,
            });
        }
        Ok(CapacityObservation { nodes })
    }

    fn ensure_node(&self, operation: &DriverOperation) -> Result<DriverOperation, String> {
        if let Some(existing) = self.operations.lock().unwrap().get(&operation.operation_id) {
            return Ok(existing.clone());
        }
        if let Some(container) = self.operation_container(
            format!("label=mesh.cluster={}", operation.cluster_id),
            &operation.operation_id,
        )? {
            let observed = self.inspect_container(&container)?;
            self.validate_adoption(&observed, operation)?;
            let mut adopted = operation.clone();
            adopted.node_id = Some(observed.node_id);
            adopted.state = DriverOperationState::Succeeded;
            self.operations
                .lock()
                .unwrap()
                .insert(operation.operation_id.clone(), adopted.clone());
            return Ok(adopted);
        }
        let name = format!(
            "mesh-{}-{}",
            self.config.pool,
            &operation.operation_id[..12.min(operation.operation_id.len())]
        );
        let identity_required = environment_value(&self.config.environment, "MESH_CLUSTER_MODE")
            .is_some_and(|mode| mode.eq_ignore_ascii_case("autonomous"));
        let cluster_port =
            environment_value(&self.config.environment, "MESH_CLUSTER_PORT").unwrap_or("4370");
        let advertised_name = format!("{name}@{name}:{cluster_port}");
        let roles = environment_value(&self.config.environment, "MESH_ROLES")
            .unwrap_or("worker")
            .split(',')
            .map(str::to_string)
            .collect::<Vec<_>>();
        let identity_environment = if identity_required {
            signed_managed_identity_entries(operation, &advertised_name, roles)?
        } else {
            vec![format!(
                "MESH_STABLE_NODE_ID={}/capacity/{}",
                operation.cluster_id, operation.operation_id
            )]
        };
        let mut arguments = vec![
            "create".to_string(),
            "--name".to_string(),
            name.clone(),
            "--hostname".to_string(),
            name,
            "--label".to_string(),
            "mesh.managed=true".to_string(),
            "--label".to_string(),
            format!("mesh.cluster={}", operation.cluster_id),
            "--label".to_string(),
            format!("mesh.pool={}", self.config.pool),
            "--label".to_string(),
            format!("mesh.template={}", operation.template_revision),
            "--label".to_string(),
            format!("mesh.operation={}", operation.operation_id),
            "--label".to_string(),
            format!("mesh.term={}", operation.control_term.0),
            "--label".to_string(),
            format!("mesh.revision={}", operation.desired_revision.0),
        ];
        if let Some(network) = &self.config.network {
            arguments.extend(["--network".to_string(), network.clone()]);
        }
        for entry in identity_environment {
            arguments.extend(["--env".to_string(), entry]);
        }
        let environment_file = self.environment_file()?;
        if let Some(file) = &environment_file {
            arguments.extend([
                "--env-file".to_string(),
                file.driver_path.to_string_lossy().into_owned(),
            ]);
        }
        arguments.push(self.config.image.clone());
        let create_result = self.docker(&arguments);
        if let Some(file) = environment_file {
            let _ = std::fs::remove_file(file.host_path);
        }
        let container_id = create_result?;
        self.docker(&["start".to_string(), container_id.clone()])?;
        let mut completed = operation.clone();
        completed.node_id = Some(container_id);
        completed.state = DriverOperationState::Succeeded;
        self.operations
            .lock()
            .unwrap()
            .insert(operation.operation_id.clone(), completed.clone());
        Ok(completed)
    }

    fn begin_drain(
        &self,
        operation: &DriverOperation,
        node_id: &str,
    ) -> Result<DriverOperation, String> {
        completed_driver_operation(&self.operations, operation, node_id)
    }

    fn terminate_node(
        &self,
        operation: &DriverOperation,
        node_id: &str,
    ) -> Result<DriverOperation, String> {
        if let Some(existing) = self.operations.lock().unwrap().get(&operation.operation_id) {
            return Ok(existing.clone());
        }
        let existing = self.docker(&[
            "ps".to_string(),
            "-aq".to_string(),
            "--filter".to_string(),
            format!("id={node_id}"),
        ])?;
        if !existing.lines().any(|candidate| !candidate.is_empty()) {
            return completed_driver_operation(&self.operations, operation, node_id);
        }
        let observed = self.inspect_container(node_id)?;
        if observed.cluster_id != operation.cluster_id || observed.pool != self.config.pool {
            return Err("docker_driver_refuses_unmanaged_container".to_string());
        }
        self.docker(&["rm".to_string(), "-f".to_string(), node_id.to_string()])?;
        completed_driver_operation(&self.operations, operation, node_id)
    }

    fn get_operation(&self, operation_id: &str) -> Result<Option<DriverOperation>, String> {
        if let Some(operation) = self.operations.lock().unwrap().get(operation_id).cloned() {
            return Ok(Some(operation));
        }
        let Some(node_id) =
            self.operation_container("label=mesh.managed=true".to_string(), operation_id)?
        else {
            return Ok(None);
        };
        let observed = self.inspect_container(&node_id)?;
        Ok(Some(DriverOperation {
            cluster_id: observed.cluster_id,
            operation_id: observed.operation_id,
            control_term: observed.control_term,
            desired_revision: observed.desired_revision,
            template_revision: observed.template_revision,
            node_id: Some(observed.node_id),
            state: DriverOperationState::Succeeded,
        }))
    }
}

fn redact(value: &str) -> String {
    value
        .split_whitespace()
        .map(|token| {
            if token.to_ascii_lowercase().contains("token")
                || token.to_ascii_lowercase().contains("password")
                || token.to_ascii_lowercase().contains("secret")
            {
                "[redacted]"
            } else {
                token
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ControlMutation {
    DesiredCapacity(DesiredCapacity),
    PolicyRevision {
        revision: u64,
        policy_json: String,
        policy_sha256: String,
    },
    MembershipIntent {
        generation: u64,
        nodes: Vec<String>,
    },
    PauseAutoscaler {
        paused: bool,
    },
    DrainIntent {
        node_id: String,
        cancelled: bool,
    },
    DriverOperation(DriverOperation),
    ManualOverride {
        worker_nodes: u16,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlLogEntry {
    pub index: u64,
    pub term: ControlTerm,
    pub actor: String,
    pub reason: String,
    pub timestamp_unix_millis: u64,
    #[serde(default)]
    pub actor_sequence: u64,
    pub mutation: ControlMutation,
}

#[derive(Debug)]
pub struct DurableControlLog {
    path: PathBuf,
    file: Mutex<File>,
    entries: Mutex<Vec<ControlLogEntry>>,
}

impl DurableControlLog {
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("control_log_directory_failed:{error}"))?;
        }
        let existing = if path.exists() {
            let file =
                File::open(path).map_err(|error| format!("control_log_open_failed:{error}"))?;
            BufReader::new(file)
                .lines()
                .map(|line| {
                    let line = line.map_err(|error| format!("control_log_read_failed:{error}"))?;
                    serde_json::from_str(&line)
                        .map_err(|error| format!("control_log_decode_failed:{error}"))
                })
                .collect::<Result<Vec<_>, _>>()?
        } else {
            Vec::new()
        };
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|error| format!("control_log_open_failed:{error}"))?;
        secure_file(path)?;
        Ok(Self {
            path: path.to_path_buf(),
            file: Mutex::new(file),
            entries: Mutex::new(existing),
        })
    }

    pub fn append(&self, mut entry: ControlLogEntry) -> Result<ControlLogEntry, String> {
        let mut entries = self.entries.lock().unwrap();
        entry.index = entries
            .last()
            .map_or(1, |last| last.index.saturating_add(1));
        let encoded = serde_json::to_string(&entry)
            .map_err(|error| format!("control_log_encode_failed:{error}"))?;
        let mut file = self.file.lock().unwrap();
        writeln!(file, "{encoded}").map_err(|error| format!("control_log_write_failed:{error}"))?;
        file.sync_data()
            .map_err(|error| format!("control_log_sync_failed:{error}"))?;
        entries.push(entry.clone());
        Ok(entry)
    }

    pub fn entries(&self) -> Vec<ControlLogEntry> {
        self.entries.lock().unwrap().clone()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(unix)]
fn secure_file(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(|error| format!("control_log_permissions_failed:{error}"))
}

#[cfg(not(unix))]
fn secure_file(_path: &Path) -> Result<(), String> {
    Ok(())
}

#[derive(Debug)]
pub struct ControllerQuorum {
    voters: BTreeSet<String>,
    leader: Mutex<Option<String>>,
    term: Mutex<ControlTerm>,
    log: Arc<DurableControlLog>,
}

/// Majority-backed mutation sink used by the capacity reconciler. The local
/// quorum is a deterministic unit-test implementation; production callers
/// commit through the embedded OpenRaft controller quorum.
pub trait ControlPlaneCommitter: Send + Sync {
    fn commit(
        &self,
        leader: &str,
        term: ControlTerm,
        acknowledgements: &BTreeSet<String>,
        actor: &str,
        reason: &str,
        mutation: ControlMutation,
    ) -> Result<ControlLogEntry, String>;
}

impl ControllerQuorum {
    pub fn new(voters: BTreeSet<String>, log: Arc<DurableControlLog>) -> Result<Self, String> {
        if voters.is_empty() || (voters.len() > 1 && voters.len().is_multiple_of(2)) {
            return Err("controller_voter_configuration_invalid".to_string());
        }
        let term = log
            .entries()
            .last()
            .map_or(ControlTerm(0), |entry| entry.term);
        Ok(Self {
            voters,
            leader: Mutex::new(None),
            term: Mutex::new(term),
            log,
        })
    }

    pub fn majority(&self) -> usize {
        self.voters.len() / 2 + 1
    }

    pub fn elect(
        &self,
        candidate: &str,
        acknowledgements: &BTreeSet<String>,
    ) -> Result<ControlTerm, String> {
        if !self.voters.contains(candidate)
            || acknowledgements.intersection(&self.voters).count() < self.majority()
        {
            return Err("controller_quorum_unavailable".to_string());
        }
        let mut term = self.term.lock().unwrap();
        term.0 = term
            .0
            .checked_add(1)
            .ok_or_else(|| "control_term_exhausted".to_string())?;
        *self.leader.lock().unwrap() = Some(candidate.to_string());
        Ok(*term)
    }

    pub fn commit(
        &self,
        leader: &str,
        term: ControlTerm,
        acknowledgements: &BTreeSet<String>,
        actor: &str,
        reason: &str,
        mutation: ControlMutation,
    ) -> Result<ControlLogEntry, String> {
        if self.leader.lock().unwrap().as_deref() != Some(leader)
            || *self.term.lock().unwrap() != term
        {
            return Err("control_leader_fence_rejected".to_string());
        }
        if acknowledgements.intersection(&self.voters).count() < self.majority() {
            return Err("controller_quorum_unavailable".to_string());
        }
        self.log.append(ControlLogEntry {
            index: 0,
            term,
            actor: actor.to_string(),
            reason: reason.to_string(),
            timestamp_unix_millis: unix_millis(),
            actor_sequence: 0,
            mutation,
        })
    }

    pub fn commit_desired_capacity(
        &self,
        leader: &str,
        term: ControlTerm,
        acknowledgements: &BTreeSet<String>,
        actor: &str,
        reason: &str,
        desired: DesiredCapacity,
    ) -> Result<CommittedDesiredCapacity, String> {
        let entry = self.commit(
            leader,
            term,
            acknowledgements,
            actor,
            reason,
            ControlMutation::DesiredCapacity(desired.clone()),
        )?;
        Ok(CommittedDesiredCapacity {
            log_index: entry.index,
            term,
            desired,
        })
    }
}

impl ControlPlaneCommitter for ControllerQuorum {
    fn commit(
        &self,
        leader: &str,
        term: ControlTerm,
        acknowledgements: &BTreeSet<String>,
        actor: &str,
        reason: &str,
        mutation: ControlMutation,
    ) -> Result<ControlLogEntry, String> {
        ControllerQuorum::commit(
            self,
            leader,
            term,
            acknowledgements,
            actor,
            reason,
            mutation,
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommittedDesiredCapacity {
    pub log_index: u64,
    pub term: ControlTerm,
    pub desired: DesiredCapacity,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReconcileNodeSafety {
    /// Provider object identity used for fenced driver operations.
    pub node_id: String,
    /// Authenticated Mesh runtime identity used for routing and continuity.
    pub runtime_node_id: String,
    pub transferable_load: u64,
    pub active_ownership_transfers: u32,
    pub active_work: u32,
    pub required_replica_responsibilities: u32,
    pub only_active_copy: bool,
    pub membership_generation_acknowledged: bool,
    pub controller_voter: bool,
    pub unique_capability: bool,
}

impl ReconcileNodeSafety {
    fn drain_complete(&self) -> bool {
        self.active_work == 0
            && self.required_replica_responsibilities == 0
            && self.membership_generation_acknowledged
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DrainPhase {
    Preparing,
    Draining,
    Terminating,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrainProgress {
    pub node_id: String,
    #[serde(default)]
    pub runtime_node_id: String,
    pub phase: DrainPhase,
    pub desired_revision: DesiredRevision,
    pub control_term: ControlTerm,
    pub drain_operation_id: String,
    pub terminate_operation_id: Option<String>,
    #[serde(default)]
    pub template_revision: String,
    #[serde(default)]
    pub started_at_unix_millis: u64,
    #[serde(default)]
    pub deadline_unix_millis: u64,
    #[serde(default)]
    pub forced_termination: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapacityReconcileOutcome {
    pub desired_workers: u16,
    pub observed_workers: u16,
    pub ensured: Vec<DriverOperation>,
    pub drains: Vec<DrainProgress>,
    pub constraints: Vec<String>,
}

pub struct CapacityReconciler {
    driver: Arc<dyn CapacityDriver>,
    last_log_index: u64,
    highest_term: ControlTerm,
    draining: BTreeMap<String, DrainProgress>,
    max_unavailable: u16,
    require_runtime_continuity_gate: bool,
    drain_timeout_millis: u64,
    force_termination_after_timeout: bool,
    unjoined_ready_since: BTreeMap<String, u64>,
}

impl std::fmt::Debug for CapacityReconciler {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CapacityReconciler")
            .field("last_log_index", &self.last_log_index)
            .field("highest_term", &self.highest_term)
            .field("draining", &self.draining)
            .field("max_unavailable", &self.max_unavailable)
            .field(
                "require_runtime_continuity_gate",
                &self.require_runtime_continuity_gate,
            )
            .finish_non_exhaustive()
    }
}

impl CapacityReconciler {
    pub fn new(driver: Arc<dyn CapacityDriver>, max_unavailable: u16) -> Result<Self, String> {
        if max_unavailable == 0 {
            return Err("capacity_reconciler_disruption_budget_zero".to_string());
        }
        driver.validate_configuration()?;
        Ok(Self {
            driver,
            last_log_index: 0,
            highest_term: ControlTerm(0),
            draining: BTreeMap::new(),
            max_unavailable,
            require_runtime_continuity_gate: false,
            drain_timeout_millis: 300_000,
            force_termination_after_timeout: false,
            unjoined_ready_since: BTreeMap::new(),
        })
    }

    pub(crate) fn new_runtime(
        driver: Arc<dyn CapacityDriver>,
        max_unavailable: u16,
        drain_timeout: Duration,
        force_termination_after_timeout: bool,
    ) -> Result<Self, String> {
        if drain_timeout.is_zero() {
            return Err("capacity_reconciler_drain_timeout_zero".to_string());
        }
        let mut reconciler = Self::new(driver, max_unavailable)?;
        reconciler.require_runtime_continuity_gate = true;
        reconciler.drain_timeout_millis = drain_timeout.as_millis().try_into().unwrap_or(u64::MAX);
        reconciler.force_termination_after_timeout = force_termination_after_timeout;
        Ok(reconciler)
    }

    fn prepare_continuity_for_drain(&self, node_id: &str) -> Result<(), String> {
        if !self.require_runtime_continuity_gate {
            return Ok(());
        }
        crate::dist::node::prepare_continuity_for_drain(node_id).map(|_| ())
    }

    pub fn drain_progress(&self) -> Vec<DrainProgress> {
        self.draining.values().cloned().collect()
    }

    /// Rebuild in-flight provider work from the majority-committed control log.
    /// This is called whenever a controller becomes leader, before it is
    /// allowed to issue provider mutations. Pending operations are retained so
    /// the same idempotency key is retried instead of selecting another node.
    pub fn restore_from_control_entries(&mut self, entries: &[ControlLogEntry]) {
        let mut active_intents = BTreeSet::new();
        let mut intent_started_at = BTreeMap::<String, u64>::new();
        let mut drain_operations: BTreeMap<String, DriverOperation> = BTreeMap::new();
        let mut terminate_operations: BTreeMap<String, DriverOperation> = BTreeMap::new();
        self.last_log_index = 0;
        self.highest_term = ControlTerm(0);
        let mut latest_desired_revision = DesiredRevision(0);

        for entry in entries {
            self.highest_term = self.highest_term.max(entry.term);
            match &entry.mutation {
                ControlMutation::DesiredCapacity(desired)
                    if desired.revision > latest_desired_revision =>
                {
                    latest_desired_revision = desired.revision;
                    self.last_log_index = entry.index;
                }
                ControlMutation::ManualOverride { .. }
                    if DesiredRevision(entry.index) > latest_desired_revision =>
                {
                    latest_desired_revision = DesiredRevision(entry.index);
                    self.last_log_index = entry.index;
                }
                ControlMutation::DrainIntent { node_id, cancelled } => {
                    if *cancelled {
                        active_intents.remove(node_id);
                        intent_started_at.remove(node_id);
                        drain_operations.remove(node_id);
                        terminate_operations.remove(node_id);
                    } else {
                        active_intents.insert(node_id.clone());
                        intent_started_at.insert(node_id.clone(), entry.timestamp_unix_millis);
                        // A fresh intent starts a new attempt even if this stable
                        // identity was drained in an older desired revision.
                        drain_operations.remove(node_id);
                        terminate_operations.remove(node_id);
                    }
                }
                ControlMutation::DriverOperation(operation) => {
                    let Some(node_id) = operation.node_id.as_ref() else {
                        continue;
                    };
                    if !active_intents.contains(node_id) {
                        continue;
                    }
                    if entry.reason.contains("terminate") {
                        terminate_operations.insert(node_id.clone(), operation.clone());
                    } else if entry.reason.contains("drain") {
                        drain_operations.insert(node_id.clone(), operation.clone());
                    }
                }
                _ => {}
            }
        }

        self.draining.clear();
        self.unjoined_ready_since.clear();
        for node_id in active_intents {
            let Some(drain) = drain_operations.get(&node_id) else {
                // The prior leader may have committed the intent and failed
                // before recording a provider operation. Normal candidate
                // selection will deterministically resume this node.
                crate::dist::operator::set_runtime_drain_intent(&node_id, true);
                continue;
            };
            let terminate = terminate_operations.get(&node_id);
            let started_at_unix_millis = intent_started_at
                .get(&node_id)
                .copied()
                .filter(|timestamp| *timestamp > 0)
                .unwrap_or_else(unix_millis);
            let phase = if terminate.is_some() {
                DrainPhase::Terminating
            } else if drain.state == DriverOperationState::Succeeded {
                DrainPhase::Draining
            } else {
                DrainPhase::Preparing
            };
            self.draining.insert(
                node_id.clone(),
                DrainProgress {
                    node_id: node_id.clone(),
                    runtime_node_id: String::new(),
                    phase,
                    desired_revision: terminate.map_or(drain.desired_revision, |operation| {
                        operation.desired_revision
                    }),
                    control_term: terminate
                        .map_or(drain.control_term, |operation| operation.control_term),
                    drain_operation_id: drain.operation_id.clone(),
                    terminate_operation_id: terminate
                        .map(|operation| operation.operation_id.clone()),
                    template_revision: terminate.map_or_else(
                        || drain.template_revision.clone(),
                        |operation| operation.template_revision.clone(),
                    ),
                    started_at_unix_millis,
                    deadline_unix_millis: started_at_unix_millis
                        .saturating_add(self.drain_timeout_millis),
                    forced_termination: terminate.is_some_and(|operation| {
                        entries.iter().any(|entry| {
                            entry
                                .reason
                                .contains("force terminate drain after deadline")
                                && matches!(
                                    &entry.mutation,
                                    ControlMutation::DriverOperation(candidate)
                                        if candidate.operation_id == operation.operation_id
                                )
                        })
                    }),
                },
            );
            crate::dist::operator::set_runtime_drain_intent(&node_id, true);
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn reconcile(
        &mut self,
        quorum: &dyn ControlPlaneCommitter,
        cluster_id: &str,
        leader: &str,
        acknowledgements: &BTreeSet<String>,
        committed: &CommittedDesiredCapacity,
        actor: &str,
        safety: &[ReconcileNodeSafety],
    ) -> Result<CapacityReconcileOutcome, String> {
        self.reconcile_with_unmanaged_capacity(
            quorum,
            cluster_id,
            leader,
            acknowledgements,
            committed,
            actor,
            safety,
            0,
        )
    }

    /// Reconcile a managed pool while counting fixed, externally administered
    /// workers toward the cluster-wide desired capacity.
    #[allow(clippy::too_many_arguments)]
    pub fn reconcile_with_unmanaged_capacity(
        &mut self,
        quorum: &dyn ControlPlaneCommitter,
        cluster_id: &str,
        leader: &str,
        acknowledgements: &BTreeSet<String>,
        committed: &CommittedDesiredCapacity,
        actor: &str,
        safety: &[ReconcileNodeSafety],
        unmanaged_ready_workers: u16,
    ) -> Result<CapacityReconcileOutcome, String> {
        let safety = safety.to_vec();
        self.reconcile_with_capacity_safety(
            quorum,
            cluster_id,
            leader,
            acknowledgements,
            committed,
            actor,
            move |_| Ok((safety, unmanaged_ready_workers)),
        )
    }

    /// Derive runtime safety from the same fenced provider snapshot used for
    /// reconciliation. This prevents a slow provider from consuming two API
    /// deadlines per controller tick while retaining the leader fence before
    /// observation and every destructive continuity gate afterward.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn reconcile_with_observed_capacity<F>(
        &mut self,
        quorum: &dyn ControlPlaneCommitter,
        cluster_id: &str,
        leader: &str,
        acknowledgements: &BTreeSet<String>,
        committed: &CommittedDesiredCapacity,
        actor: &str,
        safety_for_observation: F,
    ) -> Result<CapacityReconcileOutcome, String>
    where
        F: FnOnce(&CapacityObservation) -> Result<(Vec<ReconcileNodeSafety>, u16), String>,
    {
        self.reconcile_with_capacity_safety(
            quorum,
            cluster_id,
            leader,
            acknowledgements,
            committed,
            actor,
            safety_for_observation,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn reconcile_with_capacity_safety<F>(
        &mut self,
        quorum: &dyn ControlPlaneCommitter,
        cluster_id: &str,
        leader: &str,
        acknowledgements: &BTreeSet<String>,
        committed: &CommittedDesiredCapacity,
        actor: &str,
        safety_for_observation: F,
    ) -> Result<CapacityReconcileOutcome, String>
    where
        F: FnOnce(&CapacityObservation) -> Result<(Vec<ReconcileNodeSafety>, u16), String>,
    {
        if committed.log_index < self.last_log_index || committed.term < self.highest_term {
            return Err("capacity_reconciler_stale_committed_state".to_string());
        }
        if committed.desired.revision.0 == 0 || committed.desired.worker_nodes == 0 {
            return Err("capacity_reconciler_desired_state_invalid".to_string());
        }
        if cluster_id.is_empty() {
            return Err("capacity_reconciler_cluster_id_missing".to_string());
        }
        // A no-op fenced commit proves that the caller is still the majority-backed leader
        // before any provider observation or mutation is used for reconciliation.
        quorum.commit(
            leader,
            committed.term,
            acknowledgements,
            actor,
            "capacity reconciliation fence",
            ControlMutation::DesiredCapacity(committed.desired.clone()),
        )?;
        self.last_log_index = committed.log_index;
        self.highest_term = committed.term;

        let observation = self.driver.observe_capacity(cluster_id)?;
        let (safety, unmanaged_ready_workers) = safety_for_observation(&observation)?;
        // A drain restored from the control log knows its provider node but
        // not the runtime member it is: that node's safety entry does.
        for progress in self.draining.values_mut() {
            if progress.runtime_node_id.is_empty() {
                if let Some(candidate) = safety.iter().find(|candidate| {
                    candidate.node_id == progress.node_id && !candidate.runtime_node_id.is_empty()
                }) {
                    progress.runtime_node_id = candidate.runtime_node_id.clone();
                }
            }
        }
        let mut active: Vec<_> = observation
            .nodes
            .iter()
            .filter(|node| {
                !matches!(
                    node.lifecycle,
                    CapacityNodeLifecycle::Removed | CapacityNodeLifecycle::Failed
                )
            })
            .cloned()
            .collect();
        active.sort_by(|left, right| left.node_id.cmp(&right.node_id));
        let managed_workers: u16 = active.len().try_into().unwrap_or(u16::MAX);
        let observed_workers = managed_workers.saturating_add(unmanaged_ready_workers);
        let mut outcome = CapacityReconcileOutcome {
            desired_workers: committed.desired.worker_nodes,
            observed_workers,
            ensured: Vec::new(),
            drains: self.drain_progress(),
            constraints: Vec::new(),
        };

        self.finish_removed_drains(quorum, leader, acknowledgements, committed, actor, &active)?;

        // A provider object can exist without ever becoming a Ready runtime
        // member (bad image, failed health check, crash during warm-up). It is
        // still inside the exact managed-label boundary, so reconcile it as a
        // fenced orphan before issuing replacement capacity. Unlabeled or
        // mismatched objects never appear in this observation and are not
        // eligible for destructive adoption.
        if let Some(failed) = observation
            .nodes
            .iter()
            .find(|node| node.lifecycle == CapacityNodeLifecycle::Failed)
        {
            let operation = DriverOperation {
                cluster_id: cluster_id.to_string(),
                operation_id: capacity_operation_id(
                    cluster_id,
                    committed.desired.revision,
                    stable_ordinal(&failed.node_id),
                    &format!("cleanup-failed:{}", failed.node_id),
                ),
                control_term: committed.term,
                desired_revision: committed.desired.revision,
                template_revision: failed.template_revision.clone(),
                node_id: Some(failed.node_id.clone()),
                state: DriverOperationState::Pending,
            };
            quorum.commit(
                leader,
                committed.term,
                acknowledgements,
                actor,
                "clean up failed managed worker",
                ControlMutation::DriverOperation(operation.clone()),
            )?;
            let result = self.driver.terminate_node(&operation, &failed.node_id)?;
            quorum.commit(
                leader,
                committed.term,
                acknowledgements,
                actor,
                "record failed managed worker cleanup result",
                ControlMutation::DriverOperation(result.clone()),
            )?;
            if result.state != DriverOperationState::Succeeded {
                outcome.constraints.push(format!(
                    "capacity_failed_worker_cleanup_incomplete:{}:{:?}",
                    failed.node_id, result.state
                ));
            }
            return Ok(outcome);
        }

        if self.require_runtime_continuity_gate {
            let now = unix_millis();
            let unjoined_ready: BTreeSet<_> = active
                .iter()
                .filter(|node| node.lifecycle == CapacityNodeLifecycle::Ready)
                .filter(|node| {
                    safety
                        .iter()
                        .find(|candidate| candidate.node_id == node.node_id)
                        .is_some_and(|candidate| candidate.runtime_node_id.is_empty())
                })
                .map(|node| node.node_id.clone())
                .collect();
            self.unjoined_ready_since
                .retain(|node_id, _| unjoined_ready.contains(node_id));
            for node_id in &unjoined_ready {
                self.unjoined_ready_since
                    .entry(node_id.clone())
                    .or_insert(now);
            }
            if let Some(node) = active.iter().find(|node| {
                self.unjoined_ready_since
                    .get(&node.node_id)
                    .is_some_and(|started| {
                        now.saturating_sub(*started) >= self.drain_timeout_millis
                    })
            }) {
                // The provider object crossed its Ready boundary but never
                // joined authenticated Mesh membership. It cannot own admitted
                // work, so a drain would be both impossible and unnecessary.
                // Terminate only after the bounded join grace and only through
                // the normal fenced managed-resource operation.
                let operation = DriverOperation {
                    cluster_id: cluster_id.to_string(),
                    operation_id: capacity_operation_id(
                        cluster_id,
                        committed.desired.revision,
                        stable_ordinal(&node.node_id),
                        &format!("cleanup-unjoined:{}", node.node_id),
                    ),
                    control_term: committed.term,
                    desired_revision: committed.desired.revision,
                    template_revision: node.template_revision.clone(),
                    node_id: Some(node.node_id.clone()),
                    state: DriverOperationState::Pending,
                };
                quorum.commit(
                    leader,
                    committed.term,
                    acknowledgements,
                    actor,
                    "clean up managed worker that never joined runtime membership",
                    ControlMutation::DriverOperation(operation.clone()),
                )?;
                let result = self.driver.terminate_node(&operation, &node.node_id)?;
                quorum.commit(
                    leader,
                    committed.term,
                    acknowledgements,
                    actor,
                    "record unjoined managed worker cleanup result",
                    ControlMutation::DriverOperation(result.clone()),
                )?;
                if result.state == DriverOperationState::Succeeded {
                    self.unjoined_ready_since.remove(&node.node_id);
                } else {
                    outcome.constraints.push(format!(
                        "capacity_unjoined_worker_cleanup_incomplete:{}:{:?}",
                        node.node_id, result.state
                    ));
                }
                return Ok(outcome);
            }
        }

        if let Some(progress) = self
            .draining
            .values()
            .find(|progress| progress.phase == DrainPhase::Preparing)
            .cloned()
        {
            let node_id = progress.node_id.clone();
            let operation = DriverOperation {
                cluster_id: cluster_id.to_string(),
                operation_id: progress.drain_operation_id,
                control_term: progress.control_term,
                desired_revision: progress.desired_revision,
                template_revision: progress.template_revision,
                node_id: Some(node_id.clone()),
                state: DriverOperationState::Pending,
            };
            let result = self.driver.begin_drain(&operation, &node_id)?;
            quorum.commit(
                leader,
                committed.term,
                acknowledgements,
                actor,
                "record resumed begin drain result",
                ControlMutation::DriverOperation(result.clone()),
            )?;
            if result.state == DriverOperationState::Succeeded {
                let runtime_node_id = progress.runtime_node_id;
                if runtime_node_id.is_empty() {
                    return Err(format!("capacity_drain_runtime_missing:{node_id}"));
                }
                crate::dist::operator::prepare_committed_drain(&runtime_node_id);
                match self.prepare_continuity_for_drain(&runtime_node_id) {
                    Ok(_) => {
                        if let Some(progress) = self.draining.get_mut(&node_id) {
                            progress.phase = DrainPhase::Draining;
                        }
                    }
                    Err(reason) => outcome.constraints.push(format!(
                        "capacity_drain_continuity_not_ready:{node_id}:{reason}"
                    )),
                }
            } else {
                outcome.constraints.push(format!(
                    "capacity_drain_not_ready:{}:{:?}",
                    result.operation_id, result.state
                ));
            }
            outcome.drains = self.drain_progress();
            return Ok(outcome);
        }

        if let Some(progress) = self
            .draining
            .values()
            .find(|progress| {
                progress.phase == DrainPhase::Terminating
                    && active.iter().any(|node| node.node_id == progress.node_id)
            })
            .cloned()
        {
            let node_id = progress.node_id.clone();
            let operation = DriverOperation {
                cluster_id: cluster_id.to_string(),
                operation_id: progress
                    .terminate_operation_id
                    .expect("a terminating drain records its terminate operation"),
                control_term: progress.control_term,
                desired_revision: progress.desired_revision,
                template_revision: progress.template_revision,
                node_id: Some(node_id.clone()),
                state: DriverOperationState::Pending,
            };
            let result = self.driver.terminate_node(&operation, &node_id)?;
            quorum.commit(
                leader,
                committed.term,
                acknowledgements,
                actor,
                "record resumed terminate worker result",
                ControlMutation::DriverOperation(result.clone()),
            )?;
            if result.state != DriverOperationState::Succeeded {
                outcome.constraints.push(format!(
                    "capacity_termination_not_complete:{}:{:?}",
                    result.operation_id, result.state
                ));
            }
            outcome.drains = self.drain_progress();
            return Ok(outcome);
        }

        if observed_workers < committed.desired.worker_nodes {
            self.cancel_pretermination_drains(quorum, leader, acknowledgements, committed, actor)?;
            let missing = committed.desired.worker_nodes - observed_workers;
            for ordinal in 0..u16::MAX {
                if outcome.ensured.len() >= missing as usize {
                    break;
                }
                let mut operation = DriverOperation {
                    cluster_id: cluster_id.to_string(),
                    operation_id: capacity_operation_id(
                        cluster_id,
                        committed.desired.revision,
                        ordinal,
                        "ensure",
                    ),
                    control_term: committed.term,
                    desired_revision: committed.desired.revision,
                    template_revision: committed.desired.template_revision.clone(),
                    node_id: None,
                    state: DriverOperationState::Pending,
                };
                let mut lineage_depth = 0_u8;
                loop {
                    let Some(existing) = self.driver.get_operation(&operation.operation_id)? else {
                        break;
                    };
                    match existing.state {
                        DriverOperationState::Succeeded => {
                            let active_node = existing.node_id.as_ref().is_some_and(|node_id| {
                                active.iter().any(|node| &node.node_id == node_id)
                            });
                            if active_node {
                                break;
                            }
                            let removed_node = existing.node_id.as_deref().unwrap_or("unknown");
                            lineage_depth = lineage_depth.saturating_add(1);
                            if lineage_depth > 64 {
                                return Err("capacity_replacement_lineage_exhausted".to_string());
                            }
                            operation.operation_id = capacity_operation_id(
                                cluster_id,
                                committed.desired.revision,
                                ordinal,
                                &format!("replace:{removed_node}"),
                            );
                        }
                        DriverOperationState::Pending => break,
                        DriverOperationState::PermanentFailure(ref reason) => {
                            outcome.constraints.push(format!(
                                "capacity_operation_permanent_failure:{}:{reason}",
                                operation.operation_id
                            ));
                            break;
                        }
                        DriverOperationState::RetryableFailure(_)
                        | DriverOperationState::Unknown => {
                            break;
                        }
                    }
                }
                if self
                    .driver
                    .get_operation(&operation.operation_id)?
                    .is_some_and(|existing| {
                        matches!(
                            existing.state,
                            DriverOperationState::Succeeded
                                | DriverOperationState::Pending
                                | DriverOperationState::PermanentFailure(_)
                        )
                    })
                {
                    continue;
                }
                quorum.commit(
                    leader,
                    committed.term,
                    acknowledgements,
                    actor,
                    "ensure worker capacity",
                    ControlMutation::DriverOperation(operation.clone()),
                )?;
                let result = self.driver.ensure_node(&operation)?;
                quorum.commit(
                    leader,
                    committed.term,
                    acknowledgements,
                    actor,
                    "record ensure worker result",
                    ControlMutation::DriverOperation(result.clone()),
                )?;
                if let DriverOperationState::RetryableFailure(reason) = &result.state {
                    outcome.constraints.push(format!(
                        "capacity_operation_retryable:{}:{reason}",
                        result.operation_id
                    ));
                }
                if let DriverOperationState::PermanentFailure(reason) = &result.state {
                    outcome.constraints.push(format!(
                        "capacity_operation_permanent_failure:{}:{reason}",
                        result.operation_id
                    ));
                }
                outcome.ensured.push(result);
            }
            if outcome.ensured.len() < missing as usize {
                outcome
                    .constraints
                    .push("capacity_pending_existing_operations".to_string());
            }
            outcome.drains = self.drain_progress();
            return Ok(outcome);
        }

        if observed_workers <= committed.desired.worker_nodes {
            self.cancel_pretermination_drains(quorum, leader, acknowledgements, committed, actor)?;
            outcome.drains = self.drain_progress();
            return Ok(outcome);
        }

        if let Some(node_id) = self.draining.iter().find_map(|(node_id, progress)| {
            (progress.phase == DrainPhase::Draining).then_some(node_id.clone())
        }) {
            let gate = safety.iter().find(|candidate| candidate.node_id == node_id);
            let timed_out = self.draining.get(&node_id).is_some_and(|progress| {
                progress.deadline_unix_millis > 0 && unix_millis() >= progress.deadline_unix_millis
            });
            let safe_to_force = gate.is_some_and(|candidate| {
                candidate.required_replica_responsibilities == 0
                    && !candidate.only_active_copy
                    && candidate.membership_generation_acknowledged
            });
            let force = timed_out && self.force_termination_after_timeout && safe_to_force;
            match gate {
                Some(gate) if gate.drain_complete() || force => {
                    if force {
                        let runtime_node_id = self
                            .draining
                            .get(&node_id)
                            .map(|progress| progress.runtime_node_id.as_str())
                            .filter(|runtime| !runtime.is_empty())
                            .ok_or_else(|| {
                                format!("forced_drain_runtime_identity_missing:{node_id}")
                            })?;
                        let fenced = crate::dist::continuity::continuity_registry()
                            .mark_owner_loss_records_for_node_loss(runtime_node_id)
                            .len();
                        crate::dist::node::recover_pending_owner_losses_if_coordinator();
                        outcome.constraints.push(format!(
                            "forced_termination_after_drain_timeout:{node_id}:active_work={}:fenced_records={fenced}",
                            gate.active_work
                        ));
                    }
                    let operation = DriverOperation {
                        cluster_id: cluster_id.to_string(),
                        operation_id: capacity_operation_id(
                            cluster_id,
                            committed.desired.revision,
                            stable_ordinal(&node_id),
                            "terminate",
                        ),
                        control_term: committed.term,
                        desired_revision: committed.desired.revision,
                        template_revision: committed.desired.template_revision.clone(),
                        node_id: Some(node_id.clone()),
                        state: DriverOperationState::Pending,
                    };
                    quorum.commit(
                        leader,
                        committed.term,
                        acknowledgements,
                        actor,
                        if force {
                            "force terminate drain after deadline with replicated continuity fence"
                        } else {
                            "terminate drained worker"
                        },
                        ControlMutation::DriverOperation(operation.clone()),
                    )?;
                    if let Some(progress) = self.draining.get_mut(&node_id) {
                        progress.phase = DrainPhase::Terminating;
                        progress.control_term = operation.control_term;
                        progress.desired_revision = operation.desired_revision;
                        progress.terminate_operation_id = Some(operation.operation_id.clone());
                        progress.template_revision = operation.template_revision.clone();
                        progress.forced_termination = force;
                    }
                    let result = self.driver.terminate_node(&operation, &node_id)?;
                    quorum.commit(
                        leader,
                        committed.term,
                        acknowledgements,
                        actor,
                        "record terminate worker result",
                        ControlMutation::DriverOperation(result.clone()),
                    )?;
                    if result.state != DriverOperationState::Succeeded {
                        outcome.constraints.push(format!(
                            "capacity_termination_not_complete:{}:{:?}",
                            result.operation_id, result.state
                        ));
                    }
                }
                Some(_) if timed_out && !self.force_termination_after_timeout => {
                    outcome.constraints.push(format!(
                        "drain_timeout_manual_intervention_required:{node_id}"
                    ));
                }
                Some(_) if timed_out => {
                    outcome.constraints.push(format!(
                        "drain_timeout_force_blocked_by_continuity_or_membership:{node_id}"
                    ));
                }
                Some(_) => outcome
                    .constraints
                    .push(format!("drain_gates_incomplete:{node_id}")),
                None => outcome
                    .constraints
                    .push(format!("drain_safety_missing:{node_id}")),
            }
            outcome.drains = self.drain_progress();
            return Ok(outcome);
        }

        // Every drain in progress returned above (a terminated one's node
        // is gone and it was finished), so none is: one node drains at a
        // time, and the budget allows at least one.
        let candidates: Vec<_> = safety
            .iter()
            .filter(|candidate| active.iter().any(|node| node.node_id == candidate.node_id))
            .map(|candidate| DrainCandidate {
                node_id: candidate.node_id.clone(),
                transferable_load: candidate.transferable_load,
                active_ownership_transfers: candidate.active_ownership_transfers,
                controller_voter: candidate.controller_voter,
                only_active_copy: candidate.only_active_copy,
                unique_capability: candidate.unique_capability,
                template_revision: active
                    .iter()
                    .find(|node| node.node_id == candidate.node_id)
                    .map(|node| node.template_revision.clone())
                    .unwrap_or_default(),
            })
            .collect();
        let selected = match select_drain_candidate(
            &candidates,
            self.draining.len() as u16,
            self.max_unavailable,
        ) {
            Ok(selected) => selected,
            // Every worker is needed where it is; scale-down waits.
            Err(reason) => {
                outcome.constraints.push(reason);
                outcome.drains = self.drain_progress();
                return Ok(outcome);
            }
        };
        let selected_runtime_node_id = safety
            .iter()
            .find(|candidate| candidate.node_id == selected.node_id)
            .map(|candidate| candidate.runtime_node_id.clone())
            .filter(|runtime| !runtime.is_empty())
            .ok_or_else(|| format!("capacity_drain_runtime_missing:{}", selected.node_id))?;
        let operation = DriverOperation {
            cluster_id: cluster_id.to_string(),
            operation_id: capacity_operation_id(
                cluster_id,
                committed.desired.revision,
                stable_ordinal(&selected.node_id),
                "drain",
            ),
            control_term: committed.term,
            desired_revision: committed.desired.revision,
            template_revision: committed.desired.template_revision.clone(),
            node_id: Some(selected.node_id.clone()),
            state: DriverOperationState::Pending,
        };
        quorum.commit(
            leader,
            committed.term,
            acknowledgements,
            actor,
            "begin worker drain",
            ControlMutation::DrainIntent {
                node_id: selected.node_id.clone(),
                cancelled: false,
            },
        )?;
        quorum.commit(
            leader,
            committed.term,
            acknowledgements,
            actor,
            "begin driver drain",
            ControlMutation::DriverOperation(operation.clone()),
        )?;
        self.draining.insert(selected.node_id.clone(), {
            let started_at_unix_millis = unix_millis();
            DrainProgress {
                node_id: selected.node_id.clone(),
                runtime_node_id: selected_runtime_node_id.clone(),
                phase: DrainPhase::Preparing,
                desired_revision: committed.desired.revision,
                control_term: committed.term,
                drain_operation_id: operation.operation_id.clone(),
                terminate_operation_id: None,
                template_revision: committed.desired.template_revision.clone(),
                started_at_unix_millis,
                deadline_unix_millis: started_at_unix_millis
                    .saturating_add(self.drain_timeout_millis),
                forced_termination: false,
            }
        });
        // Stop new routing before asking the provider/runtime to prepare the
        // node. The intent is durable and the operation remains Preparing
        // until every active continuity record has a safe replacement.
        crate::dist::operator::prepare_committed_drain(&selected_runtime_node_id);
        let driver_result = self.driver.begin_drain(&operation, &selected.node_id)?;
        quorum.commit(
            leader,
            committed.term,
            acknowledgements,
            actor,
            "record begin drain result",
            ControlMutation::DriverOperation(driver_result.clone()),
        )?;
        if driver_result.state != DriverOperationState::Succeeded {
            outcome.constraints.push(format!(
                "capacity_drain_not_ready:{}:{:?}",
                driver_result.operation_id, driver_result.state
            ));
            outcome.drains = self.drain_progress();
            return Ok(outcome);
        }
        match self.prepare_continuity_for_drain(&selected_runtime_node_id) {
            Ok(_) => {
                if let Some(progress) = self.draining.get_mut(&selected.node_id) {
                    progress.phase = DrainPhase::Draining;
                }
            }
            Err(reason) => outcome.constraints.push(format!(
                "capacity_drain_continuity_not_ready:{}:{reason}",
                selected.node_id
            )),
        }
        outcome.drains = self.drain_progress();
        Ok(outcome)
    }

    /// A terminated node the provider no longer has is drained: its intent
    /// ends in the control log too, or every later leader would resume the
    /// drain and every operator would see the node draining for good.
    fn finish_removed_drains(
        &mut self,
        quorum: &dyn ControlPlaneCommitter,
        leader: &str,
        acknowledgements: &BTreeSet<String>,
        committed: &CommittedDesiredCapacity,
        actor: &str,
        active: &[ObservedCapacityNode],
    ) -> Result<(), String> {
        let removed: Vec<_> = self
            .draining
            .iter()
            .filter(|(node_id, progress)| {
                progress.phase == DrainPhase::Terminating
                    && !active.iter().any(|node| &node.node_id == *node_id)
            })
            .map(|(node_id, progress)| (node_id.clone(), progress.runtime_node_id.clone()))
            .collect();
        for (node_id, runtime_node_id) in removed {
            quorum.commit(
                leader,
                committed.term,
                acknowledgements,
                actor,
                "finish drain of removed worker",
                ControlMutation::DrainIntent {
                    node_id: node_id.clone(),
                    cancelled: true,
                },
            )?;
            // The fence this leader set on the node's runtime name.
            if !runtime_node_id.is_empty() {
                crate::dist::operator::set_runtime_drain_intent(&runtime_node_id, false);
            }
            self.draining.remove(&node_id);
        }
        Ok(())
    }

    fn cancel_pretermination_drains(
        &mut self,
        quorum: &dyn ControlPlaneCommitter,
        leader: &str,
        acknowledgements: &BTreeSet<String>,
        committed: &CommittedDesiredCapacity,
        actor: &str,
    ) -> Result<(), String> {
        let cancellable: Vec<_> = self
            .draining
            .iter()
            .filter_map(|(node_id, progress)| {
                matches!(progress.phase, DrainPhase::Preparing | DrainPhase::Draining)
                    .then_some((node_id.clone(), progress.runtime_node_id.clone()))
            })
            .collect();
        for (node_id, runtime_node_id) in cancellable {
            quorum.commit(
                leader,
                committed.term,
                acknowledgements,
                actor,
                "cancel worker drain after desired capacity rebound",
                ControlMutation::DrainIntent {
                    node_id: node_id.clone(),
                    cancelled: true,
                },
            )?;
            crate::dist::operator::cancel_committed_drain(if runtime_node_id.is_empty() {
                &node_id
            } else {
                runtime_node_id.as_str()
            });
            self.draining.remove(&node_id);
        }
        Ok(())
    }
}

fn stable_ordinal(value: &str) -> u16 {
    let digest = Sha256::digest(value.as_bytes());
    u16::from_be_bytes([digest[0], digest[1]])
}

pub fn capacity_operation_id(
    cluster_id: &str,
    revision: DesiredRevision,
    ordinal: u16,
    action: &str,
) -> String {
    let mut hasher = Sha256::new();
    let revision_bytes = revision.0.to_be_bytes();
    let ordinal_bytes = ordinal.to_be_bytes();
    for component in [
        cluster_id.as_bytes(),
        &revision_bytes,
        &ordinal_bytes,
        action.as_bytes(),
    ] {
        hasher.update((component.len() as u64).to_be_bytes());
        hasher.update(component);
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DrainCandidate {
    pub node_id: String,
    pub transferable_load: u64,
    pub active_ownership_transfers: u32,
    pub controller_voter: bool,
    pub only_active_copy: bool,
    pub unique_capability: bool,
    pub template_revision: String,
}

pub fn select_drain_candidate(
    candidates: &[DrainCandidate],
    unavailable: u16,
    max_unavailable: u16,
) -> Result<DrainCandidate, String> {
    if unavailable >= max_unavailable {
        return Err("drain_disruption_budget_exhausted".to_string());
    }
    candidates
        .iter()
        .filter(|candidate| {
            !candidate.controller_voter
                && !candidate.only_active_copy
                && !candidate.unique_capability
        })
        .min_by_key(|candidate| {
            (
                candidate.transferable_load,
                candidate.active_ownership_transfers,
                candidate.node_id.clone(),
            )
        })
        .cloned()
        .ok_or_else(|| "drain_no_safe_candidate".to_string())
}

pub fn reconcile_scale_up(
    driver: &dyn CapacityDriver,
    cluster_id: &str,
    term: ControlTerm,
    desired: &DesiredCapacity,
    observed_workers: u16,
) -> Result<Vec<DriverOperation>, String> {
    if observed_workers >= desired.worker_nodes {
        return Ok(Vec::new());
    }
    let mut operations = Vec::new();
    for ordinal in observed_workers..desired.worker_nodes {
        let operation = DriverOperation {
            cluster_id: cluster_id.to_string(),
            operation_id: capacity_operation_id(cluster_id, desired.revision, ordinal, "ensure"),
            control_term: term,
            desired_revision: desired.revision,
            template_revision: desired.template_revision.clone(),
            node_id: None,
            state: DriverOperationState::Pending,
        };
        operations.push(driver.ensure_node(&operation)?);
    }
    Ok(operations)
}

fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn driver_operation(operation_id: &str, action_node: Option<&str>) -> DriverOperation {
        DriverOperation {
            cluster_id: "cluster-a".to_string(),
            operation_id: operation_id.to_string(),
            control_term: ControlTerm(3),
            desired_revision: DesiredRevision(7),
            template_revision: "template-v1".to_string(),
            node_id: action_node.map(str::to_string),
            state: DriverOperationState::Pending,
        }
    }

    fn high_sample(observed_at: Instant) -> ScalingSample {
        ScalingSample {
            observed_at,
            cluster_inflight: 600,
            cluster_pressure_ewma: 2.0,
            ready_nodes: 2,
            reports_complete: true,
            driver_healthy: true,
            controller_stable: true,
            continuity_healthy: true,
            drain_incomplete: false,
        }
    }

    fn low_sample(observed_at: Instant) -> ScalingSample {
        ScalingSample {
            observed_at,
            cluster_inflight: 1,
            cluster_pressure_ewma: 0.1,
            ready_nodes: 5,
            reports_complete: true,
            driver_healthy: true,
            controller_stable: true,
            continuity_healthy: true,
            drain_incomplete: false,
        }
    }

    #[test]
    fn process_driver_debug_redacts_arguments_and_environment_values() {
        let config = ProcessDriverConfig {
            command: vec![
                "mesh-worker".to_string(),
                "--token=argument-secret".to_string(),
            ],
            working_directory: PathBuf::from("."),
            environment: BTreeMap::from([
                (
                    "DATABASE_URL".to_string(),
                    "postgres://environment-secret".to_string(),
                ),
                ("MESH_ROLES".to_string(), "worker".to_string()),
            ]),
        };

        let rendered = format!("{config:?}");

        assert!(rendered.contains("mesh-worker"));
        assert!(rendered.contains("argument_count: 1"));
        assert!(rendered.contains("DATABASE_URL"));
        assert!(!rendered.contains("argument-secret"));
        assert!(!rendered.contains("environment-secret"));
        assert!(!rendered.contains("postgres://"));
        assert!(rendered.contains("[redacted; 2]"));
    }

    #[cfg(unix)]
    #[test]
    fn process_driver_uses_idempotent_argv_lifecycle_without_shell_interpolation() {
        let directory = tempfile::tempdir().expect("process driver working directory");
        let driver = ProcessCapacityDriver::new(ProcessDriverConfig {
            command: vec![
                "sh".to_string(),
                "-c".to_string(),
                "while :; do sleep 1; done".to_string(),
            ],
            working_directory: directory.path().to_path_buf(),
            environment: BTreeMap::from([("MESH_ROLES".to_string(), "worker".to_string())]),
        });
        let ensure = driver_operation("process-ensure-operation", None);

        let first = driver.ensure_node(&ensure).expect("spawn managed process");
        let second = driver
            .ensure_node(&ensure)
            .expect("idempotent process ensure");
        let node_id = first.node_id.clone().expect("managed process id");
        let observed_before = driver
            .observe_capacity("cluster-a")
            .expect("observe managed process");
        let drain = driver_operation("process-drain-operation", Some(&node_id));
        let drained = driver
            .begin_drain(&drain, &node_id)
            .expect("record process drain");
        let terminate = driver_operation("process-terminate-operation", Some(&node_id));
        let terminated = driver
            .terminate_node(&terminate, &node_id)
            .expect("terminate managed process");
        let observed_after = driver
            .observe_capacity("cluster-a")
            .expect("observe removed process");

        assert_eq!(first, second);
        assert_eq!(observed_before.nodes.len(), 1);
        assert_eq!(observed_before.nodes[0].node_id, node_id);
        assert_eq!(
            observed_before.nodes[0].lifecycle,
            CapacityNodeLifecycle::Ready
        );
        assert_eq!(drained.state, DriverOperationState::Succeeded);
        assert_eq!(terminated.state, DriverOperationState::Succeeded);
        assert!(observed_after.nodes.is_empty());
        assert_eq!(
            driver
                .get_operation("process-terminate-operation")
                .expect("lookup process terminate operation"),
            Some(terminated)
        );
    }

    #[test]
    fn autoscaler_applies_scale_up_stabilization_and_step_bound() {
        let policy = ScalingPolicy {
            scale_up_window_millis: 1_000,
            scale_down_window_millis: 10_000,
            cooldown_millis: 0,
            max_scale_up_step: 2,
            ..ScalingPolicy::default()
        };
        let mut autoscaler = Autoscaler::new(policy).expect("valid policy");
        let start = Instant::now();
        assert_eq!(
            autoscaler.evaluate(2, high_sample(start)).action,
            ScalingAction::Hold
        );

        let decision = autoscaler.evaluate(2, low_sample(start + Duration::from_secs(1)));
        assert_eq!(decision.action, ScalingAction::ScaleUp);
        assert_eq!(decision.bounded_desired, 4);
    }

    #[test]
    fn autoscaler_scale_up_window_retains_peak_across_sampling_boundary() {
        let policy = ScalingPolicy {
            scale_up_window_millis: 1_000,
            scale_down_window_millis: 10_000,
            cooldown_millis: 0,
            ..ScalingPolicy::default()
        };
        let mut autoscaler = Autoscaler::new(policy).expect("valid policy");
        let start = Instant::now();

        autoscaler.evaluate(2, high_sample(start));
        autoscaler.evaluate(2, low_sample(start + Duration::from_millis(500)));
        let decision = autoscaler.evaluate(2, low_sample(start + Duration::from_secs(1)));

        assert_eq!(decision.action, ScalingAction::ScaleUp);
        assert!(decision.raw_desired > 2);
    }

    #[test]
    fn autoscaler_freezes_scale_down_when_telemetry_is_missing() {
        let policy = ScalingPolicy {
            scale_up_window_millis: 100,
            scale_down_window_millis: 1_000,
            cooldown_millis: 0,
            ..ScalingPolicy::default()
        };
        let mut autoscaler = Autoscaler::new(policy).expect("valid policy");
        let start = Instant::now();
        autoscaler.evaluate(5, low_sample(start));
        let mut missing = low_sample(start + Duration::from_secs(1));
        missing.reports_complete = false;

        let decision = autoscaler.evaluate(5, missing);
        assert_eq!(decision.action, ScalingAction::Frozen);
        assert!(decision
            .constraints
            .contains(&"scale_down_reports_incomplete".to_string()));
    }

    #[test]
    fn autoscaler_action_gates_keep_observe_only_decisions_non_mutating() {
        let policy = ScalingPolicy {
            scale_up_window_millis: 1,
            scale_down_window_millis: 2,
            cooldown_millis: 0,
            ..ScalingPolicy::default()
        };
        let mut autoscaler = Autoscaler::new(policy).expect("valid policy");
        autoscaler.set_action_gates(false, false);
        let start = Instant::now();
        autoscaler.evaluate(2, high_sample(start));
        let up = autoscaler.evaluate(2, high_sample(start + Duration::from_millis(3)));
        assert_eq!(up.bounded_desired, 2);
        assert!(up.constraints.contains(&"scale_up_disabled".to_string()));

        let mut autoscaler = Autoscaler::new(ScalingPolicy {
            scale_up_window_millis: 1,
            scale_down_window_millis: 2,
            cooldown_millis: 0,
            ..ScalingPolicy::default()
        })
        .expect("valid policy");
        autoscaler.set_action_gates(false, false);
        autoscaler.evaluate(5, low_sample(start));
        let down = autoscaler.evaluate(5, low_sample(start + Duration::from_millis(3)));
        assert_eq!(down.bounded_desired, 5);
        assert!(down
            .constraints
            .contains(&"scale_down_disabled".to_string()));
    }

    #[test]
    fn duplicate_driver_operation_creates_one_node() {
        let driver = FakeCapacityDriver::new();
        let desired = DesiredCapacity {
            revision: DesiredRevision(1),
            worker_nodes: 1,
            gateway_nodes: 0,
            template_revision: "v1".to_string(),
        };
        let first = reconcile_scale_up(&driver, "cluster", ControlTerm(1), &desired, 0)
            .expect("first reconciliation");
        let second = reconcile_scale_up(&driver, "cluster", ControlTerm(1), &desired, 0)
            .expect("retry reconciliation");

        assert_eq!(first, second);
        assert_eq!(
            driver
                .observe_capacity("cluster")
                .expect("observe")
                .nodes
                .len(),
            1
        );
    }

    #[test]
    fn reconciler_removes_failed_managed_orphan_before_replacement() {
        let directory = tempfile::tempdir().expect("tempdir");
        let log =
            Arc::new(DurableControlLog::open(&directory.path().join("control.log")).expect("log"));
        let voters = BTreeSet::from(["a".to_string(), "b".to_string(), "c".to_string()]);
        let quorum = ControllerQuorum::new(voters.clone(), log).expect("quorum");
        let term = quorum.elect("a", &voters).expect("leader");
        let driver = Arc::new(FakeCapacityDriver::new());
        driver.nodes.lock().unwrap().insert(
            "failed-node".to_string(),
            ObservedCapacityNode {
                node_id: "failed-node".to_string(),
                operation_id: "initial-create".to_string(),
                control_term: term,
                desired_revision: DesiredRevision(1),
                template_revision: "v1".to_string(),
                lifecycle: CapacityNodeLifecycle::Failed,
            },
        );
        let committed = quorum
            .commit_desired_capacity(
                "a",
                term,
                &voters,
                "autoscaler",
                "desired minimum",
                DesiredCapacity {
                    revision: DesiredRevision(1),
                    worker_nodes: 1,
                    gateway_nodes: 0,
                    template_revision: "v1".to_string(),
                },
            )
            .expect("desired capacity");
        let mut reconciler =
            CapacityReconciler::new(driver.clone(), 1).expect("capacity reconciler");

        let cleanup = reconciler
            .reconcile(
                &quorum,
                "cluster",
                "a",
                &voters,
                &committed,
                "autoscaler",
                &[],
            )
            .expect("failed worker cleanup");
        assert!(cleanup.ensured.is_empty());
        assert_eq!(
            driver.nodes.lock().unwrap()["failed-node"].lifecycle,
            CapacityNodeLifecycle::Removed
        );

        let replacement = reconciler
            .reconcile(
                &quorum,
                "cluster",
                "a",
                &voters,
                &committed,
                "autoscaler",
                &[],
            )
            .expect("replacement capacity");
        assert_eq!(replacement.ensured.len(), 1);
        assert_eq!(replacement.observed_workers, 0);
    }

    #[test]
    fn runtime_reconciler_removes_ready_provider_node_that_never_joins() {
        let directory = tempfile::tempdir().expect("tempdir");
        let log =
            Arc::new(DurableControlLog::open(&directory.path().join("control.log")).expect("log"));
        let voters = BTreeSet::from(["a".to_string(), "b".to_string(), "c".to_string()]);
        let quorum = ControllerQuorum::new(voters.clone(), log).expect("quorum");
        let term = quorum.elect("a", &voters).expect("leader");
        let driver = Arc::new(FakeCapacityDriver::new());
        driver.nodes.lock().unwrap().insert(
            "unjoined-node".to_string(),
            ObservedCapacityNode {
                node_id: "unjoined-node".to_string(),
                operation_id: "initial-create".to_string(),
                control_term: term,
                desired_revision: DesiredRevision(1),
                template_revision: "v1".to_string(),
                lifecycle: CapacityNodeLifecycle::Ready,
            },
        );
        let committed = quorum
            .commit_desired_capacity(
                "a",
                term,
                &voters,
                "autoscaler",
                "desired minimum",
                DesiredCapacity {
                    revision: DesiredRevision(1),
                    worker_nodes: 1,
                    gateway_nodes: 0,
                    template_revision: "v1".to_string(),
                },
            )
            .expect("desired capacity");
        let safety = [ReconcileNodeSafety {
            node_id: "unjoined-node".to_string(),
            runtime_node_id: String::new(),
            transferable_load: u64::MAX,
            active_ownership_transfers: u32::MAX,
            active_work: u32::MAX,
            required_replica_responsibilities: u32::MAX,
            only_active_copy: true,
            membership_generation_acknowledged: false,
            controller_voter: false,
            unique_capability: true,
        }];
        let mut reconciler =
            CapacityReconciler::new_runtime(driver.clone(), 1, Duration::from_millis(1), false)
                .expect("runtime reconciler");

        reconciler
            .reconcile(
                &quorum,
                "cluster",
                "a",
                &voters,
                &committed,
                "autoscaler",
                &safety,
            )
            .expect("start join grace");
        std::thread::sleep(Duration::from_millis(2));
        reconciler
            .reconcile(
                &quorum,
                "cluster",
                "a",
                &voters,
                &committed,
                "autoscaler",
                &safety,
            )
            .expect("clean up unjoined provider node");

        assert_eq!(
            driver.nodes.lock().unwrap()["unjoined-node"].lifecycle,
            CapacityNodeLifecycle::Removed
        );
    }

    #[test]
    fn old_leader_term_is_fenced_after_new_election() {
        let directory = tempfile::tempdir().expect("tempdir");
        let log =
            Arc::new(DurableControlLog::open(&directory.path().join("control.log")).expect("log"));
        let voters = BTreeSet::from(["a".to_string(), "b".to_string(), "c".to_string()]);
        let quorum = ControllerQuorum::new(voters.clone(), log.clone()).expect("quorum");
        let old_term = quorum.elect("a", &voters).expect("old leader");
        let _new_term = quorum.elect("b", &voters).expect("new leader");

        assert_eq!(
            quorum.commit(
                "a",
                old_term,
                &voters,
                "autoscaler",
                "stale",
                ControlMutation::PauseAutoscaler { paused: true },
            ),
            Err("control_leader_fence_rejected".to_string())
        );
    }

    #[test]
    fn controller_minority_cannot_elect_or_commit_control_mutations() {
        let directory = tempfile::tempdir().expect("tempdir");
        let log =
            Arc::new(DurableControlLog::open(&directory.path().join("control.log")).expect("log"));
        let voters = BTreeSet::from(["a".to_string(), "b".to_string(), "c".to_string()]);
        let quorum = ControllerQuorum::new(voters.clone(), log).expect("quorum");
        let minority = BTreeSet::from(["a".to_string()]);
        assert_eq!(
            quorum.elect("a", &minority),
            Err("controller_quorum_unavailable".to_string())
        );
        let term = quorum.elect("a", &voters).expect("majority election");
        assert_eq!(
            quorum.commit(
                "a",
                term,
                &minority,
                "autoscaler",
                "partitioned minority",
                ControlMutation::PauseAutoscaler { paused: true },
            ),
            Err("controller_quorum_unavailable".to_string())
        );
    }

    /// Arguments that cannot reconcile are refused before the fence is
    /// committed: a refused call leaves the control log as it was.
    #[test]
    fn invalid_reconcile_arguments_commit_nothing() {
        let directory = tempfile::tempdir().expect("tempdir");
        let log =
            Arc::new(DurableControlLog::open(&directory.path().join("control.log")).expect("log"));
        let voters = BTreeSet::from(["a".to_string(), "b".to_string(), "c".to_string()]);
        let quorum = ControllerQuorum::new(voters.clone(), log.clone()).expect("quorum");
        let term = quorum.elect("a", &voters).expect("leader");
        let committed = CommittedDesiredCapacity {
            log_index: 3,
            term,
            desired: DesiredCapacity {
                revision: DesiredRevision(1),
                worker_nodes: 1,
                gateway_nodes: 0,
                template_revision: "v1".to_string(),
            },
        };
        let mut reconciler =
            CapacityReconciler::new(Arc::new(FakeCapacityDriver::new()), 1).expect("reconciler");
        let mut reconcile = |cluster: &str, committed: &CommittedDesiredCapacity| {
            reconciler.reconcile(&quorum, cluster, "a", &voters, committed, "autoscaler", &[])
        };

        assert_eq!(
            reconcile("", &committed),
            Err("capacity_reconciler_cluster_id_missing".to_string())
        );
        let mut empty = committed.clone();
        empty.desired.worker_nodes = 0;
        assert_eq!(
            reconcile("cluster", &empty),
            Err("capacity_reconciler_desired_state_invalid".to_string())
        );
        assert!(log.entries().is_empty(), "{:?}", log.entries());

        reconcile("cluster", &committed).expect("reconcile");
        let mut older = committed.clone();
        older.log_index = 2;
        assert_eq!(
            reconcile("cluster", &older),
            Err("capacity_reconciler_stale_committed_state".to_string())
        );
    }

    #[test]
    fn runtime_reconcile_uses_one_capacity_snapshot_after_the_leader_fence() {
        let directory = tempfile::tempdir().expect("tempdir");
        let log =
            Arc::new(DurableControlLog::open(&directory.path().join("control.log")).expect("log"));
        let voters = BTreeSet::from(["a".to_string(), "b".to_string(), "c".to_string()]);
        let quorum = ControllerQuorum::new(voters.clone(), log.clone()).expect("quorum");
        let term = quorum.elect("a", &voters).expect("leader");
        let committed = quorum
            .commit_desired_capacity(
                "a",
                term,
                &voters,
                "autoscaler",
                "desired minimum",
                DesiredCapacity {
                    revision: DesiredRevision(1),
                    worker_nodes: 1,
                    gateway_nodes: 0,
                    template_revision: "v1".to_string(),
                },
            )
            .expect("desired capacity");
        let driver = Arc::new(FakeCapacityDriver::new());
        let mut reconciler = CapacityReconciler::new(driver.clone(), 1).expect("reconciler");
        let log_at_observation = log.clone();

        let outcome = reconciler
            .reconcile_with_observed_capacity(
                &quorum,
                "cluster",
                "a",
                &voters,
                &committed,
                "autoscaler",
                move |observation| {
                    assert!(log_at_observation
                        .entries()
                        .iter()
                        .any(|entry| entry.reason == "capacity reconciliation fence"));
                    assert!(observation.nodes.is_empty());
                    Ok((Vec::new(), 0))
                },
            )
            .expect("runtime reconciliation");

        assert_eq!(outcome.ensured.len(), 1);
        assert_eq!(driver.observations.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn drain_candidate_excludes_controller_and_only_copy() {
        let candidates = vec![
            DrainCandidate {
                node_id: "controller".to_string(),
                transferable_load: 0,
                active_ownership_transfers: 0,
                controller_voter: true,
                only_active_copy: false,
                unique_capability: false,
                template_revision: "v1".to_string(),
            },
            DrainCandidate {
                node_id: "only-copy".to_string(),
                transferable_load: 0,
                active_ownership_transfers: 0,
                controller_voter: false,
                only_active_copy: true,
                unique_capability: false,
                template_revision: "v1".to_string(),
            },
            DrainCandidate {
                node_id: "safe".to_string(),
                transferable_load: 1,
                active_ownership_transfers: 0,
                controller_voter: false,
                only_active_copy: false,
                unique_capability: false,
                template_revision: "v1".to_string(),
            },
        ];

        assert_eq!(
            select_drain_candidate(&candidates, 0, 1)
                .expect("safe candidate")
                .node_id,
            "safe"
        );
    }

    #[test]
    fn fenced_reconciler_scales_up_then_drains_before_termination() {
        let directory = tempfile::tempdir().expect("tempdir");
        let log =
            Arc::new(DurableControlLog::open(&directory.path().join("control.log")).expect("log"));
        let voters = BTreeSet::from(["a".to_string(), "b".to_string(), "c".to_string()]);
        let quorum = ControllerQuorum::new(voters.clone(), log.clone()).expect("quorum");
        let term = quorum.elect("a", &voters).expect("leader");
        let driver = Arc::new(FakeCapacityDriver::new());
        let mut reconciler =
            CapacityReconciler::new(driver.clone(), 1).expect("capacity reconciler");
        let desired_two = quorum
            .commit_desired_capacity(
                "a",
                term,
                &voters,
                "autoscaler",
                "sustained pressure",
                DesiredCapacity {
                    revision: DesiredRevision(1),
                    worker_nodes: 2,
                    gateway_nodes: 0,
                    template_revision: "v1".to_string(),
                },
            )
            .expect("desired capacity");

        let up = reconciler
            .reconcile(
                &quorum,
                "cluster",
                "a",
                &voters,
                &desired_two,
                "autoscaler",
                &[],
            )
            .expect("scale up");
        assert_eq!(up.ensured.len(), 2);
        let nodes = driver.observe_capacity("cluster").expect("observe").nodes;
        assert_eq!(nodes.len(), 2);

        let desired_one = quorum
            .commit_desired_capacity(
                "a",
                term,
                &voters,
                "autoscaler",
                "sustained idle",
                DesiredCapacity {
                    revision: DesiredRevision(2),
                    worker_nodes: 1,
                    gateway_nodes: 0,
                    template_revision: "v1".to_string(),
                },
            )
            .expect("lower desired capacity");
        let safety: Vec<_> = nodes
            .iter()
            .map(|node| ReconcileNodeSafety {
                node_id: node.node_id.clone(),
                runtime_node_id: node.node_id.clone(),
                transferable_load: 0,
                active_ownership_transfers: 0,
                active_work: 0,
                required_replica_responsibilities: 0,
                only_active_copy: false,
                membership_generation_acknowledged: true,
                controller_voter: false,
                unique_capability: false,
            })
            .collect();
        let draining = reconciler
            .reconcile(
                &quorum,
                "cluster",
                "a",
                &voters,
                &desired_one,
                "autoscaler",
                &safety,
            )
            .expect("begin drain");
        assert_eq!(draining.drains.len(), 1);
        assert_eq!(draining.drains[0].phase, DrainPhase::Draining);

        let mut reconciler =
            CapacityReconciler::new(driver.clone(), 1).expect("recovered reconciler");
        reconciler.restore_from_control_entries(&log.entries());
        let restored = reconciler.drain_progress();
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].node_id, draining.drains[0].node_id);
        assert_eq!(restored[0].phase, draining.drains[0].phase);
        assert_eq!(
            restored[0].drain_operation_id,
            draining.drains[0].drain_operation_id
        );
        assert!(restored[0].deadline_unix_millis > restored[0].started_at_unix_millis);

        let terminating = reconciler
            .reconcile(
                &quorum,
                "cluster",
                "a",
                &voters,
                &desired_one,
                "autoscaler",
                &safety,
            )
            .expect("terminate drained node");
        assert_eq!(terminating.drains[0].phase, DrainPhase::Terminating);
        assert_eq!(
            driver
                .observe_capacity("cluster")
                .expect("observe after termination")
                .nodes
                .iter()
                .filter(|node| node.lifecycle != CapacityNodeLifecycle::Removed)
                .count(),
            1
        );

        // Once the node is gone its drain is over, in the log as well: a
        // later leader has nothing to resume.
        let finished = reconciler
            .reconcile(
                &quorum,
                "cluster",
                "a",
                &voters,
                &desired_one,
                "autoscaler",
                &safety,
            )
            .expect("finish drain");
        assert!(finished.drains.is_empty(), "{:?}", finished.drains);
        assert!(log.entries().iter().any(|entry| matches!(
            &entry.mutation,
            ControlMutation::DrainIntent { node_id, cancelled: true }
                if *node_id == draining.drains[0].node_id
        )));
        let mut recovered =
            CapacityReconciler::new(driver.clone(), 1).expect("recovered reconciler");
        recovered.restore_from_control_entries(&log.entries());
        assert!(recovered.drain_progress().is_empty());
    }

    /// Scale-down with no worker safe to drain is a state to wait in, not a
    /// failed reconcile: the controller reported it as a failed tick.
    #[test]
    fn scale_down_without_a_safe_candidate_waits() {
        let directory = tempfile::tempdir().expect("tempdir");
        let log =
            Arc::new(DurableControlLog::open(&directory.path().join("control.log")).expect("log"));
        let voters = BTreeSet::from(["a".to_string(), "b".to_string(), "c".to_string()]);
        let quorum = ControllerQuorum::new(voters.clone(), log).expect("quorum");
        let term = quorum.elect("a", &voters).expect("leader");
        let driver = Arc::new(FakeCapacityDriver::new());
        let desired_two = DesiredCapacity {
            revision: DesiredRevision(1),
            worker_nodes: 2,
            gateway_nodes: 0,
            template_revision: "v1".to_string(),
        };
        reconcile_scale_up(&*driver, "cluster", term, &desired_two, 0).expect("seed workers");
        let committed = CommittedDesiredCapacity {
            log_index: 1,
            term,
            desired: DesiredCapacity {
                worker_nodes: 1,
                revision: DesiredRevision(2),
                ..desired_two
            },
        };
        let only_copies: Vec<_> = driver
            .observe_capacity("cluster")
            .expect("nodes")
            .nodes
            .iter()
            .map(|node| ReconcileNodeSafety {
                node_id: node.node_id.clone(),
                runtime_node_id: node.node_id.clone(),
                transferable_load: 0,
                active_ownership_transfers: 0,
                active_work: 0,
                required_replica_responsibilities: 0,
                only_active_copy: true,
                membership_generation_acknowledged: true,
                controller_voter: false,
                unique_capability: false,
            })
            .collect();
        let mut reconciler = CapacityReconciler::new(driver, 1).expect("reconciler");

        let waiting = reconciler
            .reconcile(
                &quorum,
                "cluster",
                "a",
                &voters,
                &committed,
                "autoscaler",
                &only_copies,
            )
            .expect("scale-down waits");

        assert_eq!(waiting.constraints, ["drain_no_safe_candidate"]);
        assert!(waiting.drains.is_empty());
    }

    /// A drain restored from the log after a leader change has no runtime
    /// identity; forcing it past its deadline refused with
    /// forced_drain_runtime_identity_missing on every tick.
    #[test]
    fn a_restored_drain_can_be_forced_after_its_deadline() {
        let directory = tempfile::tempdir().expect("tempdir");
        let log =
            Arc::new(DurableControlLog::open(&directory.path().join("control.log")).expect("log"));
        let voters = BTreeSet::from(["a".to_string(), "b".to_string(), "c".to_string()]);
        let quorum = ControllerQuorum::new(voters.clone(), log.clone()).expect("quorum");
        let term = quorum.elect("a", &voters).expect("leader");
        let driver = Arc::new(FakeCapacityDriver::new());
        let desired_two = DesiredCapacity {
            revision: DesiredRevision(1),
            worker_nodes: 2,
            gateway_nodes: 0,
            template_revision: "v1".to_string(),
        };
        reconcile_scale_up(&*driver, "cluster", term, &desired_two, 0).expect("seed workers");
        let nodes = driver.observe_capacity("cluster").expect("nodes").nodes;
        let draining_node = nodes[0].node_id.clone();
        let commit = |reason: &str, mutation| {
            quorum
                .commit("a", term, &voters, "autoscaler", reason, mutation)
                .expect("commit")
        };
        commit(
            "begin worker drain",
            ControlMutation::DrainIntent {
                node_id: draining_node.clone(),
                cancelled: false,
            },
        );
        let mut drain = driver_operation("restored-drain", Some(&draining_node));
        drain.state = DriverOperationState::Succeeded;
        commit(
            "record begin drain result",
            ControlMutation::DriverOperation(drain),
        );
        let committed = quorum
            .commit_desired_capacity(
                "a",
                term,
                &voters,
                "autoscaler",
                "sustained idle",
                DesiredCapacity {
                    revision: DesiredRevision(2),
                    worker_nodes: 1,
                    gateway_nodes: 0,
                    template_revision: "v1".to_string(),
                },
            )
            .expect("lower desired capacity");
        let mut reconciler =
            CapacityReconciler::new_runtime(driver.clone(), 1, Duration::from_millis(1), true)
                .expect("reconciler");
        reconciler.restore_from_control_entries(&log.entries());
        assert_eq!(reconciler.drain_progress()[0].phase, DrainPhase::Draining);
        assert_eq!(reconciler.drain_progress()[0].runtime_node_id, "");
        std::thread::sleep(Duration::from_millis(5));
        let safety: Vec<_> = nodes
            .iter()
            .map(|node| ReconcileNodeSafety {
                node_id: node.node_id.clone(),
                runtime_node_id: format!("runtime-{}@host:4370", node.node_id),
                transferable_load: 1,
                active_ownership_transfers: 0,
                active_work: 1,
                required_replica_responsibilities: 0,
                only_active_copy: false,
                membership_generation_acknowledged: true,
                controller_voter: false,
                unique_capability: false,
            })
            .collect();

        let forced = reconciler
            .reconcile(
                &quorum,
                "cluster",
                "a",
                &voters,
                &committed,
                "autoscaler",
                &safety,
            )
            .expect("forced termination");

        assert_eq!(forced.drains[0].phase, DrainPhase::Terminating);
        assert!(forced.drains[0].forced_termination);
        assert_eq!(
            forced.drains[0].runtime_node_id,
            format!("runtime-{draining_node}@host:4370")
        );
    }

    #[test]
    fn drain_timeout_requires_manual_action_unless_explicit_safe_force_policy_is_enabled() {
        let directory = tempfile::tempdir().expect("tempdir");
        let log =
            Arc::new(DurableControlLog::open(&directory.path().join("control.log")).expect("log"));
        let voters = BTreeSet::from(["a".to_string(), "b".to_string(), "c".to_string()]);
        let quorum = ControllerQuorum::new(voters.clone(), log).expect("quorum");
        let term = quorum.elect("a", &voters).expect("leader");
        let driver = Arc::new(FakeCapacityDriver::new());
        let desired_two = DesiredCapacity {
            revision: DesiredRevision(1),
            worker_nodes: 2,
            gateway_nodes: 0,
            template_revision: "v1".to_string(),
        };
        reconcile_scale_up(&*driver, "cluster", term, &desired_two, 0).expect("seed workers");
        let nodes = driver.observe_capacity("cluster").expect("nodes").nodes;
        let draining_node = nodes[0].node_id.clone();
        let mut reconciler = CapacityReconciler::new(driver.clone(), 1).expect("reconciler");
        reconciler.draining.insert(
            draining_node.clone(),
            DrainProgress {
                node_id: draining_node.clone(),
                runtime_node_id: draining_node.clone(),
                phase: DrainPhase::Draining,
                desired_revision: DesiredRevision(2),
                control_term: term,
                drain_operation_id: "drain-timeout-test".to_string(),
                terminate_operation_id: None,
                template_revision: "v1".to_string(),
                started_at_unix_millis: 1,
                deadline_unix_millis: 1,
                forced_termination: false,
            },
        );
        let committed = CommittedDesiredCapacity {
            log_index: 1,
            term,
            desired: DesiredCapacity {
                revision: DesiredRevision(2),
                worker_nodes: 1,
                gateway_nodes: 0,
                template_revision: "v1".to_string(),
            },
        };
        let safety = nodes
            .iter()
            .map(|node| ReconcileNodeSafety {
                node_id: node.node_id.clone(),
                runtime_node_id: node.node_id.clone(),
                transferable_load: 0,
                active_ownership_transfers: 0,
                active_work: u32::from(node.node_id == draining_node),
                required_replica_responsibilities: 0,
                only_active_copy: false,
                membership_generation_acknowledged: true,
                controller_voter: false,
                unique_capability: false,
            })
            .collect::<Vec<_>>();

        let blocked = reconciler
            .reconcile(
                &quorum,
                "cluster",
                "a",
                &voters,
                &committed,
                "autoscaler",
                &safety,
            )
            .expect("safe default");
        assert_eq!(blocked.drains[0].phase, DrainPhase::Draining);
        assert!(blocked.constraints.iter().any(|constraint| {
            constraint.starts_with("drain_timeout_manual_intervention_required")
        }));

        reconciler.force_termination_after_timeout = true;
        let forced = reconciler
            .reconcile(
                &quorum,
                "cluster",
                "a",
                &voters,
                &committed,
                "autoscaler",
                &safety,
            )
            .expect("explicit forced policy");
        assert_eq!(forced.drains[0].phase, DrainPhase::Terminating);
        assert!(forced.drains[0].forced_termination);
        assert!(forced
            .constraints
            .iter()
            .any(|constraint| constraint.starts_with("forced_termination_after_drain_timeout")));
    }

    #[test]
    fn local_scheduler_autoscaler_requires_sustained_pressure_and_idle() {
        let start = Instant::now();
        let mut autoscaler = LocalSchedulerAutoscaler::new(LocalScalingPolicy {
            min_workers: 1,
            max_workers: 4,
            target_runnable_per_worker: 1.0,
            target_queue_wait: Duration::from_millis(25),
            scale_up_window: Duration::from_secs(2),
            scale_down_window: Duration::from_secs(5),
            cooldown: Duration::ZERO,
        })
        .expect("valid local policy");

        assert!(!autoscaler.evaluate(1, 4, Duration::ZERO, start).changed);
        let scale_up = autoscaler.evaluate(1, 4, Duration::ZERO, start + Duration::from_secs(2));
        assert_eq!(scale_up.desired_workers, 4);
        assert!(scale_up.changed);

        assert!(
            !autoscaler
                .evaluate(4, 0, Duration::ZERO, start + Duration::from_secs(3))
                .changed
        );
        let scale_down = autoscaler.evaluate(4, 0, Duration::ZERO, start + Duration::from_secs(8));
        assert_eq!(scale_down.desired_workers, 3);
        assert!(scale_down.changed);
    }

    #[test]
    fn instrumented_capacity_driver_records_operation_count_and_latency() {
        let count = || {
            crate::dist::telemetry::runtime_telemetry()
                .snapshot()
                .capacity_driver_operations
                .into_iter()
                .find(|operation| operation.operation == "ensure")
                .expect("ensure telemetry")
                .count
        };
        let before = count();
        let driver = instrument_capacity_driver(Arc::new(FakeCapacityDriver::new()));

        driver
            .ensure_node(&driver_operation("instrumented-ensure", None))
            .expect("fake capacity ensure");

        assert!(count() >= before.saturating_add(1));
    }

    /// Enough of the Docker CLI for the driver, keeping each container as a
    /// directory under the state directory. A file named for a fault in
    /// that directory injects it.
    const FAKE_DOCKER: &str = r#"
state="$1"
shift
echo "$*" >> "$state/calls"
command="$1"
shift
case "$command" in
version)
  [ -f "$state/sleep" ] && exec sleep 2
  [ -f "$state/fail" ] && { echo "daemon refused token=abc123 because" >&2; exit 1; }
  [ -f "$state/invalid-utf8" ] && { printf '\377'; exit 0; }
  echo 27.0 ;;
ps)
  shift
  filters=""
  while [ "$#" -gt 1 ]; do filters="$filters $2"; shift 2; done
  for dir in "$state"/containers/*; do
    [ -d "$dir" ] || continue
    id=${dir##*/}
    keep=1
    for filter in $filters; do
      case "$filter" in
        label=*) grep -qxF "${filter#label=}" "$dir/labels" || keep=0 ;;
        id=*) case "$id" in "${filter#id=}"*) ;; *) keep=0 ;; esac ;;
      esac
    done
    [ "$keep" = 1 ] && echo "$id"
  done
  exit 0 ;;
inspect)
  [ -f "$state/inspect" ] && { cat "$state/inspect"; exit 0; }
  dir="$state/containers/$1"
  labels=$(sed 's/^\([^=]*\)=\(.*\)$/"\1":"\2"/' "$dir/labels" | paste -sd, -)
  health=""
  [ -f "$dir/health" ] && health=",\"Health\":{\"Status\":\"$(cat "$dir/health")\"}"
  printf '[{"Id":"%s","Config":{"Labels":{%s}},"State":{"Status":"%s"%s}}]\n' \
    "$1" "$labels" "$(cat "$dir/status")" "$health" ;;
create)
  [ -f "$state/fail-create" ] && { echo "create refused password=hunter2" >&2; exit 1; }
  count=$(cat "$state/count" 2>/dev/null || echo 0)
  count=$((count + 1))
  echo "$count" > "$state/count"
  id="container$count"
  dir="$state/containers/$id"
  mkdir -p "$dir"
  : > "$dir/labels"
  while [ "$#" -gt 1 ]; do
    case "$1" in
      --label) echo "$2" >> "$dir/labels" ;;
      --env) echo "$2" >> "$dir/env" ;;
      --env-file) echo "$2" > "$dir/env-file-path"; cat "$2" > "$dir/env-file" 2>/dev/null ;;
      --network) echo "$2" > "$dir/network" ;;
    esac
    shift 2
  done
  echo "$1" > "$dir/image"
  echo created > "$dir/status"
  echo "$id" ;;
start) echo running > "$state/containers/$1/status" ;;
rm) rm -rf "$state/containers/$2" ;;
esac
"#;

    /// A Docker driver whose `docker` is the fake above, run through
    /// `/bin/sh` as the execution prefix.
    fn docker_driver(state: &Path, environment: Vec<String>) -> DockerCapacityDriver {
        std::fs::create_dir_all(state.join("containers")).expect("fake docker state");
        let script = state.join("docker.sh");
        std::fs::write(&script, FAKE_DOCKER).expect("fake docker script");
        DockerCapacityDriver::new(DockerDriverConfig {
            binary: PathBuf::from("/bin/sh"),
            execution_prefix: vec![script.display().to_string(), state.display().to_string()],
            image: "image@sha256:abc".to_string(),
            pool: "workers".to_string(),
            network: Some("mesh-net".to_string()),
            environment,
            environment_file_mount: None,
            operation_timeout: Duration::from_secs(10),
        })
    }

    fn docker_calls(state: &Path) -> Vec<String> {
        std::fs::read_to_string(state.join("calls"))
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }

    /// A managed container as the driver labels one, in `status`.
    fn add_container(state: &Path, id: &str, operation: &DriverOperation, status: &str) {
        let directory = state.join("containers").join(id);
        std::fs::create_dir_all(&directory).expect("container directory");
        std::fs::write(
            directory.join("labels"),
            format!(
                "mesh.managed=true\nmesh.cluster={}\nmesh.pool=workers\nmesh.template={}\n\
                 mesh.operation={}\nmesh.term={}\nmesh.revision={}\n",
                operation.cluster_id,
                operation.template_revision,
                operation.operation_id,
                operation.control_term.0,
                operation.desired_revision.0,
            ),
        )
        .expect("container labels");
        std::fs::write(directory.join("status"), status).expect("container status");
    }

    #[cfg(unix)]
    #[test]
    fn docker_driver_validates_through_the_cli_and_redacts_its_errors() {
        let state = tempfile::tempdir().expect("fake docker state");
        let driver = docker_driver(state.path(), Vec::new());
        assert_eq!(driver.validate_configuration(), Ok(()));
        assert_eq!(
            docker_calls(state.path()),
            ["version --format {{.Server.Version}}"]
        );

        std::fs::write(state.path().join("fail"), "").unwrap();
        assert_eq!(
            driver.validate_configuration(),
            Err("docker_driver_api_error:daemon refused [redacted] because".to_string())
        );
        std::fs::remove_file(state.path().join("fail")).unwrap();
        std::fs::write(state.path().join("invalid-utf8"), "").unwrap();
        assert_eq!(
            driver.validate_configuration(),
            Err("docker_driver_output_invalid_utf8".to_string())
        );
        std::fs::remove_file(state.path().join("invalid-utf8")).unwrap();
        std::fs::write(state.path().join("sleep"), "").unwrap();
        let mut slow = docker_driver(state.path(), Vec::new());
        slow.config.operation_timeout = Duration::from_millis(100);
        assert_eq!(
            slow.validate_configuration(),
            Err("docker_driver_api_timeout".to_string())
        );
        slow.config.operation_timeout = Duration::ZERO;
        assert_eq!(
            slow.validate_configuration(),
            Err("docker_driver_operation_timeout_invalid".to_string())
        );
        slow.config.pool = " ".to_string();
        assert_eq!(
            slow.validate_configuration(),
            Err("docker_driver_template_invalid".to_string())
        );
        let mut missing = docker_driver(state.path(), Vec::new());
        missing.config.binary = PathBuf::from("/nonexistent/docker");
        assert!(missing
            .validate_configuration()
            .unwrap_err()
            .starts_with("docker_driver_command_failed:"));
        assert_eq!(
            redact("TOKEN=a Password:b my-secret plain"),
            "[redacted] [redacted] [redacted] plain"
        );
    }

    #[cfg(unix)]
    #[test]
    fn docker_driver_creates_starts_and_remembers_a_labelled_container() {
        let state = tempfile::tempdir().expect("fake docker state");
        let driver = docker_driver(
            state.path(),
            vec!["PORT=8080".to_string(), "MESH_ROLES=worker".to_string()],
        );
        let ensure = driver_operation("0123456789abcdef-ensure", None);

        let created = driver.ensure_node(&ensure).expect("create container");

        assert_eq!(created.node_id.as_deref(), Some("container1"));
        assert_eq!(created.state, DriverOperationState::Succeeded);
        let container = state.path().join("containers/container1");
        let labels = std::fs::read_to_string(container.join("labels")).unwrap();
        for label in [
            "mesh.managed=true",
            "mesh.cluster=cluster-a",
            "mesh.pool=workers",
            "mesh.template=template-v1",
            "mesh.operation=0123456789abcdef-ensure",
            "mesh.term=3",
            "mesh.revision=7",
        ] {
            assert!(
                labels.lines().any(|line| line == label),
                "{label}: {labels}"
            );
        }
        assert_eq!(
            std::fs::read_to_string(container.join("env")).unwrap(),
            "MESH_STABLE_NODE_ID=cluster-a/capacity/0123456789abcdef-ensure\n"
        );
        assert_eq!(
            std::fs::read_to_string(container.join("env-file")).unwrap(),
            "PORT=8080\nMESH_ROLES=worker\n"
        );
        // The environment file only lives for the create call.
        let env_file = std::fs::read_to_string(container.join("env-file-path")).unwrap();
        assert!(!Path::new(env_file.trim()).exists());
        assert_eq!(
            std::fs::read_to_string(container.join("network")).unwrap(),
            "mesh-net\n"
        );
        assert_eq!(
            std::fs::read_to_string(container.join("status")).unwrap(),
            "running\n"
        );

        // The operation is remembered: a retry calls Docker no more.
        let calls = docker_calls(state.path()).len();
        assert_eq!(driver.ensure_node(&ensure), Ok(created.clone()));
        assert_eq!(
            driver.get_operation(&ensure.operation_id),
            Ok(Some(created.clone()))
        );
        assert_eq!(docker_calls(state.path()).len(), calls);

        // A driver that lost that memory adopts the container instead.
        let restarted = docker_driver(state.path(), Vec::new());
        assert_eq!(restarted.ensure_node(&ensure), Ok(created.clone()));
        let fresh = docker_driver(state.path(), Vec::new());
        assert_eq!(
            fresh.get_operation(&ensure.operation_id),
            Ok(Some(created.clone()))
        );
        assert_eq!(fresh.get_operation("no-such-operation"), Ok(None));
        let observed = fresh.observe_capacity("cluster-a").expect("observe");
        assert_eq!(observed.nodes.len(), 1);
        assert_eq!(observed.nodes[0].lifecycle, CapacityNodeLifecycle::Ready);
        assert_eq!(observed.nodes[0].operation_id, ensure.operation_id);

        // Another term's operation for the same id is not this one.
        let mut stale = ensure.clone();
        stale.control_term = ControlTerm(9);
        assert_eq!(
            docker_driver(state.path(), Vec::new()).ensure_node(&stale),
            Err("docker_driver_adoption_identity_mismatch".to_string())
        );
        add_container(state.path(), "duplicate", &ensure, "running");
        assert_eq!(
            docker_driver(state.path(), Vec::new()).ensure_node(&ensure),
            Err("docker_driver_duplicate_operation_containers".to_string())
        );
    }

    #[cfg(unix)]
    #[test]
    fn docker_driver_refuses_bad_environments_and_cleans_up_failed_creates() {
        let state = tempfile::tempdir().expect("fake docker state");
        let ensure = driver_operation("create-failure", None);
        let invalid = docker_driver(state.path(), vec!["NOT_AN_ASSIGNMENT".to_string()]);
        assert_eq!(
            invalid.ensure_node(&ensure),
            Err("docker_driver_environment_invalid".to_string())
        );

        let host = state.path().join("env-host");
        let mut mounted = docker_driver(state.path(), vec!["PORT=8080".to_string()]);
        mounted.config.environment_file_mount = Some(DockerEnvironmentFileMount {
            host_directory: host.clone(),
            driver_directory: PathBuf::from("/driver-env"),
        });
        std::fs::write(state.path().join("fail-create"), "").unwrap();
        assert_eq!(
            mounted.ensure_node(&ensure),
            Err("docker_driver_api_error:create refused [redacted]".to_string())
        );
        // The file was handed to Docker by its path inside the driver's
        // mount, and removed once the create call returned.
        let arguments = docker_calls(state.path()).pop().unwrap();
        assert!(
            arguments.contains("--env-file /driver-env/env-"),
            "{arguments}"
        );
        assert_eq!(std::fs::read_dir(&host).unwrap().count(), 0);

        let blocked = state.path().join("not-a-directory");
        std::fs::write(&blocked, "").unwrap();
        mounted.config.environment_file_mount = Some(DockerEnvironmentFileMount {
            host_directory: blocked.join("env"),
            driver_directory: PathBuf::from("/driver-env"),
        });
        assert!(mounted
            .ensure_node(&ensure)
            .unwrap_err()
            .starts_with("docker_driver_env_directory_failed:"));
    }

    #[cfg(unix)]
    #[test]
    fn autonomous_docker_workers_get_a_signed_identity() {
        static SIGNING_KEY_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = SIGNING_KEY_LOCK.lock().unwrap();
        let key_env = super::super::identity_claim::CAPACITY_IDENTITY_SIGNING_KEY_ENV;
        let state = tempfile::tempdir().expect("fake docker state");
        let environment = vec![
            "MESH_CLUSTER_MODE=Autonomous".to_string(),
            "MESH_CLUSTER_PORT=4371".to_string(),
            "MESH_ROLES=worker,gateway".to_string(),
        ];
        let driver = docker_driver(state.path(), environment.clone());
        let ensure = driver_operation("signed-identity-operation", None);
        std::env::remove_var(key_env);
        assert_eq!(
            driver.ensure_node(&ensure),
            Err("capacity_identity_signing_key_missing".to_string())
        );

        let (signing_key, verify_key) =
            super::super::identity_claim::generate_identity_signing_material().unwrap();
        std::env::set_var(key_env, &signing_key);
        let created = driver.ensure_node(&ensure);
        let mut controller = docker_driver(
            state.path(),
            vec![
                "MESH_CLUSTER_MODE=autonomous".to_string(),
                "MESH_ROLES=controller-ish".to_string(),
            ],
        );
        controller.config.pool = "controllers".to_string();
        let refused = controller.ensure_node(&driver_operation("bad-roles", None));
        std::env::remove_var(key_env);
        created.expect("create signed worker");
        assert_eq!(refused, Err("node_identity_roles_invalid".to_string()));

        let env = std::fs::read_to_string(state.path().join("containers/container1/env")).unwrap();
        let name = "mesh-workers-signed-ident";
        let advertised = format!("{name}@{name}:4371");
        assert!(env.contains("MESH_STABLE_NODE_ID=cluster-a/capacity/signed-identity-operation\n"));
        assert!(
            env.contains(&format!("MESH_NODE_NAME={advertised}\n")),
            "{env}"
        );
        let envelope = env
            .lines()
            .find_map(|line| {
                line.strip_prefix(&format!(
                    "{}=",
                    super::super::identity_claim::IDENTITY_ENVELOPE_ENV
                ))
            })
            .expect("identity envelope");
        let claim = super::super::identity_claim::decode_and_verify_identity(
            &super::super::identity_claim::decode_envelope_b64(envelope).unwrap(),
            &verify_key,
            "cluster-a",
            &advertised,
            unix_millis(),
        )
        .expect("verified claim");
        assert_eq!(claim.roles, ["gateway", "worker"]);
    }

    #[cfg(unix)]
    #[test]
    fn docker_observation_maps_container_states_to_lifecycles() {
        let state = tempfile::tempdir().expect("fake docker state");
        let driver = docker_driver(state.path(), Vec::new());
        let cases = [
            ("running", Some("healthy"), CapacityNodeLifecycle::Ready),
            ("running", None, CapacityNodeLifecycle::Ready),
            ("running", Some("unhealthy"), CapacityNodeLifecycle::Failed),
            (
                "running",
                Some("starting"),
                CapacityNodeLifecycle::Provisioning,
            ),
            ("created", None, CapacityNodeLifecycle::Provisioning),
            ("removing", None, CapacityNodeLifecycle::Terminating),
            ("exited", None, CapacityNodeLifecycle::Failed),
            ("migrating", None, CapacityNodeLifecycle::Failed),
        ];
        for (index, (status, health, _)) in cases.iter().enumerate() {
            let id = format!("node{index}");
            add_container(
                state.path(),
                &id,
                &driver_operation(&format!("operation{index}"), None),
                status,
            );
            if let Some(health) = health {
                std::fs::write(
                    state.path().join("containers").join(&id).join("health"),
                    health,
                )
                .unwrap();
            }
        }
        // Another cluster's container is not observed.
        let mut other = driver_operation("other-cluster", None);
        other.cluster_id = "cluster-b".to_string();
        add_container(state.path(), "other", &other, "running");

        let observed = driver.observe_capacity("cluster-a").expect("observe");

        let lifecycles: Vec<_> = observed.nodes.iter().map(|node| node.lifecycle).collect();
        let expected: Vec<_> = cases.iter().map(|(_, _, lifecycle)| *lifecycle).collect();
        assert_eq!(lifecycles, expected);
        assert_eq!(observed.nodes[0].desired_revision, DesiredRevision(7));

        // A Docker that answers for another scope is refused, not trusted.
        std::fs::write(
            state.path().join("inspect"),
            r#"[{"Id":"x","Config":{"Labels":{"mesh.managed":"true","mesh.cluster":"cluster-b",
               "mesh.pool":"workers","mesh.template":"t","mesh.operation":"o",
               "mesh.term":"1","mesh.revision":"1"}},"State":{"Status":"running"}}]"#,
        )
        .unwrap();
        assert_eq!(
            driver.observe_capacity("cluster-a"),
            Err("docker_driver_observation_scope_mismatch".to_string())
        );
    }

    #[cfg(unix)]
    #[test]
    fn docker_inspection_refuses_what_it_cannot_trust() {
        let state = tempfile::tempdir().expect("fake docker state");
        let driver = docker_driver(state.path(), Vec::new());
        let labels = |extra: &str| {
            format!(
                r#"[{{"Config":{{"Labels":{{"mesh.managed":"true","mesh.cluster":"c",
                   "mesh.pool":"workers","mesh.template":"t","mesh.operation":"o"{extra}}}}},
                   "State":{{}}}}]"#
            )
        };
        let cases = [
            (
                "not json".to_string(),
                "docker_driver_inspect_decode_failed:",
            ),
            ("[]".to_string(), "docker_driver_inspect_empty"),
            (
                r#"[{"Config":{}}]"#.to_string(),
                "docker_driver_labels_missing",
            ),
            (
                r#"[{"Config":{"Labels":{"mesh.managed":"false"}}}]"#.to_string(),
                "docker_driver_refuses_unmanaged_container",
            ),
            (
                r#"[{"Config":{"Labels":{"mesh.managed":""}}}]"#.to_string(),
                "docker_driver_label_missing:mesh.managed",
            ),
            (labels(""), "docker_driver_label_missing:mesh.term"),
            (
                labels(r#","mesh.term":"x","mesh.revision":"1""#),
                "docker_driver_term_label_invalid",
            ),
            (
                labels(r#","mesh.term":"1","mesh.revision":"x""#),
                "docker_driver_revision_label_invalid",
            ),
        ];
        for (document, expected) in cases {
            std::fs::write(state.path().join("inspect"), document).unwrap();
            let error = driver.inspect_container("any").unwrap_err();
            assert!(error.starts_with(expected), "{expected}: {error}");
        }
        // Without an Id, the container is the one asked about; a missing
        // state reads as failed.
        std::fs::write(
            state.path().join("inspect"),
            labels(r#","mesh.term":"2","mesh.revision":"3""#),
        )
        .unwrap();
        let observed = driver.inspect_container("asked-about").unwrap();
        assert_eq!(observed.node_id, "asked-about");
        assert_eq!(observed.lifecycle, CapacityNodeLifecycle::Failed);
        assert_eq!(observed.control_term, ControlTerm(2));
    }

    #[cfg(unix)]
    #[test]
    fn docker_driver_drains_and_removes_only_its_own_containers() {
        let state = tempfile::tempdir().expect("fake docker state");
        let driver = docker_driver(state.path(), Vec::new());
        let node = driver_operation("terminate-target", None);
        add_container(state.path(), "target", &node, "running");

        let drain = driver_operation("drain-operation", Some("target"));
        let drained = driver.begin_drain(&drain, "target").expect("drain");
        assert_eq!(drained.state, DriverOperationState::Succeeded);
        assert_eq!(driver.begin_drain(&drain, "other"), Ok(drained));

        let mut foreign = driver_operation("terminate-foreign", Some("target"));
        foreign.cluster_id = "cluster-b".to_string();
        assert_eq!(
            driver.terminate_node(&foreign, "target"),
            Err("docker_driver_refuses_unmanaged_container".to_string())
        );

        let terminate = driver_operation("terminate-operation", Some("target"));
        let terminated = driver.terminate_node(&terminate, "target").expect("remove");
        assert_eq!(terminated.node_id.as_deref(), Some("target"));
        assert!(!state.path().join("containers/target").exists());
        assert!(docker_calls(state.path()).contains(&"rm -f target".to_string()));
        assert_eq!(driver.terminate_node(&terminate, "target"), Ok(terminated));

        // A container that is already gone is terminated.
        let calls = docker_calls(state.path()).len();
        let gone = driver_operation("terminate-gone", Some("gone"));
        assert_eq!(
            driver
                .terminate_node(&gone, "gone")
                .expect("already gone")
                .state,
            DriverOperationState::Succeeded
        );
        assert_eq!(
            docker_calls(state.path())[calls..],
            ["ps -aq --filter id=gone"]
        );
    }

    #[test]
    fn docker_driver_config_debug_redacts_environment_values() {
        let config = DockerDriverConfig {
            binary: PathBuf::from("docker"),
            execution_prefix: Vec::new(),
            image: "image".to_string(),
            pool: "workers".to_string(),
            network: None,
            environment: vec!["DATABASE_URL=postgres://secret".to_string()],
            environment_file_mount: None,
            operation_timeout: Duration::from_secs(1),
        };
        let rendered = format!("{config:?}");
        assert!(!rendered.contains("postgres://secret"), "{rendered}");
        assert!(rendered.contains("[redacted; 1]"), "{rendered}");
    }
}
