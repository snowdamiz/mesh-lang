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

use clap::{Args, Subcommand};
use mesh_rt::{
    query_operator_continuity_list_remote, query_operator_runtime_remote, ContinuityRecord,
    ControlMutation, OperatorContinuityList, OperatorRuntimeSnapshot,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const COOKIE: &str = "proof-cookie-0123456789abcdef0123";
const OPERATOR_KEY: &str = "proof-operator-key-0123456789abcdef";
const MIN_WORKERS: u16 = 2;
const PROOF_HTTP_READ_TIMEOUT: Duration = Duration::from_secs(15);
// The 1,000-request burst saturates the machine rather than measuring Mesh's
// latency: on a GitHub runner placing all eleven proof containers on four
// cores the median alone is around 3.8s, and p95 to max spans under a second,
// which is uniform queueing, not a tail.
//
// 6s sat inside the run-to-run variance and made this a coin flip. Two
// consecutive runs of the same code measured p99 5,910ms (passed by 90ms) and
// 6,025ms (failed by 25ms), both with 1,000 of 1,000 requests succeeding and
// no failures. The second only became visible once the scale-down timeout
// above stopped aborting the proof first.
//
// 9s keeps the gate meaningful -- a genuine regression to half the throughput
// still trips it -- without failing on a 2% wobble. What this assertion is
// really for is caught by its neighbours regardless: every request has to
// succeed, be unique, and execute remotely.
const CONCURRENT_BURST_P99_BUDGET_MILLIS: u64 = 9_000;
const FAILURE_LOAD_P99_BUDGET_MILLIS: u64 = 10_000;
const BURST_OPERATOR_QUERY_BUDGET_MILLIS: u64 = 3_000;
const BURST_GATEWAY_HEALTH_BUDGET_MILLIS: u64 = 3_000;
// The proof can remove three workers serially (max five down to min two).
// Each step has a 4s scale-down window, a 30s drain deadline, and a 30s
// termination deadline, and the waiter also has to cover one final 30s
// provider observation, controller tick polling, and consensus propagation.
//
// Derived rather than written as a number, because the number was the bug: the
// envelope below already sums to 222s and the timeout was 240s, leaving about
// eighteen seconds for everything the envelope does not name. That held on a
// developer machine and failed on GitHub's runners, which run all eleven proof
// containers on four cores. The evidence bundles from those runs show the
// cluster reaching the desired two workers moments after the deadline -- the
// final provider inspection has exactly two running managed workers with
// matching labels -- so the proof was giving up during convergence, not
// catching a stall. Doubling the envelope makes a slow machine a slow pass; a
// cluster that genuinely cannot converge still fails, just later.
const RUNTIME_SCALE_DOWN_STEPS: u64 = 3;
const RUNTIME_SCALE_DOWN_STEP_SECONDS: u64 = 4 + 30 + 30;
const RUNTIME_SCALE_DOWN_FINAL_OBSERVATION_SECONDS: u64 = 30;
const RUNTIME_SCALE_DOWN_PROOF_TIMEOUT: Duration = Duration::from_secs(
    RUNTIME_SCALE_DOWN_STEPS * RUNTIME_SCALE_DOWN_STEP_SECONDS
        + RUNTIME_SCALE_DOWN_FINAL_OBSERVATION_SECONDS,
);

/// How much slower this machine is than the one the budgets were written on.
///
/// Every wall-clock deadline here was calibrated on a developer machine. CI
/// runs all eleven proof containers on four cores, and three separate
/// deadlines have now expired there mid-convergence -- scale-down, then the
/// concurrent-burst p99, then managed-worker readiness -- each time with the
/// evidence showing the cluster reaching the expected state moments later.
/// Raising them one at a time just moves the failure to the next one, so the
/// budgets stay at their declared values and the machine scales them.
///
/// A stall still fails, it just takes longer to say so.
/// `MESH_PROOF_TIME_SCALE` overrides the estimate.
fn proof_time_scale() -> u32 {
    time_scale(
        std::env::var("MESH_PROOF_TIME_SCALE").ok().as_deref(),
        std::thread::available_parallelism().map_or(4, std::num::NonZeroUsize::get),
    )
}

/// The scale an override (1 to 10) asks for, else the one `cores` call for.
fn time_scale(requested: Option<&str>, cores: usize) -> u32 {
    match requested.and_then(|raw| raw.parse::<u32>().ok()) {
        Some(scale) => scale.clamp(1, 10),
        None => match cores {
            0..=4 => 3,
            5..=8 => 2,
            _ => 1,
        },
    }
}

/// Everything the Docker proof does outside itself: run a command (Docker,
/// git, cargo), ask a controller for its runtime or continuity, send HTTP
/// to a gateway, spawn a thread, and wait. The proof runs against local
/// Docker; its tests put a scripted cluster with a clock of its own here.
trait ProofWorld: Send + Sync {
    /// Runs `program` in `dir` with `env` added to the environment.
    fn run(
        &self,
        program: &str,
        args: &[&str],
        dir: &Path,
        env: &BTreeMap<String, &str>,
    ) -> std::io::Result<Output>;
    fn runtime(&self, target: &str, timeout: Duration) -> Result<OperatorRuntimeSnapshot, String>;
    fn continuity(&self, target: &str, timeout: Duration) -> Result<ContinuityEvidence, String>;
    fn http(
        &self,
        port: u16,
        method: &str,
        path: &str,
        body: &str,
        headers: &[(&str, &str)],
    ) -> Result<HttpResponse, String>;
    /// Starts `work` on a thread of its own with `stack_size` bytes of
    /// stack, as the burst's requests run.
    fn spawn(
        &self,
        name: String,
        stack_size: usize,
        work: Box<dyn FnOnce() + Send>,
    ) -> std::io::Result<thread::JoinHandle<()>>;
    fn now(&self) -> Instant;
    /// A deadline for `timeout` worth of work, stretched for a slow machine.
    fn deadline(&self, timeout: Duration) -> Instant;
    fn pause(&self, duration: Duration);
}

/// The proof's world: this machine's Docker and the cluster it runs.
struct LocalDocker;

impl ProofWorld for LocalDocker {
    fn run(
        &self,
        program: &str,
        args: &[&str],
        dir: &Path,
        env: &BTreeMap<String, &str>,
    ) -> std::io::Result<Output> {
        Command::new(program)
            .args(args)
            .current_dir(dir)
            .envs(env)
            .stdin(Stdio::null())
            .output()
    }

    fn runtime(&self, target: &str, timeout: Duration) -> Result<OperatorRuntimeSnapshot, String> {
        query_operator_runtime_remote(target, COOKIE, timeout).map_err(|error| error.to_string())
    }

    fn continuity(&self, target: &str, timeout: Duration) -> Result<ContinuityEvidence, String> {
        query_operator_continuity_list_remote(target, COOKIE, Some(2_000), timeout)
            .map(|list| ContinuityEvidence::of(&list))
            .map_err(|error| error.to_string())
    }

    fn http(
        &self,
        port: u16,
        method: &str,
        path: &str,
        body: &str,
        headers: &[(&str, &str)],
    ) -> Result<HttpResponse, String> {
        http_request(port, method, path, body, headers)
    }

    fn spawn(
        &self,
        name: String,
        stack_size: usize,
        work: Box<dyn FnOnce() + Send>,
    ) -> std::io::Result<thread::JoinHandle<()>> {
        thread::Builder::new()
            .name(name)
            .stack_size(stack_size)
            .spawn(work)
    }

    fn now(&self) -> Instant {
        Instant::now()
    }

    fn deadline(&self, timeout: Duration) -> Instant {
        Instant::now() + timeout * proof_time_scale()
    }

    fn pause(&self, duration: Duration) {
        thread::park_timeout(duration);
    }
}

/// Tries `attempt` every `interval` until it succeeds or `timeout` has
/// passed, when the error is the last attempt's.
fn poll<T>(
    world: &dyn ProofWorld,
    timeout: Duration,
    interval: Duration,
    mut attempt: impl FnMut() -> Result<T, String>,
) -> Result<T, String> {
    let deadline = world.deadline(timeout);
    loop {
        let last = match attempt() {
            Ok(value) => return Ok(value),
            Err(last) => last,
        };
        world.pause(interval);
        if world.now() >= deadline {
            return Err(last);
        }
    }
}

#[derive(Subcommand, Debug)]
pub enum ProofCommand {
    /// Run the mandatory local Docker/PostgreSQL autonomous-scaling proof.
    DockerAutoscaling(DockerAutoscalingArgs),
    /// Run the bounded-retention continuity soak gate.
    ContinuitySoak(crate::proof_gates::ContinuitySoakArgs),
    /// Run deterministic autonomous-cluster performance gates.
    AutonomousPerformance(crate::proof_gates::AutonomousPerformanceArgs),
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
        ProofCommand::AutonomousChaos(args) => run_autonomous_chaos(args),
    }
}

