//! Embedded autonomous-cluster configuration and controller service.
//!
//! `meshc` normalizes the validated manifest into this versioned schema and
//! emits it into the executable. Node-specific identity and credentials remain
//! environment sourced; scaling policy and driver templates do not depend on
//! an out-of-band policy process.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Once, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::scaling::ScalingPolicy;
use super::scaling::{
    Autoscaler, CapacityDriver, CapacityReconcileOutcome, CapacityReconciler,
    CommittedDesiredCapacity, ControlLogEntry, ControlMutation, ControlPlaneCommitter, ControlTerm,
    DesiredCapacity, DesiredRevision, DockerCapacityDriver, DockerDriverConfig,
    DockerEnvironmentFileMount, ProcessCapacityDriver, ProcessDriverConfig, ReconcileNodeSafety,
    ScalingDecision, ScalingSample,
};
use sha2::{Digest, Sha256};

pub const AUTONOMOUS_CONFIG_SCHEMA_VERSION: u16 = 4;

fn default_managed_roles() -> Vec<String> {
    vec!["worker".to_string()]
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeFeatureGates {
    pub protocol_two: bool,
    pub durable_continuity: bool,
    pub telemetry: bool,
    pub local_scheduler_autoscaling: bool,
    pub adaptive_routing: bool,
    pub controller_quorum: bool,
    pub horizontal_autoscaling: bool,
    pub horizontal_observe_only: bool,
    pub automatic_scale_up: bool,
    pub automatic_scale_down: bool,
}

impl Default for RuntimeFeatureGates {
    fn default() -> Self {
        Self {
            protocol_two: true,
            durable_continuity: true,
            telemetry: true,
            local_scheduler_autoscaling: true,
            adaptive_routing: true,
            controller_quorum: true,
            horizontal_autoscaling: true,
            horizontal_observe_only: false,
            automatic_scale_up: true,
            automatic_scale_down: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RuntimeSchedulerConfig {
    pub min_workers: u16,
    pub max_workers: u16,
    pub target_runnable_per_worker: f64,
    pub target_queue_wait_millis: u64,
    pub scale_up_window_millis: u64,
    pub scale_down_window_millis: u64,
    pub cooldown_millis: u64,
}

impl Default for RuntimeSchedulerConfig {
    fn default() -> Self {
        Self {
            min_workers: 1,
            max_workers: 1,
            target_runnable_per_worker: 1.0,
            target_queue_wait_millis: 25,
            scale_up_window_millis: 10_000,
            scale_down_window_millis: 300_000,
            cooldown_millis: 30_000,
        }
    }
}

impl RuntimeSchedulerConfig {
    fn validate(&self) -> Result<(), String> {
        if self.min_workers == 0
            || self.min_workers > self.max_workers
            || !self.target_runnable_per_worker.is_finite()
            || self.target_runnable_per_worker <= 0.0
            || self.target_queue_wait_millis == 0
            || self.scale_up_window_millis == 0
            || self.scale_down_window_millis <= self.scale_up_window_millis
        {
            return Err("autonomous_runtime_scheduler_config_invalid".to_string());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeRoutingConfig {
    pub adaptive: bool,
    pub load_report_interval_millis: u64,
    pub load_report_ttl_millis: u64,
    pub target_inflight: u32,
    pub target_queue_wait_millis: u64,
    pub max_inflight: u32,
    pub max_queued_items: u32,
    pub max_queued_bytes: u64,
    pub retry_budget_percent: u8,
}

impl Default for RuntimeRoutingConfig {
    fn default() -> Self {
        Self {
            adaptive: true,
            load_report_interval_millis: 500,
            load_report_ttl_millis: 2_000,
            target_inflight: 128,
            target_queue_wait_millis: 25,
            max_inflight: 256,
            max_queued_items: 512,
            max_queued_bytes: 64 * 1024 * 1024,
            retry_budget_percent: 10,
        }
    }
}

impl RuntimeRoutingConfig {
    fn validate(&self) -> Result<(), String> {
        if self.load_report_interval_millis == 0
            || self.load_report_ttl_millis <= self.load_report_interval_millis
            || self.target_inflight == 0
            || self.target_queue_wait_millis == 0
            || self.max_inflight == 0
            || self.max_queued_items == 0
            || self.max_queued_bytes == 0
            || self.retry_budget_percent > 100
        {
            return Err("autonomous_runtime_routing_config_invalid".to_string());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeContinuityConfig {
    pub strict_durability: bool,
    pub terminal_retention_millis: u64,
    pub tombstone_retention_millis: u64,
    pub max_terminal_records: u64,
    pub max_disk_bytes: u64,
    pub snapshot_chunk_bytes: u64,
    pub path: Option<PathBuf>,
}

impl Default for RuntimeContinuityConfig {
    fn default() -> Self {
        Self {
            strict_durability: true,
            terminal_retention_millis: 86_400_000,
            tombstone_retention_millis: 172_800_000,
            max_terminal_records: 1_000_000,
            max_disk_bytes: 8 * 1024 * 1024 * 1024,
            snapshot_chunk_bytes: 1024 * 1024,
            path: None,
        }
    }
}

impl RuntimeContinuityConfig {
    fn validate(&self) -> Result<(), String> {
        if self.terminal_retention_millis == 0
            || self.tombstone_retention_millis <= self.terminal_retention_millis
            || self.max_terminal_records == 0
            || self.max_disk_bytes == 0
            || !(128..16 * 1024 * 1024).contains(&self.snapshot_chunk_bytes)
        {
            return Err("autonomous_runtime_continuity_config_invalid".to_string());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RuntimeAutonomousConfig {
    pub schema_version: u16,
    pub enabled: bool,
    #[serde(default)]
    pub features: RuntimeFeatureGates,
    pub policy_revision: u64,
    pub policy: ScalingPolicy,
    /// Roles assigned to every node created by this capacity pool. Worker is
    /// required; gateway may be added to create a combined ingress/worker pool.
    #[serde(default = "default_managed_roles")]
    pub managed_roles: Vec<String>,
    pub gateway_nodes: u16,
    pub template_revision: String,
    pub reconcile_interval_millis: u64,
    pub startup_timeout_millis: u64,
    pub drain_timeout_millis: u64,
    pub termination_timeout_millis: u64,
    #[serde(default)]
    pub force_termination_after_drain_timeout: bool,
    #[serde(default)]
    pub scheduler: RuntimeSchedulerConfig,
    #[serde(default)]
    pub routing: RuntimeRoutingConfig,
    #[serde(default)]
    pub continuity: RuntimeContinuityConfig,
    pub driver: RuntimeCapacityDriverConfig,
}

impl RuntimeAutonomousConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version == 0
            || self.schema_version > AUTONOMOUS_CONFIG_SCHEMA_VERSION
            || !self.enabled
            || self.policy_revision == 0
            || self.managed_roles.is_empty()
            || !self.managed_roles.iter().any(|role| role == "worker")
            || self
                .managed_roles
                .iter()
                .any(|role| role != "worker" && role != "gateway")
            || self.managed_roles.iter().collect::<BTreeSet<_>>().len() != self.managed_roles.len()
            || self.template_revision.trim().is_empty()
            || self.reconcile_interval_millis == 0
            || self.startup_timeout_millis == 0
            || self.drain_timeout_millis == 0
            || self.termination_timeout_millis == 0
        {
            return Err("autonomous_runtime_config_invalid".to_string());
        }
        if self.features.horizontal_autoscaling
            && (!self.features.protocol_two
                || !self.features.durable_continuity
                || !self.features.telemetry
                || !self.features.controller_quorum
                || matches!(&self.driver, RuntimeCapacityDriverConfig::Disabled))
        {
            return Err("autonomous_runtime_horizontal_prerequisite_missing".to_string());
        }
        // Scaling down drains a node: with no disruption budget the
        // reconciler cannot start, and the controller would never run.
        if self.features.horizontal_autoscaling && self.policy.max_unavailable == 0 {
            return Err("autonomous_runtime_disruption_budget_zero".to_string());
        }
        self.policy.validate()?;
        self.scheduler.validate()?;
        self.routing.validate()?;
        self.continuity.validate()?;
        self.driver.validate()
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RuntimeCapacityDriverConfig {
    Disabled,
    Process {
        command: Vec<String>,
        working_directory: PathBuf,
    },
    Docker {
        image: String,
        pool: String,
        network: Option<String>,
        environment: Vec<String>,
    },
}

impl std::fmt::Debug for RuntimeCapacityDriverConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disabled => formatter.write_str("Disabled"),
            Self::Process {
                command,
                working_directory,
            } => formatter
                .debug_struct("Process")
                .field("executable", &command.first())
                .field("argument_count", &command.len().saturating_sub(1))
                .field("working_directory", working_directory)
                .finish(),
            Self::Docker {
                image,
                pool,
                network,
                environment,
            } => formatter
                .debug_struct("Docker")
                .field("image", image)
                .field("pool", pool)
                .field("network", network)
                .field(
                    "environment",
                    &format_args!("[redacted; {}]", environment.len()),
                )
                .finish(),
        }
    }
}

impl RuntimeCapacityDriverConfig {
    fn validate(&self) -> Result<(), String> {
        match self {
            Self::Disabled => Ok(()),
            Self::Process {
                command,
                working_directory,
            } if command
                .first()
                .is_some_and(|value| !value.trim().is_empty())
                && !working_directory.as_os_str().is_empty() =>
            {
                Ok(())
            }
            Self::Docker {
                image,
                pool,
                environment,
                ..
            } if !image.trim().is_empty()
                && !pool.trim().is_empty()
                && environment
                    .iter()
                    .all(|entry| entry.contains('=') && !entry.contains(['\n', '\r'])) =>
            {
                Ok(())
            }
            _ => Err("autonomous_runtime_driver_config_invalid".to_string()),
        }
    }
}

static EMBEDDED_AUTONOMOUS_CONFIG: OnceLock<RuntimeAutonomousConfig> = OnceLock::new();
static AUTONOMOUS_CONTROLLER_STARTED: Once = Once::new();
static AUTONOMOUS_CONTROLLER_STATUS: OnceLock<Mutex<AutonomousControllerStatus>> = OnceLock::new();

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AutonomousControllerStatus {
    pub configured: bool,
    pub running: bool,
    pub leader: bool,
    pub state: String,
    pub policy_revision: u64,
    pub observe_only: bool,
    pub automatic_scale_up: bool,
    pub automatic_scale_down: bool,
    pub desired_workers: u16,
    pub membership_generation: u64,
    pub tick_sequence: u64,
    pub last_tick_unix_millis: u64,
    pub last_error: Option<String>,
    pub last_decision: Option<ScalingDecision>,
    pub last_reconcile: Option<CapacityReconcileOutcome>,
}

fn controller_status() -> &'static Mutex<AutonomousControllerStatus> {
    AUTONOMOUS_CONTROLLER_STATUS.get_or_init(|| Mutex::new(AutonomousControllerStatus::default()))
}

pub fn autonomous_controller_status() -> AutonomousControllerStatus {
    controller_status().lock().unwrap().clone()
}

pub fn embedded_autonomous_config() -> Option<&'static RuntimeAutonomousConfig> {
    EMBEDDED_AUTONOMOUS_CONFIG.get()
}

pub fn register_autonomous_config_json(json: &[u8]) -> Result<(), String> {
    let config: RuntimeAutonomousConfig = serde_json::from_slice(json)
        .map_err(|error| format!("autonomous_runtime_config_decode_failed:{error}"))?;
    config.validate()?;
    EMBEDDED_AUTONOMOUS_CONFIG
        .set(config)
        .map_err(|_| "autonomous_runtime_config_already_registered".to_string())
}

struct RuntimeConsensusCommitter {
    cluster_id: String,
    sequence: AtomicU64,
}

impl RuntimeConsensusCommitter {
    fn new(cluster_id: &str) -> Self {
        Self {
            cluster_id: cluster_id.to_string(),
            sequence: AtomicU64::new(1),
        }
    }

    fn command_id(&self, actor: &str, reason: &str, mutation: &ControlMutation) -> String {
        let sequence = self.sequence.fetch_add(1, Ordering::Relaxed);
        let mut hasher = Sha256::new();
        hasher.update(self.cluster_id.as_bytes());
        hasher.update(actor.as_bytes());
        hasher.update(reason.as_bytes());
        hasher.update(serde_json::to_vec(mutation).unwrap_or_default());
        // Reconciliation fences are intentionally idempotent while operation
        // results and state revisions receive distinct payload hashes.
        if !matches!(mutation, ControlMutation::DesiredCapacity(_)) {
            hasher.update(sequence.to_be_bytes());
        }
        format!("runtime-{:x}", hasher.finalize())
    }
}

impl ControlPlaneCommitter for RuntimeConsensusCommitter {
    fn commit(
        &self,
        _leader: &str,
        _term: ControlTerm,
        _acknowledgements: &BTreeSet<String>,
        actor: &str,
        reason: &str,
        mutation: ControlMutation,
    ) -> Result<ControlLogEntry, String> {
        let response = super::consensus::commit_consensus_command(
            super::consensus::ConsensusCommand {
                command_id: self.command_id(actor, reason, &mutation),
                actor: actor.to_string(),
                reason: reason.to_string(),
                timestamp_unix_millis: unix_millis(),
                actor_sequence: 0,
                mutation: mutation.clone(),
            },
            Duration::from_secs(10),
        )?;
        Ok(ControlLogEntry {
            index: response.log_index,
            term: ControlTerm(response.control_term),
            actor: actor.to_string(),
            reason: reason.to_string(),
            timestamp_unix_millis: unix_millis(),
            actor_sequence: 0,
            mutation,
        })
    }
}

fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn controller_role_enabled() -> bool {
    super::readiness::local_roles().contains(super::telemetry::NodeRoles::CONTROLLER)
}

/// Reads one deployment environment variable: the runtime passes
/// `std::env::var_os`, tests a table of their own.
pub(super) type EnvironmentLookup<'a> = &'a dyn Fn(&str) -> Option<OsString>;

pub(super) fn environment_text(env: EnvironmentLookup<'_>, name: &str) -> Option<String> {
    env(name).and_then(|value| value.into_string().ok())
}

fn capacity_worker_environment(
    configured: &[String],
    managed_roles: &[String],
    env: EnvironmentLookup<'_>,
) -> Result<Vec<String>, String> {
    let mut environment = configured.to_vec();
    if let Some(raw) = environment_text(env, "MESH_CAPACITY_WORKER_ENV_ALLOWLIST") {
        for name in raw
            .split(',')
            .map(str::trim)
            .filter(|name| !name.is_empty())
        {
            if name.contains('=') || name.contains('\0') {
                return Err("capacity_worker_environment_name_invalid".to_string());
            }
            if let Some(value) = environment_text(env, name) {
                if value.contains(['\n', '\r', '\0']) {
                    return Err("capacity_worker_environment_value_invalid".to_string());
                }
                environment.push(format!("{name}={value}"));
            }
        }
    }
    // Role allocation is control-plane policy, not an inheritable controller
    // environment setting. Always replace any template value and never allow
    // managed nodes to acquire the controller role.
    environment.retain(|entry| entry.split_once('=').map(|value| value.0) != Some("MESH_ROLES"));
    environment.push(format!("MESH_ROLES={}", managed_roles.join(",")));
    environment.sort();
    environment.dedup_by(|left, right| {
        left.split_once('=').map(|value| value.0) == right.split_once('=').map(|value| value.0)
    });
    Ok(environment)
}

fn build_capacity_driver(
    config: &RuntimeAutonomousConfig,
    env: EnvironmentLookup<'_>,
) -> Result<Arc<dyn CapacityDriver>, String> {
    let operation_timeout = Duration::from_millis(
        config
            .startup_timeout_millis
            .min(config.termination_timeout_millis),
    );
    let driver: Arc<dyn CapacityDriver> = match &config.driver {
        RuntimeCapacityDriverConfig::Disabled => {
            return Err("autonomous_capacity_driver_disabled".to_string());
        }
        RuntimeCapacityDriverConfig::Process {
            command,
            working_directory,
        } => Arc::new(ProcessCapacityDriver::new(ProcessDriverConfig {
            command: command.clone(),
            working_directory: working_directory.clone(),
            environment: BTreeMap::from([(
                "MESH_ROLES".to_string(),
                config.managed_roles.join(","),
            )]),
        })),
        RuntimeCapacityDriverConfig::Docker {
            image,
            pool,
            network,
            environment,
        } => {
            let network = environment_text(env, "MESH_CAPACITY_DOCKER_NETWORK")
                .filter(|value| !value.trim().is_empty())
                .or_else(|| network.clone());
            let environment = capacity_worker_environment(environment, &config.managed_roles, env)?;
            if env("MESH_DOCKER_DRIVER_ENDPOINT").is_some() {
                let template = super::driver_service::RemoteDockerTemplate {
                    image: image.clone(),
                    pool: pool.clone(),
                    network,
                    environment,
                    operation_timeout_millis: operation_timeout.as_millis() as u64,
                };
                Arc::new(
                    super::driver_service::RemoteDockerCapacityDriver::from_environment(
                        template, env,
                    )?,
                )
            } else {
                let execution_prefix =
                    match environment_text(env, "MESH_DOCKER_EXECUTION_PREFIX_JSON") {
                        Some(raw) => serde_json::from_str::<Vec<String>>(&raw)
                            .map_err(|_| "docker_execution_prefix_invalid".to_string())?,
                        None => Vec::new(),
                    };
                let environment_file_mount = match (
                    env("MESH_DOCKER_ENV_HOST_DIRECTORY"),
                    env("MESH_DOCKER_ENV_DRIVER_DIRECTORY"),
                ) {
                    (Some(host), Some(driver)) => Some(DockerEnvironmentFileMount {
                        host_directory: PathBuf::from(host),
                        driver_directory: PathBuf::from(driver),
                    }),
                    (None, None) => None,
                    _ => return Err("docker_environment_mount_incomplete".to_string()),
                };
                Arc::new(DockerCapacityDriver::new(DockerDriverConfig {
                    binary: env("MESH_DOCKER_BINARY")
                        .map_or_else(|| PathBuf::from("docker"), PathBuf::from),
                    execution_prefix,
                    image: image.clone(),
                    pool: pool.clone(),
                    network,
                    environment,
                    environment_file_mount,
                    operation_timeout,
                }))
            }
        }
    };
    Ok(super::scaling::instrument_capacity_driver(driver))
}

fn latest_committed_desired(
    entries: &[ControlLogEntry],
    config: &RuntimeAutonomousConfig,
) -> Option<CommittedDesiredCapacity> {
    let mut current = None;
    for entry in entries {
        match &entry.mutation {
            ControlMutation::DesiredCapacity(desired) => {
                current = Some(CommittedDesiredCapacity {
                    log_index: entry.index,
                    term: entry.term,
                    desired: desired.clone(),
                });
            }
            ControlMutation::ManualOverride { worker_nodes } => {
                current = Some(CommittedDesiredCapacity {
                    log_index: entry.index,
                    term: entry.term,
                    desired: DesiredCapacity {
                        revision: DesiredRevision(entry.index.max(1)),
                        worker_nodes: *worker_nodes,
                        gateway_nodes: desired_gateway_nodes(config, *worker_nodes),
                        template_revision: config.template_revision.clone(),
                    },
                });
            }
            _ => {}
        }
    }
    current
}

fn desired_gateway_nodes(config: &RuntimeAutonomousConfig, worker_nodes: u16) -> u16 {
    if config.managed_roles.iter().any(|role| role == "gateway") {
        worker_nodes
    } else {
        config.gateway_nodes
    }
}

fn runtime_safety(
    observation: &super::scaling::CapacityObservation,
    snapshot: &super::operator::OperatorRuntimeSnapshot,
) -> Result<(Vec<ReconcileNodeSafety>, u16), String> {
    let current_generation = snapshot
        .nodes
        .iter()
        .map(|node| node.membership_generation)
        .max()
        .unwrap_or(0);
    let mut managed_runtime_names = BTreeSet::new();
    let safety = observation
        .nodes
        .iter()
        .map(|node| {
            let runtime = snapshot
                .nodes
                .iter()
                .find(|runtime| managed_runtime_matches(node, &runtime.node_id));
            let Some(runtime) = runtime else {
                return ReconcileNodeSafety {
                    node_id: node.node_id.clone(),
                    runtime_node_id: String::new(),
                    transferable_load: u64::MAX,
                    active_ownership_transfers: u32::MAX,
                    active_work: u32::MAX,
                    required_replica_responsibilities: u32::MAX,
                    only_active_copy: true,
                    membership_generation_acknowledged: false,
                    controller_voter: false,
                    unique_capability: true,
                };
            };
            managed_runtime_names.insert(runtime.node_id.clone());
            let unique_capability = runtime.handlers.iter().any(|handler| {
                snapshot
                    .nodes
                    .iter()
                    .filter(|candidate| candidate.handlers.contains(handler))
                    .count()
                    == 1
            });
            let active_work = runtime
                .inflight
                .saturating_add(runtime.continuity_active_work);
            ReconcileNodeSafety {
                node_id: node.node_id.clone(),
                runtime_node_id: runtime.node_id.clone(),
                transferable_load: u64::from(active_work)
                    .saturating_add(u64::from(runtime.continuity_replica_responsibilities)),
                active_ownership_transfers: runtime.continuity_active_ownership_transfers,
                active_work,
                required_replica_responsibilities: runtime.continuity_replica_responsibilities,
                only_active_copy: runtime.continuity_only_active_copy,
                membership_generation_acknowledged: snapshot.telemetry_complete
                    && runtime.membership_generation == current_generation,
                controller_voter: runtime.roles.iter().any(|role| role == "controller"),
                unique_capability,
            }
        })
        .collect();
    let unmanaged_ready = snapshot
        .nodes
        .iter()
        .filter(|node| {
            node.roles.iter().any(|role| role == "worker")
                && node.routing_eligible
                && !managed_runtime_names.contains(&node.node_id)
        })
        .count()
        .try_into()
        .unwrap_or(u16::MAX);
    Ok((safety, unmanaged_ready))
}

fn managed_runtime_matches(
    node: &super::scaling::ObservedCapacityNode,
    runtime_name: &str,
) -> bool {
    let provider_prefix = &node.node_id[..node.node_id.len().min(12)];
    if runtime_name.starts_with(provider_prefix) {
        return true;
    }
    let operation_prefix = &node.operation_id[..node.operation_id.len().min(12)];
    runtime_name
        .split_once('@')
        .map(|(name, _)| name.ends_with(operation_prefix))
        .unwrap_or(false)
}

fn commit_policy_if_needed(
    committer: &dyn ControlPlaneCommitter,
    entries: &[ControlLogEntry],
    config: &RuntimeAutonomousConfig,
) -> Result<(), String> {
    if entries.iter().any(|entry| {
        matches!(
            entry.mutation,
            ControlMutation::PolicyRevision { revision, .. }
                if revision == config.policy_revision
        )
    }) {
        return Ok(());
    }
    let policy_json = serde_json::to_string(&config.policy).expect("a scaling policy encodes");
    let policy_sha256 = format!("{:x}", Sha256::digest(policy_json.as_bytes()));
    committer.commit(
        "runtime-openraft",
        ControlTerm(0),
        &BTreeSet::new(),
        "mesh-autoscaler",
        "register embedded scaling policy",
        ControlMutation::PolicyRevision {
            revision: config.policy_revision,
            policy_json,
            policy_sha256,
        },
    )?;
    Ok(())
}

fn commit_membership_if_changed(
    committer: &dyn ControlPlaneCommitter,
    entries: &[ControlLogEntry],
    snapshot: &super::operator::OperatorRuntimeSnapshot,
) -> Result<u64, String> {
    let mut nodes: Vec<_> = snapshot
        .nodes
        .iter()
        .map(|node| node.node_id.clone())
        .collect();
    nodes.sort();
    nodes.dedup();
    if !snapshot.telemetry_complete || nodes.is_empty() {
        return Ok(entries
            .iter()
            .filter_map(|entry| match &entry.mutation {
                ControlMutation::MembershipIntent { generation, .. } => Some(*generation),
                _ => None,
            })
            .max()
            .unwrap_or(0));
    }
    let previous = entries
        .iter()
        .rev()
        .find_map(|entry| match &entry.mutation {
            ControlMutation::MembershipIntent { generation, nodes } => {
                Some((*generation, nodes.clone()))
            }
            _ => None,
        });
    if previous
        .as_ref()
        .is_some_and(|(_, previous_nodes)| previous_nodes == &nodes)
    {
        return Ok(previous.map_or(0, |(generation, _)| generation));
    }
    let generation = previous.map_or(1, |(generation, _)| generation.saturating_add(1));
    committer.commit(
        "runtime-openraft",
        ControlTerm(0),
        &BTreeSet::new(),
        "mesh-membership-controller",
        "record observed runtime membership",
        ControlMutation::MembershipIntent { generation, nodes },
    )?;
    Ok(generation)
}

fn initial_desired(
    committer: &dyn ControlPlaneCommitter,
    config: &RuntimeAutonomousConfig,
) -> Result<CommittedDesiredCapacity, String> {
    let desired = DesiredCapacity {
        revision: DesiredRevision(1),
        worker_nodes: config.policy.min_nodes,
        gateway_nodes: desired_gateway_nodes(config, config.policy.min_nodes),
        template_revision: config.template_revision.clone(),
    };
    let entry = committer.commit(
        "runtime-openraft",
        ControlTerm(0),
        &BTreeSet::new(),
        "mesh-autoscaler",
        "initialize desired capacity",
        ControlMutation::DesiredCapacity(desired.clone()),
    )?;
    Ok(CommittedDesiredCapacity {
        log_index: entry.index,
        term: entry.term,
        desired,
    })
}

/// What one leader tick decided.
#[derive(Debug)]
struct ControllerTick {
    decision: ScalingDecision,
    reconcile: CapacityReconcileOutcome,
    desired_workers: u16,
    membership_generation: u64,
}

/// The policy and reconciliation state a controller keeps across ticks.
struct AutonomousController {
    config: RuntimeAutonomousConfig,
    autoscaler: Autoscaler,
    reconciler: CapacityReconciler,
    driver: Arc<dyn CapacityDriver>,
    cluster_id: String,
}

impl AutonomousController {
    /// Fails when the driver refuses its configuration; the policy was
    /// validated with the embedded config.
    fn new(
        config: RuntimeAutonomousConfig,
        driver: Arc<dyn CapacityDriver>,
        cluster_id: String,
    ) -> Result<Self, String> {
        let mut autoscaler = Autoscaler::new(config.policy.clone())?;
        autoscaler.set_action_gates(
            !config.features.horizontal_observe_only && config.features.automatic_scale_up,
            !config.features.horizontal_observe_only && config.features.automatic_scale_down,
        );
        let reconciler = CapacityReconciler::new_runtime(
            driver.clone(),
            config.policy.max_unavailable,
            Duration::from_millis(config.drain_timeout_millis),
            config.force_termination_after_drain_timeout,
        )?;
        Ok(Self {
            config,
            autoscaler,
            reconciler,
            driver,
            cluster_id,
        })
    }

    /// One leader tick: register the policy, record membership, evaluate
    /// `snapshot` against the committed desired capacity, commit a changed
    /// target, and reconcile the provider toward it.
    fn tick(
        &mut self,
        committer: &dyn ControlPlaneCommitter,
        consensus: &super::consensus::ConsensusRuntimeSnapshot,
        snapshot: &super::operator::OperatorRuntimeSnapshot,
        paused: bool,
    ) -> Result<ControllerTick, String> {
        let config = &self.config;
        commit_policy_if_needed(committer, &consensus.entries, config)?;
        let mut committed = match latest_committed_desired(&consensus.entries, config) {
            Some(committed) => committed,
            None => initial_desired(committer, config)?,
        };
        // Desired state may predate this election; provider operations must
        // always be fenced by the current OpenRaft term.
        committed.term = ControlTerm(consensus.current_term);
        let membership_generation =
            commit_membership_if_changed(committer, &consensus.entries, snapshot)?;
        self.autoscaler.set_paused(paused);
        let workers: Vec<_> = snapshot
            .nodes
            .iter()
            .filter(|node| node.roles.iter().any(|role| role == "worker"))
            .collect();
        let gateway_inflight: u64 = snapshot
            .nodes
            .iter()
            .filter(|node| node.roles.iter().any(|role| role == "gateway"))
            .map(|node| u64::from(node.inflight))
            .sum();
        let worker_inflight: u64 = workers.iter().map(|node| u64::from(node.inflight)).sum();
        let sample = ScalingSample {
            observed_at: Instant::now(),
            // A clustered request is admitted at the gateway and reserved
            // again at its worker. Take the larger side of that pipeline
            // so ingress demand is visible without double counting.
            cluster_inflight: gateway_inflight.max(worker_inflight),
            cluster_pressure_ewma: workers
                .iter()
                .map(|node| node.pressure)
                .fold(0.0_f64, f64::max),
            ready_nodes: workers
                .iter()
                .filter(|node| node.routing_eligible)
                .count()
                .try_into()
                .unwrap_or(u16::MAX),
            reports_complete: snapshot.telemetry_complete,
            driver_healthy: true,
            controller_stable: consensus.voter_ids.len() == 1 || consensus.voter_ids.len() >= 3,
            // Scale-down needs at least one safe retirement candidate; it
            // does not require every worker to be disposable. The
            // reconciler applies the stricter per-candidate ownership,
            // replica, capability, quorum, and generation gates before it
            // can begin a drain, one node at a time.
            continuity_healthy: workers.iter().any(|node| !node.continuity_only_active_copy),
            drain_incomplete: !self.reconciler.drain_progress().is_empty(),
        };
        let decision = self
            .autoscaler
            .evaluate(committed.desired.worker_nodes, sample);
        if decision.bounded_desired != committed.desired.worker_nodes {
            let desired = DesiredCapacity {
                revision: DesiredRevision(committed.desired.revision.0.saturating_add(1)),
                worker_nodes: decision.bounded_desired,
                gateway_nodes: desired_gateway_nodes(config, decision.bounded_desired),
                template_revision: config.template_revision.clone(),
            };
            let entry = committer.commit(
                "runtime-openraft",
                committed.term,
                &BTreeSet::new(),
                "mesh-autoscaler",
                "autonomous scaling decision",
                ControlMutation::DesiredCapacity(desired.clone()),
            )?;
            committed = CommittedDesiredCapacity {
                log_index: entry.index,
                term: entry.term,
                desired,
            };
        }
        let reconcile = if config.features.horizontal_observe_only {
            let observation = self.driver.observe_capacity(&self.cluster_id)?;
            CapacityReconcileOutcome {
                desired_workers: committed.desired.worker_nodes,
                observed_workers: observation
                    .nodes
                    .iter()
                    .filter(|node| {
                        !matches!(
                            node.lifecycle,
                            super::scaling::CapacityNodeLifecycle::Removed
                                | super::scaling::CapacityNodeLifecycle::Failed
                        )
                    })
                    .count()
                    .try_into()
                    .unwrap_or(u16::MAX),
                ensured: Vec::new(),
                drains: Vec::new(),
                constraints: vec!["horizontal_observe_only".to_string()],
            }
        } else {
            self.reconciler.reconcile_with_observed_capacity(
                committer,
                &self.cluster_id,
                &consensus.node_name,
                &BTreeSet::new(),
                &committed,
                "mesh-reconciler",
                |observation| runtime_safety(observation, snapshot),
            )?
        };
        Ok(ControllerTick {
            decision,
            reconcile,
            desired_workers: committed.desired.worker_nodes,
            membership_generation,
        })
    }
}

fn controller_loop(mut controller: AutonomousController) {
    let committer = RuntimeConsensusCommitter::new(&controller.cluster_id);
    let interval = Duration::from_millis(controller.config.reconcile_interval_millis);
    let mut was_leader = false;
    loop {
        let Some(state) = super::node::node_state() else {
            break;
        };
        let Some(consensus) = super::consensus::consensus_runtime_snapshot() else {
            std::thread::park_timeout(interval);
            continue;
        };
        let leader =
            consensus.state == "leader" && consensus.current_leader == Some(consensus.node_id);
        {
            let mut status = controller_status().lock().unwrap();
            status.running = true;
            status.leader = leader;
            status.state = if leader { "leader" } else { "standby" }.to_string();
            status.tick_sequence = status.tick_sequence.saturating_add(1);
            status.last_tick_unix_millis = unix_millis();
        }
        if !leader {
            was_leader = false;
            std::thread::park_timeout(interval);
            continue;
        }

        if !was_leader {
            controller
                .reconciler
                .restore_from_control_entries(&consensus.entries);
            was_leader = true;
        }

        // Disconnect delivery and continuity-record replication are separate
        // streams. Re-drive recovery from the fenced leader so either arrival
        // order converges without relying on one edge-triggered callback.
        super::node::recover_pending_owner_losses_if_coordinator();

        let snapshot = super::operator::runtime_snapshot_from_state(state);
        let tick = controller.tick(
            &committer,
            &consensus,
            &snapshot,
            super::operator::autoscaler_paused(),
        );
        let mut status = controller_status().lock().unwrap();
        match tick {
            Ok(tick) => {
                status.last_decision = Some(tick.decision);
                status.last_reconcile = Some(tick.reconcile);
                status.desired_workers = tick.desired_workers;
                status.membership_generation = tick.membership_generation;
                status.last_error = None;
            }
            Err(error) => {
                status.last_error = Some(error.clone());
                eprintln!("mesh autonomous: transition=tick_failed reason={error}");
            }
        }
        drop(status);
        std::thread::park_timeout(interval);
    }
    let mut status = controller_status().lock().unwrap();
    status.running = false;
    status.leader = false;
    status.state = "stopped".to_string();
}

/// Starts the embedded policy/reconciliation service on controller nodes.
/// Followers keep a warm driver instance but provider calls are leader-fenced.
pub fn start_autonomous_controller() -> Result<bool, String> {
    let Some(config) = embedded_autonomous_config().cloned() else {
        return Ok(false);
    };
    // A validated config enables horizontal autoscaling only together with
    // controller quorum and a capacity driver.
    if !controller_role_enabled() || !config.features.horizontal_autoscaling {
        return Ok(false);
    }
    let cluster_id = std::env::var("MESH_CLUSTER_ID")
        .map_err(|_| "autonomous_cluster_id_missing".to_string())?;
    let driver = build_capacity_driver(&config, &|name| std::env::var_os(name))?;
    let controller = AutonomousController::new(config, driver, cluster_id)?;
    {
        let features = &controller.config.features;
        let mut status = controller_status().lock().unwrap();
        status.configured = true;
        status.policy_revision = controller.config.policy_revision;
        status.observe_only = features.horizontal_observe_only;
        status.automatic_scale_up = features.automatic_scale_up;
        status.automatic_scale_down = features.automatic_scale_down;
        status.state = "starting".to_string();
    }
    let mut started = false;
    AUTONOMOUS_CONTROLLER_STARTED.call_once(|| {
        started = true;
        std::thread::Builder::new()
            .name("mesh-autonomous-controller".to_string())
            .spawn(move || controller_loop(controller))
            .expect("failed to start Mesh autonomous controller");
    });
    if !started {
        return Err("autonomous_controller_already_started".to_string());
    }
    Ok(true)
}

#[no_mangle]
pub extern "C" fn mesh_register_autonomous_config_json(data: *const u8, len: u64) -> i32 {
    if data.is_null() || len == 0 || len > 1024 * 1024 {
        return -1;
    }
    // At most 1 MiB, so the length fits any usize.
    let bytes = unsafe { std::slice::from_raw_parts(data, len as usize) };
    match register_autonomous_config_json(bytes) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("mesh autonomous: embedded_config_rejected reason={error}");
            -1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::operator::{OperatorNodeRuntimeSnapshot, OperatorRuntimeSnapshot};
    use super::super::scaling::{
        CapacityNodeLifecycle, CapacityObservation, DriverOperation, DriverOperationState,
        FakeCapacityDriver, ObservedCapacityNode,
    };
    use super::*;

    #[test]
    fn managed_runtime_matches_provider_id_or_operation_derived_name() {
        let observed = super::super::scaling::ObservedCapacityNode {
            node_id: "dcb65e371624a8cef6267e0361c5679231218feceb49d287b1434ef6302f12d5".to_string(),
            operation_id: "5fdb15dad23d6924d4a4acd809e6829c62cc979f8f39a474c8365c706db24afc"
                .to_string(),
            control_term: ControlTerm(2),
            desired_revision: DesiredRevision(4),
            template_revision: "proof-v1".to_string(),
            lifecycle: super::super::scaling::CapacityNodeLifecycle::Ready,
        };
        assert!(managed_runtime_matches(
            &observed,
            "dcb65e371624@provider:4370"
        ));
        assert!(managed_runtime_matches(
            &observed,
            "mesh-workers-5fdb15dad23d@mesh-workers-5fdb15dad23d:4370"
        ));
        assert!(!managed_runtime_matches(&observed, "worker1@worker1:4370"));
    }

    fn config() -> RuntimeAutonomousConfig {
        RuntimeAutonomousConfig {
            schema_version: AUTONOMOUS_CONFIG_SCHEMA_VERSION,
            enabled: true,
            features: RuntimeFeatureGates::default(),
            policy_revision: 1,
            policy: ScalingPolicy {
                min_nodes: 2,
                max_nodes: 5,
                target_inflight_per_node: 32,
                scale_up_window_millis: 1_000,
                scale_down_window_millis: 10_000,
                cooldown_millis: 2_000,
                max_scale_up_step: 2,
                max_scale_down_step: 1,
                max_unavailable: 1,
            },
            managed_roles: vec!["gateway".to_string(), "worker".to_string()],
            gateway_nodes: 2,
            template_revision: "sha256:abc".to_string(),
            reconcile_interval_millis: 250,
            startup_timeout_millis: 30_000,
            drain_timeout_millis: 30_000,
            termination_timeout_millis: 30_000,
            force_termination_after_drain_timeout: false,
            scheduler: RuntimeSchedulerConfig::default(),
            routing: RuntimeRoutingConfig::default(),
            continuity: RuntimeContinuityConfig::default(),
            driver: RuntimeCapacityDriverConfig::Docker {
                image: "registry.example.com/app@sha256:abc".to_string(),
                pool: "workers".to_string(),
                network: Some("app-private".to_string()),
                environment: vec!["PORT=8080".to_string()],
            },
        }
    }

    #[test]
    fn embedded_runtime_config_round_trips() {
        let config = config();
        let encoded = serde_json::to_vec(&config).unwrap();
        let decoded: RuntimeAutonomousConfig = serde_json::from_slice(&encoded).unwrap();
        decoded.validate().unwrap();
        assert_eq!(decoded, config);
    }

    /// The reconciler refuses a zero disruption budget, so a controller
    /// started with one stopped at once and never scaled.
    #[test]
    fn horizontal_autoscaling_refuses_a_zero_disruption_budget() {
        let mut config = config();
        config.policy.max_unavailable = 0;
        assert_eq!(
            config.validate(),
            Err("autonomous_runtime_disruption_budget_zero".to_string())
        );
        config.features.horizontal_autoscaling = false;
        assert_eq!(config.validate(), Ok(()));
    }

    #[test]
    fn runtime_driver_debug_redacts_worker_environment_values() {
        let driver = RuntimeCapacityDriverConfig::Docker {
            image: "image@sha256:abc".to_string(),
            pool: "workers".to_string(),
            network: Some("mesh".to_string()),
            environment: vec!["DATABASE_URL=postgres://debug-secret".to_string()],
        };

        let rendered = format!("{driver:?}");

        assert!(!rendered.contains("postgres://debug-secret"));
        assert!(rendered.contains("[redacted; 1]"));
    }

    #[test]
    fn process_and_disabled_drivers_debug_without_their_arguments() {
        assert_eq!(
            format!("{:?}", RuntimeCapacityDriverConfig::Disabled),
            "Disabled"
        );
        let rendered = format!(
            "{:?}",
            RuntimeCapacityDriverConfig::Process {
                command: vec!["worker".to_string(), "--token=secret".to_string()],
                working_directory: PathBuf::from("/srv"),
            }
        );
        assert!(rendered.contains("argument_count: 1"), "{rendered}");
        assert!(!rendered.contains("secret"), "{rendered}");
    }

    /// Each rule a config breaks is refused, naming the section it is in.
    #[test]
    fn invalid_configs_are_refused_by_section() {
        type Change = fn(&mut RuntimeAutonomousConfig);
        let cases: &[(Change, &str)] = &[
            (
                |c| c.schema_version = 0,
                "autonomous_runtime_config_invalid",
            ),
            (
                |c| c.managed_roles = vec!["gateway".to_string()],
                "autonomous_runtime_config_invalid",
            ),
            (
                |c| c.managed_roles.push("controller".to_string()),
                "autonomous_runtime_config_invalid",
            ),
            (
                |c| c.managed_roles.push("worker".to_string()),
                "autonomous_runtime_config_invalid",
            ),
            (
                |c| c.features.telemetry = false,
                "autonomous_runtime_horizontal_prerequisite_missing",
            ),
            (
                |c| c.driver = RuntimeCapacityDriverConfig::Disabled,
                "autonomous_runtime_horizontal_prerequisite_missing",
            ),
            (|c| c.policy.min_nodes = 0, "scaling_node_bounds_invalid"),
            (
                |c| c.scheduler.min_workers = 0,
                "autonomous_runtime_scheduler_config_invalid",
            ),
            (
                |c| c.routing.retry_budget_percent = 101,
                "autonomous_runtime_routing_config_invalid",
            ),
            (
                |c| c.continuity.snapshot_chunk_bytes = 64,
                "autonomous_runtime_continuity_config_invalid",
            ),
            (
                |c| {
                    c.driver = RuntimeCapacityDriverConfig::Process {
                        command: vec![" ".to_string()],
                        working_directory: PathBuf::from("/srv"),
                    }
                },
                "autonomous_runtime_driver_config_invalid",
            ),
            (
                |c| {
                    c.driver = RuntimeCapacityDriverConfig::Docker {
                        image: "image".to_string(),
                        pool: "workers".to_string(),
                        network: None,
                        environment: vec!["NOT_AN_ASSIGNMENT".to_string()],
                    }
                },
                "autonomous_runtime_driver_config_invalid",
            ),
        ];
        for (change, expected) in cases {
            let mut config = config();
            change(&mut config);
            assert_eq!(config.validate(), Err(expected.to_string()), "{expected}");
        }

        let mut config = config();
        config.driver = RuntimeCapacityDriverConfig::Process {
            command: vec!["./worker".to_string()],
            working_directory: PathBuf::from("/srv"),
        };
        assert_eq!(config.validate(), Ok(()));
        config.features.horizontal_autoscaling = false;
        config.driver = RuntimeCapacityDriverConfig::Disabled;
        assert_eq!(config.validate(), Ok(()));
    }

    #[test]
    fn a_config_without_managed_roles_manages_workers() {
        let mut value = serde_json::to_value(config()).unwrap();
        value.as_object_mut().unwrap().remove("managed_roles");
        let decoded: RuntimeAutonomousConfig = serde_json::from_value(value).unwrap();
        assert_eq!(decoded.managed_roles, vec!["worker".to_string()]);
    }

    /// Nothing in this process registers a config: a registered one would
    /// switch every test here into autonomous mode.
    #[test]
    fn registration_refuses_undecodable_and_invalid_configs() {
        assert!(register_autonomous_config_json(b"{")
            .unwrap_err()
            .starts_with("autonomous_runtime_config_decode_failed:"));
        let mut disabled = config();
        disabled.enabled = false;
        assert_eq!(
            register_autonomous_config_json(&serde_json::to_vec(&disabled).unwrap()),
            Err("autonomous_runtime_config_invalid".to_string())
        );
        assert_eq!(
            mesh_register_autonomous_config_json(std::ptr::null(), 1),
            -1
        );
        assert_eq!(mesh_register_autonomous_config_json(b"{}".as_ptr(), 0), -1);
        assert_eq!(
            mesh_register_autonomous_config_json(b"{}".as_ptr(), 1024 * 1024 + 1),
            -1
        );
        assert_eq!(mesh_register_autonomous_config_json(b"{".as_ptr(), 1), -1);
        assert!(embedded_autonomous_config().is_none());
        assert_eq!(start_autonomous_controller(), Ok(false));
        assert!(!autonomous_controller_status().configured);
    }

    fn lookup<'a>(table: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        move |name| {
            table
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| OsString::from(value))
        }
    }

    #[test]
    fn worker_environment_adds_allowlisted_values_and_owns_the_roles() {
        let configured = [
            "PORT=8080".to_string(),
            "MESH_ROLES=controller".to_string(),
            "PORT=9090".to_string(),
        ];
        let roles = ["worker".to_string(), "gateway".to_string()];
        let env = lookup(&[
            (
                "MESH_CAPACITY_WORKER_ENV_ALLOWLIST",
                " DATABASE_URL, ,UNSET ",
            ),
            ("DATABASE_URL", "postgres://db"),
        ]);
        assert_eq!(
            capacity_worker_environment(&configured, &roles, &env),
            Ok(vec![
                "DATABASE_URL=postgres://db".to_string(),
                "MESH_ROLES=worker,gateway".to_string(),
                "PORT=8080".to_string(),
            ])
        );
        assert_eq!(
            capacity_worker_environment(&[], &roles, &lookup(&[])),
            Ok(vec!["MESH_ROLES=worker,gateway".to_string()])
        );
        let env = lookup(&[("MESH_CAPACITY_WORKER_ENV_ALLOWLIST", "A=B")]);
        assert_eq!(
            capacity_worker_environment(&[], &roles, &env),
            Err("capacity_worker_environment_name_invalid".to_string())
        );
        let env = lookup(&[
            ("MESH_CAPACITY_WORKER_ENV_ALLOWLIST", "SECRET"),
            ("SECRET", "two\nlines"),
        ]);
        assert_eq!(
            capacity_worker_environment(&[], &roles, &env),
            Err("capacity_worker_environment_value_invalid".to_string())
        );
    }

    #[test]
    fn capacity_drivers_are_built_from_the_config_and_environment() {
        let docker = config();
        let env = lookup(&[
            ("MESH_CAPACITY_DOCKER_NETWORK", "override"),
            ("MESH_DOCKER_BINARY", "/nonexistent/mesh-docker"),
            ("MESH_DOCKER_EXECUTION_PREFIX_JSON", "[\"exec\"]"),
            ("MESH_DOCKER_ENV_HOST_DIRECTORY", "/tmp/mesh-env"),
            ("MESH_DOCKER_ENV_DRIVER_DIRECTORY", "/env"),
        ]);
        let driver = build_capacity_driver(&docker, &env)
            .unwrap_or_else(|error| panic!("docker driver: {error}"));
        // The driver runs the configured binary.
        assert!(driver
            .validate_configuration()
            .unwrap_err()
            .starts_with("docker_driver_command_failed:"));
        assert!(build_capacity_driver(&docker, &lookup(&[])).is_ok());

        let refusals: &[(&[(&str, &str)], &str)] = &[
            (
                &[("MESH_DOCKER_EXECUTION_PREFIX_JSON", "exec")],
                "docker_execution_prefix_invalid",
            ),
            (
                &[("MESH_DOCKER_ENV_HOST_DIRECTORY", "/tmp/mesh-env")],
                "docker_environment_mount_incomplete",
            ),
            (
                &[("MESH_CAPACITY_WORKER_ENV_ALLOWLIST", "A=B")],
                "capacity_worker_environment_name_invalid",
            ),
            // The remote driver reads the rest of its settings from the
            // same place.
            (
                &[("MESH_DOCKER_DRIVER_ENDPOINT", "127.0.0.1:1")],
                "docker_driver_shared_key_missing_or_invalid",
            ),
        ];
        for (table, expected) in refusals {
            assert_eq!(
                build_capacity_driver(&docker, &lookup(table)).err(),
                Some(expected.to_string())
            );
        }

        let mut process = config();
        process.driver = RuntimeCapacityDriverConfig::Process {
            command: vec!["./worker".to_string()],
            working_directory: PathBuf::from("/nonexistent"),
        };
        let driver = build_capacity_driver(&process, &lookup(&[]))
            .unwrap_or_else(|error| panic!("process driver: {error}"));
        assert_eq!(
            driver.validate_configuration(),
            Err("process_driver_working_directory_invalid".to_string())
        );
        process.driver = RuntimeCapacityDriverConfig::Disabled;
        assert_eq!(
            build_capacity_driver(&process, &lookup(&[])).err(),
            Some("autonomous_capacity_driver_disabled".to_string())
        );
    }

    fn entry(index: u64, mutation: ControlMutation) -> ControlLogEntry {
        ControlLogEntry {
            index,
            term: ControlTerm(1),
            actor: "test".to_string(),
            reason: "test".to_string(),
            timestamp_unix_millis: 1,
            actor_sequence: 0,
            mutation,
        }
    }

    #[test]
    fn the_latest_desired_capacity_or_manual_override_is_the_target() {
        let mut config = config();
        assert_eq!(latest_committed_desired(&[], &config), None);
        let desired = DesiredCapacity {
            revision: DesiredRevision(3),
            worker_nodes: 4,
            gateway_nodes: 4,
            template_revision: "v".to_string(),
        };
        let entries = [
            entry(5, ControlMutation::DesiredCapacity(desired.clone())),
            entry(6, ControlMutation::PauseAutoscaler { paused: true }),
        ];
        assert_eq!(
            latest_committed_desired(&entries, &config),
            Some(CommittedDesiredCapacity {
                log_index: 5,
                term: ControlTerm(1),
                desired,
            })
        );

        let entries = [
            entries[0].clone(),
            entry(7, ControlMutation::ManualOverride { worker_nodes: 3 }),
        ];
        let expected = |gateway_nodes| CommittedDesiredCapacity {
            log_index: 7,
            term: ControlTerm(1),
            desired: DesiredCapacity {
                revision: DesiredRevision(7),
                worker_nodes: 3,
                gateway_nodes,
                template_revision: "sha256:abc".to_string(),
            },
        };
        // A pool that manages gateways sizes them with its workers.
        assert_eq!(
            latest_committed_desired(&entries, &config),
            Some(expected(3))
        );
        config.managed_roles = vec!["worker".to_string()];
        assert_eq!(
            latest_committed_desired(&entries, &config),
            Some(expected(2))
        );
    }

    fn runtime_node(name: &str, roles: &[&str], inflight: u32) -> OperatorNodeRuntimeSnapshot {
        OperatorNodeRuntimeSnapshot {
            node_id: name.to_string(),
            protocol_version: 2,
            protocol_capabilities: 0,
            autonomous_protocol_enabled: true,
            protocol_disabled_reason: None,
            roles: roles.iter().map(|role| role.to_string()).collect(),
            state: "ready".to_string(),
            routing_eligible: true,
            capacity_units: 1,
            active_workers: 1,
            runnable_actors: 0,
            inflight,
            continuity_active_work: 0,
            continuity_replica_responsibilities: 0,
            continuity_active_ownership_transfers: 0,
            continuity_only_active_copy: false,
            queued_items: 0,
            queued_bytes: 0,
            reservations: 0,
            pressure: 0.0,
            dominant_signal: "inflight".to_string(),
            report_sequence: 1,
            control_term: 1,
            membership_generation: 1,
            failure_domain: String::new(),
            handlers: Vec::new(),
        }
    }

    fn runtime_snapshot(nodes: Vec<OperatorNodeRuntimeSnapshot>) -> OperatorRuntimeSnapshot {
        OperatorRuntimeSnapshot {
            schema_version: 6,
            local_node: "controller@c:4370".to_string(),
            telemetry_complete: true,
            desired_capacity: 0,
            observed_capacity: 0,
            ready_capacity: 0,
            draining_capacity: 0,
            autoscaler_paused: false,
            scheduler_min_workers: 1,
            scheduler_max_workers: 1,
            scheduler_active_workers: 1,
            local_readiness: Default::default(),
            consensus: None,
            autonomous: Default::default(),
            local_telemetry: Default::default(),
            local_peer_sessions: Vec::new(),
            local_continuity_store: None,
            local_continuity_store_error: None,
            nodes,
        }
    }

    fn observed(node_id: &str, operation_id: &str) -> ObservedCapacityNode {
        ObservedCapacityNode {
            node_id: node_id.to_string(),
            operation_id: operation_id.to_string(),
            control_term: ControlTerm(1),
            desired_revision: DesiredRevision(1),
            template_revision: "v".to_string(),
            lifecycle: CapacityNodeLifecycle::Ready,
        }
    }

    #[test]
    fn runtime_safety_matches_provider_nodes_to_runtime_members() {
        let mut managed = runtime_node("abcdef123456@abcdef123456:4370", &["worker"], 2);
        managed.continuity_active_work = 1;
        managed.continuity_replica_responsibilities = 3;
        managed.continuity_active_ownership_transfers = 4;
        managed.handlers = vec!["only-here".to_string(), "shared".to_string()];
        let mut fixed = runtime_node("fixed@fixed:4370", &["worker"], 0);
        fixed.handlers = vec!["shared".to_string()];
        let mut stale = runtime_node("stale@stale:4370", &["worker", "controller"], 0);
        stale.membership_generation = 0;
        stale.routing_eligible = false;
        let snapshot = runtime_snapshot(vec![managed, fixed, stale]);
        let observation = CapacityObservation {
            nodes: vec![
                observed("abcdef123456ffffffff", "operation-a"),
                observed("unknown-provider-node", "operation-b"),
                observed("provider-id-for-stale", "stale"),
            ],
        };

        let (safety, unmanaged_ready) = runtime_safety(&observation, &snapshot).unwrap();

        assert_eq!(
            safety[0],
            ReconcileNodeSafety {
                node_id: "abcdef123456ffffffff".to_string(),
                runtime_node_id: "abcdef123456@abcdef123456:4370".to_string(),
                transferable_load: 6,
                active_ownership_transfers: 4,
                active_work: 3,
                required_replica_responsibilities: 3,
                only_active_copy: false,
                membership_generation_acknowledged: true,
                controller_voter: false,
                unique_capability: true,
            }
        );
        // A provider node no member answers for is unsafe in every way.
        assert_eq!(safety[1].runtime_node_id, "");
        assert_eq!(safety[1].active_work, u32::MAX);
        assert!(safety[1].only_active_copy && safety[1].unique_capability);
        // Matched by the operation id its name ends with.
        assert_eq!(safety[2].runtime_node_id, "stale@stale:4370");
        assert!(safety[2].controller_voter);
        assert!(!safety[2].membership_generation_acknowledged);
        assert!(!safety[2].unique_capability);
        // Only the fixed worker counts as unmanaged Ready capacity.
        assert_eq!(unmanaged_ready, 1);
    }

    #[test]
    fn runtime_command_ids_repeat_only_for_desired_capacity() {
        let committer = RuntimeConsensusCommitter::new("cluster");
        let desired = ControlMutation::DesiredCapacity(DesiredCapacity {
            revision: DesiredRevision(1),
            worker_nodes: 2,
            gateway_nodes: 0,
            template_revision: "v".to_string(),
        });
        assert_eq!(
            committer.command_id("actor", "reason", &desired),
            committer.command_id("actor", "reason", &desired)
        );
        let pause = ControlMutation::PauseAutoscaler { paused: true };
        assert_ne!(
            committer.command_id("actor", "reason", &pause),
            committer.command_id("actor", "reason", &pause)
        );
        // No test here runs the embedded consensus: nothing takes the commit.
        assert_eq!(
            committer.commit(
                "leader",
                ControlTerm(0),
                &BTreeSet::new(),
                "actor",
                "reason",
                pause
            ),
            Err("consensus_rpc_server_unavailable".to_string())
        );
    }

    /// A control log that takes every commit but the one it refuses.
    #[derive(Default)]
    struct TestLog {
        entries: Mutex<Vec<ControlLogEntry>>,
        refused: Option<&'static str>,
    }

    impl TestLog {
        fn refusing(reason: &'static str) -> Self {
            Self {
                refused: Some(reason),
                ..Self::default()
            }
        }

        fn reasons(&self) -> Vec<String> {
            let entries = self.entries.lock().unwrap();
            entries.iter().map(|entry| entry.reason.clone()).collect()
        }

        fn consensus(&self) -> super::super::consensus::ConsensusRuntimeSnapshot {
            super::super::consensus::ConsensusRuntimeSnapshot {
                node_id: 1,
                node_name: "controller@c:4370".to_string(),
                state: "leader".to_string(),
                current_term: 3,
                current_leader: Some(1),
                last_applied_log: None,
                voter_ids: vec![1],
                entries: self.entries.lock().unwrap().clone(),
            }
        }
    }

    impl ControlPlaneCommitter for TestLog {
        fn commit(
            &self,
            _leader: &str,
            term: ControlTerm,
            _acknowledgements: &BTreeSet<String>,
            actor: &str,
            reason: &str,
            mutation: ControlMutation,
        ) -> Result<ControlLogEntry, String> {
            if self.refused == Some(reason) {
                return Err(format!("refused:{reason}"));
            }
            let mut entries = self.entries.lock().unwrap();
            let entry = ControlLogEntry {
                index: entries.len() as u64 + 1,
                term,
                actor: actor.to_string(),
                reason: reason.to_string(),
                timestamp_unix_millis: 1,
                actor_sequence: 0,
                mutation,
            };
            entries.push(entry.clone());
            Ok(entry)
        }
    }

    fn controller(config: RuntimeAutonomousConfig) -> AutonomousController {
        let driver: Arc<dyn CapacityDriver> = Arc::new(FakeCapacityDriver::new());
        AutonomousController::new(config, driver, "cluster".to_string())
            .unwrap_or_else(|error| panic!("controller: {error}"))
    }

    #[test]
    fn a_controller_needs_a_driver_that_accepts_its_configuration() {
        let refusing: Arc<dyn CapacityDriver> =
            Arc::new(ProcessCapacityDriver::new(ProcessDriverConfig {
                command: vec!["./worker".to_string()],
                working_directory: PathBuf::from("/nonexistent"),
                environment: BTreeMap::new(),
            }));
        assert_eq!(
            AutonomousController::new(config(), refusing, "cluster".to_string()).err(),
            Some("process_driver_working_directory_invalid".to_string())
        );
        let mut invalid = config();
        invalid.policy.min_nodes = 0;
        let driver: Arc<dyn CapacityDriver> = Arc::new(FakeCapacityDriver::new());
        assert_eq!(
            AutonomousController::new(invalid, driver, "cluster".to_string()).err(),
            Some("scaling_node_bounds_invalid".to_string())
        );
    }

    #[test]
    fn a_first_leader_tick_registers_policy_membership_and_minimum_capacity() {
        let log = TestLog::default();
        let mut controller = controller(config());
        let snapshot =
            runtime_snapshot(vec![runtime_node("controller@c:4370", &["controller"], 0)]);

        let tick = controller
            .tick(&log, &log.consensus(), &snapshot, false)
            .expect("first tick");

        assert_eq!(
            log.reasons(),
            [
                "register embedded scaling policy",
                "initialize desired capacity",
                "record observed runtime membership",
                "capacity reconciliation fence",
                "ensure worker capacity",
                "record ensure worker result",
                "ensure worker capacity",
                "record ensure worker result",
            ]
        );
        assert_eq!(
            tick.decision.action,
            super::super::scaling::ScalingAction::Hold
        );
        assert_eq!(tick.desired_workers, 2);
        assert_eq!(tick.membership_generation, 1);
        assert_eq!(tick.reconcile.ensured.len(), 2);

        // The next tick finds the policy, target, and membership committed.
        let committed = log.reasons().len();
        let tick = controller
            .tick(&log, &log.consensus(), &snapshot, false)
            .expect("second tick");
        assert_eq!(
            log.reasons()[committed..],
            ["capacity reconciliation fence"]
        );
        assert_eq!(tick.membership_generation, 1);
        assert!(tick.reconcile.ensured.is_empty());

        // A new member is a new generation; incomplete telemetry keeps it.
        let mut grown = runtime_snapshot(vec![
            runtime_node("controller@c:4370", &["controller"], 0),
            runtime_node("gateway@g:4370", &["gateway"], 0),
        ]);
        let tick = controller
            .tick(&log, &log.consensus(), &grown, false)
            .expect("membership tick");
        assert_eq!(tick.membership_generation, 2);
        grown.telemetry_complete = false;
        let committed = log.reasons().len();
        let tick = controller
            .tick(&log, &log.consensus(), &grown, false)
            .expect("incomplete telemetry tick");
        assert_eq!(tick.membership_generation, 2);
        assert_eq!(
            log.reasons()[committed..],
            ["capacity reconciliation fence"]
        );
    }

    #[test]
    fn sustained_pressure_commits_a_larger_target_and_ensures_it() {
        let mut config = config();
        config.policy.scale_up_window_millis = 1;
        config.policy.cooldown_millis = 0;
        let log = TestLog::default();
        let mut controller = controller(config);
        let snapshot = runtime_snapshot(vec![
            runtime_node("gateway@g:4370", &["gateway"], 400),
            runtime_node("fixed@fixed:4370", &["worker"], 100),
        ]);
        controller
            .tick(&log, &log.consensus(), &snapshot, false)
            .expect("stabilizing tick");
        std::thread::sleep(Duration::from_millis(5));

        let tick = controller
            .tick(&log, &log.consensus(), &snapshot, false)
            .expect("scale-up tick");

        assert_eq!(
            tick.decision.action,
            super::super::scaling::ScalingAction::ScaleUp
        );
        assert_eq!(tick.desired_workers, 4);
        let entries = log.entries.lock().unwrap().clone();
        let decision = entries
            .iter()
            .find(|entry| entry.reason == "autonomous scaling decision")
            .expect("scaling decision committed");
        assert_eq!(decision.term, ControlTerm(3));
        assert!(matches!(
            &decision.mutation,
            ControlMutation::DesiredCapacity(desired)
                if desired.worker_nodes == 4 && desired.gateway_nodes == 4
                    && desired.revision == DesiredRevision(2)
        ));
        // The fixed worker and the one the first tick ensured count toward
        // the target: two more are ensured.
        assert_eq!(tick.reconcile.ensured.len(), 2);

        // Paused, the autoscaler holds whatever the pressure.
        let tick = controller
            .tick(&log, &log.consensus(), &snapshot, true)
            .expect("paused tick");
        assert_eq!(
            tick.decision.action,
            super::super::scaling::ScalingAction::Paused
        );
    }

    #[test]
    fn observe_only_controllers_report_capacity_without_changing_it() {
        let mut config = config();
        config.features.horizontal_observe_only = true;
        let log = TestLog::default();
        // The provider holds one live node and one it already removed.
        let driver = FakeCapacityDriver::new();
        for id in ["live", "removed"] {
            let operation = DriverOperation {
                cluster_id: "cluster".to_string(),
                operation_id: format!("ensure-{id}"),
                control_term: ControlTerm(1),
                desired_revision: DesiredRevision(1),
                template_revision: "v1".to_string(),
                node_id: None,
                state: DriverOperationState::Pending,
            };
            let node = driver.ensure_node(&operation).unwrap().node_id.unwrap();
            if id == "removed" {
                let terminate = DriverOperation {
                    operation_id: format!("terminate-{id}"),
                    ..operation
                };
                driver.terminate_node(&terminate, &node).unwrap();
            }
        }
        let driver: Arc<dyn CapacityDriver> = Arc::new(driver);
        let mut controller = AutonomousController::new(config, driver, "cluster".to_string())
            .unwrap_or_else(|error| panic!("controller: {error}"));
        let snapshot = runtime_snapshot(vec![runtime_node("fixed@fixed:4370", &["worker"], 500)]);

        let tick = controller
            .tick(&log, &log.consensus(), &snapshot, false)
            .expect("observe-only tick");

        assert_eq!(tick.reconcile.constraints, ["horizontal_observe_only"]);
        assert_eq!(tick.reconcile.observed_workers, 1);
        assert!(tick.reconcile.ensured.is_empty());
        assert!(tick
            .decision
            .constraints
            .contains(&"scale_up_disabled".to_string()));
        assert!(!log
            .reasons()
            .iter()
            .any(|reason| reason == "autonomous scaling decision"
                || reason == "capacity reconciliation fence"));
    }

    #[test]
    fn a_refused_commit_fails_the_tick() {
        let snapshot = runtime_snapshot(vec![runtime_node("fixed@fixed:4370", &["worker"], 500)]);
        for reason in [
            "register embedded scaling policy",
            "initialize desired capacity",
            "record observed runtime membership",
            "capacity reconciliation fence",
        ] {
            let log = TestLog::refusing(reason);
            let mut controller = controller(config());
            assert_eq!(
                controller
                    .tick(&log, &log.consensus(), &snapshot, false)
                    .err(),
                Some(format!("refused:{reason}"))
            );
        }

        let mut config = config();
        config.policy.scale_up_window_millis = 1;
        config.policy.cooldown_millis = 0;
        let log = TestLog::refusing("autonomous scaling decision");
        let mut controller = controller(config);
        controller
            .tick(&log, &log.consensus(), &snapshot, false)
            .expect("stabilizing tick");
        std::thread::sleep(Duration::from_millis(5));
        assert_eq!(
            controller
                .tick(&log, &log.consensus(), &snapshot, false)
                .err(),
            Some("refused:autonomous scaling decision".to_string())
        );
    }
}
