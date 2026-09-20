//! Embedded autonomous-cluster configuration and controller service.
//!
//! `meshc` normalizes the validated manifest into this versioned schema and
//! emits it into the executable. Node-specific identity and credentials remain
//! environment sourced; scaling policy and driver templates do not depend on
//! an out-of-band policy process.

use std::collections::{BTreeMap, BTreeSet};
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
    DockerEnvironmentFileMount, FlyMachinesCapacityDriver, FlyMachinesDriverConfig,
    ProcessCapacityDriver, ProcessDriverConfig, ReconcileNodeSafety, ScalingDecision,
    ScalingSample,
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
    Fly {
        api_base_url: String,
        app_name: String,
        token_env: String,
        image: String,
        region: Option<String>,
        pool: String,
        environment: BTreeMap<String, String>,
        cpu_kind: String,
        cpus: u8,
        memory_mb: u32,
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
            Self::Fly {
                api_base_url,
                app_name,
                token_env,
                image,
                region,
                pool,
                environment,
                cpu_kind,
                cpus,
                memory_mb,
            } => formatter
                .debug_struct("Fly")
                .field("api_base_url", api_base_url)
                .field("app_name", app_name)
                .field("token_env", token_env)
                .field("image", image)
                .field("region", region)
                .field("pool", pool)
                .field(
                    "environment",
                    &format_args!("[redacted; {}]", environment.len()),
                )
                .field("cpu_kind", cpu_kind)
                .field("cpus", cpus)
                .field("memory_mb", memory_mb)
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
            Self::Fly {
                api_base_url,
                app_name,
                token_env,
                image,
                pool,
                cpu_kind,
                cpus,
                memory_mb,
                ..
            } if api_base_url.trim_end_matches('/') == "https://api.machines.dev"
                && !app_name.trim().is_empty()
                && !token_env.trim().is_empty()
                && !image.trim().is_empty()
                && !pool.trim().is_empty()
                && !cpu_kind.trim().is_empty()
                && *cpus > 0
                && *memory_mb >= 128 =>
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
    std::env::var("MESH_ROLES")
        .unwrap_or_default()
        .split(',')
        .any(|role| role.trim().eq_ignore_ascii_case("controller"))
}

