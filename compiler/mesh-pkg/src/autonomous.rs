//! Typed manifest contract for runtime-owned clustering and elasticity.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use serde::{de, Deserialize, Deserializer, Serialize, Serializer};

pub const MAX_TOTAL_REPLICAS: u32 = 32;
pub const DEFAULT_TRANSPORT_FRAME_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct HumanDuration(Duration);

impl HumanDuration {
    pub const fn from_millis(millis: u64) -> Self {
        Self(Duration::from_millis(millis))
    }

    pub const fn as_duration(self) -> Duration {
        self.0
    }

    pub fn as_millis(self) -> u64 {
        self.0.as_millis().try_into().unwrap_or(u64::MAX)
    }
}

impl fmt::Display for HumanDuration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let millis = self.as_millis();
        if millis.is_multiple_of(86_400_000) {
            write!(formatter, "{}d", millis / 86_400_000)
        } else if millis.is_multiple_of(3_600_000) {
            write!(formatter, "{}h", millis / 3_600_000)
        } else if millis.is_multiple_of(60_000) {
            write!(formatter, "{}m", millis / 60_000)
        } else if millis.is_multiple_of(1_000) {
            write!(formatter, "{}s", millis / 1_000)
        } else {
            write!(formatter, "{millis}ms")
        }
    }
}

impl Serialize for HumanDuration {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for HumanDuration {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        parse_duration(&raw).map_err(de::Error::custom)
    }
}

fn parse_duration(raw: &str) -> Result<HumanDuration, String> {
    let (digits, multiplier) = [
        ("ms", 1u64),
        ("s", 1_000),
        ("m", 60_000),
        ("h", 3_600_000),
        ("d", 86_400_000),
    ]
    .into_iter()
    .find_map(|(suffix, multiplier)| raw.strip_suffix(suffix).map(|digits| (digits, multiplier)))
    .ok_or_else(|| {
        format!("invalid duration `{raw}`; expected an integer followed by ms, s, m, h, or d")
    })?;

    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(format!(
            "invalid duration `{raw}`; fractional, signed, and whitespace-padded values are not allowed"
        ));
    }
    let value = digits
        .parse::<u64>()
        .map_err(|_| format!("duration `{raw}` exceeds the supported range"))?;
    let millis = value
        .checked_mul(multiplier)
        .ok_or_else(|| format!("duration `{raw}` exceeds the supported range"))?;
    Ok(HumanDuration::from_millis(millis))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ByteSize(u64);

impl ByteSize {
    pub const fn from_bytes(bytes: u64) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(self) -> u64 {
        self.0
    }
}

impl fmt::Display for ByteSize {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let bytes = self.0;
        for (suffix, divisor) in [
            ("TiB", 1u64 << 40),
            ("GiB", 1u64 << 30),
            ("MiB", 1u64 << 20),
            ("KiB", 1u64 << 10),
        ] {
            if bytes >= divisor && bytes.is_multiple_of(divisor) {
                return write!(formatter, "{}{}", bytes / divisor, suffix);
            }
        }
        write!(formatter, "{bytes}B")
    }
}

impl Serialize for ByteSize {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for ByteSize {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        parse_byte_size(&raw).map_err(de::Error::custom)
    }
}