fn run_autonomous_chaos(args: AutonomousChaosArgs) -> Result<(), String> {
    if args.rounds == 0 || args.rounds > 100 {
        return Err("autonomous_chaos_rounds_must_be_1_to_100".to_string());
    }
    let root = repository_root()?;
    let evidence = evidence_directory(&root, "autonomous-chaos", unix_millis(), args.evidence_dir);
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
    let mut run = |round: u16, offset: usize| -> Result<bool, String> {
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
            .env(
                "RUST_TEST_THREADS",
                if round.is_multiple_of(2) { "1" } else { "4" },
            )
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
            "test_threads": if round.is_multiple_of(2) { 1 } else { 4 },
            "exit_code": output.status.code(),
            "passed": success
        }));
        Ok(success)
    };
    // Every filter of every round, until one fails.
    let passed = (0..args.rounds)
        .flat_map(|round| (0..filters.len()).map(move |offset| (round, offset)))
        .map(|(round, offset)| run(round, offset))
        .find(|outcome| !matches!(outcome, Ok(true)))
        .transpose()?
        .is_none();
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
    crate::proof_gates::ensure(passed, "autonomous_chaos_gate_failed")
}

struct ProofHarness {
    world: Arc<dyn ProofWorld>,
    root: PathBuf,
    compose_file: PathBuf,
    /// Where instrumented containers write coverage profiles
    /// (`MESH_PROOF_COVERAGE_DIR`); the instrumented objects that read them
    /// go under `objects/` in it.
    coverage_dir: Option<PathBuf>,
    evidence: PathBuf,
    project: String,
    cluster_id: String,
    image: String,
    driver_image: String,
    keep_running: bool,
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
    std::path::absolute(&path).map_err(|error| format!("proof_path_invalid:{error}"))
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
        // Absolute: compose resolves a relative bind path from its own file.
        if let Some(coverage_dir) = self.coverage_dir.as_ref().and_then(|dir| dir.to_str()) {
            environment.insert("MESH_PROOF_COVERAGE_DIR".to_string(), coverage_dir);
        }
        environment
    }

    fn checked(&self, program: &str, args: &[&str]) -> Result<String, String> {
        let output = self
            .world
            .run(program, args, &self.root, &self.proof_environment())
            .map_err(|error| format!("proof_command_start_failed:{program}:{error}"))?;
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
        let coverage_file = self
            .compose_file
            .with_file_name("docker-compose.coverage.yml");
        if self.coverage_dir.is_some() {
            command.extend(["-f", coverage_file.to_str().unwrap()]);
        }
        command.extend_from_slice(args);
        self.checked("docker", &command)
    }

    /// `docker build` of one Dockerfile stage, instrumented for coverage when
    /// the proof collects it.
    fn build_stage(&self, target: &str, output: &[&str]) -> Result<String, String> {
        let dockerfile = self
            .root
            .join("proof/docker-autoscaling/Dockerfile")
            .to_string_lossy()
            .into_owned();
        let mut args = vec!["build", "--file", &dockerfile, "--target", target];
        if self.coverage_dir.is_some() {
            args.extend(["--build-arg", "MESH_COVERAGE=1"]);
        }
        args.extend_from_slice(output);
        args.push(".");
        self.checked("docker", &args)
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
    let evidence = evidence_directory(&root, "docker-autoscaling", timestamp, evidence_dir);
    let connection_file = start_only
        .then(|| connection_file.unwrap_or_else(|| evidence.join("connection.json")))
        .map(absolute_path)
        .transpose()?;
    if let Some(connection_file) = &connection_file {
        ensure_connection_outputs_are_new(connection_file)?;
    }
    fs::create_dir_all(&evidence)
        .map_err(|error| format!("proof_evidence_directory_failed:{error}"))?;
    let (
        tls_ca_der_b64,
        tls_cert_der_b64,
        tls_key_der_b64,
        driver_cert_der_b64,
        driver_key_der_b64,
    ) = generate_proof_mtls()?;
    let driver_shared_key = format!(
        "{:x}",
        Sha256::digest(format!("{timestamp}:{tls_key_der_b64}").as_bytes())
    );
    let (identity_signing_key_der_b64, identity_verify_key_b64) =
        mesh_rt::generate_identity_signing_material()?;
    let issued_at = unix_millis();
    let expires_at = issued_at.saturating_add(30 * 24 * 60 * 60 * 1_000);
    // Each node's signed identity, under its environment name.
    let identity_envelopes = [
        ("CONTROLLER1", "controller", "controller1"),
        ("CONTROLLER2", "controller", "controller2"),
        ("CONTROLLER3", "controller", "controller3"),
        ("GATEWAY1", "gateway", "gateway1"),
        ("GATEWAY2", "gateway", "gateway2"),
        ("WORKER1", "worker", "worker1"),
        ("WORKER2", "worker", "worker2"),
        ("OPERATOR", "operator", "proof"),
    ]
    .into_iter()
    .map(|(name, role, host)| {
        let claim = mesh_rt::NodeIdentityClaim {
            schema_version: mesh_rt::IDENTITY_SCHEMA_VERSION,
            cluster_id: project.clone(),
            stable_node_id: format!("{project}/{role}/{host}"),
            // The operator connects from anywhere.
            advertised_name: if role == "operator" {
                "*".to_string()
            } else {
                format!("{host}@{host}:4370")
            },
            roles: vec![role.to_string()],
            issued_at_unix_millis: issued_at,
            expires_at_unix_millis: expires_at,
        };
        let envelope = mesh_rt::sign_identity_claim(&claim, &identity_signing_key_der_b64)?;
        Ok((name.to_string(), envelope))
    })
    .collect::<Result<BTreeMap<_, _>, String>>()?;
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
    let coverage_dir = std::env::var_os("MESH_PROOF_COVERAGE_DIR")
        .map(PathBuf::from)
        .map(absolute_path)
        .transpose()?;
    if let Some(coverage_dir) = &coverage_dir {
        // The containers write their profiles as an unprivileged user.
        fs::create_dir_all(coverage_dir)
            .map_err(|error| format!("proof_coverage_directory_failed:{error}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(coverage_dir, fs::Permissions::from_mode(0o777))
                .map_err(|error| format!("proof_coverage_directory_failed:{error}"))?;
        }
    }
    let mut harness = ProofHarness {
        world: Arc::new(LocalDocker),
        compose_file: root.join("proof/docker-autoscaling/docker-compose.yml"),
        coverage_dir,
        root,
        evidence,
        cluster_id: project.clone(),
        image,
        driver_image,
        project,
        keep_running,
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

fn run_proof(
    harness: &mut ProofHarness,
    no_build: bool,
    connection_file: Option<&Path>,
) -> Result<(), String> {
    let world = Arc::clone(&harness.world);
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
        let _ = harness.build_stage("runtime", &["--tag", &harness.image])?;
        let _ = harness.checked(
            "docker",
            &["tag", &harness.image, "mesh-autoscaling-proof:local"],
        )?;
        let _ = harness.build_stage("driver", &["--tag", &harness.driver_image])?;
        let _ = harness.checked(
            "docker",
            &[
                "tag",
                &harness.driver_image,
                "mesh-autoscaling-driver:local",
            ],
        )?;
        if let Some(coverage_dir) = &harness.coverage_dir {
            let destination = format!("type=local,dest={}", coverage_dir.join("objects").display());
            let _ = harness.build_stage("coverage-objects", &["--output", &destination])?;
        }
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
    wait_for_http(&*world, 18081, "/health", Duration::from_secs(180))?;
    wait_for_http(&*world, 18082, "/health", Duration::from_secs(180))?;
    let mut controller_target = "controller1@127.0.0.1:14371".to_string();
    let baseline = wait_for_runtime(
        &*world,
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
        return Ok(());
    }

    let acknowledged_mutations = seed_postgres_mutations(&*world)?;
    // Keep policy pressure active through worker replacement and controller
    // failover. Docker creates workers sequentially, so a short fixed burst can
    // end while max-capacity workers are still warming and accidentally turn
    // the crash-replacement assertion into a race with scale-down.
    let load = start_load(&world);
    let (peak, initial_desired) = wait_for_runtime_desired(
        &*world,
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
    let (peak, desired) =
        wait_for_stable_runtime(&*world, &controller_target, Duration::from_secs(120))?;
    let peak_managed = desired.saturating_sub(MIN_WORKERS) as usize;
    harness.write(
        "capacity-peak-ready.json",
        serde_json::to_vec_pretty(&peak).expect("serialize ready peak"),
    )?;
    let concurrent_burst = run_concurrent_remote_burst(&world, &controller_target, 1_000)?;
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
        &*world,
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
        &*world,
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
        &*world,
        &controller_target,
        &dynamic_workers,
        Duration::from_secs(30),
    )?;
    let peak_routing_counts = pressure_routing_counts(&peak_continuity);
    harness.write(
        "continuity-peak.json",
        serde_json::to_vec_pretty(&peak_continuity.json()).expect("serialize peak continuity"),
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
    wait_for_driver_recovery(&*world, &controller_target, Duration::from_secs(45))?;
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
    // The first create after the driver restart is the one the unhealthy-worker
    // fault stops a second after it starts, so the count alone can be met by a
    // doomed container. Wait until the replacement serves and reconcile is
    // idle; otherwise the failover check below watches it being replaced.
    wait_for_stable_runtime(&*world, &controller_target, Duration::from_secs(120))?;
    harness
        .assertions
        .insert("killed_worker_replaced".to_string(), true);

    let _ = harness.compose(&["kill", "controller1"])?;
    let (failover_target, failover_runtime) = wait_for_autonomous_leader(
        &*world,
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
        &*world,
        "controller2@127.0.0.1:14372",
        failover_index,
        Duration::from_secs(30),
    )?;
    let consensus_after_controller3 = wait_for_consensus_applied(
        &*world,
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
    world.pause(Duration::from_secs(2));
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
    let service_after_failover = [18081, 18082].into_iter().all(|port| {
        wait_for_http(&*world, port, "/proof/pressure", Duration::from_secs(30)).is_ok()
    });
    harness.assertions.insert(
        "controller_failover_preserved_service".to_string(),
        service_after_failover,
    );

    let (final_snapshot, drain_snapshot, drain_load) = match wait_for_runtime_scale_down(
        &world,
        &controller_target,
        MIN_WORKERS,
        RUNTIME_SCALE_DOWN_PROOF_TIMEOUT,
    ) {
        Ok(result) => result,
        Err(error) => {
            if let Ok(snapshot) = world.runtime(&controller_target, Duration::from_secs(3)) {
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
    let final_continuity = world
        .continuity(&controller_target, Duration::from_secs(5))
        .map_err(|error| format!("proof_final_continuity_query_failed:{error}"))?;
    harness.write(
        "continuity-final.json",
        serde_json::to_vec_pretty(&final_continuity.json()).expect("serialize final continuity"),
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
                        .replica_nodes
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

/// The gate a burst's threads wait at, so that every request starts at once.
type StartGate = Arc<(Mutex<bool>, std::sync::Condvar)>;

fn wait_at(gate: &StartGate) {
    let (open, signal) = &**gate;
    let mut allowed = open.lock().unwrap();
    while !*allowed {
        allowed = signal.wait(allowed).unwrap();
    }
}

fn open(gate: &StartGate) {
    let (open, signal) = &**gate;
    *open.lock().unwrap() = true;
    signal.notify_all();
}

/// `requests` requests through the gateways at once, and meanwhile one
/// operator query and a health check of each gateway, which a busy handler
/// must not starve.
fn run_concurrent_remote_burst(
    world: &Arc<dyn ProofWorld>,
    controller_target: &str,
    requests: usize,
) -> Result<ConcurrentBurstSummary, String> {
    let start_gate: StartGate = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
    let (sender, receiver) = std::sync::mpsc::channel();
    let mut workers: Vec<thread::JoinHandle<()>> = Vec::with_capacity(requests + 1);
    // A thread that cannot start ends the burst: the gate opens for those
    // waiting, and they finish before the error returns.
    let mut start = |name: String, stack_size: usize, work: Box<dyn FnOnce() + Send>| {
        let started = world.spawn(name, stack_size, work).inspect_err(|_| {
            open(&start_gate);
            for worker in workers.drain(..) {
                let _ = worker.join();
            }
        })?;
        workers.push(started);
        Ok::<(), std::io::Error>(())
    };
    for index in 0..requests {
        let gate = Arc::clone(&start_gate);
        let sender = sender.clone();
        let world = Arc::clone(world);
        start(
            format!("mesh-proof-concurrent-{index}"),
            256 * 1024,
            Box::new(move || {
                let port = if index % 2 == 0 { 18081 } else { 18082 };
                wait_at(&gate);
                let started = Instant::now();
                let result = world.http(port, "GET", "/proof/pressure", "", &[]);
                let elapsed: u64 = started.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
                let _ = sender.send((result, elapsed));
            }),
        )
        .map_err(|error| format!("proof_concurrent_thread_start_failed:{error}"))?;
    }
    drop(sender);
    let (probe_sender, probe_receiver) = std::sync::mpsc::channel();
    let gate = Arc::clone(&start_gate);
    let probe_world = Arc::clone(world);
    let controller_target = controller_target.to_string();
    // A thread's usual stack: the operator query runs the runtime's client.
    start(
        "mesh-proof-burst-isolation".to_string(),
        2 * 1024 * 1024,
        Box::new(move || {
            wait_at(&gate);
            let world = probe_world;
            let mut failure_reasons = Vec::new();
            let operator_started = Instant::now();
            let operator_query_succeeded =
                match world.runtime(&controller_target, Duration::from_secs(3)) {
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
                match world.http(port, "GET", "/health", "", &[]) {
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
            let _ = probe_sender.send(BurstIsolationProbe {
                operator_query_succeeded,
                operator_query_latency_millis,
                gateway_health_successes,
                gateway_health_max_latency_millis,
                failure_reasons,
            });
        }),
    )
    .map_err(|error| format!("proof_isolation_probe_thread_start_failed:{error}"))?;
    open(&start_gate);
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
    let isolation_probe = probe_receiver
        .recv()
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

fn start_load(world: &Arc<dyn ProofWorld>) -> RunningLoad {
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);
    let world = Arc::clone(world);
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
            let world = Arc::clone(&world);
            workers.push(thread::spawn(move || {
                let port = if index % 2 == 0 { 18081 } else { 18082 };
                while !stop.load(Ordering::Relaxed) {
                    requests.fetch_add(1, Ordering::Relaxed);
                    let started = Instant::now();
                    match world.http(port, "GET", "/proof/pressure", "", &[]) {
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

/// The proof's mTLS material, each DER in base64: a CA, the certificate and
/// key every node presents, and the capacity driver's. openssl writes them as
/// PEM (whose body is that base64) in a directory removed on return.
fn generate_proof_mtls() -> Result<(String, String, String, String, String), String> {
    let directory = tempfile::Builder::new()
        .prefix("mesh-proof-mtls-")
        .tempdir()
        .map_err(|error| format!("proof_mtls_directory_failed:{error}"))?;
    let file = |name: &str| directory.path().join(name).to_string_lossy().into_owned();
    let openssl = |arguments: &[&str]| -> Result<(), String> {
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
    let pem_base64 = |path: &str| -> Result<String, String> {
        let pem = fs::read_to_string(path)
            .map_err(|error| format!("proof_mtls_read_failed:{path}:{error}"))?;
        Ok(pem
            .lines()
            .filter(|line| !line.starts_with("-----"))
            .collect())
    };
    let (ca_key, ca_cert) = (file("ca.key.pem"), file("ca.cert.pem"));
    #[rustfmt::skip]
    openssl(&[
        "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-sha256", "-days", "1",
        "-subj", "/CN=mesh-proof-ca", "-keyout", &ca_key, "-out", &ca_cert,
    ])?;
    // A key for `name`, and its certificate from the CA with `extensions`.
    let issue = |name: &str, extensions: &[&str]| -> Result<(String, String), String> {
        let (key, request, cert) = (
            file(&format!("{name}.key.pem")),
            file(&format!("{name}.csr.pem")),
            file(&format!("{name}.cert.pem")),
        );
        let subject = format!("/CN={name}");
        let mut arguments = vec!["req", "-newkey", "rsa:2048", "-nodes", "-sha256"];
        arguments.extend(["-subj", &subject, "-keyout", &key, "-out", &request]);
        for extension in extensions {
            arguments.extend(["-addext", extension]);
        }
        openssl(&arguments)?;
        #[rustfmt::skip]
        openssl(&[
            "x509", "-req", "-sha256", "-days", "1", "-in", &request, "-CA", &ca_cert,
            "-CAkey", &ca_key, "-CAcreateserial", "-copy_extensions", "copy", "-out", &cert,
        ])?;
        Ok((pem_base64(&cert)?, pem_base64(&key)?))
    };
    let (node_cert, node_key) = issue("mesh-node", &["subjectAltName=DNS:mesh-node"])?;
    let (driver_cert, driver_key) = issue(
        "docker-driver",
        &[
            "subjectAltName=DNS:docker-driver",
            "extendedKeyUsage=serverAuth",
        ],
    )?;
    Ok((
        pem_base64(&ca_cert)?,
        node_cert,
        node_key,
        driver_cert,
        driver_key,
    ))
}

fn wait_for_runtime(
    world: &dyn ProofWorld,
    target: &str,
    minimum_nodes: usize,
    timeout: Duration,
) -> Result<OperatorRuntimeSnapshot, String> {
    poll(world, timeout, Duration::from_millis(250), || {
        let snapshot = world.runtime(target, Duration::from_secs(3))?;
        let workers = || {
            snapshot
                .nodes
                .iter()
                .filter(|node| node.roles.iter().any(|role| role == "worker"))
        };
        if workers().filter(|node| node.routing_eligible).count() >= minimum_nodes
            && snapshot.telemetry_complete
        {
            return Ok(snapshot);
        }
        Err(format!(
            "observed_nodes={} observed_workers={} telemetry_complete={}",
            snapshot.nodes.len(),
            workers().count(),
            snapshot.telemetry_complete
        ))
    })
    .map_err(|last| format!("proof_runtime_readiness_timeout:{last}"))
}

/// Waits for scaled-up capacity to hold still: every desired worker ready
/// and reconcile idle, unchanged for two seconds.
fn wait_for_stable_runtime(
    world: &dyn ProofWorld,
    target: &str,
    timeout: Duration,
) -> Result<(OperatorRuntimeSnapshot, u16), String> {
    const STABLE_FOR: Duration = Duration::from_secs(2);

    let mut stable_since: Option<(Instant, u16)> = None;
    poll(world, timeout, Duration::from_millis(100), || {
        let snapshot = world.runtime(target, Duration::from_secs(3)).inspect_err(|_| {
            stable_since = None;
        })?;
        let workers: Vec<_> = snapshot
            .nodes
            .iter()
            .filter(|node| node.roles.iter().any(|role| role == "worker"))
            .collect();
        let desired = latest_desired_workers(&snapshot).filter(|desired| {
            *desired > MIN_WORKERS
                && snapshot.telemetry_complete
                && workers.len() == usize::from(*desired)
                && workers.iter().all(|node| node.routing_eligible)
                && snapshot.draining_capacity == 0
                && snapshot
                    .autonomous
                    .last_reconcile
                    .as_ref()
                    .is_some_and(|reconcile| {
                        reconcile.desired_workers == *desired
                            && reconcile.observed_workers == reconcile.desired_workers
                            && reconcile.drains.is_empty()
                            && reconcile.ensured.is_empty()
                            && reconcile.constraints.is_empty()
                    })
        });
        let Some(desired) = desired else {
            stable_since = None;
            return Err(format!(
                "desired={:?}:workers={}:ready={}:states={:?}:draining={}:telemetry_complete={}:last_error={:?}:reconcile={:?}",
                latest_desired_workers(&snapshot),
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
            ));
        };
        // Stable since this observation, unless the last one was already
        // stable at the same capacity.
        let since = match stable_since {
            Some((since, stable_desired)) if stable_desired == desired => since,
            _ => world.now(),
        };
        stable_since = Some((since, desired));
        if world.now() - since >= STABLE_FOR {
            return Ok((snapshot, desired));
        }
        Err(format!("stable_for_less_than={STABLE_FOR:?}:desired={desired}"))
    })
    .map_err(|last| format!("proof_runtime_stability_timeout:{last}"))
}

fn wait_for_consensus_applied(
    world: &dyn ProofWorld,
    target: &str,
    minimum_log_index: u64,
    timeout: Duration,
) -> Result<OperatorRuntimeSnapshot, String> {
    poll(world, timeout, Duration::from_millis(200), || {
        let snapshot = world.runtime(target, Duration::from_secs(3))?;
        if snapshot.consensus.as_ref().is_some_and(|consensus| {
            consensus.voter_ids.len() == 3
                && consensus
                    .last_applied_log
                    .is_some_and(|index| index >= minimum_log_index)
        }) {
            return Ok(snapshot);
        }
        Err(format!("consensus={:?}", snapshot.consensus))
    })
    .map_err(|last| format!("proof_consensus_apply_timeout:{last}"))
}

fn wait_for_managed_count(
    harness: &ProofHarness,
    expected: usize,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = harness.world.deadline(timeout);
    loop {
        if managed_running_count(harness)? >= expected {
            return Ok(());
        }
        if harness.world.now() >= deadline {
            return Err("proof_managed_worker_readiness_timeout".to_string());
        }
        harness.world.pause(Duration::from_millis(250));
    }
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
    let deadline = harness.world.deadline(timeout);
    loop {
        let observed = managed_running_count(harness)?;
        if observed == expected {
            return Ok(());
        }
        if harness.world.now() >= deadline {
            return Err(format!(
                "proof_managed_worker_exact_count_timeout:expected={expected}:observed={observed}"
            ));
        }
        harness.world.pause(Duration::from_millis(200));
    }
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
    world: &dyn ProofWorld,
    target: &str,
    predicate: impl Fn(u16) -> bool,
    timeout: Duration,
) -> Result<(OperatorRuntimeSnapshot, u16), String> {
    poll(world, timeout, Duration::from_millis(150), || {
        let snapshot = world.runtime(target, Duration::from_secs(3))?;
        match latest_desired_workers(&snapshot) {
            Some(desired) if predicate(desired) => Ok((snapshot, desired)),
            desired => Err(format!(
                "desired={desired:?}:autonomous={:?}",
                snapshot.autonomous
            )),
        }
    })
    .map_err(|last| format!("proof_runtime_desired_timeout:{last}"))
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
    world: &dyn ProofWorld,
    target: &str,
    inspection: &Value,
    cluster_id: &str,
    timeout: Duration,
) -> Result<Vec<mesh_rt::DriverOperation>, String> {
    poll(world, timeout, Duration::from_millis(100), || {
        let operations =
            successful_driver_operations(&world.runtime(target, Duration::from_secs(3))?);
        if managed_labels_match_operations(inspection, &operations, cluster_id) {
            return Ok(operations);
        }
        Err(format!(
            "managed_labels_not_yet_committed:successful_operations={}",
            operations.len()
        ))
    })
    .map_err(|last| format!("proof_managed_operation_commit_timeout:{last}"))
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
            let term = operation.control_term.0.to_string();
            let revision = operation.desired_revision.0.to_string();
            labels["mesh.managed"] == "true"
                && labels["mesh.cluster"] == cluster_id
                && labels["mesh.pool"] == "workers"
                && labels["mesh.template"] == operation.template_revision
                && labels["mesh.term"] == term.as_str()
                && labels["mesh.revision"] == revision.as_str()
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

fn pressure_routing_counts(list: &ContinuityEvidence) -> BTreeMap<String, u64> {
    let mut counts = BTreeMap::new();
    for record in &list.records {
        if record.handler == PRESSURE_HANDLER
            && record.phase == "completed"
            && record.result == "succeeded"
            && !record.execution_node.is_empty()
        {
            *counts.entry(record.execution_node.clone()).or_default() += 1;
        }
    }
    counts
}

fn wait_for_routing_evidence(
    world: &dyn ProofWorld,
    target: &str,
    dynamic_workers: &BTreeSet<String>,
    timeout: Duration,
) -> Result<ContinuityEvidence, String> {
    poll(world, timeout, Duration::from_millis(200), || {
        let list = world.continuity(target, Duration::from_secs(3))?;
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
        Err(format!(
            "records={} total_pressure={total} dynamic_received={dynamic_received} counts={counts:?}",
            list.total_records
        ))
    })
    .map_err(|last| format!("proof_routing_evidence_timeout:{last}"))
}

/// What the proof reads of a controller's continuity list: the records'
/// evidence, and the fields its assertions check.
struct ContinuityEvidence {
    total_records: usize,
    truncated: bool,
    records: Vec<RecordEvidence>,
}

struct RecordEvidence {
    request_key: String,
    handler: String,
    phase: String,
    result: String,
    owner_node: String,
    execution_node: String,
    replica_nodes: Vec<String>,
    /// The whole record, as the evidence bundle shows it.
    json: Value,
}

impl ContinuityEvidence {
    fn of(list: &OperatorContinuityList) -> Self {
        let record = |record: &ContinuityRecord| RecordEvidence {
            request_key: record.request_key.clone(),
            handler: record.declared_handler_runtime_name().to_string(),
            phase: record.phase.as_str().to_string(),
            result: record.result.as_str().to_string(),
            owner_node: record.owner_node.clone(),
            execution_node: record.execution_node.clone(),
            replica_nodes: record.replica_nodes().to_vec(),
            json: json!({
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
            }),
        };
        ContinuityEvidence {
            total_records: list.total_records,
            truncated: list.truncated,
            records: list.records.iter().map(record).collect(),
        }
    }

    fn json(&self) -> Value {
        json!({
            "total_records": self.total_records,
            "truncated": self.truncated,
            "records": self.records.iter().map(|record| &record.json).collect::<Vec<_>>(),
        })
    }
}

/// The first of `targets` to lead the autonomous controller, and what it
/// reported.
fn wait_for_autonomous_leader(
    world: &dyn ProofWorld,
    targets: &[&str],
    timeout: Duration,
) -> Result<(String, OperatorRuntimeSnapshot), String> {
    let leading = |target: &str| {
        let snapshot = world
            .runtime(target, Duration::from_secs(3))
            .map_err(|error| format!("{target}:{error}"))?;
        if snapshot.autonomous.running
            && snapshot.autonomous.leader
            && snapshot
                .consensus
                .as_ref()
                .is_some_and(|consensus| consensus.state == "leader")
        {
            return Ok((target.to_string(), snapshot));
        }
        Err(format!("{target}:{:?}", snapshot.autonomous))
    };
    poll(world, timeout, Duration::from_millis(200), || {
        let mut last = Err("no candidate".to_string());
        for target in targets {
            last = leading(target);
            if last.is_ok() {
                break;
            }
        }
        last
    })
    .map_err(|last| format!("proof_autonomous_leader_timeout:{last}"))
}

fn wait_for_driver_recovery(
    world: &dyn ProofWorld,
    target: &str,
    timeout: Duration,
) -> Result<(), String> {
    poll(world, timeout, Duration::from_millis(200), || {
        let autonomous = world.runtime(target, Duration::from_secs(3))?.autonomous;
        if autonomous.running
            && autonomous.leader
            && autonomous.last_error.is_none()
            && autonomous.last_reconcile.is_some()
        {
            return Ok(());
        }
        Err(format!("autonomous={autonomous:?}"))
    })
    .map_err(|last| format!("proof_driver_restart_recovery_timeout:{last}"))
}

#[derive(Default, serde::Serialize)]
struct DrainLoadSummary {
    successes: u32,
    request_keys: Vec<String>,
}

fn start_drain_continuity_load(
    world: &Arc<dyn ProofWorld>,
) -> thread::JoinHandle<DrainLoadSummary> {
    let world = Arc::clone(world);
    thread::spawn(move || {
        let mut requests = Vec::new();
        for index in 0..4 {
            let world = Arc::clone(&world);
            requests.push(thread::spawn(move || {
                let port = if index % 2 == 0 { 18081 } else { 18082 };
                world
                    .http(port, "GET", "/proof/pressure", "", &[])
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

/// Waits for capacity to return to `final_desired` workers with reconcile
/// idle; when a node is first seen draining, starts requests to see where
/// they go. Returns the final snapshot, the draining one, and those
/// requests' results.
fn wait_for_runtime_scale_down(
    world: &Arc<dyn ProofWorld>,
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
    let mut draining = None;
    let mut drain_load = None;
    let (snapshot, drain_load) = poll(&**world, timeout, Duration::from_millis(100), || {
        let snapshot = world.runtime(target, Duration::from_secs(3))?;
        if draining.is_none()
            && snapshot
                .nodes
                .iter()
                .any(|node| node.state == "draining" && !node.routing_eligible)
        {
            draining = Some(snapshot.clone());
            drain_load = Some(start_drain_continuity_load(world));
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
            return Ok((snapshot, drain_load.take()));
        }
        // Lead with the three numbers that decide this wait. They are
        // all inside the status dump that follows, but finding them
        // there means reading a struct printed on one line.
        let observed = snapshot
            .autonomous
            .last_reconcile
            .as_ref()
            .map(|reconcile| reconcile.observed_workers);
        Err(format!(
            "final_desired={final_desired}:desired={desired:?}:observed={observed:?}:autonomous={:?}",
            snapshot.autonomous
        ))
    })
    .map_err(|last| format!("proof_runtime_scale_down_timeout:{last}"))?;
    let drain_load = drain_load
        .map(|load| {
            load.join()
                .map_err(|_| "proof_drain_load_thread_panicked".to_string())
        })
        .transpose()?
        .unwrap_or_default();
    Ok((snapshot, draining, drain_load))
}

fn seed_postgres_mutations(world: &dyn ProofWorld) -> Result<u64, String> {
    let mut acknowledged = 0;
    for index in 0..12 {
        let port = if index % 2 == 0 { 18081 } else { 18082 };
        let body = format!("{{\"title\":\"proof-{index}\"}}");
        let idempotency_key = format!("proof-seed-{index}");
        let deadline = world.now() + Duration::from_secs(30);
        let mut last: String;
        loop {
            match world.http(
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
            if world.now() >= deadline {
                return Err(format!("proof_seed_mutation_timeout:{last}"));
            }
            world.pause(Duration::from_millis(100));
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
        &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
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

fn wait_for_http(
    world: &dyn ProofWorld,
    port: u16,
    path: &str,
    timeout: Duration,
) -> Result<(), String> {
    poll(world, timeout, Duration::from_millis(250), || {
        let response = world.http(port, "GET", path, "", &[])?;
        if response.status == 200 {
            return Ok(());
        }
        Err(format!("status={}", response.status))
    })
    .map_err(|last| format!("proof_http_readiness_timeout:{port}:{last}"))
}

/// Where a proof writes its evidence: the directory asked for, else one
/// named for `timestamp` under the repository's `target/proof/<kind>`.
fn evidence_directory(
    root: &Path,
    kind: &str,
    timestamp: u64,
    requested: Option<PathBuf>,
) -> PathBuf {
    requested.unwrap_or_else(|| {
        root.join("target")
            .join("proof")
            .join(kind)
            .join(timestamp.to_string())
    })
}

fn repository_root() -> Result<PathBuf, String> {
    repository_root_from(absolute_path(PathBuf::from("."))?)
}

/// The first directory from `current` up that holds the workspace and the
/// proof's compose file.
fn repository_root_from(mut current: PathBuf) -> Result<PathBuf, String> {
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
mod scripted;

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

    #[test]
    fn time_scale_follows_an_override_else_the_cores() {
        assert_eq!(time_scale(Some("7"), 2), 7);
        assert_eq!(time_scale(Some("99"), 2), 10);
        assert_eq!(time_scale(Some("0"), 2), 1);
        assert_eq!(time_scale(Some("fast"), 16), 1);
        assert_eq!([2, 6, 12].map(|cores| time_scale(None, cores)), [3, 2, 1]);
    }

    #[test]
    fn paths_resolve_against_the_working_directory_and_find_the_repository() {
        let here = std::env::current_dir().unwrap();
        assert_eq!(absolute_path(PathBuf::from("a/b")), Ok(here.join("a/b")));
        assert!(absolute_path(PathBuf::new())
            .unwrap_err()
            .starts_with("proof_path_invalid:"));
        let root = repository_root().unwrap();
        assert!(root
            .join("proof/docker-autoscaling/docker-compose.yml")
            .is_file());
        assert_eq!(
            repository_root_from(root.join("compiler/meshc/src")),
            Ok(root)
        );
        let elsewhere = tempfile::tempdir().unwrap();
        assert_eq!(
            repository_root_from(elsewhere.path().to_path_buf()),
            Err("proof_repository_root_not_found".to_string())
        );
    }

    #[test]
    fn connection_outputs_must_be_new_and_distinct() {
        let directory = tempfile::tempdir().unwrap();
        let manifest = directory.path().join("topology.json");
        assert_eq!(ensure_connection_outputs_are_new(&manifest), Ok(()));
        for clash in ["topology.cookie", "topology.operator-key"] {
            assert_eq!(
                ensure_connection_outputs_are_new(&directory.path().join(clash)),
                Err("proof_connection_file_extension_invalid".to_string())
            );
        }
        fs::write(directory.path().join("topology.cookie"), "").unwrap();
        assert!(ensure_connection_outputs_are_new(&manifest)
            .unwrap_err()
            .starts_with("proof_connection_refuses_existing_output:"));
    }

    #[test]
    fn owner_only_files_are_never_overwritten_or_placed_under_a_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("secret");
        write_owner_only_new(&path, b"one", "test").unwrap();
        assert!(write_owner_only_new(&path, b"two", "test")
            .unwrap_err()
            .starts_with("test_open_failed:"));
        assert_eq!(fs::read(&path).unwrap(), b"one");
        assert!(write_owner_only_new(&path.join("below"), b"", "test")
            .unwrap_err()
            .starts_with("test_directory_failed:"));
        // A path with no parent has no directory to make: the root, which
        // is no file to create.
        assert!(write_owner_only_new(Path::new("/"), b"", "test")
            .unwrap_err()
            .starts_with("test_open_failed:"));
    }

    /// Managed containers match the committed operations only as a list,
    /// each labelled with its operation.
    #[test]
    fn managed_labels_are_read_from_a_list_of_labelled_containers() {
        let unlisted = json!({"Config": {"Labels": {"mesh.operation": "op1"}}});
        assert!(!managed_labels_match_operations(&unlisted, &[], "c"));
        assert!(!managed_operation_labels_unique(&unlisted));
        let unlabelled = json!([{"Config": {"Labels": {}}}]);
        assert!(!managed_labels_match_operations(&unlabelled, &[], "c"));
    }

    #[test]
    fn http_readiness_times_out_with_the_last_answer() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        // Every attempt gets the 503, however many a slow machine's
        // stretched deadline allows.
        let server = thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = stream.unwrap();
                let _ = stream.read(&mut [0; 1024]);
                let _ = stream.write_all(
                    b"HTTP/1.1 503 Service Unavailable\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                );
            }
        });
        let error =
            wait_for_http(&LocalDocker, port, "/health", Duration::from_millis(300)).unwrap_err();
        assert_eq!(
            error,
            format!("proof_http_readiness_timeout:{port}:status=503")
        );
        drop(server);
        // Nothing listening: the connection error is the last answer.
        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = closed.local_addr().unwrap().port();
        drop(closed);
        let error =
            wait_for_http(&LocalDocker, port, "/health", Duration::from_millis(100)).unwrap_err();
        assert!(
            error.starts_with(&format!(
                "proof_http_readiness_timeout:{port}:proof_http_connect_failed:"
            )),
            "{error}"
        );
    }

    #[test]
    fn percentiles_of_no_samples_are_zero() {
        assert_eq!(percentile_millis(&[], 0.99), 0);
        assert_eq!(percentile_millis(&[1, 2, 3, 4], 0.5), 3);
        assert_eq!(percentile_millis(&[1, 2, 3, 4], 2.0), 4);
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