fn capacity_worker_environment(
    configured: &[String],
    managed_roles: &[String],
) -> Result<Vec<String>, String> {
    let mut environment = configured.to_vec();
    if let Ok(raw) = std::env::var("MESH_CAPACITY_WORKER_ENV_ALLOWLIST") {
        for name in raw
            .split(',')
            .map(str::trim)
            .filter(|name| !name.is_empty())
        {
            if name.contains('=') || name.contains('\0') {
                return Err("capacity_worker_environment_name_invalid".to_string());
            }
            if let Ok(value) = std::env::var(name) {
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
) -> Result<Arc<dyn CapacityDriver>, String> {
    let operation_timeout = Duration::from_millis(
        config
            .startup_timeout_millis
            .min(config.termination_timeout_millis),
    );
    let driver: Arc<dyn CapacityDriver> = match &config.driver {
        RuntimeCapacityDriverConfig::Disabled => {
            Err("autonomous_capacity_driver_disabled".to_string())
        }
        RuntimeCapacityDriverConfig::Process {
            command,
            working_directory,
        } => Ok(Arc::new(ProcessCapacityDriver::new(ProcessDriverConfig {
            command: command.clone(),
            working_directory: working_directory.clone(),
            environment: BTreeMap::from([(
                "MESH_ROLES".to_string(),
                config.managed_roles.join(","),
            )]),
        })) as Arc<dyn CapacityDriver>),
        RuntimeCapacityDriverConfig::Docker {
            image,
            pool,
            network,
            environment,
        } => {
            let network = std::env::var("MESH_CAPACITY_DOCKER_NETWORK")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .map(Some)
                .unwrap_or_else(|| network.clone());
            let worker_environment =
                capacity_worker_environment(environment, &config.managed_roles)?;
            if std::env::var_os("MESH_DOCKER_DRIVER_ENDPOINT").is_some() {
                Ok(Arc::new(
                    super::driver_service::RemoteDockerCapacityDriver::from_environment(
                        super::driver_service::RemoteDockerTemplate {
                            image: image.clone(),
                            pool: pool.clone(),
                            network: network.clone(),
                            environment: worker_environment,
                            operation_timeout_millis: operation_timeout.as_millis() as u64,
                        },
                    )?,
                ) as Arc<dyn CapacityDriver>)
            } else {
                let binary = std::env::var_os("MESH_DOCKER_BINARY")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("docker"));
                let execution_prefix = std::env::var("MESH_DOCKER_EXECUTION_PREFIX_JSON")
                    .ok()
                    .map(|raw| {
                        serde_json::from_str::<Vec<String>>(&raw)
                            .map_err(|_| "docker_execution_prefix_invalid".to_string())
                    })
                    .transpose()?
                    .unwrap_or_default();
                let environment_file_mount = match (
                    std::env::var_os("MESH_DOCKER_ENV_HOST_DIRECTORY"),
                    std::env::var_os("MESH_DOCKER_ENV_DRIVER_DIRECTORY"),
                ) {
                    (Some(host), Some(driver)) => Some(DockerEnvironmentFileMount {
                        host_directory: PathBuf::from(host),
                        driver_directory: PathBuf::from(driver),
                    }),
                    (None, None) => None,
                    _ => return Err("docker_environment_mount_incomplete".to_string()),
                };
                Ok(Arc::new(DockerCapacityDriver::new(DockerDriverConfig {
                    binary,
                    execution_prefix,
                    image: image.clone(),
                    pool: pool.clone(),
                    network,
                    environment: worker_environment,
                    environment_file_mount,
                    operation_timeout,
                })) as Arc<dyn CapacityDriver>)
            }
        }
        RuntimeCapacityDriverConfig::Fly {
            api_base_url,
            app_name,
            token_env,
            image,
            region,
            pool,
            environment,
            cpu_kind,
            cpus,
            memory_mb,
        } => {
            let api_token = std::env::var(token_env)
                .map_err(|_| format!("fly_driver_token_environment_missing:{token_env}"))?;
            let configured_environment = environment
                .iter()
                .map(|(name, value)| format!("{name}={value}"))
                .collect::<Vec<_>>();
            let environment =
                capacity_worker_environment(&configured_environment, &config.managed_roles)?
                    .into_iter()
                    .map(|entry| {
                        entry
                            .split_once('=')
                            .map(|(name, value)| (name.to_string(), value.to_string()))
                            .ok_or_else(|| "fly_driver_environment_invalid".to_string())
                    })
                    .collect::<Result<BTreeMap<_, _>, _>>()?;
            Ok(
                Arc::new(FlyMachinesCapacityDriver::new(FlyMachinesDriverConfig {
                    api_base_url: api_base_url.clone(),
                    app_name: app_name.clone(),
                    api_token,
                    image: image.clone(),
                    region: region.clone(),
                    pool: pool.clone(),
                    environment,
                    cpu_kind: cpu_kind.clone(),
                    cpus: *cpus,
                    memory_mb: *memory_mb,
                    operation_timeout,
                })) as Arc<dyn CapacityDriver>,
            )
        }
    }?;
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
    committer: &RuntimeConsensusCommitter,
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
    let policy_json = serde_json::to_string(&config.policy)
        .map_err(|_| "autonomous_policy_encode_failed".to_string())?;
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
    committer: &RuntimeConsensusCommitter,
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
    committer: &RuntimeConsensusCommitter,
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

fn controller_loop(
    config: RuntimeAutonomousConfig,
    driver: Arc<dyn CapacityDriver>,
    cluster_id: String,
) {
    let mut autoscaler = match Autoscaler::new(config.policy.clone()) {
        Ok(autoscaler) => autoscaler,
        Err(error) => {
            controller_status().lock().unwrap().last_error = Some(error);
            return;
        }
    };
    autoscaler.set_action_gates(
        !config.features.horizontal_observe_only && config.features.automatic_scale_up,
        !config.features.horizontal_observe_only && config.features.automatic_scale_down,
    );
    let mut reconciler = match CapacityReconciler::new_runtime(
        driver.clone(),
        config.policy.max_unavailable,
        Duration::from_millis(config.drain_timeout_millis),
        config.force_termination_after_drain_timeout,
    ) {
        Ok(reconciler) => reconciler,
        Err(error) => {
            controller_status().lock().unwrap().last_error = Some(error);
            return;
        }
    };
    let committer = RuntimeConsensusCommitter::new(&cluster_id);
    let interval = Duration::from_millis(config.reconcile_interval_millis);
    let mut was_leader = false;
    loop {
        if super::node::node_state()
            .is_none_or(|state| state.listener_shutdown.load(Ordering::Acquire))
        {
            break;
        }
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
            if let Err(error) = reconciler.restore_from_control_entries(&consensus.entries) {
                controller_status().lock().unwrap().last_error = Some(error);
                std::thread::park_timeout(interval);
                continue;
            }
            was_leader = true;
        }

        // Disconnect delivery and continuity-record replication are separate
        // streams. Re-drive recovery from the fenced leader so either arrival
        // order converges without relying on one edge-triggered callback.
        super::node::recover_pending_owner_losses_if_coordinator();

        let tick = (|| -> Result<(ScalingDecision, CapacityReconcileOutcome, u16, u64), String> {
            commit_policy_if_needed(&committer, &consensus.entries, &config)?;
            let mut committed = latest_committed_desired(&consensus.entries, &config)
                .map(Ok)
                .unwrap_or_else(|| initial_desired(&committer, &config))?;
            // Desired state may predate this election; provider operations must
            // always be fenced by the current OpenRaft term.
            committed.term = ControlTerm(consensus.current_term);
            let snapshot =
                super::operator::operator_runtime_snapshot().map_err(|error| error.to_string())?;
            let membership_generation =
                commit_membership_if_changed(&committer, &consensus.entries, &snapshot)?;
            autoscaler.set_paused(super::operator::autoscaler_paused());
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
                drain_incomplete: !reconciler.drain_progress().is_empty(),
            };
            let decision = autoscaler.evaluate(committed.desired.worker_nodes, sample);
            if decision.bounded_desired != committed.desired.worker_nodes {
                let desired = DesiredCapacity {
                    revision: DesiredRevision(committed.desired.revision.0.saturating_add(1)),
                    worker_nodes: decision.bounded_desired,
                    gateway_nodes: desired_gateway_nodes(&config, decision.bounded_desired),
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
                let observation = driver.observe_capacity(&cluster_id)?;
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
                reconciler.reconcile_with_observed_capacity(
                    &committer,
                    &cluster_id,
                    &consensus.node_name,
                    &BTreeSet::new(),
                    &committed,
                    "mesh-reconciler",
                    |observation| runtime_safety(observation, &snapshot),
                )?
            };
            Ok((
                decision,
                reconcile,
                committed.desired.worker_nodes,
                membership_generation,
            ))
        })();
        let mut status = controller_status().lock().unwrap();
        match tick {
            Ok((decision, reconcile, desired, membership_generation)) => {
                status.last_decision = Some(decision);
                status.last_reconcile = Some(reconcile);
                status.desired_workers = desired;
                status.membership_generation = membership_generation;
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
    if !controller_role_enabled() {
        return Ok(false);
    }
    if !config.features.horizontal_autoscaling || !config.features.controller_quorum {
        return Ok(false);
    }
    let cluster_id = std::env::var("MESH_CLUSTER_ID")
        .map_err(|_| "autonomous_cluster_id_missing".to_string())?;
    let driver = build_capacity_driver(&config)?;
    driver.validate_configuration()?;
    {
        let mut status = controller_status().lock().unwrap();
        status.configured = true;
        status.policy_revision = config.policy_revision;
        status.observe_only = config.features.horizontal_observe_only;
        status.automatic_scale_up = config.features.automatic_scale_up;
        status.automatic_scale_down = config.features.automatic_scale_down;
        status.state = "starting".to_string();
    }
    let mut started = false;
    AUTONOMOUS_CONTROLLER_STARTED.call_once(|| {
        started = true;
        std::thread::Builder::new()
            .name("mesh-autonomous-controller".to_string())
            .spawn(move || controller_loop(config, driver, cluster_id))
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
    let Ok(len) = usize::try_from(len) else {
        return -1;
    };
    let bytes = unsafe { std::slice::from_raw_parts(data, len) };
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

    #[test]
    fn embedded_runtime_config_round_trips_without_credentials() {
        let config = RuntimeAutonomousConfig {
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
            driver: RuntimeCapacityDriverConfig::Fly {
                api_base_url: "https://api.machines.dev".to_string(),
                app_name: "mesh-app".to_string(),
                token_env: "FLY_API_TOKEN".to_string(),
                image: "registry.fly.io/app@sha256:abc".to_string(),
                region: Some("lax".to_string()),
                pool: "workers".to_string(),
                environment: BTreeMap::new(),
                cpu_kind: "shared".to_string(),
                cpus: 1,
                memory_mb: 256,
            },
        };
        let encoded = serde_json::to_vec(&config).unwrap();
        let decoded: RuntimeAutonomousConfig = serde_json::from_slice(&encoded).unwrap();
        decoded.validate().unwrap();
        assert_eq!(decoded, config);
        assert!(!String::from_utf8(encoded).unwrap().contains("api_token"));
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
}