fn parse_byte_size(raw: &str) -> Result<ByteSize, String> {
    let (digits, multiplier) = [
        ("TiB", 1u64 << 40),
        ("GiB", 1u64 << 30),
        ("MiB", 1u64 << 20),
        ("KiB", 1u64 << 10),
        ("B", 1u64),
    ]
    .into_iter()
    .find_map(|(suffix, multiplier)| raw.strip_suffix(suffix).map(|digits| (digits, multiplier)))
    .ok_or_else(|| {
        format!(
            "invalid byte size `{raw}`; expected an integer followed by B, KiB, MiB, GiB, or TiB"
        )
    })?;
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(format!(
            "invalid byte size `{raw}`; fractional, signed, and whitespace-padded values are not allowed"
        ));
    }
    let value = digits
        .parse::<u64>()
        .map_err(|_| format!("byte size `{raw}` exceeds the supported range"))?;
    let bytes = value
        .checked_mul(multiplier)
        .ok_or_else(|| format!("byte size `{raw}` exceeds the supported range"))?;
    Ok(ByteSize::from_bytes(bytes))
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ClusterMode {
    #[default]
    Manual,
    Autonomous,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DurabilityMode {
    #[default]
    Strict,
    Degraded,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ManagedRole {
    Gateway,
    Worker,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RoutingAlgorithm {
    Static,
    #[default]
    Adaptive,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CapacityDriverKind {
    Process,
    Docker,
    Fly,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ForcedTerminationPolicy {
    #[default]
    Never,
    AfterDrainTimeout,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AutonomousClusterConfig {
    #[serde(default)]
    pub mode: ClusterMode,
    #[serde(default = "default_replicas")]
    pub default_replicas: u32,
    #[serde(default)]
    pub durability: DurabilityMode,
    #[serde(default)]
    pub controllers: ControllerConfig,
    #[serde(default)]
    pub roles: RoleConfig,
    #[serde(default)]
    pub features: ClusterFeatureConfig,
    #[serde(default)]
    pub scheduler: SchedulerConfig,
    #[serde(default)]
    pub autoscaling: AutoscalingConfig,
    #[serde(default)]
    pub routing: RoutingConfig,
    #[serde(default)]
    pub continuity: ContinuityConfig,
    #[serde(default)]
    pub capacity: CapacityConfig,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ClusterFeatureConfig {
    #[serde(default = "enabled")]
    pub protocol_two: bool,
    #[serde(default = "enabled")]
    pub durable_continuity: bool,
    #[serde(default = "enabled")]
    pub telemetry: bool,
    #[serde(default = "enabled")]
    pub local_scheduler_autoscaling: bool,
    #[serde(default = "enabled")]
    pub adaptive_routing: bool,
    #[serde(default = "enabled")]
    pub controller_quorum: bool,
    #[serde(default)]
    pub horizontal_observe_only: bool,
    #[serde(default = "enabled")]
    pub automatic_scale_up: bool,
    #[serde(default = "enabled")]
    pub automatic_scale_down: bool,
}

impl Default for ClusterFeatureConfig {
    fn default() -> Self {
        Self {
            protocol_two: true,
            durable_continuity: true,
            telemetry: true,
            local_scheduler_autoscaling: true,
            adaptive_routing: true,
            controller_quorum: true,
            horizontal_observe_only: false,
            automatic_scale_up: true,
            automatic_scale_down: true,
        }
    }
}

fn default_replicas() -> u32 {
    2
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ControllerConfig {
    #[serde(default = "default_controller_voters")]
    pub voters: u16,
    #[serde(default)]
    pub autoscale: bool,
}

fn default_controller_voters() -> u16 {
    1
}

impl Default for ControllerConfig {
    fn default() -> Self {
        Self {
            voters: default_controller_voters(),
            autoscale: false,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RoleConfig {
    #[serde(default = "enabled")]
    pub gateway: bool,
    #[serde(default = "enabled")]
    pub worker: bool,
}

fn enabled() -> bool {
    true
}

impl Default for RoleConfig {
    fn default() -> Self {
        Self {
            gateway: true,
            worker: true,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SchedulerConfig {
    #[serde(default = "default_scheduler_min")]
    pub min_workers: u16,
    #[serde(default = "default_scheduler_max")]
    pub max_workers: u16,
    #[serde(default = "default_target_runnable")]
    pub target_runnable_per_worker: f64,
    #[serde(default = "default_local_scale_up_window")]
    pub scale_up_window: HumanDuration,
    #[serde(default = "default_local_scale_down_window")]
    pub scale_down_window: HumanDuration,
}

fn default_scheduler_min() -> u16 {
    1
}
fn default_scheduler_max() -> u16 {
    std::thread::available_parallelism()
        .map(|parallelism| parallelism.get().min(u16::MAX as usize) as u16)
        .unwrap_or(1)
}
fn default_target_runnable() -> f64 {
    1.0
}
fn default_local_scale_up_window() -> HumanDuration {
    HumanDuration::from_millis(10_000)
}
fn default_local_scale_down_window() -> HumanDuration {
    HumanDuration::from_millis(300_000)
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            min_workers: default_scheduler_min(),
            max_workers: default_scheduler_max(),
            target_runnable_per_worker: default_target_runnable(),
            scale_up_window: default_local_scale_up_window(),
            scale_down_window: default_local_scale_down_window(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AutoscalingConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_managed_roles")]
    pub managed_roles: Vec<ManagedRole>,
    #[serde(default = "default_min_nodes")]
    pub min_nodes: u16,
    #[serde(default = "default_max_nodes")]
    pub max_nodes: u16,
    #[serde(default = "default_target_inflight")]
    pub target_inflight_per_node: u32,
    #[serde(default = "default_target_queue_wait")]
    pub target_queue_wait: HumanDuration,
    #[serde(default = "default_scale_up_window")]
    pub scale_up_window: HumanDuration,
    #[serde(default = "default_scale_down_window")]
    pub scale_down_window: HumanDuration,
    #[serde(default = "default_cooldown")]
    pub cooldown: HumanDuration,
    #[serde(default = "default_scale_up_step")]
    pub max_scale_up_step: u16,
    #[serde(default = "default_scale_down_step")]
    pub max_scale_down_step: u16,
    #[serde(default = "default_max_unavailable")]
    pub max_unavailable: u16,
}

fn default_managed_roles() -> Vec<ManagedRole> {
    vec![ManagedRole::Worker]
}
fn default_min_nodes() -> u16 {
    2
}
fn default_max_nodes() -> u16 {
    20
}
fn default_target_inflight() -> u32 {
    128
}
fn default_target_queue_wait() -> HumanDuration {
    HumanDuration::from_millis(25)
}
fn default_scale_up_window() -> HumanDuration {
    HumanDuration::from_millis(30_000)
}
fn default_scale_down_window() -> HumanDuration {
    HumanDuration::from_millis(600_000)
}
fn default_cooldown() -> HumanDuration {
    HumanDuration::from_millis(120_000)
}
fn default_scale_up_step() -> u16 {
    4
}
fn default_scale_down_step() -> u16 {
    1
}
fn default_max_unavailable() -> u16 {
    1
}

impl Default for AutoscalingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            managed_roles: default_managed_roles(),
            min_nodes: default_min_nodes(),
            max_nodes: default_max_nodes(),
            target_inflight_per_node: default_target_inflight(),
            target_queue_wait: default_target_queue_wait(),
            scale_up_window: default_scale_up_window(),
            scale_down_window: default_scale_down_window(),
            cooldown: default_cooldown(),
            max_scale_up_step: default_scale_up_step(),
            max_scale_down_step: default_scale_down_step(),
            max_unavailable: default_max_unavailable(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RoutingConfig {
    #[serde(default)]
    pub algorithm: RoutingAlgorithm,
    #[serde(default = "default_load_report_interval")]
    pub load_report_interval: HumanDuration,
    #[serde(default = "default_load_report_ttl")]
    pub load_report_ttl: HumanDuration,
    #[serde(default = "default_max_inflight")]
    pub max_inflight_per_node: u32,
    #[serde(default = "default_max_queued")]
    pub max_queued_per_node: u32,
    #[serde(default = "default_max_queued_bytes")]
    pub max_queued_bytes_per_node: ByteSize,
    #[serde(default = "default_retry_budget")]
    pub retry_budget_percent: u8,
}

fn default_load_report_interval() -> HumanDuration {
    HumanDuration::from_millis(500)
}
fn default_load_report_ttl() -> HumanDuration {
    HumanDuration::from_millis(2_000)
}
fn default_max_inflight() -> u32 {
    256
}
fn default_max_queued() -> u32 {
    512
}
fn default_max_queued_bytes() -> ByteSize {
    ByteSize::from_bytes(64 * 1024 * 1024)
}
fn default_retry_budget() -> u8 {
    10
}

impl Default for RoutingConfig {
    fn default() -> Self {
        Self {
            algorithm: RoutingAlgorithm::Adaptive,
            load_report_interval: default_load_report_interval(),
            load_report_ttl: default_load_report_ttl(),
            max_inflight_per_node: default_max_inflight(),
            max_queued_per_node: default_max_queued(),
            max_queued_bytes_per_node: default_max_queued_bytes(),
            retry_budget_percent: default_retry_budget(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ContinuityConfig {
    #[serde(default = "default_terminal_retention")]
    pub terminal_retention: HumanDuration,
    #[serde(default = "default_tombstone_retention")]
    pub tombstone_retention: HumanDuration,
    #[serde(default = "default_max_terminal_records")]
    pub max_terminal_records: u64,
    #[serde(default = "default_max_disk_bytes")]
    pub max_disk_bytes: ByteSize,
    #[serde(default = "default_snapshot_chunk_bytes")]
    pub snapshot_chunk_bytes: ByteSize,
    #[serde(default)]
    pub path: Option<PathBuf>,
}

fn default_terminal_retention() -> HumanDuration {
    HumanDuration::from_millis(86_400_000)
}
fn default_tombstone_retention() -> HumanDuration {
    HumanDuration::from_millis(172_800_000)
}
fn default_max_terminal_records() -> u64 {
    1_000_000
}
fn default_max_disk_bytes() -> ByteSize {
    ByteSize::from_bytes(8 * 1024 * 1024 * 1024)
}
fn default_snapshot_chunk_bytes() -> ByteSize {
    ByteSize::from_bytes(1024 * 1024)
}

impl Default for ContinuityConfig {
    fn default() -> Self {
        Self {
            terminal_retention: default_terminal_retention(),
            tombstone_retention: default_tombstone_retention(),
            max_terminal_records: default_max_terminal_records(),
            max_disk_bytes: default_max_disk_bytes(),
            snapshot_chunk_bytes: default_snapshot_chunk_bytes(),
            path: None,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProcessDriverConfig {
    #[serde(default)]
    pub command: Vec<String>,
    #[serde(default)]
    pub working_directory: PathBuf,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DockerDriverConfig {
    #[serde(default)]
    pub image: String,
    #[serde(default = "default_docker_pool")]
    pub pool: String,
    #[serde(default = "default_template_revision")]
    pub template_revision: String,
    #[serde(default)]
    pub network: Option<String>,
    #[serde(default)]
    pub env: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FlyDriverConfig {
    #[serde(default = "default_fly_api_base_url")]
    pub api_base_url: String,
    #[serde(default)]
    pub app_name: String,
    #[serde(default = "default_fly_token_env")]
    pub token_env: String,
    #[serde(default)]
    pub image: String,
    #[serde(default)]
    pub region: Option<String>,
    #[serde(default = "default_docker_pool")]
    pub pool: String,
    #[serde(default = "default_template_revision")]
    pub template_revision: String,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default = "default_fly_cpu_kind")]
    pub cpu_kind: String,
    #[serde(default = "default_fly_cpus")]
    pub cpus: u8,
    #[serde(default = "default_fly_memory_mb")]
    pub memory_mb: u32,
}

impl Default for FlyDriverConfig {
    fn default() -> Self {
        Self {
            api_base_url: default_fly_api_base_url(),
            app_name: String::new(),
            token_env: default_fly_token_env(),
            image: String::new(),
            region: None,
            pool: default_docker_pool(),
            template_revision: default_template_revision(),
            env: BTreeMap::new(),
            cpu_kind: default_fly_cpu_kind(),
            cpus: default_fly_cpus(),
            memory_mb: default_fly_memory_mb(),
        }
    }
}

fn default_fly_api_base_url() -> String {
    "https://api.machines.dev".to_string()
}

fn official_fly_api_base_url(value: &str) -> bool {
    value.trim_end_matches('/') == "https://api.machines.dev"
}

fn default_fly_token_env() -> String {
    "FLY_API_TOKEN".to_string()
}

fn default_fly_cpu_kind() -> String {
    "shared".to_string()
}

fn default_fly_cpus() -> u8 {
    1
}

fn default_fly_memory_mb() -> u32 {
    256
}

fn default_docker_pool() -> String {
    "workers".to_string()
}
fn default_template_revision() -> String {
    "v1".to_string()
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CapacityConfig {
    #[serde(default)]
    pub driver: Option<CapacityDriverKind>,
    #[serde(default = "default_startup_timeout")]
    pub startup_timeout: HumanDuration,
    #[serde(default = "default_drain_timeout")]
    pub drain_timeout: HumanDuration,
    #[serde(default = "default_termination_timeout")]
    pub termination_timeout: HumanDuration,
    #[serde(default)]
    pub forced_termination: ForcedTerminationPolicy,
    #[serde(default)]
    pub process: Option<ProcessDriverConfig>,
    #[serde(default)]
    pub docker: Option<DockerDriverConfig>,
    #[serde(default)]
    pub fly: Option<FlyDriverConfig>,
}

fn default_startup_timeout() -> HumanDuration {
    HumanDuration::from_millis(120_000)
}
fn default_drain_timeout() -> HumanDuration {
    HumanDuration::from_millis(300_000)
}
fn default_termination_timeout() -> HumanDuration {
    HumanDuration::from_millis(120_000)
}

impl Default for CapacityConfig {
    fn default() -> Self {
        Self {
            driver: None,
            startup_timeout: default_startup_timeout(),
            drain_timeout: default_drain_timeout(),
            termination_timeout: default_termination_timeout(),
            forced_termination: ForcedTerminationPolicy::Never,
            process: None,
            docker: None,
            fly: None,
        }
    }
}

impl AutonomousClusterConfig {
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();
        if !(1..=MAX_TOTAL_REPLICAS).contains(&self.default_replicas) {
            errors.push(format!(
                "[cluster].default_replicas must be between 1 and {MAX_TOTAL_REPLICAS}"
            ));
        }
        if self.controllers.voters == 0 {
            errors.push("[cluster.controllers].voters must be positive".to_string());
        }
        if self.controllers.autoscale {
            errors.push(
                "controller voters are fixed; [cluster.controllers].autoscale must be false"
                    .to_string(),
            );
        }
        if self.mode == ClusterMode::Autonomous
            && self.controllers.voters > 1
            && self.controllers.voters.is_multiple_of(2)
        {
            errors.push("autonomous controller quorum must use an odd voter count".to_string());
        }
        if self.scheduler.min_workers == 0
            || self.scheduler.min_workers > self.scheduler.max_workers
        {
            errors.push(
                "[cluster.scheduler].min_workers must be positive and no greater than max_workers"
                    .to_string(),
            );
        }
        if !self.scheduler.target_runnable_per_worker.is_finite()
            || self.scheduler.target_runnable_per_worker <= 0.0
        {
            errors.push(
                "[cluster.scheduler].target_runnable_per_worker must be a positive finite number"
                    .to_string(),
            );
        }
        if self.scheduler.scale_down_window <= self.scheduler.scale_up_window {
            errors.push(
                "[cluster.scheduler].scale_down_window must be longer than scale_up_window"
                    .to_string(),
            );
        }
        if self.autoscaling.min_nodes == 0
            || self.autoscaling.min_nodes > self.autoscaling.max_nodes
        {
            errors.push(
                "[cluster.autoscaling].min_nodes must be positive and no greater than max_nodes"
                    .to_string(),
            );
        }
        if self.autoscaling.target_inflight_per_node == 0
            || self.autoscaling.max_scale_up_step == 0
            || self.autoscaling.max_scale_down_step == 0
        {
            errors.push("autoscaling targets and step bounds must be positive".to_string());
        }
        if self.autoscaling.max_unavailable >= self.autoscaling.min_nodes {
            errors.push(
                "[cluster.autoscaling].max_unavailable must be smaller than min_nodes".to_string(),
            );
        }
        if self.autoscaling.scale_down_window <= self.autoscaling.scale_up_window {
            errors.push(
                "[cluster.autoscaling].scale_down_window must be longer than scale_up_window"
                    .to_string(),
            );
        }
        if self.autoscaling.enabled {
            if !self.features.protocol_two
                || !self.features.durable_continuity
                || !self.features.telemetry
                || !self.features.controller_quorum
            {
                errors.push(
                    "horizontal autoscaling requires protocol_two, durable_continuity, telemetry, and controller_quorum feature gates"
                        .to_string(),
                );
            }
            let unique_roles: BTreeSet<_> = self.autoscaling.managed_roles.iter().collect();
            if self.autoscaling.managed_roles.is_empty()
                || unique_roles.len() != self.autoscaling.managed_roles.len()
            {
                errors.push(
                    "[cluster.autoscaling].managed_roles must contain unique roles".to_string(),
                );
            }
            if !self
                .autoscaling
                .managed_roles
                .contains(&ManagedRole::Worker)
            {
                errors.push(
                    "the first autonomous capacity pool must include the worker role".to_string(),
                );
            }
            if self
                .autoscaling
                .managed_roles
                .contains(&ManagedRole::Worker)
                && !self.roles.worker
            {
                errors.push("managed worker role is disabled by [cluster.roles]".to_string());
            }
            if self
                .autoscaling
                .managed_roles
                .contains(&ManagedRole::Gateway)
                && !self.roles.gateway
            {
                errors.push("managed gateway role is disabled by [cluster.roles]".to_string());
            }
        }
        if self.features.adaptive_routing && self.routing.algorithm != RoutingAlgorithm::Adaptive {
            errors.push(
                "[cluster.features].adaptive_routing requires routing.algorithm = \"adaptive\""
                    .to_string(),
            );
        }
        if self.routing.max_inflight_per_node == 0
            || self.routing.max_queued_per_node == 0
            || self.routing.max_queued_bytes_per_node.as_bytes() == 0
        {
            errors.push("routing in-flight, item, and byte bounds must be positive".to_string());
        }
        if self.routing.retry_budget_percent > 100 {
            errors.push("[cluster.routing].retry_budget_percent must not exceed 100".to_string());
        }
        if self.routing.load_report_interval >= self.routing.load_report_ttl {
            errors.push(
                "[cluster.routing].load_report_ttl must be longer than load_report_interval"
                    .to_string(),
            );
        }
        if self.continuity.tombstone_retention <= self.continuity.terminal_retention {
            errors.push(
                "[cluster.continuity].tombstone_retention must be longer than terminal_retention"
                    .to_string(),
            );
        }
        if self.continuity.max_terminal_records == 0
            || self.continuity.max_disk_bytes.as_bytes() == 0
        {
            errors.push("continuity record and disk bounds must be positive".to_string());
        }
        if self.continuity.snapshot_chunk_bytes.as_bytes() == 0
            || self.continuity.snapshot_chunk_bytes.as_bytes() >= DEFAULT_TRANSPORT_FRAME_BYTES
        {
            errors.push(format!(
                "[cluster.continuity].snapshot_chunk_bytes must be positive and below {DEFAULT_TRANSPORT_FRAME_BYTES}B"
            ));
        }
        if self.durability == DurabilityMode::Strict
            && self.default_replicas > u32::from(self.autoscaling.max_nodes)
        {
            errors.push(
                "strict durability default_replicas exceeds maximum eligible node capacity"
                    .to_string(),
            );
        }
        if self.mode == ClusterMode::Autonomous && self.autoscaling.enabled {
            if self.capacity.startup_timeout.as_millis() == 0
                || self.capacity.drain_timeout.as_millis() == 0
                || self.capacity.termination_timeout.as_millis() == 0
            {
                errors.push(
                    "capacity startup, drain, and termination timeouts must be positive"
                        .to_string(),
                );
            }
            if self.controllers.voters != 1 && self.controllers.voters < 3 {
                errors.push("autonomous horizontal scaling requires one development voter or at least three production voters".to_string());
            }
            match self.capacity.driver {
                None => errors.push("autonomous mode requires [cluster.capacity].driver".to_string()),
                Some(CapacityDriverKind::Process) => match &self.capacity.process {
                    Some(process)
                        if !process.command.is_empty()
                            && !process.command[0].trim().is_empty()
                            && !process.working_directory.as_os_str().is_empty() => {}
                    _ => errors.push("the process capacity driver requires a typed command and working_directory".to_string()),
                },
                Some(CapacityDriverKind::Docker) => match &self.capacity.docker {
                    Some(docker)
                        if !docker.image.trim().is_empty()
                            && !docker.pool.trim().is_empty()
                            && !docker.template_revision.trim().is_empty() => {}
                    _ => errors.push("the Docker capacity driver requires image, pool, and template_revision".to_string()),
                },
                Some(CapacityDriverKind::Fly) => match &self.capacity.fly {
                    Some(fly)
                        if official_fly_api_base_url(&fly.api_base_url)
                            && !fly.app_name.trim().is_empty()
                            && !fly.token_env.trim().is_empty()
                            && !fly.image.trim().is_empty()
                            && !fly.pool.trim().is_empty()
                            && !fly.template_revision.trim().is_empty()
                            && !fly.cpu_kind.trim().is_empty()
                            && fly.cpus > 0
                            && fly.memory_mb >= 128 => {}
                    _ => errors.push("the Fly Machines capacity driver requires the official https://api.machines.dev origin, app_name, token_env, image, pool, template_revision, and a valid guest size".to_string()),
                },
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_config(source: &str) -> AutonomousClusterConfig {
        toml::from_str(source).expect("autonomous cluster config should parse")
    }

    #[test]
    fn duration_parser_rejects_fractional_values() {
        let error = toml::from_str::<SchedulerConfig>(
            "min_workers=1\nmax_workers=2\ntarget_runnable_per_worker=1.0\nscale_up_window=\"1.5s\"\nscale_down_window=\"2m\"",
        )
        .expect_err("fractional durations must fail");

        assert!(error.to_string().contains("fractional"));
    }

    #[test]
    fn byte_size_round_trips_canonically() {
        let size = parse_byte_size("8GiB").expect("valid size");

        assert_eq!(size.to_string(), "8GiB");
    }

    #[test]
    fn autonomous_config_rejects_scale_down_shorter_than_scale_up() {
        let config = parse_config(
            r#"
mode = "autonomous"

[controllers]
voters = 3

[autoscaling]
enabled = true
min_nodes = 2
max_nodes = 5
scale_up_window = "30s"
scale_down_window = "10s"

[capacity]
driver = "docker"

[capacity.docker]
image = "mesh-worker@sha256:abc"
pool = "workers"
template_revision = "v1"
"#,
        );

        let errors = config.validate().expect_err("invalid hysteresis must fail");
        assert!(errors
            .iter()
            .any(|error| error.contains("scale_down_window")));
    }

    #[test]
    fn autonomous_data_plane_can_stage_without_horizontal_driver_authority() {
        let config = parse_config(
            r#"
mode = "autonomous"

[controllers]
voters = 3

[autoscaling]
enabled = false
"#,
        );

        assert_eq!(config.validate(), Ok(()));
        assert!(!config.autoscaling.enabled);
        assert_eq!(config.capacity.driver, None);
    }

    #[test]
    fn horizontal_autoscaling_rejects_disabled_safety_prerequisite() {
        let config = parse_config(
            r#"
mode = "autonomous"

[controllers]
voters = 3

[features]
durable_continuity = false

[autoscaling]
enabled = true
min_nodes = 2
max_nodes = 5

[capacity]
driver = "docker"

[capacity.docker]
image = "mesh-worker@sha256:abc"
pool = "workers"
template_revision = "v1"
"#,
        );

        assert!(config
            .validate()
            .expect_err("disabled continuity must fence horizontal scaling")
            .iter()
            .any(|error| error.contains("horizontal autoscaling requires")));
    }

    #[test]
    fn production_autonomous_config_accepts_typed_docker_driver() {
        let config = parse_config(
            r#"
mode = "autonomous"
default_replicas = 2
durability = "strict"

[controllers]
voters = 3
autoscale = false

[scheduler]
min_workers = 2
max_workers = 8
scale_up_window = "10s"
scale_down_window = "5m"

[autoscaling]
enabled = true
managed_roles = ["worker"]
min_nodes = 2
max_nodes = 5
scale_up_window = "30s"
scale_down_window = "10m"
max_unavailable = 1

[capacity]
driver = "docker"

[capacity.docker]
image = "mesh-worker@sha256:abc"
pool = "workers"
template_revision = "v1"
"#,
        );

        assert_eq!(config.validate(), Ok(()));
    }

    #[test]
    fn autonomous_config_accepts_combined_gateway_worker_pool() {
        let mut config = parse_config(
            r#"
mode = "autonomous"

[controllers]
voters = 3

[roles]
gateway = true
worker = true

[autoscaling]
enabled = true
managed_roles = ["gateway", "worker"]
min_nodes = 2
max_nodes = 5
scale_up_window = "30s"
scale_down_window = "10m"
max_unavailable = 1

[capacity]
driver = "docker"

[capacity.docker]
image = "mesh-worker@sha256:abc"
pool = "workers"
template_revision = "v1"
"#,
        );

        assert_eq!(config.validate(), Ok(()));
        config.autoscaling.managed_roles = vec![ManagedRole::Gateway];
        assert!(config
            .validate()
            .expect_err("gateway-only pool is not supported in the first release")
            .iter()
            .any(|error| error.contains("must include the worker role")));
    }

    #[test]
    fn production_autonomous_config_accepts_typed_fly_driver_without_inline_credentials() {
        let config = parse_config(
            r#"
mode = "autonomous"
default_replicas = 2
durability = "strict"

[controllers]
voters = 3

[autoscaling]
enabled = true
managed_roles = ["worker"]
min_nodes = 2
max_nodes = 5
scale_up_window = "30s"
scale_down_window = "10m"
max_unavailable = 1

[capacity]
driver = "fly"

[capacity.fly]
app_name = "mesh-production"
token_env = "FLY_API_TOKEN"
image = "registry.fly.io/mesh-production@sha256:abc"
region = "lax"
pool = "workers"
template_revision = "v1"
cpu_kind = "shared"
cpus = 1
memory_mb = 256
"#,
        );

        assert_eq!(config.validate(), Ok(()));
        let fly = config.capacity.fly.expect("typed Fly configuration");
        assert_eq!(fly.token_env, "FLY_API_TOKEN");
        assert_eq!(fly.api_base_url, "https://api.machines.dev");
    }

    #[test]
    fn fly_manifest_origin_is_pinned_before_token_lookup() {
        assert!(official_fly_api_base_url("https://api.machines.dev"));
        assert!(official_fly_api_base_url("https://api.machines.dev/"));
        assert!(!official_fly_api_base_url("https://attacker.example"));
        assert!(!official_fly_api_base_url(
            "https://api.machines.dev.attacker.example"
        ));
    }
}
