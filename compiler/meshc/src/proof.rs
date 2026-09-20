use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use clap::{Args, Subcommand};
use mesh_rt::{
    query_operator_continuity_list_remote, query_operator_runtime_remote, CapacityDriver,
    CapacityNodeLifecycle, ContinuityRecord, ControlMutation, ControlTerm, DesiredRevision,
    DriverOperation, DriverOperationState, FlyMachinesCapacityDriver, FlyMachinesDriverConfig,
    OperatorContinuityList, OperatorRuntimeSnapshot,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const COOKIE: &str = "proof-cookie-0123456789abcdef0123";
const OPERATOR_KEY: &str = "proof-operator-key-0123456789abcdef";
const MIN_WORKERS: u16 = 2;
const PROOF_HTTP_READ_TIMEOUT: Duration = Duration::from_secs(15);
const CONCURRENT_BURST_P99_BUDGET_MILLIS: u64 = 6_000;
const FAILURE_LOAD_P99_BUDGET_MILLIS: u64 = 10_000;
const BURST_OPERATOR_QUERY_BUDGET_MILLIS: u64 = 3_000;
const BURST_GATEWAY_HEALTH_BUDGET_MILLIS: u64 = 3_000;
// The proof can remove three workers serially (max five down to min two).
// Each step has a 4s scale-down window, a 30s drain deadline, and a 30s
// termination deadline. Keep the waiter bounded while covering that declared
// policy envelope, one final 30s provider observation, controller polling, and
// consensus propagation.
const RUNTIME_SCALE_DOWN_PROOF_TIMEOUT: Duration = Duration::from_secs(240);

#[derive(Subcommand, Debug)]
pub enum ProofCommand {
    /// Run the mandatory local Docker/PostgreSQL autonomous-scaling proof.
    DockerAutoscaling(DockerAutoscalingArgs),
    /// Run the bounded-retention continuity soak gate.
    ContinuitySoak(crate::proof_gates::ContinuitySoakArgs),
    /// Run deterministic autonomous-cluster performance gates.
    AutonomousPerformance(crate::proof_gates::AutonomousPerformanceArgs),
    /// Run the credential-free Fly Machines fake-API certification suite.
    FlyDriverConformance(FlyDriverConformanceArgs),
    /// Create, ready, cordon, and remove one Machine in a credentialed Fly staging app.
    FlyDriverStaging(FlyDriverStagingArgs),
    /// Generate owner-only TLS and signed identities for the full Fly autoscaling proof.
    FlyAutoscalingMaterialize(FlyAutoscalingMaterializeArgs),
    /// Repeated deterministic fault/model gate for partitions, retries, disk bounds, and recovery.
    AutonomousChaos(AutonomousChaosArgs),
}

#[derive(Args, Debug)]
pub struct DockerAutoscalingArgs {
    /// Keep proof containers and networks running after evidence collection.
    #[arg(long)]
    pub keep_running: bool,

    /// Override the timestamped evidence output directory.
    #[arg(long)]
    pub evidence_dir: Option<PathBuf>,

    /// Reuse an already-built proof image.
    #[arg(long)]
    pub no_build: bool,

    /// Start the healthy proof topology and skip the fault-injection proof sequence.
    #[arg(long)]
    pub start_only: bool,

    /// Owner-only connection manifest to create for the running topology.
    #[arg(long, value_name = "PATH", requires = "start_only")]
    pub connection_file: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub struct FlyDriverConformanceArgs {
    /// Override the timestamped evidence output directory.
    #[arg(long)]
    pub evidence_dir: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub struct FlyDriverStagingArgs {
    /// Fly application used exclusively for staging certification.
    #[arg(long)]
    pub app_name: String,

    /// Immutable worker image reference to start.
    #[arg(long)]
    pub image: String,

    /// Cluster identity written into managed Machine metadata.
    #[arg(long)]
    pub cluster_id: String,

    /// Environment variable containing the Fly API bearer token.
    #[arg(long, default_value = "FLY_API_TOKEN")]
    pub token_env: String,

    /// Environment variable name to forward into the staging worker; repeat as needed.
    #[arg(long = "worker-env")]
    pub worker_env: Vec<String>,

    /// Fly Machines API base URL.
    #[arg(long, default_value = "https://api.machines.dev")]
    pub api_base_url: String,

    /// Optional Fly region.
    #[arg(long)]
    pub region: Option<String>,

    /// Managed capacity pool label.
    #[arg(long, default_value = "workers")]
    pub pool: String,

    /// Immutable template revision recorded in Machine metadata.
    #[arg(long)]
    pub template_revision: Option<String>,

    /// Fly guest CPU kind.
    #[arg(long, default_value = "shared")]
    pub cpu_kind: String,

    /// Fly guest CPU count.
    #[arg(long, default_value_t = 1)]
    pub cpus: u8,

    /// Fly guest memory in MiB.
    #[arg(long, default_value_t = 256)]
    pub memory_mb: u32,

    /// Maximum time for create, readiness, cordon, delete, and observed removal.
    #[arg(long, default_value_t = 300)]
    pub deadline_seconds: u64,

    /// Required acknowledgement that this proof creates and deletes one staging Machine.
    #[arg(long)]
    pub confirm_create_and_delete: bool,

    /// Override the timestamped evidence output directory.
    #[arg(long)]
    pub evidence_dir: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub struct FlyAutoscalingMaterializeArgs {
    /// Fixed-controller Fly application name.
    #[arg(long)]
    pub controller_app: String,

    /// Gateway, baseline-worker, and dynamically managed-worker Fly application name.
    #[arg(long)]
    pub data_app: String,

    /// Stable cluster identity for this isolated proof run.
    #[arg(long)]
    pub cluster_id: String,

    /// New owner-only JSON file to create. Existing files are never overwritten.
    #[arg(long)]
    pub output: PathBuf,
}

#[derive(Args, Debug)]
pub struct AutonomousChaosArgs {
    /// Number of complete deterministic fault-suite repetitions.
    #[arg(long, default_value_t = 5)]
    pub rounds: u16,

    /// Override the timestamped evidence output directory.
    #[arg(long)]
    pub evidence_dir: Option<PathBuf>,
}

pub fn run_proof_command(command: ProofCommand) -> Result<(), String> {
    match command {
        ProofCommand::DockerAutoscaling(args) => run_docker_autoscaling(args),
        ProofCommand::ContinuitySoak(args) => crate::proof_gates::run_continuity_soak(args),
        ProofCommand::AutonomousPerformance(args) => {
            crate::proof_gates::run_autonomous_performance(args)
        }
        ProofCommand::FlyDriverConformance(args) => run_fly_driver_conformance(args),
        ProofCommand::FlyDriverStaging(args) => run_fly_driver_staging(args),
        ProofCommand::FlyAutoscalingMaterialize(args) => run_fly_autoscaling_materialize(args),
        ProofCommand::AutonomousChaos(args) => run_autonomous_chaos(args),
    }
}

fn run_fly_autoscaling_materialize(args: FlyAutoscalingMaterializeArgs) -> Result<(), String> {
    let valid_app = |value: &str| {
        !value.is_empty()
            && value.len() <= 63
            && value.starts_with(|character: char| character.is_ascii_alphanumeric())
            && value.ends_with(|character: char| character.is_ascii_alphanumeric())
            && value
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    };
    if !valid_app(&args.controller_app)
        || !valid_app(&args.data_app)
        || args.cluster_id.trim().is_empty()
        || args.cluster_id.len() > 128
    {
        return Err("fly_autoscaling_materialize_input_invalid".to_string());
    }
    if args.output.exists() {
        return Err("fly_autoscaling_materialize_refuses_existing_output".to_string());
    }
    if let Some(parent) = args.output.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("fly_autoscaling_materialize_directory_failed:{error}"))?;
    }
    let tls_dir = std::env::temp_dir().join(format!(
        "mesh-fly-autoscaling-mtls-{}-{}",
        std::process::id(),
        unix_millis()
    ));
    let (ca_der_b64, cert_der_b64, key_der_b64, _, _) = generate_proof_mtls(&tls_dir)?;
    let _ = fs::remove_dir_all(&tls_dir);
    let (identity_signing_key_der_b64, identity_verify_key_b64) =
        mesh_rt::generate_identity_signing_material()?;
    let issued_at = unix_millis();
    let expires_at = issued_at.saturating_add(2 * 24 * 60 * 60 * 1_000);
    let node_name = |label: &str, app: &str| {
        format!("{label}@{label}.mesh_node.kv._metadata.{app}.internal:4370")
    };
    let mut identities = BTreeMap::new();
    for (key, role, label, app) in [
        (
            "controller1",
            "controller",
            "controller-1",
            args.controller_app.as_str(),
        ),
        (
            "controller2",
            "controller",
            "controller-2",
            args.controller_app.as_str(),
        ),
        (
            "controller3",
            "controller",
            "controller-3",
            args.controller_app.as_str(),
        ),
        ("gateway1", "gateway", "gateway-1", args.data_app.as_str()),
        ("gateway2", "gateway", "gateway-2", args.data_app.as_str()),
        ("worker1", "worker", "worker-1", args.data_app.as_str()),
        ("worker2", "worker", "worker-2", args.data_app.as_str()),
    ] {
        let (identity_key, envelope) = proof_identity_envelope(
            key,
            &args.cluster_id,
            &format!("{}/{role}/{label}", args.cluster_id),
            &node_name(label, app),
            &[role],
            issued_at,
            expires_at,
            &identity_signing_key_der_b64,
        )?;
        identities.insert(identity_key, envelope);
    }
    let (_, operator_envelope) = proof_identity_envelope(
        "operator",
        &args.cluster_id,
        &format!("{}/operator/fly-proof", args.cluster_id),
        "*",
        &["operator"],
        issued_at,
        expires_at,
        &identity_signing_key_der_b64,
    )?;
    identities.insert("operator".to_string(), operator_envelope);
    let voters = (1..=3)
        .map(|ordinal| {
            let label = format!("controller-{ordinal}");
            format!(
                "{}/controller/{label}|{}",
                args.cluster_id,
                node_name(&label, &args.controller_app)
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let entropy = format!(
        "{}:{}:{}:{}",
        args.cluster_id, issued_at, identity_signing_key_der_b64, key_der_b64
    );
    let cookie = format!("mesh-proof-{:x}", Sha256::digest(entropy.as_bytes()));
    let operator_key = format!(
        "mesh-operator-{:x}",
        Sha256::digest(format!("operator:{entropy}").as_bytes())
    );
    let document = json!({
        "schema_version": 1,
        "cluster_id": args.cluster_id,
        "controller_app": args.controller_app,
        "data_app": args.data_app,
        "controller_seed": format!("{}.internal", args.controller_app),
        "controller_target": node_name("controller-1", &args.controller_app),
        "voters": voters,
        "cookie": cookie,
        "operator_key": operator_key,
        "tls_ca_der_b64": ca_der_b64,
        "tls_cert_der_b64": cert_der_b64,
        "tls_key_der_b64": key_der_b64,
        "identity_signing_key_der_b64": identity_signing_key_der_b64,
        "identity_verify_key_b64": identity_verify_key_b64,
        "identities": identities,
    });
    let encoded = serde_json::to_vec_pretty(&document)
        .map_err(|error| format!("fly_autoscaling_materialize_encode_failed:{error}"))?;
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    let mut output = options
        .open(&args.output)
        .map_err(|error| format!("fly_autoscaling_materialize_open_failed:{error}"))?;
    output
        .write_all(&encoded)
        .map_err(|error| format!("fly_autoscaling_materialize_write_failed:{error}"))?;
    output
        .write_all(b"\n")
        .map_err(|error| format!("fly_autoscaling_materialize_write_failed:{error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&args.output, fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("fly_autoscaling_materialize_chmod_failed:{error}"))?;
    }
    println!("fly_autoscaling_material: {}", args.output.display());
    Ok(())
}

fn run_autonomous_chaos(args: AutonomousChaosArgs) -> Result<(), String> {
    if args.rounds == 0 || args.rounds > 100 {
        return Err("autonomous_chaos_rounds_must_be_1_to_100".to_string());
    }
    let root = repository_root()?;
    let evidence = args.evidence_dir.unwrap_or_else(|| {
        root.join("target")
            .join("proof")
            .join("autonomous-chaos")
            .join(unix_millis().to_string())
    });
    fs::create_dir_all(&evidence)
        .map_err(|error| format!("autonomous_chaos_evidence_directory_failed:{error}"))?;
    let filters = [
        "model_property_",
        "retry_budget_bounds_recovery_amplification",
        "disk_limit_rejects_new_work_without_evicting_active_records",
        "tombstone_prevents_delayed_record_resurrection",
        "interrupted_snapshot_resumes_from_next_verified_chunk",
        "controller_minority_cannot_elect_or_commit_control_mutations",
        "old_leader_term_is_fenced_after_new_election",
        "three_voter_consensus_replicates_and_survives_leader_failure",
        "automatic_recovery_rolls_attempt_after_owner_loss",
        "state_machine_deduplicates_a_retried_command_id",
    ];
    let mut assertions = BTreeMap::new();
    let mut executions = Vec::new();
    let mut passed = true;
    for round in 0..args.rounds {
        for offset in 0..filters.len() {
            // Rotate the deterministic execution order each round so hidden
            // process-global ordering dependencies cannot pass by accident.
            let index = (offset + usize::from(round)) % filters.len();
            let filter = filters[index];
            let output = Command::new("cargo")
                .args([
                    "test",
                    "-p",
                    "mesh-rt",
                    "--locked",
                    filter,
                    "--",
                    "--nocapture",
                ])
                .env("CARGO_INCREMENTAL", "0")
                .env("RUST_TEST_THREADS", if round % 2 == 0 { "1" } else { "4" })
                .current_dir(&root)
                .stdin(Stdio::null())
                .output()
                .map_err(|error| format!("autonomous_chaos_test_start_failed:{filter}:{error}"))?;
            let name = format!("round-{round:03}-{index:02}-{filter}");
            fs::write(evidence.join(format!("{name}.stdout.log")), &output.stdout)
                .map_err(|error| format!("autonomous_chaos_stdout_write_failed:{error}"))?;
            fs::write(
                evidence.join(format!("{name}.stderr.log")),
                redact(&String::from_utf8_lossy(&output.stderr)),
            )
            .map_err(|error| format!("autonomous_chaos_stderr_write_failed:{error}"))?;
            let success = output.status.success();
            assertions
                .entry(filter.to_string())
                .and_modify(|value| *value = *value && success)
                .or_insert(success);
            executions.push(json!({
                "round": round,
                "filter": filter,
                "test_threads": if round % 2 == 0 { 1 } else { 4 },
                "exit_code": output.status.code(),
                "passed": success
            }));
            passed &= success;
            if !success {
                break;
            }
        }
        if !passed {
            break;
        }
    }
    let summary = json!({
        "schema_version": 1,
        "rounds_requested": args.rounds,
        "rounds_completed": executions.len() / filters.len(),
        "filters": filters,
        "assertions": assertions,
        "executions": executions,
        "passed": passed
    });
    fs::write(
        evidence.join("summary.json"),
        serde_json::to_vec_pretty(&summary).expect("serialize autonomous chaos summary"),
    )
    .map_err(|error| format!("autonomous_chaos_summary_write_failed:{error}"))?;
    println!(
        "autonomous_chaos: {} ({} rounds, evidence {})",
        if passed { "PASS" } else { "FAIL" },
        args.rounds,
        evidence.display()
    );
    if passed {
        Ok(())
    } else {
        Err("autonomous_chaos_gate_failed".to_string())
    }
}

fn run_fly_driver_conformance(args: FlyDriverConformanceArgs) -> Result<(), String> {
    let root = repository_root()?;
    let evidence = args.evidence_dir.unwrap_or_else(|| {
        root.join("target")
            .join("proof")
            .join("fly-driver-conformance")
            .join(unix_millis().to_string())
    });
    fs::create_dir_all(&evidence)
        .map_err(|error| format!("fly_conformance_evidence_directory_failed:{error}"))?;
    let output = Command::new("cargo")
        .args([
            "test",
            "-p",
            "mesh-rt",
            "--locked",
            "fly_driver_conformance_",
            "--",
            "--nocapture",
        ])
        .env("CARGO_INCREMENTAL", "0")
        .current_dir(&root)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("fly_conformance_test_start_failed:{error}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    fs::write(evidence.join("cargo-test.stdout.log"), stdout.as_bytes())
        .map_err(|error| format!("fly_conformance_stdout_write_failed:{error}"))?;
    fs::write(evidence.join("cargo-test.stderr.log"), redact(&stderr))
        .map_err(|error| format!("fly_conformance_stderr_write_failed:{error}"))?;
    let passed_tests = stdout
        .lines()
        .filter(|line| {
            line.starts_with("test dist::scaling::tests::fly_driver_conformance_")
                && line.ends_with(" ... ok")
        })
        .count();
    let required_assertions = BTreeMap::from([
        (
            "api_configuration_validation",
            stdout.contains("fly_driver_conformance_validates_configuration_against_api ... ok"),
        ),
        (
            "api_origin_token_exfiltration_fence",
            stdout.contains(
                "fly_driver_conformance_rejects_token_forwarding_to_unapproved_origin ... ok",
            ),
        ),
        (
            "credential_and_environment_redaction",
            stdout.contains(
                "fly_driver_conformance_debug_redacts_credentials_and_environment ... ok",
            ),
        ),
        (
            "fenced_idempotent_create",
            stdout.contains(
                "fly_driver_conformance_ensure_is_idempotent_and_sends_fenced_metadata ... ok",
            ),
        ),
        (
            "autonomous_worker_requires_signed_identity",
            stdout.contains(
                "fly_driver_conformance_requires_signing_material_for_autonomous_workers ... ok",
            ),
        ),
        (
            "create_response_loss_adoption",
            stdout.contains("fly_driver_conformance_adopts_after_create_response_loss ... ok"),
        ),
        (
            "controller_restart_adoption",
            stdout.contains(
                "fly_driver_conformance_adopts_existing_operation_after_controller_restart ... ok",
            ),
        ),
        (
            "retryable_failure_and_backoff",
            stdout.contains(
                "fly_driver_conformance_retry_backoff_suppresses_immediate_provider_replay ... ok",
            ),
        ),
        (
            "exact_metadata_adoption_fence",
            stdout.contains("fly_driver_conformance_refuses_mismatched_adoption ... ok"),
        ),
        (
            "cluster_and_pool_scoping",
            stdout.contains("fly_driver_conformance_observation_is_cluster_and_pool_scoped ... ok"),
        ),
        (
            "cordon_before_delete",
            stdout.contains(
                "fly_driver_conformance_cordons_before_force_deleting_managed_machine ... ok",
            ),
        ),
        (
            "unmanaged_resource_protection",
            stdout.contains("fly_driver_conformance_never_deletes_unmanaged_machine ... ok"),
        ),
        (
            "already_absent_delete_idempotency",
            stdout
                .contains("fly_driver_conformance_absent_termination_is_idempotent_success ... ok"),
        ),
    ]);
    let passed = output.status.success()
        && passed_tests >= required_assertions.len()
        && required_assertions.values().all(|assertion| *assertion);
    let summary = json!({
        "schema_version": 1,
        "driver": "fly_machines",
        "credentialed": false,
        "test_filter": "fly_driver_conformance_",
        "passed_tests": passed_tests,
        "required_assertions": required_assertions,
        "passed": passed,
        "staging_certification_required_for_general_availability": true
    });
    fs::write(
        evidence.join("summary.json"),
        serde_json::to_vec_pretty(&summary).expect("serialize Fly conformance summary"),
    )
    .map_err(|error| format!("fly_conformance_summary_write_failed:{error}"))?;
    println!(
        "fly_driver_conformance: {} ({} tests, evidence {})",
        if passed { "PASS" } else { "FAIL" },
        passed_tests,
        evidence.display()
    );
    if passed {
        Ok(())
    } else {
        Err(format!(
            "fly_driver_conformance_failed:exit={:?}",
            output.status.code()
        ))
    }
}

fn wait_fly_driver_operation(
    deadline: Instant,
    label: &str,
    mut attempt: impl FnMut() -> Result<DriverOperation, String>,
) -> Result<DriverOperation, String> {
    let mut last = "not_attempted".to_string();
    while Instant::now() < deadline {
        let operation = attempt()?;
        match &operation.state {
            DriverOperationState::Succeeded => return Ok(operation),
            DriverOperationState::PermanentFailure(reason) => {
                return Err(format!("fly_staging_{label}_permanent_failure:{reason}"));
            }
            DriverOperationState::RetryableFailure(reason) => last = reason.clone(),
            DriverOperationState::Pending => last = "pending".to_string(),
            DriverOperationState::Unknown => last = "unknown".to_string(),
        }
        thread::park_timeout(Duration::from_millis(250));
    }
    Err(format!("fly_staging_{label}_timeout:{last}"))
}

fn run_fly_driver_staging(args: FlyDriverStagingArgs) -> Result<(), String> {
    if !args.confirm_create_and_delete {
        return Err(
            "fly_staging_requires_--confirm-create-and-delete_for_one_staging_machine".to_string(),
        );
    }
    if args.deadline_seconds == 0
        || args.app_name.trim().is_empty()
        || args.image.trim().is_empty()
        || args.cluster_id.trim().is_empty()
        || args.token_env.trim().is_empty()
    {
        return Err("fly_staging_arguments_invalid".to_string());
    }
    let api_token = std::env::var(&args.token_env)
        .map_err(|_| format!("fly_staging_token_environment_missing:{}", args.token_env))?;
    let mut environment = BTreeMap::new();
    for name in &args.worker_env {
        if name.is_empty() || name.contains('=') || environment.contains_key(name) {
            return Err(format!(
                "fly_staging_worker_environment_name_invalid:{name}"
            ));
        }
        let value = std::env::var(name)
            .map_err(|_| format!("fly_staging_worker_environment_missing:{name}"))?;
        environment.insert(name.clone(), value);
    }

    let root = repository_root()?;
    let timestamp = unix_millis();
    let evidence = args.evidence_dir.unwrap_or_else(|| {
        root.join("target")
            .join("proof")
            .join("fly-driver-staging")
            .join(timestamp.to_string())
    });
    fs::create_dir_all(&evidence)
        .map_err(|error| format!("fly_staging_evidence_directory_failed:{error}"))?;
    let template_revision = args
        .template_revision
        .unwrap_or_else(|| format!("fly-staging-{timestamp}"));
    let operation_timeout = Duration::from_secs(args.deadline_seconds.min(30));
    let driver = FlyMachinesCapacityDriver::new(FlyMachinesDriverConfig {
        api_base_url: args.api_base_url.clone(),
        app_name: args.app_name.clone(),
        api_token: api_token.clone(),
        image: args.image.clone(),
        region: args.region.clone(),
        pool: args.pool.clone(),
        environment,
        cpu_kind: args.cpu_kind.clone(),
        cpus: args.cpus,
        memory_mb: args.memory_mb,
        operation_timeout,
    });
    let deadline = Instant::now() + Duration::from_secs(args.deadline_seconds);
    let desired_revision = DesiredRevision(timestamp.max(1));
    let mut assertions = BTreeMap::new();
    let mut events = Vec::new();
    let mut created_node = None::<String>;

    let execution = (|| -> Result<(), String> {
        driver.validate_configuration()?;
        assertions.insert("configuration_and_credentials_valid", true);

        let ensure = DriverOperation {
            cluster_id: args.cluster_id.clone(),
            operation_id: format!("fly-staging-{timestamp}-ensure"),
            control_term: ControlTerm(1),
            desired_revision,
            template_revision: template_revision.clone(),
            node_id: None,
            state: DriverOperationState::Pending,
        };
        let ensured =
            wait_fly_driver_operation(deadline, "ensure", || driver.ensure_node(&ensure))?;
        let node_id = ensured
            .node_id
            .clone()
            .ok_or_else(|| "fly_staging_ensure_missing_machine_id".to_string())?;
        created_node = Some(node_id.clone());
        assertions.insert("machine_created_with_fenced_operation", true);
        events.push(json!({"phase": "ensure", "operation": ensured}));

        let mut last_lifecycle = "not_observed".to_string();
        loop {
            if Instant::now() >= deadline {
                return Err(format!("fly_staging_readiness_timeout:{last_lifecycle}"));
            }
            let observation = driver.observe_capacity(&args.cluster_id)?;
            match observation
                .nodes
                .iter()
                .find(|node| node.node_id == node_id)
            {
                Some(node) if node.lifecycle == CapacityNodeLifecycle::Ready => break,
                Some(node) if node.lifecycle == CapacityNodeLifecycle::Failed => {
                    return Err("fly_staging_machine_failed_before_readiness".to_string());
                }
                Some(node) => last_lifecycle = format!("{:?}", node.lifecycle),
                None => last_lifecycle = "absent".to_string(),
            }
            thread::park_timeout(Duration::from_millis(500));
        }
        assertions.insert("machine_reached_ready", true);
        events.push(json!({"phase": "ready", "node_id": node_id}));

        let drain = DriverOperation {
            cluster_id: args.cluster_id.clone(),
            operation_id: format!("fly-staging-{timestamp}-drain"),
            control_term: ControlTerm(1),
            desired_revision,
            template_revision: template_revision.clone(),
            node_id: Some(node_id.clone()),
            state: DriverOperationState::Pending,
        };
        let drained =
            wait_fly_driver_operation(deadline, "drain", || driver.begin_drain(&drain, &node_id))?;
        assertions.insert("machine_cordoned_before_delete", true);
        events.push(json!({"phase": "drain", "operation": drained}));

        let terminate = DriverOperation {
            cluster_id: args.cluster_id.clone(),
            operation_id: format!("fly-staging-{timestamp}-terminate"),
            control_term: ControlTerm(1),
            desired_revision,
            template_revision: template_revision.clone(),
            node_id: Some(node_id.clone()),
            state: DriverOperationState::Pending,
        };
        let terminated = wait_fly_driver_operation(deadline, "terminate", || {
            driver.terminate_node(&terminate, &node_id)
        })?;
        assertions.insert("machine_delete_succeeded", true);
        events.push(json!({"phase": "terminate", "operation": terminated}));

        loop {
            if Instant::now() >= deadline {
                return Err("fly_staging_observed_removal_timeout".to_string());
            }
            let observation = driver.observe_capacity(&args.cluster_id)?;
            let removed = observation
                .nodes
                .iter()
                .find(|node| node.node_id == node_id)
                .is_none_or(|node| node.lifecycle == CapacityNodeLifecycle::Removed);
            if removed {
                break;
            }
            thread::park_timeout(Duration::from_millis(500));
        }
        assertions.insert("machine_removal_observed", true);
        Ok(())
    })();

    let mut cleanup = None::<String>;
    if execution.is_err() {
        if let Some(node_id) = created_node.as_deref() {
            let cleanup_deadline = Instant::now() + Duration::from_secs(60);
            let drain = DriverOperation {
                cluster_id: args.cluster_id.clone(),
                operation_id: format!("fly-staging-{timestamp}-cleanup-drain"),
                control_term: ControlTerm(1),
                desired_revision,
                template_revision: template_revision.clone(),
                node_id: Some(node_id.to_string()),
                state: DriverOperationState::Pending,
            };
            let terminate = DriverOperation {
                operation_id: format!("fly-staging-{timestamp}-cleanup-terminate"),
                ..drain.clone()
            };
            let result = wait_fly_driver_operation(cleanup_deadline, "cleanup_drain", || {
                driver.begin_drain(&drain, node_id)
            })
            .and_then(|_| {
                wait_fly_driver_operation(cleanup_deadline, "cleanup_terminate", || {
                    driver.terminate_node(&terminate, node_id)
                })
            });
            cleanup = Some(match result {
                Ok(_) => "succeeded".to_string(),
                Err(reason) => format!("failed:{reason}"),
            });
        }
    }

    let passed = execution.is_ok()
        && [
            "configuration_and_credentials_valid",
            "machine_created_with_fenced_operation",
            "machine_reached_ready",
            "machine_cordoned_before_delete",
            "machine_delete_succeeded",
            "machine_removal_observed",
        ]
        .iter()
        .all(|name| assertions.get(*name) == Some(&true));
    let error = execution
        .as_ref()
        .err()
        .map(|reason| reason.replace(&api_token, "[redacted]"));
    let summary = json!({
        "schema_version": 1,
        "driver": "fly_machines",
        "credentialed": true,
        "app_name": args.app_name,
        "cluster_id": args.cluster_id,
        "pool": args.pool,
        "region": args.region,
        "image": args.image,
        "template_revision": template_revision,
        "token_environment": args.token_env,
        "forwarded_environment_names": args.worker_env,
        "created_node_id": created_node,
        "assertions": assertions,
        "events": events,
        "cleanup_after_failure": cleanup,
        "error": error,
        "passed": passed
    });
    fs::write(
        evidence.join("summary.json"),
        serde_json::to_vec_pretty(&summary).expect("serialize Fly staging summary"),
    )
    .map_err(|write_error| format!("fly_staging_summary_write_failed:{write_error}"))?;
    println!(
        "fly_driver_staging: {} (evidence {})",
        if passed { "PASS" } else { "FAIL" },
        evidence.display()
    );
    if passed {
        Ok(())
    } else {
        Err(error.unwrap_or_else(|| "fly_staging_required_assertion_failed".to_string()))
    }
}

struct ProofHarness {
    root: PathBuf,
    compose_file: PathBuf,
    evidence: PathBuf,
    project: String,
    cluster_id: String,
    image: String,
    driver_image: String,
    keep_running: bool,
    tls_dir: PathBuf,
    tls_ca_der_b64: String,
    tls_cert_der_b64: String,
    tls_key_der_b64: String,
    driver_cert_der_b64: String,
    driver_key_der_b64: String,
    driver_shared_key: String,
    identity_signing_key_der_b64: String,
    identity_verify_key_b64: String,
    identity_envelopes: BTreeMap<String, String>,
    assertions: BTreeMap<String, bool>,
    events: Vec<Value>,
}

fn write_owner_only_new(path: &Path, contents: &[u8], label: &str) -> Result<(), String> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|error| format!("{label}_directory_failed:{error}"))?;
    }
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut output = options
        .open(path)
        .map_err(|error| format!("{label}_open_failed:{error}"))?;
    output
        .write_all(contents)
        .map_err(|error| format!("{label}_write_failed:{error}"))?;
    output
        .sync_all()
        .map_err(|error| format!("{label}_sync_failed:{error}"))
}

fn absolute_path(path: PathBuf) -> Result<PathBuf, String> {
    if path.is_absolute() {
        Ok(path)
    } else {
        std::env::current_dir()
            .map(|directory| directory.join(path))
            .map_err(|error| format!("proof_current_directory_failed:{error}"))
    }
}

fn connection_secret_paths(path: &Path) -> (PathBuf, PathBuf) {
    (
        path.with_extension("cookie"),
        path.with_extension("operator-key"),
    )
}

fn ensure_connection_outputs_are_new(path: &Path) -> Result<(), String> {
    let (cookie_file, operator_key_file) = connection_secret_paths(path);
    if path == cookie_file || path == operator_key_file {
        return Err("proof_connection_file_extension_invalid".to_string());
    }
    for output in [path.to_path_buf(), cookie_file, operator_key_file] {
        if output.exists() {
            return Err(format!(
                "proof_connection_refuses_existing_output:{}",
                output.display()
            ));
        }
    }
    Ok(())
}

impl ProofHarness {
    fn redact(&self, value: &str) -> String {
        redact(value)
            .replace(&self.tls_key_der_b64, "[redacted-private-key]")
            .replace(&self.driver_key_der_b64, "[redacted-driver-private-key]")
            .replace(
                &self.identity_signing_key_der_b64,
                "[redacted-identity-signing-key]",
            )
            .replace(&self.driver_shared_key, "[redacted-driver-shared-key]")
    }

    fn proof_environment(&self) -> BTreeMap<String, &str> {
        let mut environment = BTreeMap::from([
            ("MESH_PROOF_PROJECT".to_string(), self.project.as_str()),
            (
                "MESH_PROOF_CLUSTER_ID".to_string(),
                self.cluster_id.as_str(),
            ),
            ("MESH_PROOF_IMAGE".to_string(), self.image.as_str()),
            (
                "MESH_PROOF_DRIVER_IMAGE".to_string(),
                self.driver_image.as_str(),
            ),
            (
                "MESH_PROOF_TLS_CA_DER_B64".to_string(),
                self.tls_ca_der_b64.as_str(),
            ),
            (
                "MESH_PROOF_TLS_CERT_DER_B64".to_string(),
                self.tls_cert_der_b64.as_str(),
            ),
            (
                "MESH_PROOF_TLS_KEY_DER_B64".to_string(),
                self.tls_key_der_b64.as_str(),
            ),
            (
                "MESH_PROOF_DRIVER_CERT_DER_B64".to_string(),
                self.driver_cert_der_b64.as_str(),
            ),
            (
                "MESH_PROOF_DRIVER_KEY_DER_B64".to_string(),
                self.driver_key_der_b64.as_str(),
            ),
            (
                "MESH_PROOF_DRIVER_SHARED_KEY".to_string(),
                self.driver_shared_key.as_str(),
            ),
            (
                "MESH_PROOF_IDENTITY_SIGNING_KEY_DER_B64".to_string(),
                self.identity_signing_key_der_b64.as_str(),
            ),
            (
                "MESH_PROOF_IDENTITY_VERIFY_KEY_B64".to_string(),
                self.identity_verify_key_b64.as_str(),
            ),
        ]);
        environment.extend(
            self.identity_envelopes
                .iter()
                .map(|(name, value)| (format!("MESH_PROOF_IDENTITY_{name}"), value.as_str())),
        );
        environment
    }

    fn command(&self, program: &str, args: &[&str]) -> Result<Output, String> {
        Command::new(program)
            .args(args)
            .current_dir(&self.root)
            .envs(self.proof_environment())
            .stdin(Stdio::null())
            .output()
            .map_err(|error| format!("proof_command_start_failed:{program}:{error}"))
    }

    fn checked(&self, program: &str, args: &[&str]) -> Result<String, String> {
        let output = self.command(program, args)?;
        if !output.status.success() {
            return Err(format!(
                "proof_command_failed:{} {}\nstdout={}\nstderr={}",
                program,
                args.join(" "),
                String::from_utf8_lossy(&output.stdout),
                self.redact(&String::from_utf8_lossy(&output.stderr)),
            ));
        }
        String::from_utf8(output.stdout)
            .map(|value| value.trim().to_string())
            .map_err(|_| "proof_command_output_invalid_utf8".to_string())
    }

    fn compose(&self, args: &[&str]) -> Result<String, String> {
        let mut command = vec!["compose", "-f", self.compose_file.to_str().unwrap()];
        command.extend_from_slice(args);
        self.checked("docker", &command)
    }

    fn write(&self, name: &str, contents: impl AsRef<[u8]>) -> Result<(), String> {
        fs::write(self.evidence.join(name), contents)
            .map_err(|error| format!("proof_evidence_write_failed:{name}:{error}"))
    }

    fn write_connection_manifest(&self, path: &Path) -> Result<(), String> {
        let (cookie_file, operator_key_file) = connection_secret_paths(path);
        write_owner_only_new(
            &cookie_file,
            format!("{COOKIE}\n").as_bytes(),
            "proof_connection_cookie",
        )?;
        write_owner_only_new(
            &operator_key_file,
            format!("{OPERATOR_KEY}\n").as_bytes(),
            "proof_connection_operator_key",
        )?;
        let meshc_path = std::env::current_exe()
            .map_err(|error| format!("proof_connection_meshc_path_failed:{error}"))?;
        let voters = format!(
            "{}/controller/controller1|controller1@controller1:4370,{}/controller/controller2|controller2@controller2:4370,{}/controller/controller3|controller3@controller3:4370",
            self.cluster_id, self.cluster_id, self.cluster_id,
        );
        let operator_identity = self
            .identity_envelopes
            .get("OPERATOR")
            .ok_or_else(|| "proof_connection_operator_identity_missing".to_string())?;
        let mut environment: BTreeMap<String, String> = self
            .proof_environment()
            .into_iter()
            .map(|(name, value)| (name, value.to_string()))
            .collect();
        environment.extend([
            (
                "MESH_TLS_CA_DER_B64".to_string(),
                self.tls_ca_der_b64.clone(),
            ),
            (
                "MESH_TLS_CERT_DER_B64".to_string(),
                self.tls_cert_der_b64.clone(),
            ),
            (
                "MESH_TLS_KEY_DER_B64".to_string(),
                self.tls_key_der_b64.clone(),
            ),
            ("MESH_CLUSTER_MODE".to_string(), "autonomous".to_string()),
            ("MESH_CLUSTER_ID".to_string(), self.cluster_id.clone()),
            ("MESH_CONTROLLER_VOTERS".to_string(), voters),
            (
                "MESH_STABLE_NODE_ID".to_string(),
                format!("{}/operator/proof", self.cluster_id),
            ),
            ("MESH_ROLES".to_string(), "operator".to_string()),
            (
                mesh_rt::IDENTITY_VERIFY_KEYS_ENV.to_string(),
                self.identity_verify_key_b64.clone(),
            ),
            (
                mesh_rt::IDENTITY_ENVELOPE_ENV.to_string(),
                operator_identity.clone(),
            ),
        ]);
        let services = BTreeMap::from([
            ("controller1@controller1:4370", "controller1"),
            ("controller2@controller2:4370", "controller2"),
            ("controller3@controller3:4370", "controller3"),
            ("gateway1@gateway1:4370", "gateway1"),
            ("gateway2@gateway2:4370", "gateway2"),
            ("worker1@worker1:4370", "worker1"),
            ("worker2@worker2:4370", "worker2"),
        ]);
        let document = json!({
            "schemaVersion": 1,
            "provider": "docker",
            "clusterId": self.cluster_id,
            "controllerTargets": [
                "controller1@127.0.0.1:14371",
                "controller2@127.0.0.1:14372",
                "controller3@127.0.0.1:14373",
            ],
            "gatewayUrls": ["http://127.0.0.1:18081", "http://127.0.0.1:18082"],
            "meshcPath": meshc_path,
            "cookieFile": cookie_file,
            "operatorKeyFile": operator_key_file,
            "environment": environment,
            "docker": {
                "project": self.project,
                "composeFile": self.compose_file,
                "services": services,
                "fence": {
                    "composeProjectLabel": self.project,
                    "managedClusterLabel": self.cluster_id,
                    "managedLabel": "true",
                },
            },
        });
        let mut encoded = serde_json::to_vec_pretty(&document)
            .map_err(|error| format!("proof_connection_encode_failed:{error}"))?;
        encoded.push(b'\n');
        write_owner_only_new(path, &encoded, "proof_connection")
    }

    fn snapshot_containers(&self, label: &str) -> Result<(), String> {
        let containers = self.checked(
            "docker",
            &[
                "ps",
                "-a",
                "--filter",
                &format!("label=com.docker.compose.project={}", self.project),
                "--format",
                "{{json .}}",
            ],
        )?;
        let managed = self.checked(
            "docker",
            &[
                "ps",
                "-a",
                "--filter",
                &format!("label=mesh.cluster={}", self.cluster_id),
                "--format",
                "{{json .}}",
            ],
        )?;
        self.write(
            &format!("containers-{label}.jsonl"),
            format!("{containers}\n{managed}\n"),
        )
    }

    fn inspect_managed_containers(&self, label: &str) -> Result<Value, String> {
        let ids = self.checked(
            "docker",
            &[
                "ps",
                "-aq",
                "--filter",
                &format!("label=mesh.cluster={}", self.cluster_id),
                "--filter",
                "label=mesh.managed=true",
            ],
        )?;
        let ids: Vec<_> = ids.lines().filter(|line| !line.is_empty()).collect();
        let inspect = if ids.is_empty() {
            Value::Array(Vec::new())
        } else {
            let mut arguments = vec!["inspect"];
            arguments.extend(ids);
            let output = self.checked("docker", &arguments)?;
            serde_json::from_str(&output)
                .map_err(|error| format!("proof_managed_inspect_invalid:{error}"))?
        };
        self.write(
            &format!("managed-containers-{label}-inspect.json"),
            serde_json::to_vec_pretty(&inspect).expect("serialize managed container inspection"),
        )?;
        Ok(inspect)
    }

    fn collect_logs(&self) {
        if let Ok(logs) = self.compose(&["logs", "--no-color", "--timestamps"]) {
            let _ = self.write("compose.log", self.redact(&logs));
        }
        if let Ok(ids) = self.checked(
            "docker",
            &[
                "ps",
                "-aq",
                "--filter",
                &format!("label=mesh.cluster={}", self.cluster_id),
            ],
        ) {
            let ids: Vec<_> = ids.lines().filter(|line| !line.is_empty()).collect();
            if !ids.is_empty() {
                let mut arguments = vec!["inspect"];
                arguments.extend(ids.iter().copied());
                if let Ok(inspect) = self.checked("docker", &arguments) {
                    let _ = self.write("managed-containers-inspect.json", self.redact(&inspect));
                }
                for id in ids {
                    if let Ok(logs) = self.checked("docker", &["logs", id]) {
                        let label = id.get(..12).unwrap_or(id);
                        let _ = self.write(
                            &format!("managed-container-{label}.log"),
                            self.redact(&logs),
                        );
                    }
                }
            }
        }
    }

    fn cleanup(&self) -> Result<(), String> {
        // Stop the controller before enumerating driver-owned capacity. Without
        // this ordering a final reconcile can create a worker between `ps` and
        // `compose down`, leaving both a container and the proof network behind.
        let _ = self.compose(&["stop", "--timeout", "10"])?;
        let managed = self.checked(
            "docker",
            &[
                "ps",
                "-aq",
                "--filter",
                &format!("label=mesh.cluster={}", self.cluster_id),
                "--filter",
                "label=mesh.managed=true",
            ],
        )?;
        let ids: Vec<&str> = managed.lines().filter(|line| !line.is_empty()).collect();
        if !ids.is_empty() {
            let mut args = vec!["rm", "-f"];
            args.extend(ids);
            let _ = self.checked("docker", &args)?;
        }
        let _ = self.compose(&["down", "--volumes", "--remove-orphans", "--timeout", "10"])?;
        let late = self.checked(
            "docker",
            &[
                "ps",
                "-aq",
                "--filter",
                &format!("label=mesh.cluster={}", self.cluster_id),
                "--filter",
                "label=mesh.managed=true",
            ],
        )?;
        let late_ids: Vec<&str> = late.lines().filter(|line| !line.is_empty()).collect();
        if !late_ids.is_empty() {
            let mut args = vec!["rm", "-f"];
            args.extend(late_ids);
            let _ = self.checked("docker", &args)?;
        }
        let _ = fs::remove_dir_all(&self.tls_dir);
        Ok(())
    }
}

fn validate_docker_autoscaling_args(args: &DockerAutoscalingArgs) -> Result<(), String> {
    if args.start_only && !args.keep_running {
        return Err("docker_autoscaling_start_only_requires_keep_running".to_string());
    }
    if !args.start_only && args.connection_file.is_some() {
        return Err("docker_autoscaling_connection_file_requires_start_only".to_string());
    }
    Ok(())
}

fn run_docker_autoscaling(args: DockerAutoscalingArgs) -> Result<(), String> {
    validate_docker_autoscaling_args(&args)?;
    let DockerAutoscalingArgs {
        keep_running,
        evidence_dir,
        no_build,
        start_only,
        connection_file,
    } = args;
    let root = repository_root()?;
    let timestamp = unix_millis();
    let project = format!("mesh-proof-{}-{timestamp}", std::process::id());
    let evidence = evidence_dir.unwrap_or_else(|| {
        root.join("target")
            .join("proof")
            .join("docker-autoscaling")
            .join(timestamp.to_string())
    });
    let connection_file = start_only
        .then(|| connection_file.unwrap_or_else(|| evidence.join("connection.json")))
        .map(absolute_path)
        .transpose()?;
    if let Some(connection_file) = &connection_file {
        ensure_connection_outputs_are_new(connection_file)?;
    }
    fs::create_dir_all(&evidence)
        .map_err(|error| format!("proof_evidence_directory_failed:{error}"))?;
    let tls_dir = std::env::temp_dir().join(format!("mesh-proof-mtls-{timestamp}"));
    let (
        tls_ca_der_b64,
        tls_cert_der_b64,
        tls_key_der_b64,
        driver_cert_der_b64,
        driver_key_der_b64,
    ) = generate_proof_mtls(&tls_dir)?;
    let driver_shared_key = format!(
        "{:x}",
        Sha256::digest(format!("{timestamp}:{tls_key_der_b64}").as_bytes())
    );
    let (identity_signing_key_der_b64, identity_verify_key_b64) =
        mesh_rt::generate_identity_signing_material()?;
    let issued_at = unix_millis();
    let expires_at = issued_at.saturating_add(30 * 24 * 60 * 60 * 1_000);
    let identity_envelopes = BTreeMap::from([
        proof_identity_envelope(
            "CONTROLLER1",
            &project,
            &format!("{project}/controller/controller1"),
            "controller1@controller1:4370",
            &["controller"],
            issued_at,
            expires_at,
            &identity_signing_key_der_b64,
        )?,
        proof_identity_envelope(
            "CONTROLLER2",
            &project,
            &format!("{project}/controller/controller2"),
            "controller2@controller2:4370",
            &["controller"],
            issued_at,
            expires_at,
            &identity_signing_key_der_b64,
        )?,
        proof_identity_envelope(
            "CONTROLLER3",
            &project,
            &format!("{project}/controller/controller3"),
            "controller3@controller3:4370",
            &["controller"],
            issued_at,
            expires_at,
            &identity_signing_key_der_b64,
        )?,
        proof_identity_envelope(
            "GATEWAY1",
            &project,
            &format!("{project}/gateway/gateway1"),
            "gateway1@gateway1:4370",
            &["gateway"],
            issued_at,
            expires_at,
            &identity_signing_key_der_b64,
        )?,
        proof_identity_envelope(
            "GATEWAY2",
            &project,
            &format!("{project}/gateway/gateway2"),
            "gateway2@gateway2:4370",
            &["gateway"],
            issued_at,
            expires_at,
            &identity_signing_key_der_b64,
        )?,
        proof_identity_envelope(
            "WORKER1",
            &project,
            &format!("{project}/worker/worker1"),
            "worker1@worker1:4370",
            &["worker"],
            issued_at,
            expires_at,
            &identity_signing_key_der_b64,
        )?,
        proof_identity_envelope(
            "WORKER2",
            &project,
            &format!("{project}/worker/worker2"),
            "worker2@worker2:4370",
            &["worker"],
            issued_at,
            expires_at,
            &identity_signing_key_der_b64,
        )?,
        proof_identity_envelope(
            "OPERATOR",
            &project,
            &format!("{project}/operator/proof"),
            "*",
            &["operator"],
            issued_at,
            expires_at,
            &identity_signing_key_der_b64,
        )?,
    ]);
    std::env::set_var("MESH_TLS_CA_DER_B64", &tls_ca_der_b64);
    std::env::set_var("MESH_TLS_CERT_DER_B64", &tls_cert_der_b64);
    std::env::set_var("MESH_TLS_KEY_DER_B64", &tls_key_der_b64);
    std::env::set_var("MESH_CLUSTER_MODE", "autonomous");
    std::env::set_var("MESH_CLUSTER_ID", &project);
    std::env::set_var(
        "MESH_CONTROLLER_VOTERS",
        format!(
            "{project}/controller/controller1|controller1@controller1:4370,{project}/controller/controller2|controller2@controller2:4370,{project}/controller/controller3|controller3@controller3:4370"
        ),
    );
    std::env::set_var("MESH_STABLE_NODE_ID", format!("{project}/operator/proof"));
    std::env::set_var("MESH_ROLES", "operator");
    std::env::set_var(mesh_rt::IDENTITY_VERIFY_KEYS_ENV, &identity_verify_key_b64);
    std::env::set_var(
        mesh_rt::IDENTITY_ENVELOPE_ENV,
        identity_envelopes
            .get("OPERATOR")
            .expect("proof operator identity generated"),
    );
    let image = if no_build {
        "mesh-autoscaling-proof:local".to_string()
    } else {
        format!("mesh-autoscaling-proof:{timestamp}")
    };
    let driver_image = if no_build {
        "mesh-autoscaling-driver:local".to_string()
    } else {
        format!("mesh-autoscaling-driver:{timestamp}")
    };
    let mut harness = ProofHarness {
        compose_file: root.join("proof/docker-autoscaling/docker-compose.yml"),
        root,
        evidence,
        cluster_id: project.clone(),
        image,
        driver_image,
        project,
        keep_running,
        tls_dir,
        tls_ca_der_b64,
        tls_cert_der_b64,
        tls_key_der_b64,
        driver_cert_der_b64,
        driver_key_der_b64,
        driver_shared_key,
        identity_signing_key_der_b64,
        identity_verify_key_b64,
        identity_envelopes,
        assertions: BTreeMap::new(),
        events: Vec::new(),
    };

    let result = run_proof(&mut harness, no_build, connection_file.as_deref());
    harness.collect_logs();
    if !start_only {
        let compose_logs =
            fs::read_to_string(harness.evidence.join("compose.log")).unwrap_or_default();
        harness.assertions.insert(
            "docker_api_timeout_injected".to_string(),
            compose_logs.contains("fault=docker_api_timeout_once"),
        );
        harness.assertions.insert(
            "docker_create_response_loss_injected".to_string(),
            compose_logs.contains("fault=ensure_response_loss_once"),
        );
        harness.assertions.insert(
            "unhealthy_new_worker_injected".to_string(),
            compose_logs.contains("fault=unhealthy_new_worker_once"),
        );
    }
    let cleanup_result = if harness.keep_running {
        Ok(())
    } else {
        harness.cleanup()
    };
    harness
        .assertions
        .insert("cleanup_completed".to_string(), cleanup_result.is_ok());
    let passed = result.is_ok()
        && cleanup_result.is_ok()
        && harness.assertions.values().all(|passed| *passed);
    let summary = json!({
        "schema_version": 1,
        "mode": if start_only { "start_only" } else { "proof" },
        "passed": passed,
        "project": harness.project,
        "cluster_id": harness.cluster_id,
        "image": harness.image,
        "assertions": harness.assertions,
        "events": harness.events,
        "error": result.as_ref().err().map(|error| harness.redact(error)),
        "cleanup_error": cleanup_result.as_ref().err().map(|error| harness.redact(error)),
    });
    harness.write(
        "summary.json",
        serde_json::to_vec_pretty(&summary).expect("serialize proof summary"),
    )?;
    println!("evidence_bundle: {}", harness.evidence.display());
    if let Some(connection_file) = &connection_file {
        println!("connection_manifest: {}", connection_file.display());
        println!(
            "docker_autoscaling_topology: {}",
            if passed { "READY" } else { "FAILED" }
        );
    } else {
        println!(
            "docker_autoscaling_proof: {}",
            if passed { "PASS" } else { "FAIL" }
        );
    }
    if passed {
        Ok(())
    } else {
        Err(result
            .err()
            .or_else(|| cleanup_result.err())
            .unwrap_or_else(|| "proof_required_assertion_failed".to_string()))
    }
}

#[allow(clippy::too_many_arguments)]
fn proof_identity_envelope(
    name: &str,
    cluster_id: &str,
    stable_node_id: &str,
    advertised_name: &str,
    roles: &[&str],
    issued_at_unix_millis: u64,
    expires_at_unix_millis: u64,
    signing_key_der_b64: &str,
) -> Result<(String, String), String> {
    let claim = mesh_rt::NodeIdentityClaim {
        schema_version: mesh_rt::IDENTITY_SCHEMA_VERSION,
        cluster_id: cluster_id.to_string(),
        stable_node_id: stable_node_id.to_string(),
        advertised_name: advertised_name.to_string(),
        roles: roles.iter().map(|role| (*role).to_string()).collect(),
        issued_at_unix_millis,
        expires_at_unix_millis,
    };
    Ok((
        name.to_string(),
        mesh_rt::sign_identity_claim(&claim, signing_key_der_b64)?,
    ))
}

fn run_proof(
    harness: &mut ProofHarness,
    no_build: bool,
    connection_file: Option<&Path>,
) -> Result<(), String> {
    if connection_file.is_none() {
        let snapshot_resume = mesh_rt::prove_interrupted_snapshot_resume()?;
        harness.write(
            "continuity-snapshot-resume.json",
            serde_json::to_vec_pretty(&snapshot_resume)
                .expect("serialize continuity snapshot resume proof"),
        )?;
        harness.assertions.insert(
            "interrupted_continuity_snapshot_resumed".to_string(),
            snapshot_resume.chunks > 1
                && snapshot_resume.acknowledged_before_interruption > 0
                && snapshot_resume.records == 64,
        );
    }
    let docker_version = harness.checked("docker", &["version", "--format", "{{json .}}"])?;
    let compose_version = harness.checked("docker", &["compose", "version", "--short"])?;
    let revision = harness
        .checked("git", &["rev-parse", "HEAD"])
        .unwrap_or_else(|_| "unknown".to_string());
    let dirty = harness
        .checked("git", &["status", "--short"])
        .unwrap_or_else(|_| "unknown".to_string());
    harness.write(
        "environment.json",
        serde_json::to_vec_pretty(&json!({
            "docker": serde_json::from_str::<Value>(&docker_version).unwrap_or(Value::String(docker_version)),
            "compose": compose_version,
            "source_revision": revision,
            "dirty_worktree": dirty,
        }))
        .expect("serialize proof environment"),
    )?;

    let resolved = harness.compose(&["config"])?;
    harness.write("compose-resolved.redacted.yml", harness.redact(&resolved))?;
    harness
        .assertions
        .insert("compose_configuration_valid".to_string(), true);
    if !no_build {
        let dockerfile = harness
            .root
            .join("proof/docker-autoscaling/Dockerfile")
            .to_string_lossy()
            .into_owned();
        let _ = harness.checked(
            "docker",
            &[
                "build",
                "--file",
                &dockerfile,
                "--target",
                "runtime",
                "--tag",
                &harness.image,
                ".",
            ],
        )?;
        let _ = harness.checked(
            "docker",
            &["tag", &harness.image, "mesh-autoscaling-proof:local"],
        )?;
        let _ = harness.checked(
            "docker",
            &[
                "build",
                "--file",
                &dockerfile,
                "--target",
                "driver",
                "--tag",
                &harness.driver_image,
                ".",
            ],
        )?;
        let _ = harness.checked(
            "docker",
            &[
                "tag",
                &harness.driver_image,
                "mesh-autoscaling-driver:local",
            ],
        )?;
    }
    let image_inspection = harness.checked(
        "docker",
        &["image", "inspect", &harness.image, &harness.driver_image],
    )?;
    harness.write("images.json", harness.redact(&image_inspection))?;
    let metadata = harness.checked("cargo", &["metadata", "--no-deps", "--format-version", "1"])?;
    let metadata: Value = serde_json::from_str(&metadata)
        .map_err(|error| format!("proof_cargo_metadata_invalid:{error}"))?;
    let mesh_versions: Vec<Value> = metadata["packages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|package| {
            package["name"]
                .as_str()
                .is_some_and(|name| name == "meshc" || name == "mesh-rt")
        })
        .map(|package| {
            json!({
                "name": package["name"],
                "version": package["version"],
            })
        })
        .collect();
    harness.write(
        "software-versions.json",
        serde_json::to_vec_pretty(&mesh_versions).expect("serialize proof software versions"),
    )?;
    let _ = harness.compose(&["up", "-d", "--remove-orphans"])?;
    wait_for_http(18081, "/health", Duration::from_secs(180))?;
    wait_for_http(18082, "/health", Duration::from_secs(180))?;
    let mut controller_target = "controller1@127.0.0.1:14371".to_string();
    let baseline = wait_for_runtime(
        &controller_target,
        MIN_WORKERS as usize,
        Duration::from_secs(90),
    )?;
    harness.write(
        "capacity-baseline.json",
        serde_json::to_vec_pretty(&baseline).expect("serialize baseline"),
    )?;
    harness.snapshot_containers("baseline")?;
    harness.assertions.insert(
        "baseline_minimum_workers_present".to_string(),
        baseline
            .nodes
            .iter()
            .filter(|node| node.roles.iter().any(|role| role == "worker"))
            .count()
            >= MIN_WORKERS as usize,
    );
    if let Some(connection_file) = connection_file {
        harness.write_connection_manifest(connection_file)?;
        fs::remove_dir_all(&harness.tls_dir)
            .map_err(|error| format!("proof_mtls_cleanup_failed:{error}"))?;
        return Ok(());
    }

    let acknowledged_mutations = seed_postgres_mutations()?;
    // Keep policy pressure active through worker replacement and controller
    // failover. Docker creates workers sequentially, so a short fixed burst can
    // end while max-capacity workers are still warming and accidentally turn
    // the crash-replacement assertion into a race with scale-down.
    let load = start_load();
    let (peak, initial_desired) = wait_for_runtime_desired(
        &controller_target,
        |workers| workers > MIN_WORKERS,
        Duration::from_secs(30),
    )?;
    harness.events.push(json!({
        "phase": "scale_up",
        "decision": peak.autonomous.last_decision,
        "desired": initial_desired,
    }));
    harness.write(
        "capacity-peak-input.json",
        serde_json::to_vec_pretty(&peak).expect("serialize peak"),
    )?;
    let initial_peak_managed = initial_desired.saturating_sub(MIN_WORKERS) as usize;
    wait_for_managed_count(
        harness,
        initial_peak_managed.max(1),
        Duration::from_secs(90),
    )?;
    let (peak, desired) = wait_for_stable_runtime(&controller_target, Duration::from_secs(120))?;
    let peak_managed = desired.saturating_sub(MIN_WORKERS) as usize;
    harness.write(
        "capacity-peak-ready.json",
        serde_json::to_vec_pretty(&peak).expect("serialize ready peak"),
    )?;
    let concurrent_burst = run_concurrent_remote_burst(&controller_target, 1_000)?;
    harness.write(
        "concurrent-1000-summary.json",
        serde_json::to_vec_pretty(&concurrent_burst).expect("serialize concurrent request summary"),
    )?;
    harness.assertions.insert(
        "one_thousand_concurrent_remote_requests_completed".to_string(),
        concurrent_burst.successes == 1_000
            && concurrent_burst.failures == 0
            && concurrent_burst.unique_request_keys == 1_000
            && concurrent_burst.remote_executions == 1_000,
    );
    harness.assertions.insert(
        "concurrent_remote_request_p99_within_budget".to_string(),
        concurrent_burst.latency_p99_millis <= CONCURRENT_BURST_P99_BUDGET_MILLIS,
    );
    harness.assertions.insert(
        "slow_handler_does_not_starve_operator_or_gateway_acceptance".to_string(),
        concurrent_burst.isolation_probe.operator_query_succeeded
            && concurrent_burst
                .isolation_probe
                .operator_query_latency_millis
                <= BURST_OPERATOR_QUERY_BUDGET_MILLIS
            && concurrent_burst.isolation_probe.gateway_health_successes == 2
            && concurrent_burst
                .isolation_probe
                .gateway_health_max_latency_millis
                <= BURST_GATEWAY_HEALTH_BUDGET_MILLIS,
    );
    let consensus_before_state = peak
        .consensus
        .as_ref()
        .ok_or_else(|| "proof_consensus_snapshot_missing".to_string())?;
    let desired_entry = consensus_before_state
        .entries
        .iter()
        .rev()
        .find(|entry| matches!(&entry.mutation, ControlMutation::DesiredCapacity(value) if value.worker_nodes == desired))
        .ok_or_else(|| "proof_runtime_desired_commit_missing".to_string())?;
    let committed_index = desired_entry.index;
    let consensus_before = wait_for_consensus_applied(
        "controller1@127.0.0.1:14371",
        committed_index,
        Duration::from_secs(30),
    )?;
    harness.write(
        "consensus-before-failover.json",
        serde_json::to_vec_pretty(&consensus_before)
            .expect("serialize pre-failover consensus snapshot"),
    )?;
    let consensus_before_state = consensus_before
        .consensus
        .as_ref()
        .ok_or_else(|| "proof_consensus_snapshot_missing".to_string())?;
    harness.assertions.insert(
        "embedded_consensus_three_voters".to_string(),
        consensus_before_state.voter_ids.len() == 3,
    );
    harness.assertions.insert(
        "embedded_consensus_majority_commit".to_string(),
        consensus_before_state.current_term > 0
            && consensus_before_state
                .last_applied_log
                .is_some_and(|index| index >= committed_index)
            && consensus_before_state.entries.iter().any(|entry| {
                matches!(
                    &entry.mutation,
                    ControlMutation::DesiredCapacity(value) if value.worker_nodes == desired
                )
            }),
    );
    let first_consensus_term = consensus_before_state.current_term;
    harness.snapshot_containers("peak")?;
    let managed_peak = harness.inspect_managed_containers("peak")?;
    // Inspect first, then wait for the corresponding operation results to be
    // committed. The synchronized burst can trigger another scale revision;
    // comparing those later containers with the earlier readiness snapshot is
    // an evidence race rather than a metadata violation.
    let operations = wait_for_committed_managed_operations(
        &controller_target,
        &managed_peak,
        &harness.cluster_id,
        Duration::from_secs(30),
    )?;
    harness.write(
        "driver-scale-up.json",
        serde_json::to_vec_pretty(&operations).expect("serialize driver operations"),
    )?;
    harness.assertions.insert(
        "policy_initiated_scale_up".to_string(),
        desired > MIN_WORKERS,
    );
    harness.assertions.insert(
        "runtime_owned_autonomous_loop".to_string(),
        peak.autonomous.configured
            && peak.autonomous.running
            && peak.autonomous.leader
            && peak.autonomous.last_error.is_none(),
    );
    harness.assertions.insert(
        "operation_ids_unique".to_string(),
        operations
            .iter()
            .map(|operation| &operation.operation_id)
            .collect::<BTreeSet<_>>()
            .len()
            == operations.len(),
    );
    harness.assertions.insert(
        "managed_labels_match_committed_operations".to_string(),
        managed_labels_match_operations(&managed_peak, &operations, &harness.cluster_id),
    );
    harness.assertions.insert(
        "provider_create_idempotent_by_operation_id".to_string(),
        managed_operation_labels_unique(&managed_peak),
    );

    let dynamic_workers: BTreeSet<String> = peak
        .nodes
        .iter()
        .filter(|node| {
            node.roles.iter().any(|role| role == "worker")
                && node.routing_eligible
                && !node.node_id.starts_with("worker1@")
                && !node.node_id.starts_with("worker2@")
        })
        .map(|node| node.node_id.clone())
        .collect();
    let peak_continuity = wait_for_routing_evidence(
        &controller_target,
        &dynamic_workers,
        Duration::from_secs(30),
    )?;
    let peak_routing_counts = pressure_routing_counts(&peak_continuity);
    harness.write(
        "continuity-peak.json",
        serde_json::to_vec_pretty(&continuity_list_json(&peak_continuity))
            .expect("serialize peak continuity"),
    )?;
    harness.write(
        "routing-counts-peak.json",
        serde_json::to_vec_pretty(&peak_routing_counts).expect("serialize peak routing counts"),
    )?;
    harness.assertions.insert(
        "managed_workers_received_traffic_after_ready".to_string(),
        !dynamic_workers.is_empty()
            && dynamic_workers
                .iter()
                .any(|node| peak_routing_counts.get(node).copied().unwrap_or(0) > 0),
    );
    let constrained_count = peak_routing_counts
        .get("worker1@worker1:4370")
        .copied()
        .unwrap_or(0);
    let larger_worker_count = peak_routing_counts
        .get("worker2@worker2:4370")
        .copied()
        .unwrap_or(0);
    harness.assertions.insert(
        "adaptive_routing_favors_larger_capacity_worker".to_string(),
        larger_worker_count > constrained_count,
    );

    let _ = harness.compose(&["restart", "docker-driver"])?;
    wait_for_driver_recovery(&controller_target, Duration::from_secs(45))?;
    harness
        .assertions
        .insert("docker_driver_restart_recovered".to_string(), true);
    harness.events.push(json!({
        "phase": "driver_restart",
        "result": "controller_reconciled_after_authenticated_driver_restart",
    }));

    let _ = harness.compose(&["kill", "worker1"])?;
    wait_for_managed_count(
        harness,
        peak_managed.saturating_add(1),
        Duration::from_secs(90),
    )?;
    harness
        .assertions
        .insert("killed_worker_replaced".to_string(), true);

    let _ = harness.compose(&["kill", "controller1"])?;
    let (failover_target, failover_runtime) = wait_for_autonomous_leader(
        &["controller2@127.0.0.1:14372", "controller3@127.0.0.1:14373"],
        Duration::from_secs(60),
    )?;
    controller_target = failover_target;
    let failover_consensus = failover_runtime
        .consensus
        .as_ref()
        .ok_or_else(|| "proof_failover_consensus_missing".to_string())?;
    let failover_index = failover_consensus.last_applied_log.unwrap_or(0);
    let consensus_after_controller2 = wait_for_consensus_applied(
        "controller2@127.0.0.1:14372",
        failover_index,
        Duration::from_secs(30),
    )?;
    let consensus_after_controller3 = wait_for_consensus_applied(
        "controller3@127.0.0.1:14373",
        failover_index,
        Duration::from_secs(30),
    )?;
    harness.write(
        "consensus-after-failover.json",
        serde_json::to_vec_pretty(&json!({
            "controller2": &consensus_after_controller2,
            "controller3": &consensus_after_controller3,
        }))
        .expect("serialize post-failover consensus snapshots"),
    )?;
    harness.assertions.insert(
        "embedded_consensus_failover_advanced_term".to_string(),
        failover_consensus.current_term > first_consensus_term,
    );
    harness.assertions.insert(
        "embedded_consensus_survivors_applied_commit".to_string(),
        [&consensus_after_controller2, &consensus_after_controller3]
            .into_iter()
            .all(|snapshot| {
                snapshot.consensus.as_ref().is_some_and(|consensus| {
                    consensus.voter_ids.len() == 3
                        && consensus
                            .last_applied_log
                            .is_some_and(|index| index >= failover_index)
                })
            }),
    );
    let managed_before_failover_settle = managed_running_count(harness)?;
    thread::park_timeout(Duration::from_secs(2));
    let managed_after_failover_settle = managed_running_count(harness)?;
    harness.assertions.insert(
        "controller_failover_created_no_duplicate_capacity".to_string(),
        managed_after_failover_settle == managed_before_failover_settle,
    );

    let load_summary = load.finish()?;
    harness.write(
        "load-summary.json",
        serde_json::to_vec_pretty(&load_summary).expect("serialize load summary"),
    )?;
    harness.assertions.insert(
        "cross_ingress_request_ids_unique".to_string(),
        load_summary.successes > 0
            && load_summary.unique_request_ids == load_summary.successes as usize
            && load_summary.duplicate_request_ids == 0,
    );
    harness.assertions.insert(
        "both_ingress_gateways_served_requests".to_string(),
        load_summary.gateway_18081 > 0 && load_summary.gateway_18082 > 0,
    );
    harness.assertions.insert(
        "load_error_rate_within_declared_budget".to_string(),
        load_summary.requests > 0
            && load_summary.failures.saturating_mul(100)
                <= load_summary.requests.saturating_mul(10),
    );
    harness.assertions.insert(
        "load_p99_latency_within_release_budget".to_string(),
        load_summary.successes > 0
            && load_summary.latency_p99_millis <= FAILURE_LOAD_P99_BUDGET_MILLIS,
    );
    let service_after_failover = wait_for_http(18081, "/proof/pressure", Duration::from_secs(30))
        .is_ok()
        && wait_for_http(18082, "/proof/pressure", Duration::from_secs(30)).is_ok();
    harness.assertions.insert(
        "controller_failover_preserved_service".to_string(),
        service_after_failover,
    );

    let (final_snapshot, drain_snapshot, drain_load) = match wait_for_runtime_scale_down(
        &controller_target,
        MIN_WORKERS,
        RUNTIME_SCALE_DOWN_PROOF_TIMEOUT,
    ) {
        Ok(result) => result,
        Err(error) => {
            if let Ok(snapshot) =
                query_operator_runtime_remote(&controller_target, COOKIE, Duration::from_secs(3))
            {
                harness.write(
                    "capacity-scale-down-timeout.json",
                    serde_json::to_vec_pretty(&snapshot)
                        .expect("serialize scale-down timeout snapshot"),
                )?;
            }
            return Err(error);
        }
    };
    harness.events.push(json!({
        "phase": "scale_down",
        "decision": final_snapshot.autonomous.last_decision,
        "drain_load_successes": drain_load.successes,
    }));
    if let Some(snapshot) = &drain_snapshot {
        harness.write(
            "capacity-draining.json",
            serde_json::to_vec_pretty(snapshot).expect("serialize drain snapshot"),
        )?;
    }
    let expected_final_managed = MIN_WORKERS.saturating_sub(1) as usize;
    wait_for_managed_exact(harness, expected_final_managed, Duration::from_secs(60))?;
    let final_consensus = final_snapshot
        .consensus
        .as_ref()
        .ok_or_else(|| "proof_final_consensus_missing".to_string())?;
    let drain_intents = final_consensus
        .entries
        .iter()
        .filter(|entry| {
            matches!(
                entry.mutation,
                ControlMutation::DrainIntent {
                    cancelled: false,
                    ..
                }
            )
        })
        .count();
    harness
        .assertions
        .insert("policy_initiated_scale_down".to_string(), drain_intents > 0);
    harness.assertions.insert(
        "draining_nodes_became_routing_ineligible".to_string(),
        drain_snapshot.as_ref().is_some_and(|snapshot| {
            snapshot
                .nodes
                .iter()
                .any(|node| node.state == "draining" && !node.routing_eligible)
        }),
    );
    harness.assertions.insert(
        "continuity_present_during_drain".to_string(),
        drain_load.successes > 0,
    );
    harness.write(
        "drain-load-summary.json",
        serde_json::to_vec_pretty(&drain_load).expect("serialize drain load summary"),
    )?;
    let final_continuity = query_operator_continuity_list_remote(
        &controller_target,
        COOKIE,
        Some(2_000),
        Duration::from_secs(5),
    )
    .map_err(|error| format!("proof_final_continuity_query_failed:{error}"))?;
    harness.write(
        "continuity-final.json",
        serde_json::to_vec_pretty(&continuity_list_json(&final_continuity))
            .expect("serialize final continuity"),
    )?;
    let draining_nodes: BTreeSet<String> = drain_snapshot
        .as_ref()
        .into_iter()
        .flat_map(|snapshot| snapshot.nodes.iter())
        .filter(|node| node.state == "draining")
        .map(|node| node.node_id.clone())
        .collect();
    let drain_records: Vec<_> = drain_load
        .request_keys
        .iter()
        .filter_map(|request_key| {
            final_continuity
                .records
                .iter()
                .find(|record| &record.request_key == request_key)
        })
        .collect();
    harness.assertions.insert(
        "draining_nodes_received_no_new_assignments".to_string(),
        !draining_nodes.is_empty()
            && drain_records.len() == drain_load.request_keys.len()
            && drain_records.iter().all(|record| {
                !draining_nodes.contains(&record.owner_node)
                    && !draining_nodes.contains(&record.execution_node)
                    && record
                        .replica_nodes()
                        .iter()
                        .all(|node| !draining_nodes.contains(node))
            }),
    );
    harness.snapshot_containers("final")?;

    let database_count = postgres_todo_count(harness)?;
    harness.write(
        "database-integrity.json",
        serde_json::to_vec_pretty(&json!({
            "acknowledged_mutations": acknowledged_mutations,
            "database_rows": database_count,
        }))
        .expect("serialize database integrity"),
    )?;
    harness.assertions.insert(
        "database_matches_acknowledged_mutations".to_string(),
        database_count == acknowledged_mutations,
    );
    harness.assertions.insert(
        "returned_to_minimum_without_oscillation".to_string(),
        latest_desired_workers(&final_snapshot) == Some(MIN_WORKERS),
    );
    harness.assertions.insert(
        "failed_managed_worker_orphan_reconciled".to_string(),
        final_consensus.entries.iter().any(|entry| {
            entry.reason == "record failed managed worker cleanup result"
                && matches!(
                    &entry.mutation,
                    ControlMutation::DriverOperation(operation)
                        if operation.state == mesh_rt::DriverOperationState::Succeeded
                )
        }),
    );
    Ok(())
}

#[derive(serde::Serialize)]
struct LoadSummary {
    requests: u64,
    successes: u64,
    failures: u64,
    failure_reasons: BTreeMap<String, u64>,
    unique_request_ids: usize,
    duplicate_request_ids: usize,
    gateway_18081: u64,
    gateway_18082: u64,
    latency_p50_millis: u64,
    latency_p95_millis: u64,
    latency_p99_millis: u64,
    latency_max_millis: u64,
}

#[derive(serde::Serialize)]
struct ConcurrentBurstSummary {
    requests: usize,
    successes: usize,
    failures: usize,
    unique_request_keys: usize,
    remote_executions: usize,
    execution_nodes: BTreeMap<String, usize>,
    failure_reasons: BTreeMap<String, usize>,
    latency_p50_millis: u64,
    latency_p95_millis: u64,
    latency_p99_millis: u64,
    latency_max_millis: u64,
    isolation_probe: BurstIsolationProbe,
}

#[derive(serde::Serialize)]
struct BurstIsolationProbe {
    operator_query_succeeded: bool,
    operator_query_latency_millis: u64,
    gateway_health_successes: usize,
    gateway_health_max_latency_millis: u64,
    failure_reasons: Vec<String>,
}

fn run_concurrent_remote_burst(
    controller_target: &str,
    requests: usize,
) -> Result<ConcurrentBurstSummary, String> {
    let start_gate = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
    let (sender, receiver) = std::sync::mpsc::channel();
    let mut workers = Vec::with_capacity(requests);
    for index in 0..requests {
        let worker_start_gate = Arc::clone(&start_gate);
        let sender = sender.clone();
        let worker = thread::Builder::new()
            .name(format!("mesh-proof-concurrent-{index}"))
            .stack_size(256 * 1024)
            .spawn(move || {
                let port = if index % 2 == 0 { 18081 } else { 18082 };
                let (started, signal) = &*worker_start_gate;
                let mut allowed = started.lock().unwrap();
                while !*allowed {
                    allowed = signal.wait(allowed).unwrap();
                }
                drop(allowed);
                let started = Instant::now();
                let result = http_request(port, "GET", "/proof/pressure", "", &[]);
                let elapsed: u64 = started.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
                let _ = sender.send((result, elapsed));
            });
        match worker {
            Ok(worker) => workers.push(worker),
            Err(error) => {
                let (started, signal) = &*start_gate;
                *started.lock().unwrap() = true;
                signal.notify_all();
                for worker in workers {
                    let _ = worker.join();
                }
                return Err(format!("proof_concurrent_thread_start_failed:{error}"));
            }
        }
    }
    drop(sender);
    let probe_gate = Arc::clone(&start_gate);
    let controller_target = controller_target.to_string();
    let isolation_probe = thread::Builder::new()
        .name("mesh-proof-burst-isolation".to_string())
        .spawn(move || {
            let (started, signal) = &*probe_gate;
            let mut allowed = started.lock().unwrap();
            while !*allowed {
                allowed = signal.wait(allowed).unwrap();
            }
            drop(allowed);

            let mut failure_reasons = Vec::new();
            let operator_started = Instant::now();
            let operator_query_succeeded = match query_operator_runtime_remote(
                &controller_target,
                COOKIE,
                Duration::from_secs(3),
            ) {
                Ok(snapshot) if !snapshot.nodes.is_empty() => true,
                Ok(_) => {
                    failure_reasons.push("operator_query_returned_no_nodes".to_string());
                    false
                }
                Err(error) => {
                    failure_reasons.push(format!("operator_query_failed:{error}"));
                    false
                }
            };
            let operator_query_latency_millis = operator_started
                .elapsed()
                .as_millis()
                .try_into()
                .unwrap_or(u64::MAX);

            let mut gateway_health_successes = 0;
            let mut gateway_health_max_latency_millis = 0;
            for port in [18081, 18082] {
                let health_started = Instant::now();
                match http_request(port, "GET", "/health", "", &[]) {
                    Ok(response) if response.status == 200 => gateway_health_successes += 1,
                    Ok(response) => failure_reasons.push(format!(
                        "gateway_{port}_health_status_{}:{}",
                        response.status, response.body
                    )),
                    Err(error) => {
                        failure_reasons.push(format!("gateway_{port}_health_failed:{error}"));
                    }
                }
                gateway_health_max_latency_millis = gateway_health_max_latency_millis.max(
                    health_started
                        .elapsed()
                        .as_millis()
                        .try_into()
                        .unwrap_or(u64::MAX),
                );
            }
            BurstIsolationProbe {
                operator_query_succeeded,
                operator_query_latency_millis,
                gateway_health_successes,
                gateway_health_max_latency_millis,
                failure_reasons,
            }
        });
    let isolation_probe = match isolation_probe {
        Ok(probe) => probe,
        Err(error) => {
            let (started, signal) = &*start_gate;
            *started.lock().unwrap() = true;
            signal.notify_all();
            for worker in workers {
                let _ = worker.join();
            }
            return Err(format!("proof_isolation_probe_thread_start_failed:{error}"));
        }
    };
    let (started, signal) = &*start_gate;
    *started.lock().unwrap() = true;
    signal.notify_all();
    let mut keys = BTreeSet::new();
    let mut successes = 0;
    let mut remote_executions = 0;
    let mut execution_nodes = BTreeMap::new();
    let mut failure_reasons = BTreeMap::new();
    let mut latencies = Vec::with_capacity(requests);
    for _ in 0..requests {
        let (result, elapsed) = receiver
            .recv_timeout(Duration::from_secs(30))
            .map_err(|error| format!("proof_concurrent_result_timeout:{error}"))?;
        match result {
            Ok(response) if response.status == 200 => {
                successes += 1;
                latencies.push(elapsed);
                if let Some(key) = response.header("x-mesh-continuity-request-key") {
                    keys.insert(key.to_string());
                }
                if response.header("x-mesh-routed-remotely") == Some("true") {
                    remote_executions += 1;
                }
                if let Some(node) = response.header("x-mesh-execution-node") {
                    *execution_nodes.entry(node.to_string()).or_default() += 1;
                }
            }
            Ok(response) => {
                *failure_reasons
                    .entry(format!("http_status_{}:{}", response.status, response.body))
                    .or_default() += 1;
            }
            Err(error) => {
                *failure_reasons.entry(error).or_default() += 1;
            }
        }
    }
    for worker in workers {
        worker
            .join()
            .map_err(|_| "proof_concurrent_thread_panicked".to_string())?;
    }
    let isolation_probe = isolation_probe
        .join()
        .map_err(|_| "proof_isolation_probe_thread_panicked".to_string())?;
    latencies.sort_unstable();
    Ok(ConcurrentBurstSummary {
        requests,
        successes,
        failures: requests.saturating_sub(successes),
        unique_request_keys: keys.len(),
        remote_executions,
        execution_nodes,
        failure_reasons,
        latency_p50_millis: percentile_millis(&latencies, 0.50),
        latency_p95_millis: percentile_millis(&latencies, 0.95),
        latency_p99_millis: percentile_millis(&latencies, 0.99),
        latency_max_millis: latencies.last().copied().unwrap_or(0),
        isolation_probe,
    })
}

fn percentile_millis(samples: &[u64], percentile: f64) -> u64 {
    if samples.is_empty() {
        return 0;
    }
    let index = ((samples.len() - 1) as f64 * percentile.clamp(0.0, 1.0)).ceil() as usize;
    samples[index]
}

struct RunningLoad {
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<LoadSummary>>,
}

impl RunningLoad {
    fn finish(mut self) -> Result<LoadSummary, String> {
        self.stop.store(true, Ordering::Relaxed);
        self.handle
            .take()
            .expect("running load handle")
            .join()
            .map_err(|_| "proof_load_thread_panicked".to_string())
    }
}

impl Drop for RunningLoad {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn start_load() -> RunningLoad {
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);
    let handle = thread::spawn(move || {
        let requests = Arc::new(AtomicU64::new(0));
        let successes = Arc::new(AtomicU64::new(0));
        let failures = Arc::new(AtomicU64::new(0));
        let gateway_a = Arc::new(AtomicU64::new(0));
        let gateway_b = Arc::new(AtomicU64::new(0));
        let keys = Arc::new(Mutex::new(Vec::new()));
        let latencies = Arc::new(Mutex::new(Vec::new()));
        let failure_reasons = Arc::new(Mutex::new(BTreeMap::<String, u64>::new()));
        let mut workers = Vec::new();
        for index in 0..8 {
            let stop = Arc::clone(&thread_stop);
            let requests = Arc::clone(&requests);
            let successes = Arc::clone(&successes);
            let failures = Arc::clone(&failures);
            let gateway_a = Arc::clone(&gateway_a);
            let gateway_b = Arc::clone(&gateway_b);
            let keys = Arc::clone(&keys);
            let latencies = Arc::clone(&latencies);
            let failure_reasons = Arc::clone(&failure_reasons);
            workers.push(thread::spawn(move || {
                let port = if index % 2 == 0 { 18081 } else { 18082 };
                while !stop.load(Ordering::Relaxed) {
                    requests.fetch_add(1, Ordering::Relaxed);
                    let started = Instant::now();
                    match http_request(port, "GET", "/proof/pressure", "", &[]) {
                        Ok(response) if response.status == 200 => {
                            latencies
                                .lock()
                                .unwrap()
                                .push(started.elapsed().as_millis().try_into().unwrap_or(u64::MAX));
                            successes.fetch_add(1, Ordering::Relaxed);
                            if port == 18081 {
                                gateway_a.fetch_add(1, Ordering::Relaxed);
                            } else {
                                gateway_b.fetch_add(1, Ordering::Relaxed);
                            }
                            if let Some(key) = response.header("x-mesh-continuity-request-key") {
                                keys.lock().unwrap().push(key.to_string());
                            }
                        }
                        Ok(response) => {
                            failures.fetch_add(1, Ordering::Relaxed);
                            let body: String = response.body.trim().chars().take(240).collect();
                            let reason = format!("http_status_{}:{body}", response.status);
                            *failure_reasons.lock().unwrap().entry(reason).or_default() += 1;
                            thread::park_timeout(Duration::from_millis(25));
                        }
                        Err(error) => {
                            failures.fetch_add(1, Ordering::Relaxed);
                            *failure_reasons
                                .lock()
                                .unwrap()
                                .entry(format!("transport:{error}"))
                                .or_default() += 1;
                            thread::park_timeout(Duration::from_millis(25));
                        }
                    }
                }
            }));
        }
        for worker in workers {
            let _ = worker.join();
        }
        let keys = keys.lock().unwrap();
        let unique: BTreeSet<_> = keys.iter().collect();
        let failure_reasons = failure_reasons.lock().unwrap().clone();
        let mut latencies = latencies.lock().unwrap().clone();
        latencies.sort_unstable();
        LoadSummary {
            requests: requests.load(Ordering::Relaxed),
            successes: successes.load(Ordering::Relaxed),
            failures: failures.load(Ordering::Relaxed),
            failure_reasons,
            unique_request_ids: unique.len(),
            duplicate_request_ids: keys.len().saturating_sub(unique.len()),
            gateway_18081: gateway_a.load(Ordering::Relaxed),
            gateway_18082: gateway_b.load(Ordering::Relaxed),
            latency_p50_millis: percentile_millis(&latencies, 0.50),
            latency_p95_millis: percentile_millis(&latencies, 0.95),
            latency_p99_millis: percentile_millis(&latencies, 0.99),
            latency_max_millis: latencies.last().copied().unwrap_or(0),
        }
    });
    RunningLoad {
        stop,
        handle: Some(handle),
    }
}

fn generate_proof_mtls(
    directory: &std::path::Path,
) -> Result<(String, String, String, String, String), String> {
    fs::create_dir_all(directory)
        .map_err(|error| format!("proof_mtls_directory_failed:{error}"))?;
    let command = |arguments: &[String]| -> Result<(), String> {
        let output = Command::new("openssl")
            .args(arguments)
            .stdin(Stdio::null())
            .output()
            .map_err(|error| format!("proof_openssl_start_failed:{error}"))?;
        if output.status.success() {
            Ok(())
        } else {
            Err(format!(
                "proof_openssl_failed:{}",
                String::from_utf8_lossy(&output.stderr)
            ))
        }
    };
    let ca_key = directory.join("ca.key.pem");
    let ca_pem = directory.join("ca.cert.pem");
    let node_key = directory.join("node.key.pem");
    let node_csr = directory.join("node.csr.pem");
    let node_pem = directory.join("node.cert.pem");
    let ca_der = directory.join("ca.cert.der");
    let node_der = directory.join("node.cert.der");
    let node_key_der = directory.join("node.key.der");
    let driver_key = directory.join("driver.key.pem");
    let driver_csr = directory.join("driver.csr.pem");
    let driver_pem = directory.join("driver.cert.pem");
    let driver_der = directory.join("driver.cert.der");
    let driver_key_der = directory.join("driver.key.der");
    let path = |path: &std::path::Path| path.to_string_lossy().into_owned();
    command(&[
        "req".into(),
        "-x509".into(),
        "-newkey".into(),
        "rsa:2048".into(),
        "-nodes".into(),
        "-sha256".into(),
        "-days".into(),
        "1".into(),
        "-subj".into(),
        "/CN=mesh-proof-ca".into(),
        "-keyout".into(),
        path(&ca_key),
        "-out".into(),
        path(&ca_pem),
    ])?;
    command(&[
        "req".into(),
        "-newkey".into(),
        "rsa:2048".into(),
        "-nodes".into(),
        "-sha256".into(),
        "-subj".into(),
        "/CN=mesh-node".into(),
        "-addext".into(),
        "subjectAltName=DNS:mesh-node".into(),
        "-keyout".into(),
        path(&node_key),
        "-out".into(),
        path(&node_csr),
    ])?;
    command(&[
        "x509".into(),
        "-req".into(),
        "-sha256".into(),
        "-days".into(),
        "1".into(),
        "-in".into(),
        path(&node_csr),
        "-CA".into(),
        path(&ca_pem),
        "-CAkey".into(),
        path(&ca_key),
        "-CAcreateserial".into(),
        "-copy_extensions".into(),
        "copy".into(),
        "-out".into(),
        path(&node_pem),
    ])?;
    command(&[
        "req".into(),
        "-newkey".into(),
        "rsa:2048".into(),
        "-nodes".into(),
        "-sha256".into(),
        "-subj".into(),
        "/CN=docker-driver".into(),
        "-addext".into(),
        "subjectAltName=DNS:docker-driver".into(),
        "-addext".into(),
        "extendedKeyUsage=serverAuth".into(),
        "-keyout".into(),
        path(&driver_key),
        "-out".into(),
        path(&driver_csr),
    ])?;
    command(&[
        "x509".into(),
        "-req".into(),
        "-sha256".into(),
        "-days".into(),
        "1".into(),
        "-in".into(),
        path(&driver_csr),
        "-CA".into(),
        path(&ca_pem),
        "-CAkey".into(),
        path(&ca_key),
        "-CAcreateserial".into(),
        "-copy_extensions".into(),
        "copy".into(),
        "-out".into(),
        path(&driver_pem),
    ])?;
    command(&[
        "x509".into(),
        "-in".into(),
        path(&ca_pem),
        "-outform".into(),
        "DER".into(),
        "-out".into(),
        path(&ca_der),
    ])?;
    command(&[
        "x509".into(),
        "-in".into(),
        path(&node_pem),
        "-outform".into(),
        "DER".into(),
        "-out".into(),
        path(&node_der),
    ])?;
    command(&[
        "pkcs8".into(),
        "-topk8".into(),
        "-nocrypt".into(),
        "-in".into(),
        path(&node_key),
        "-outform".into(),
        "DER".into(),
        "-out".into(),
        path(&node_key_der),
    ])?;
    command(&[
        "x509".into(),
        "-in".into(),
        path(&driver_pem),
        "-outform".into(),
        "DER".into(),
        "-out".into(),
        path(&driver_der),
    ])?;
    command(&[
        "pkcs8".into(),
        "-topk8".into(),
        "-nocrypt".into(),
        "-in".into(),
        path(&driver_key),
        "-outform".into(),
        "DER".into(),
        "-out".into(),
        path(&driver_key_der),
    ])?;
    let encode = |path: &std::path::Path| -> Result<String, String> {
        fs::read(path)
            .map(|bytes| base64::engine::general_purpose::STANDARD.encode(bytes))
            .map_err(|error| format!("proof_mtls_read_failed:{}:{error}", path.display()))
    };
    Ok((
        encode(&ca_der)?,
        encode(&node_der)?,
        encode(&node_key_der)?,
        encode(&driver_der)?,
        encode(&driver_key_der)?,
    ))
}

fn wait_for_runtime(
    target: &str,
    minimum_nodes: usize,
    timeout: Duration,
) -> Result<OperatorRuntimeSnapshot, String> {
    let deadline = Instant::now() + timeout;
    let mut last_error = "no observation".to_string();
    while Instant::now() < deadline {
        match query_operator_runtime_remote(target, COOKIE, Duration::from_secs(3)) {
            Ok(snapshot)
                if snapshot
                    .nodes
                    .iter()
                    .filter(|node| {
                        node.roles.iter().any(|role| role == "worker") && node.routing_eligible
                    })
                    .count()
                    >= minimum_nodes
                    && snapshot.telemetry_complete =>
            {
                return Ok(snapshot);
            }
            Ok(snapshot) => {
                let workers = snapshot
                    .nodes
                    .iter()
                    .filter(|node| node.roles.iter().any(|role| role == "worker"))
                    .count();
                last_error = format!(
                    "observed_nodes={} observed_workers={workers} telemetry_complete={}",
                    snapshot.nodes.len(),
                    snapshot.telemetry_complete
                );
            }
            Err(error) => last_error = error.to_string(),
        }
        thread::park_timeout(Duration::from_millis(250));
    }
    Err(format!("proof_runtime_readiness_timeout:{last_error}"))
}

fn wait_for_stable_runtime(
    target: &str,
    timeout: Duration,
) -> Result<(OperatorRuntimeSnapshot, u16), String> {
    const STABLE_FOR: Duration = Duration::from_secs(2);

    let deadline = Instant::now() + timeout;
    let mut stable_since: Option<(Instant, u16)> = None;
    let mut last = "no runtime snapshot".to_string();
    while Instant::now() < deadline {
        match query_operator_runtime_remote(target, COOKIE, Duration::from_secs(3)) {
            Ok(snapshot) => {
                let workers: Vec<_> = snapshot
                    .nodes
                    .iter()
                    .filter(|node| node.roles.iter().any(|role| role == "worker"))
                    .collect();
                let desired = latest_desired_workers(&snapshot);
                let stable = desired.is_some_and(|desired| desired > MIN_WORKERS)
                    && snapshot.telemetry_complete
                    && workers.len() == usize::from(desired.unwrap_or_default())
                    && workers.iter().all(|node| node.routing_eligible)
                    && snapshot.draining_capacity == 0
                    && snapshot
                        .autonomous
                        .last_reconcile
                        .as_ref()
                        .is_some_and(|reconcile| {
                            Some(reconcile.desired_workers) == desired
                                && reconcile.observed_workers == reconcile.desired_workers
                                && reconcile.drains.is_empty()
                                && reconcile.ensured.is_empty()
                                && reconcile.constraints.is_empty()
                        });
                if stable {
                    let desired = desired.expect("stable desired capacity");
                    match stable_since {
                        Some((since, stable_desired)) if stable_desired == desired => {
                            if since.elapsed() >= STABLE_FOR {
                                return Ok((snapshot, desired));
                            }
                        }
                        _ => stable_since = Some((Instant::now(), desired)),
                    }
                } else {
                    stable_since = None;
                    last = format!(
                        "desired={desired:?}:workers={}:ready={}:states={:?}:draining={}:telemetry_complete={}:last_error={:?}:reconcile={:?}",
                        workers.len(),
                        workers.iter().filter(|node| node.routing_eligible).count(),
                        workers
                            .iter()
                            .map(|node| (&node.node_id, &node.state, node.routing_eligible))
                            .collect::<Vec<_>>(),
                        snapshot.draining_capacity,
                        snapshot.telemetry_complete,
                        snapshot.autonomous.last_error,
                        snapshot.autonomous.last_reconcile
                    );
                }
            }
            Err(error) => {
                stable_since = None;
                last = error.to_string();
            }
        }
        thread::park_timeout(Duration::from_millis(100));
    }
    Err(format!("proof_runtime_stability_timeout:{last}"))
}

fn wait_for_consensus_applied(
    target: &str,
    minimum_log_index: u64,
    timeout: Duration,
) -> Result<OperatorRuntimeSnapshot, String> {
    let deadline = Instant::now() + timeout;
    let mut last = "consensus snapshot unavailable".to_string();
    while Instant::now() < deadline {
        match query_operator_runtime_remote(target, COOKIE, Duration::from_secs(3)) {
            Ok(snapshot)
                if snapshot.consensus.as_ref().is_some_and(|consensus| {
                    consensus.voter_ids.len() == 3
                        && consensus
                            .last_applied_log
                            .is_some_and(|index| index >= minimum_log_index)
                }) =>
            {
                return Ok(snapshot);
            }
            Ok(snapshot) => {
                last = format!("consensus={:?}", snapshot.consensus);
            }
            Err(error) => last = error.to_string(),
        }
        thread::park_timeout(Duration::from_millis(200));
    }
    Err(format!("proof_consensus_apply_timeout:{last}"))
}

fn wait_for_managed_count(
    harness: &ProofHarness,
    expected: usize,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let ids = harness.checked(
            "docker",
            &[
                "ps",
                "-q",
                "--filter",
                &format!("label=mesh.cluster={}", harness.cluster_id),
                "--filter",
                "label=mesh.managed=true",
            ],
        )?;
        if ids.lines().filter(|line| !line.is_empty()).count() >= expected {
            return Ok(());
        }
        thread::park_timeout(Duration::from_millis(250));
    }
    Err("proof_managed_worker_readiness_timeout".to_string())
}

fn managed_running_count(harness: &ProofHarness) -> Result<usize, String> {
    let ids = harness.checked(
        "docker",
        &[
            "ps",
            "-q",
            "--filter",
            &format!("label=mesh.cluster={}", harness.cluster_id),
            "--filter",
            "label=mesh.managed=true",
        ],
    )?;
    Ok(ids.lines().filter(|line| !line.is_empty()).count())
}

fn wait_for_managed_exact(
    harness: &ProofHarness,
    expected: usize,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    let mut last = usize::MAX;
    while Instant::now() < deadline {
        last = managed_running_count(harness)?;
        if last == expected {
            return Ok(());
        }
        thread::park_timeout(Duration::from_millis(200));
    }
    Err(format!(
        "proof_managed_worker_exact_count_timeout:expected={expected}:observed={last}"
    ))
}

fn latest_desired_workers(snapshot: &OperatorRuntimeSnapshot) -> Option<u16> {
    snapshot
        .consensus
        .as_ref()?
        .entries
        .iter()
        .rev()
        .find_map(|entry| match &entry.mutation {
            ControlMutation::DesiredCapacity(desired) => Some(desired.worker_nodes),
            ControlMutation::ManualOverride { worker_nodes } => Some(*worker_nodes),
            _ => None,
        })
}

fn wait_for_runtime_desired(
    target: &str,
    predicate: impl Fn(u16) -> bool,
    timeout: Duration,
) -> Result<(OperatorRuntimeSnapshot, u16), String> {
    let deadline = Instant::now() + timeout;
    let mut last = "no runtime snapshot".to_string();
    while Instant::now() < deadline {
        match query_operator_runtime_remote(target, COOKIE, Duration::from_secs(3)) {
            Ok(snapshot) => {
                if let Some(desired) = latest_desired_workers(&snapshot) {
                    if predicate(desired) {
                        return Ok((snapshot, desired));
                    }
                    last = format!("desired={desired}:autonomous={:?}", snapshot.autonomous);
                } else {
                    last = format!("desired_missing:autonomous={:?}", snapshot.autonomous);
                }
            }
            Err(error) => last = error.to_string(),
        }
        thread::park_timeout(Duration::from_millis(150));
    }
    Err(format!("proof_runtime_desired_timeout:{last}"))
}

fn successful_driver_operations(
    snapshot: &OperatorRuntimeSnapshot,
) -> Vec<mesh_rt::DriverOperation> {
    let mut operations = BTreeMap::new();
    if let Some(consensus) = &snapshot.consensus {
        for entry in &consensus.entries {
            if let ControlMutation::DriverOperation(operation) = &entry.mutation {
                if operation.state == mesh_rt::DriverOperationState::Succeeded {
                    operations.insert(operation.operation_id.clone(), operation.clone());
                }
            }
        }
    }
    operations.into_values().collect()
}

fn wait_for_committed_managed_operations(
    target: &str,
    inspection: &Value,
    cluster_id: &str,
    timeout: Duration,
) -> Result<Vec<mesh_rt::DriverOperation>, String> {
    let deadline = Instant::now() + timeout;
    let mut last = "operator runtime unavailable".to_string();
    while Instant::now() < deadline {
        match query_operator_runtime_remote(target, COOKIE, Duration::from_secs(3)) {
            Ok(snapshot) => {
                let operations = successful_driver_operations(&snapshot);
                if managed_labels_match_operations(inspection, &operations, cluster_id) {
                    return Ok(operations);
                }
                last = format!(
                    "managed_labels_not_yet_committed:successful_operations={}",
                    operations.len()
                );
            }
            Err(error) => last = error.to_string(),
        }
        thread::park_timeout(Duration::from_millis(100));
    }
    Err(format!("proof_managed_operation_commit_timeout:{last}"))
}

fn managed_labels_match_operations(
    inspection: &Value,
    operations: &[mesh_rt::DriverOperation],
    cluster_id: &str,
) -> bool {
    let operations: BTreeMap<_, _> = operations
        .iter()
        .map(|operation| (operation.operation_id.as_str(), operation))
        .collect();
    let Some(containers) = inspection.as_array() else {
        return false;
    };
    !containers.is_empty()
        && containers.iter().all(|container| {
            let labels = &container["Config"]["Labels"];
            let Some(operation_id) = labels["mesh.operation"].as_str() else {
                return false;
            };
            let Some(operation) = operations.get(operation_id) else {
                return false;
            };
            labels["mesh.managed"] == "true"
                && labels["mesh.cluster"] == cluster_id
                && labels["mesh.pool"] == "workers"
                && labels["mesh.template"] == operation.template_revision
                && labels["mesh.term"] == operation.control_term.0.to_string()
                && labels["mesh.revision"] == operation.desired_revision.0.to_string()
        })
}

fn managed_operation_labels_unique(inspection: &Value) -> bool {
    let Some(containers) = inspection.as_array() else {
        return false;
    };
    let labels: Vec<_> = containers
        .iter()
        .filter_map(|container| container["Config"]["Labels"]["mesh.operation"].as_str())
        .collect();
    !labels.is_empty() && labels.iter().collect::<BTreeSet<_>>().len() == labels.len()
}

const PRESSURE_HANDLER: &str = "Api.Todos.handle_pressure_probe";

fn pressure_routing_counts(list: &OperatorContinuityList) -> BTreeMap<String, u64> {
    let mut counts = BTreeMap::new();
    for record in &list.records {
        if record.declared_handler_runtime_name() == PRESSURE_HANDLER
            && record.phase.as_str() == "completed"
            && record.result.as_str() == "succeeded"
            && !record.execution_node.is_empty()
        {
            *counts.entry(record.execution_node.clone()).or_default() += 1;
        }
    }
    counts
}

fn wait_for_routing_evidence(
    target: &str,
    dynamic_workers: &BTreeSet<String>,
    timeout: Duration,
) -> Result<OperatorContinuityList, String> {
    let deadline = Instant::now() + timeout;
    let mut last = "continuity unavailable".to_string();
    while Instant::now() < deadline {
        match query_operator_continuity_list_remote(
            target,
            COOKIE,
            Some(2_000),
            Duration::from_secs(3),
        ) {
            Ok(list) => {
                let counts = pressure_routing_counts(&list);
                let total: u64 = counts.values().sum();
                let dynamic_received = dynamic_workers
                    .iter()
                    .any(|node| counts.get(node).copied().unwrap_or(0) > 0);
                let constrained = counts.get("worker1@worker1:4370").copied().unwrap_or(0);
                let larger = counts.get("worker2@worker2:4370").copied().unwrap_or(0);
                if total >= 12 && dynamic_received && larger > constrained {
                    return Ok(list);
                }
                last = format!(
                    "records={} total_pressure={total} dynamic_received={dynamic_received} counts={counts:?}",
                    list.total_records
                );
            }
            Err(error) => last = error.to_string(),
        }
        thread::park_timeout(Duration::from_millis(200));
    }
    Err(format!("proof_routing_evidence_timeout:{last}"))
}

fn continuity_record_json(record: &ContinuityRecord) -> Value {
    json!({
        "request_key": record.request_key,
        "attempt_id": record.attempt_id,
        "phase": record.phase.as_str(),
        "result": record.result.as_str(),
        "ingress_node": record.ingress_node,
        "owner_node": record.owner_node,
        "replica_node": record.replica_node,
        "replica_nodes": record.replica_nodes(),
        "acknowledged_replica_nodes": record.acknowledged_replica_nodes(),
        "replication_count": record.replication_count,
        "replica_status": record.replica_status.as_str(),
        "replication_health": record.replication_health.as_str(),
        "execution_node": record.execution_node,
        "routed_remotely": record.routed_remotely,
        "fell_back_locally": record.fell_back_locally,
        "error": record.error,
        "declared_handler_runtime_name": record.declared_handler_runtime_name(),
    })
}

fn continuity_list_json(list: &OperatorContinuityList) -> Value {
    json!({
        "total_records": list.total_records,
        "truncated": list.truncated,
        "records": list.records.iter().map(continuity_record_json).collect::<Vec<_>>(),
    })
}

fn wait_for_autonomous_leader(
    targets: &[&str],
    timeout: Duration,
) -> Result<(String, OperatorRuntimeSnapshot), String> {
    let deadline = Instant::now() + timeout;
    let mut last = "no candidate".to_string();
    while Instant::now() < deadline {
        for target in targets {
            match query_operator_runtime_remote(target, COOKIE, Duration::from_secs(3)) {
                Ok(snapshot)
                    if snapshot.autonomous.running
                        && snapshot.autonomous.leader
                        && snapshot
                            .consensus
                            .as_ref()
                            .is_some_and(|consensus| consensus.state == "leader") =>
                {
                    return Ok(((*target).to_string(), snapshot));
                }
                Ok(snapshot) => last = format!("{target}:{:?}", snapshot.autonomous),
                Err(error) => last = format!("{target}:{error}"),
            }
        }
        thread::park_timeout(Duration::from_millis(200));
    }
    Err(format!("proof_autonomous_leader_timeout:{last}"))
}

fn wait_for_driver_recovery(target: &str, timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    let mut last = "runtime unavailable".to_string();
    while Instant::now() < deadline {
        match query_operator_runtime_remote(target, COOKIE, Duration::from_secs(3)) {
            Ok(snapshot)
                if snapshot.autonomous.running
                    && snapshot.autonomous.leader
                    && snapshot.autonomous.last_error.is_none()
                    && snapshot.autonomous.last_reconcile.is_some() =>
            {
                return Ok(());
            }
            Ok(snapshot) => last = format!("autonomous={:?}", snapshot.autonomous),
            Err(error) => last = error.to_string(),
        }
        thread::park_timeout(Duration::from_millis(200));
    }
    Err(format!("proof_driver_restart_recovery_timeout:{last}"))
}

#[derive(Default, serde::Serialize)]
struct DrainLoadSummary {
    successes: u32,
    request_keys: Vec<String>,
}

fn start_drain_continuity_load() -> thread::JoinHandle<DrainLoadSummary> {
    thread::spawn(|| {
        let mut requests = Vec::new();
        for index in 0..4 {
            requests.push(thread::spawn(move || {
                let port = if index % 2 == 0 { 18081 } else { 18082 };
                http_request(port, "GET", "/proof/pressure", "", &[])
                    .ok()
                    .filter(|response| response.status == 200)
                    .and_then(|response| {
                        response
                            .header("x-mesh-continuity-request-key")
                            .map(str::to_string)
                    })
            }));
        }
        let request_keys: Vec<String> = requests
            .into_iter()
            .filter_map(|request| request.join().ok().flatten())
            .collect();
        DrainLoadSummary {
            successes: request_keys.len().try_into().unwrap_or(u32::MAX),
            request_keys,
        }
    })
}

fn wait_for_runtime_scale_down(
    target: &str,
    final_desired: u16,
    timeout: Duration,
) -> Result<
    (
        OperatorRuntimeSnapshot,
        Option<OperatorRuntimeSnapshot>,
        DrainLoadSummary,
    ),
    String,
> {
    let deadline = Instant::now() + timeout;
    let mut draining = None;
    let mut drain_load = None;
    let mut last = "no runtime snapshot".to_string();
    while Instant::now() < deadline {
        match query_operator_runtime_remote(target, COOKIE, Duration::from_secs(3)) {
            Ok(snapshot) => {
                if draining.is_none()
                    && snapshot
                        .nodes
                        .iter()
                        .any(|node| node.state == "draining" && !node.routing_eligible)
                {
                    draining = Some(snapshot.clone());
                    drain_load = Some(start_drain_continuity_load());
                }
                let desired = latest_desired_workers(&snapshot);
                if desired == Some(final_desired)
                    && snapshot.autonomous.last_error.is_none()
                    && snapshot
                        .autonomous
                        .last_reconcile
                        .as_ref()
                        .is_some_and(|reconcile| {
                            reconcile.desired_workers == final_desired
                                && reconcile.observed_workers == final_desired
                                && reconcile.drains.is_empty()
                        })
                {
                    let drain_load_summary = drain_load
                        .take()
                        .map(|load| {
                            load.join()
                                .map_err(|_| "proof_drain_load_thread_panicked".to_string())
                        })
                        .transpose()?
                        .unwrap_or_default();
                    return Ok((snapshot, draining, drain_load_summary));
                }
                last = format!("desired={desired:?}:autonomous={:?}", snapshot.autonomous);
            }
            Err(error) => last = error.to_string(),
        }
        thread::park_timeout(Duration::from_millis(100));
    }
    Err(format!("proof_runtime_scale_down_timeout:{last}"))
}

fn seed_postgres_mutations() -> Result<u64, String> {
    let mut acknowledged = 0;
    for index in 0..12 {
        let port = if index % 2 == 0 { 18081 } else { 18082 };
        let body = format!("{{\"title\":\"proof-{index}\"}}");
        let idempotency_key = format!("proof-seed-{index}");
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut last: String;
        loop {
            match http_request(
                port,
                "POST",
                "/todos",
                &body,
                &[
                    ("Content-Type", "application/json"),
                    ("Idempotency-Key", &idempotency_key),
                ],
            ) {
                Ok(response) if matches!(response.status, 200 | 201) => {
                    acknowledged += 1;
                    break;
                }
                Ok(response) if matches!(response.status, 429 | 502 | 503 | 504) => {
                    last = format!("status={}", response.status);
                }
                Ok(response) => {
                    return Err(format!(
                        "proof_seed_mutation_failed:status={}:body={}",
                        response.status,
                        redact(&response.body)
                    ));
                }
                Err(error) => last = error,
            }
            if Instant::now() >= deadline {
                return Err(format!("proof_seed_mutation_timeout:{last}"));
            }
            thread::park_timeout(Duration::from_millis(100));
        }
    }
    Ok(acknowledged)
}

fn postgres_todo_count(harness: &ProofHarness) -> Result<u64, String> {
    let output = harness.compose(&[
        "exec",
        "-T",
        "postgres",
        "psql",
        "-U",
        "postgres",
        "-d",
        "mesh_proof",
        "-Atc",
        "SELECT COUNT(*) FROM todos",
    ])?;
    output
        .trim()
        .parse()
        .map_err(|_| format!("proof_database_count_invalid:{output}"))
}

struct HttpResponse {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl HttpResponse {
    fn header(&self, wanted: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(wanted))
            .map(|(_, value)| value.as_str())
    }
}

fn http_request(
    port: u16,
    method: &str,
    path: &str,
    body: &str,
    extra_headers: &[(&str, &str)],
) -> Result<HttpResponse, String> {
    let mut stream = TcpStream::connect_timeout(
        &format!("127.0.0.1:{port}")
            .parse()
            .map_err(|_| "proof_http_address_invalid".to_string())?,
        Duration::from_secs(2),
    )
    .map_err(|error| format!("proof_http_connect_failed:{port}:{error}"))?;
    stream
        .set_read_timeout(Some(PROOF_HTTP_READ_TIMEOUT))
        .map_err(|error| format!("proof_http_timeout_failed:{error}"))?;
    let mut request = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    );
    for (name, value) in extra_headers {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str("\r\n");
    request.push_str(body);
    stream
        .write_all(request.as_bytes())
        .map_err(|error| format!("proof_http_write_failed:{error}"))?;
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .map_err(|error| format!("proof_http_read_failed:{error}"))?;
    let (head, body) = response
        .split_once("\r\n\r\n")
        .unwrap_or((response.as_str(), ""));
    let mut lines = head.lines();
    let status = lines
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|status| status.parse().ok())
        .ok_or_else(|| "proof_http_status_invalid".to_string())?;
    let headers = lines
        .take_while(|line| !line.is_empty() && *line != "\r")
        .filter_map(|line| line.trim_end_matches('\r').split_once(':'))
        .map(|(name, value)| (name.trim().to_string(), value.trim().to_string()))
        .collect();
    Ok(HttpResponse {
        status,
        headers,
        body: body.to_string(),
    })
}

fn wait_for_http(port: u16, path: &str, timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    let mut last = "not attempted".to_string();
    while Instant::now() < deadline {
        match http_request(port, "GET", path, "", &[]) {
            Ok(response) if response.status == 200 => return Ok(()),
            Ok(response) => last = format!("status={}", response.status),
            Err(error) => last = error,
        }
        thread::park_timeout(Duration::from_millis(250));
    }
    Err(format!("proof_http_readiness_timeout:{port}:{last}"))
}

fn repository_root() -> Result<PathBuf, String> {
    let mut current = std::env::current_dir()
        .map_err(|error| format!("proof_current_directory_failed:{error}"))?;
    loop {
        if current.join("Cargo.toml").is_file()
            && current
                .join("proof/docker-autoscaling/docker-compose.yml")
                .is_file()
        {
            return Ok(current);
        }
        if !current.pop() {
            return Err("proof_repository_root_not_found".to_string());
        }
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

fn redact(value: &str) -> String {
    value
        .replace(COOKIE, "[redacted]")
        .replace(OPERATOR_KEY, "[redacted]")
        .replace("postgres:postgres", "[redacted]")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn docker_args() -> DockerAutoscalingArgs {
        DockerAutoscalingArgs {
            keep_running: false,
            evidence_dir: None,
            no_build: true,
            start_only: false,
            connection_file: None,
        }
    }

    #[test]
    fn start_only_requires_keep_running() {
        let args = DockerAutoscalingArgs {
            start_only: true,
            ..docker_args()
        };

        assert_eq!(
            validate_docker_autoscaling_args(&args),
            Err("docker_autoscaling_start_only_requires_keep_running".to_string())
        );
    }

    #[test]
    fn connection_file_requires_start_only() {
        let args = DockerAutoscalingArgs {
            connection_file: Some(PathBuf::from("connection.json")),
            ..docker_args()
        };

        assert_eq!(
            validate_docker_autoscaling_args(&args),
            Err("docker_autoscaling_connection_file_requires_start_only".to_string())
        );
    }

    #[cfg(unix)]
    #[test]
    fn owner_only_file_is_created_with_mode_0600() {
        use std::os::unix::fs::PermissionsExt as _;

        let directory = tempfile::tempdir().expect("create temporary directory");
        let path = directory.path().join("connection.json");
        write_owner_only_new(&path, b"{}\n", "test_owner_only").expect("create owner-only file");

        assert_eq!(
            fs::metadata(path)
                .expect("read owner-only metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}
