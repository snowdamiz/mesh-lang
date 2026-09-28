//! The Docker proof against a scripted cluster. `FakeCluster` answers the
//! proof's commands, operator queries and HTTP requests as a healthy
//! topology does, on a clock of its own, and a test makes one of them fail
//! to see the proof stop there and say why.
#![cfg(unix)]

use super::*;
use mesh_rt::{
    AutonomousControllerStatus, CapacityReconcileOutcome, ConsensusRuntimeSnapshot,
    ControlLogEntry, ControlTerm, DesiredCapacity, DesiredRevision, DriverOperation,
    DriverOperationState,
};
use std::os::unix::process::ExitStatusExt as _;
use std::sync::atomic::AtomicUsize;

/// What the scripted cluster gets wrong. Calls are counted from 1.
#[derive(Clone, Default)]
struct Faults {
    /// The command call that exits 1.
    command: Option<usize>,
    /// From this operator runtime query on, every query fails, or (with
    /// `true`) answers a snapshot no wait accepts.
    runtime: Option<(usize, bool)>,
    /// The same for continuity queries.
    continuity: Option<(usize, bool)>,
    /// Requests to this path, after this many of them, get this status (0:
    /// the connection fails).
    http: Option<(&'static str, usize, u16)>,
    /// The thread that does not start.
    spawn: Option<usize>,
    /// Capacity stays at its peak after the controller fails over.
    no_scale_down: bool,
    /// No node is ever seen draining.
    no_draining: bool,
    /// Docker reports no managed container.
    no_managed: bool,
    /// The managed containers Docker reports running, whatever the phase.
    running: Option<&'static str>,
}

struct FakeCluster {
    faults: Faults,
    start: Instant,
    elapsed: Mutex<Duration>,
    commands: Mutex<Vec<String>>,
    runtime_calls: AtomicUsize,
    continuity_calls: AtomicUsize,
    path_calls: AtomicUsize,
    spawns: AtomicUsize,
    /// The request keys the gateways have answered with.
    keys: Mutex<Vec<String>>,
    failed_over: AtomicBool,
}

const CLUSTER: &str = "mesh-proof-scripted";
const MANAGED: [&str; 3] = ["managed-a", "managed-b", "managed-c"];

fn output(status: i32, stdout: &str, stderr: &str) -> Output {
    Output {
        status: std::process::ExitStatus::from_raw(status << 8),
        stdout: stdout.as_bytes().to_vec(),
        stderr: stderr.as_bytes().to_vec(),
    }
}

fn node(node_id: &str, role: &str, state: &str, routing_eligible: bool) -> Value {
    json!({
        "node_id": node_id, "roles": [role], "state": state,
        "routing_eligible": routing_eligible, "capacity_units": 1, "active_workers": 1,
        "runnable_actors": 0, "inflight": 0, "queued_items": 0, "queued_bytes": 0,
        "reservations": 0, "pressure": 0.5, "dominant_signal": "inflight",
        "report_sequence": 1, "control_term": 1, "membership_generation": 1,
        "failure_domain": "local",
    })
}

fn entry(index: u64, reason: &str, mutation: ControlMutation) -> ControlLogEntry {
    ControlLogEntry {
        index,
        term: ControlTerm(1),
        actor: "controller".to_string(),
        reason: reason.to_string(),
        timestamp_unix_millis: 0,
        actor_sequence: 0,
        mutation,
    }
}

fn desired(index: u64, worker_nodes: u16) -> ControlLogEntry {
    entry(
        index,
        "scale",
        ControlMutation::DesiredCapacity(DesiredCapacity {
            revision: DesiredRevision(index),
            worker_nodes,
            gateway_nodes: 2,
            template_revision: "tmpl".to_string(),
        }),
    )
}

fn operation(index: u64, operation_id: &str) -> DriverOperation {
    DriverOperation {
        cluster_id: CLUSTER.to_string(),
        operation_id: operation_id.to_string(),
        control_term: ControlTerm(1),
        desired_revision: DesiredRevision(1),
        template_revision: "tmpl".to_string(),
        node_id: Some(format!("node-{index}")),
        state: DriverOperationState::Succeeded,
    }
}

fn record(key: &str, execution_node: &str) -> RecordEvidence {
    RecordEvidence {
        request_key: key.to_string(),
        handler: PRESSURE_HANDLER.to_string(),
        phase: "completed".to_string(),
        result: "succeeded".to_string(),
        owner_node: "worker2@worker2:4370".to_string(),
        execution_node: execution_node.to_string(),
        replica_nodes: vec!["worker1@worker1:4370".to_string()],
        json: json!({ "request_key": key }),
    }
}

impl FakeCluster {
    fn new(faults: Faults) -> Arc<Self> {
        Arc::new(FakeCluster {
            faults,
            start: Instant::now(),
            elapsed: Mutex::new(Duration::ZERO),
            commands: Mutex::new(Vec::new()),
            runtime_calls: AtomicUsize::new(0),
            continuity_calls: AtomicUsize::new(0),
            path_calls: AtomicUsize::new(0),
            spawns: AtomicUsize::new(0),
            keys: Mutex::new(Vec::new()),
            failed_over: AtomicBool::new(false),
        })
    }

    /// The managed containers running: three at the peak, one after the
    /// scale-down.
    fn running(&self) -> &'static str {
        if let Some(running) = self.faults.running {
            running
        } else if self.failed_over.load(Ordering::SeqCst) {
            "m1"
        } else {
            "m1\nm2\nm3"
        }
    }

    fn docker(&self, args: &str) -> String {
        if args.starts_with("version") {
            r#"{"Client":{}}"#.to_string()
        } else if args == "compose version --short" {
            "2.29.0".to_string()
        } else if args.ends_with(" config") {
            "services: {}".to_string()
        } else if args.starts_with("ps -a --filter") {
            r#"{"ID":"c1"}"#.to_string()
        } else if args.starts_with("ps -q") {
            self.running().to_string()
        } else if args.starts_with("ps -aq") {
            // Listing every container, running or not.
            if self.faults.no_managed {
                String::new()
            } else {
                self.running().to_string()
            }
        } else if let Some(ids) = args.strip_prefix("inspect ") {
            let containers: Vec<Value> = ids
                .split(' ')
                .enumerate()
                .map(|(index, _)| {
                    json!({"Config": {"Labels": {
                        "mesh.operation": format!("op{}", index + 1),
                        "mesh.managed": "true",
                        "mesh.cluster": CLUSTER,
                        "mesh.pool": "workers",
                        "mesh.template": "tmpl",
                        "mesh.term": "1",
                        "mesh.revision": "1",
                    }}})
                })
                .collect();
            Value::Array(containers).to_string()
        } else if args.ends_with("kill controller1") {
            self.failed_over.store(true, Ordering::SeqCst);
            String::new()
        } else if args.contains(" exec -T postgres") {
            "12".to_string()
        } else if args.contains(" logs --no-color") {
            "fault=docker_api_timeout_once fault=ensure_response_loss_once \
             fault=unhealthy_new_worker_once"
                .to_string()
        } else {
            String::new()
        }
    }

    /// A snapshot every wait accepts in the phase the cluster is in.
    fn snapshot(&self) -> OperatorRuntimeSnapshot {
        let failed_over = self.failed_over.load(Ordering::SeqCst);
        let scaled_down = failed_over && !self.faults.no_scale_down;
        let mut nodes = vec![
            node("controller1@controller1:4370", "controller", "ready", false),
            node("worker1@worker1:4370", "worker", "ready", true),
            node("worker2@worker2:4370", "worker", "ready", true),
        ];
        if scaled_down {
            if !self.faults.no_draining {
                nodes.push(node(
                    "managed-a@managed-a:4370",
                    "worker",
                    "draining",
                    false,
                ));
            }
        } else {
            nodes.extend([
                node("managed-a@managed-a:4370", "worker", "ready", true),
                node("managed-b@managed-b:4370", "worker", "ready", true),
            ]);
        }
        let workers = if scaled_down { 2 } else { 4 };
        let mut snapshot: OperatorRuntimeSnapshot = serde_json::from_value(json!({
            "schema_version": 1, "local_node": "controller1@controller1:4370",
            "telemetry_complete": true, "desired_capacity": workers,
            "observed_capacity": workers, "ready_capacity": workers,
            "draining_capacity": 0, "autoscaler_paused": false,
            "scheduler_min_workers": 1, "scheduler_max_workers": 4,
            "scheduler_active_workers": 2, "nodes": nodes,
        }))
        .expect("a scripted runtime snapshot");
        let mut entries = vec![desired(1, 4)];
        entries.extend(MANAGED.iter().enumerate().map(|(index, _)| {
            let id = format!("op{}", index + 1);
            entry(
                2 + index as u64,
                "ensure",
                ControlMutation::DriverOperation(operation(index as u64, &id)),
            )
        }));
        // An operator's override of the same capacity, the latest word on it.
        entries.push(entry(
            5,
            "override",
            ControlMutation::ManualOverride { worker_nodes: 4 },
        ));
        if scaled_down {
            entries.push(desired(5, 2));
            entries.push(entry(
                6,
                "drain",
                ControlMutation::DrainIntent {
                    node_id: "managed-a@managed-a:4370".to_string(),
                    cancelled: false,
                },
            ));
            entries.push(entry(
                7,
                "record failed managed worker cleanup result",
                ControlMutation::DriverOperation(operation(7, "cleanup")),
            ));
        }
        snapshot.consensus = Some(ConsensusRuntimeSnapshot {
            node_id: 1,
            node_name: "controller".to_string(),
            state: "leader".to_string(),
            current_term: if failed_over { 3 } else { 2 },
            current_leader: Some(1),
            last_applied_log: Some(entries.len() as u64),
            voter_ids: vec![1, 2, 3],
            entries,
        });
        snapshot.autonomous = AutonomousControllerStatus {
            configured: true,
            running: true,
            leader: true,
            last_reconcile: Some(CapacityReconcileOutcome {
                desired_workers: workers,
                observed_workers: workers,
                ensured: Vec::new(),
                drains: Vec::new(),
                constraints: Vec::new(),
            }),
            ..AutonomousControllerStatus::default()
        };
        snapshot
    }

    /// Whether the fault named by `fault` applies to the `calls`th query, and
    /// as an unready answer rather than an error.
    fn faulted(fault: Option<(usize, bool)>, calls: &AtomicUsize) -> Option<bool> {
        let call = calls.fetch_add(1, Ordering::SeqCst) + 1;
        fault
            .filter(|(from, _)| call >= *from)
            .map(|(_, unready)| unready)
    }
}

impl ProofWorld for FakeCluster {
    fn run(
        &self,
        program: &str,
        args: &[&str],
        _dir: &Path,
        _env: &BTreeMap<String, &str>,
    ) -> std::io::Result<Output> {
        let args = args.join(" ");
        let call = {
            let mut commands = self.commands.lock().unwrap();
            commands.push(format!("{program} {args}"));
            commands.len()
        };
        if self.faults.command == Some(call) {
            return Ok(output(1, "", "injected failure"));
        }
        Ok(match program {
            "docker" => output(0, &self.docker(&args), ""),
            "cargo" => output(
                0,
                r#"{"packages":[{"name":"meshc","version":"9.9.9"},{"name":"other"}]}"#,
                "",
            ),
            _ => output(0, "abc123", ""),
        })
    }

    fn runtime(
        &self,
        _target: &str,
        _timeout: Duration,
    ) -> Result<OperatorRuntimeSnapshot, String> {
        match Self::faulted(self.faults.runtime, &self.runtime_calls) {
            Some(false) => Err("injected runtime failure".to_string()),
            Some(true) => Ok(serde_json::from_value(json!({
                "schema_version": 1, "local_node": "", "telemetry_complete": false,
                "desired_capacity": 0, "observed_capacity": 0, "ready_capacity": 0,
                "draining_capacity": 0, "autoscaler_paused": false,
                "scheduler_min_workers": 0, "scheduler_max_workers": 0,
                "scheduler_active_workers": 0, "nodes": [],
            }))
            .expect("an unready snapshot")),
            None => Ok(self.snapshot()),
        }
    }

    fn continuity(&self, _target: &str, _timeout: Duration) -> Result<ContinuityEvidence, String> {
        match Self::faulted(self.faults.continuity, &self.continuity_calls) {
            Some(false) => Err("injected continuity failure".to_string()),
            Some(true) => Ok(ContinuityEvidence {
                total_records: 0,
                truncated: false,
                records: Vec::new(),
            }),
            None => {
                let mut records: Vec<_> = (0..13)
                    .map(|index| {
                        let node = match index {
                            0..8 => "worker2@worker2:4370",
                            8..10 => "worker1@worker1:4370",
                            _ => "managed-a@managed-a:4370",
                        };
                        record(&format!("seed-{index}"), node)
                    })
                    .collect();
                let keys = self.keys.lock().unwrap();
                records.extend(keys.iter().map(|key| record(key, "worker2@worker2:4370")));
                Ok(ContinuityEvidence {
                    total_records: records.len(),
                    truncated: false,
                    records,
                })
            }
        }
    }

    fn http(
        &self,
        _port: u16,
        method: &str,
        path: &str,
        _body: &str,
        _headers: &[(&str, &str)],
    ) -> Result<HttpResponse, String> {
        if let Some((faulty, after, status)) = self.faults.http {
            if path == faulty && self.path_calls.fetch_add(1, Ordering::SeqCst) >= after {
                if status == 0 {
                    return Err("injected connection failure".to_string());
                }
                return Ok(HttpResponse {
                    status,
                    headers: Vec::new(),
                    body: "injected".to_string(),
                });
            }
        }
        let mut headers = Vec::new();
        if path == "/proof/pressure" {
            // A request takes a moment, as the load's do.
            thread::sleep(Duration::from_millis(1));
            let key = {
                let mut keys = self.keys.lock().unwrap();
                let key = format!("request-{}", keys.len());
                keys.push(key.clone());
                key
            };
            headers = vec![
                ("X-Mesh-Continuity-Request-Key".to_string(), key),
                ("X-Mesh-Routed-Remotely".to_string(), "true".to_string()),
                (
                    "X-Mesh-Execution-Node".to_string(),
                    "worker2@worker2:4370".to_string(),
                ),
            ];
        }
        Ok(HttpResponse {
            status: if method == "POST" { 201 } else { 200 },
            headers,
            body: String::new(),
        })
    }

    fn spawn(
        &self,
        name: String,
        stack_size: usize,
        work: Box<dyn FnOnce() + Send>,
    ) -> std::io::Result<thread::JoinHandle<()>> {
        let spawn = self.spawns.fetch_add(1, Ordering::SeqCst) + 1;
        if self.faults.spawn == Some(spawn) {
            return Err(std::io::Error::other("injected spawn failure"));
        }
        LocalDocker.spawn(name, stack_size, work)
    }

    fn now(&self) -> Instant {
        self.start + *self.elapsed.lock().unwrap()
    }

    fn deadline(&self, timeout: Duration) -> Instant {
        self.now() + timeout
    }

    fn pause(&self, duration: Duration) {
        *self.elapsed.lock().unwrap() += duration;
    }
}

fn harness(world: Arc<FakeCluster>, evidence: &Path) -> ProofHarness {
    ProofHarness {
        world,
        root: PathBuf::from("/"),
        compose_file: PathBuf::from("/proof/docker-compose.yml"),
        coverage_dir: None,
        evidence: evidence.to_path_buf(),
        project: CLUSTER.to_string(),
        cluster_id: CLUSTER.to_string(),
        image: "proof:test".to_string(),
        driver_image: "driver:test".to_string(),
        keep_running: false,
        tls_ca_der_b64: "tls-ca".to_string(),
        tls_cert_der_b64: "tls-cert".to_string(),
        tls_key_der_b64: "tls-key".to_string(),
        driver_cert_der_b64: "driver-cert".to_string(),
        driver_key_der_b64: "driver-key".to_string(),
        driver_shared_key: "driver-shared".to_string(),
        identity_signing_key_der_b64: "identity-signing".to_string(),
        identity_verify_key_b64: "identity-verify".to_string(),
        identity_envelopes: BTreeMap::from([("OPERATOR".to_string(), "envelope".to_string())]),
        assertions: BTreeMap::new(),
        events: Vec::new(),
    }
}

/// A proof run: its result, its harness (the evidence directory lasts as
/// long as the run) and the cluster it ran against.
struct Run {
    result: Result<(), String>,
    harness: ProofHarness,
    cluster: Arc<FakeCluster>,
    _evidence: tempfile::TempDir,
}

/// The proof run against a cluster with `faults`, with the evidence files
/// named in `blocked` made unwritable.
fn prove(faults: Faults, blocked: &[&str]) -> Run {
    let evidence = tempfile::tempdir().unwrap();
    for name in blocked {
        fs::create_dir(evidence.path().join(name)).unwrap();
    }
    let cluster = FakeCluster::new(faults);
    let mut harness = harness(Arc::clone(&cluster), evidence.path());
    let result = run_proof(&mut harness, false, None);
    Run {
        result,
        harness,
        cluster,
        _evidence: evidence,
    }
}

#[test]
fn a_healthy_cluster_passes_every_step() {
    let evidence = tempfile::tempdir().unwrap();
    let cluster = FakeCluster::new(Faults::default());
    let mut harness = harness(Arc::clone(&cluster), evidence.path());
    let coverage = tempfile::tempdir().unwrap();
    harness.coverage_dir = Some(coverage.path().to_path_buf());
    assert_eq!(run_proof(&mut harness, false, None), Ok(()));
    let failed: Vec<_> = harness
        .assertions
        .iter()
        .filter(|(_, passed)| !**passed)
        .collect();
    assert!(failed.is_empty(), "{failed:?}");
    let commands = cluster.commands.lock().unwrap().join("\n");
    assert!(commands.contains("--target coverage-objects"), "{commands}");
    for file in [
        "capacity-draining.json",
        "continuity-final.json",
        "database-integrity.json",
    ] {
        assert!(evidence.path().join(file).is_file(), "{file}");
    }
}

#[test]
fn every_failing_command_stops_the_proof_where_it_fails() {
    let healthy = prove(Faults::default(), &[]);
    assert_eq!(healthy.result, Ok(()));
    let commands = healthy.cluster.commands.lock().unwrap().clone();
    for (index, command) in commands.iter().enumerate() {
        let faults = Faults {
            command: Some(index + 1),
            ..Faults::default()
        };
        let result = prove(faults, &[]).result;
        if command.starts_with("git ") {
            // The revision and worktree state are evidence, not steps.
            assert_eq!(result, Ok(()), "{command}");
            continue;
        }
        let error = result.expect_err(command);
        assert!(
            error.starts_with(&format!("proof_command_failed:{command}\n"))
                && error.ends_with("stderr=injected failure"),
            "{command}: {error}"
        );
    }
}

#[test]
fn every_evidence_file_that_cannot_be_written_stops_the_proof() {
    let healthy = prove(Faults::default(), &[]);
    assert_eq!(healthy.result, Ok(()));
    for file in fs::read_dir(&healthy.harness.evidence).unwrap() {
        let name = file.unwrap().file_name().into_string().unwrap();
        let error = prove(Faults::default(), &[&name]).result.unwrap_err();
        assert!(
            error.starts_with(&format!("proof_evidence_write_failed:{name}:")),
            "{name}: {error}"
        );
    }
}

#[test]
fn a_controller_that_stops_answering_fails_the_wait_it_was_in() {
    let queries = prove(Faults::default(), &[])
        .cluster
        .runtime_calls
        .load(Ordering::SeqCst);
    let mut waits = BTreeSet::new();
    for from in 1..=queries {
        for unready in [false, true] {
            let faults = Faults {
                runtime: Some((from, unready)),
                ..Faults::default()
            };
            let error = prove(faults, &[]).result.unwrap_err();
            assert!(error.contains("_timeout:"), "{from} {unready}: {error}");
            waits.insert(error.split(':').next().unwrap().to_string());
        }
    }
    for wait in [
        "proof_runtime_readiness_timeout",
        "proof_runtime_desired_timeout",
        "proof_runtime_stability_timeout",
        "proof_consensus_apply_timeout",
        "proof_managed_operation_commit_timeout",
        "proof_driver_restart_recovery_timeout",
        "proof_autonomous_leader_timeout",
        "proof_runtime_scale_down_timeout",
    ] {
        assert!(waits.contains(wait), "{wait} never timed out: {waits:?}");
    }
}

#[test]
fn a_continuity_list_that_stops_answering_fails_the_proof() {
    for (from, unready, expected) in [
        (
            1,
            false,
            "proof_routing_evidence_timeout:injected continuity failure",
        ),
        (1, true, "proof_routing_evidence_timeout:records=0"),
        (
            2,
            false,
            "proof_final_continuity_query_failed:injected continuity failure",
        ),
    ] {
        let faults = Faults {
            continuity: Some((from, unready)),
            ..Faults::default()
        };
        let error = prove(faults, &[]).result.unwrap_err();
        assert!(error.starts_with(expected), "{error}");
    }
}

#[test]
fn gateways_that_fail_fail_the_step_that_needs_them() {
    for (path, after, status, expected) in [
        (
            "/health",
            0,
            503,
            Err("proof_http_readiness_timeout:18081:status=503"),
        ),
        (
            "/health",
            0,
            0,
            Err("proof_http_readiness_timeout:18081:injected"),
        ),
        (
            "/todos",
            0,
            500,
            Err("proof_seed_mutation_failed:status=500"),
        ),
        (
            "/todos",
            0,
            503,
            Err("proof_seed_mutation_timeout:status=503"),
        ),
        ("/todos", 0, 0, Err("proof_seed_mutation_timeout:injected")),
        // The burst's own health checks, and every request under load:
        // reasons in the evidence, not an end to the proof.
        ("/health", 2, 503, Ok("gateway_18081_health_status_503")),
        ("/health", 2, 0, Ok("gateway_18081_health_failed")),
        ("/proof/pressure", 0, 500, Ok("http_status_500")),
        ("/proof/pressure", 0, 0, Ok("injected connection failure")),
    ] {
        let faults = Faults {
            http: Some((path, after, status)),
            ..Faults::default()
        };
        let run = prove(faults, &[]);
        match expected {
            Err(expected) => {
                let error = run.result.unwrap_err();
                assert!(error.starts_with(expected), "{path} {status}: {error}");
            }
            Ok(reason) => {
                assert_eq!(run.result, Ok(()), "{path} {status}");
                let burst =
                    fs::read_to_string(run.harness.evidence.join("concurrent-1000-summary.json"))
                        .unwrap();
                assert!(burst.contains(reason), "{path} {status}: {burst}");
            }
        }
    }
}

#[test]
fn a_thread_that_cannot_start_ends_the_burst() {
    for (spawn, error) in [
        (
            1,
            "proof_concurrent_thread_start_failed:injected spawn failure",
        ),
        (
            500,
            "proof_concurrent_thread_start_failed:injected spawn failure",
        ),
        (
            1_001,
            "proof_isolation_probe_thread_start_failed:injected spawn failure",
        ),
    ] {
        let faults = Faults {
            spawn: Some(spawn),
            ..Faults::default()
        };
        assert_eq!(prove(faults, &[]).result, Err(error.to_string()));
    }
}

#[test]
fn a_scale_down_that_never_ends_leaves_the_last_snapshot() {
    let faults = Faults {
        no_scale_down: true,
        ..Faults::default()
    };
    let run = prove(faults.clone(), &[]);
    let error = run.result.unwrap_err();
    assert!(
        error.starts_with("proof_runtime_scale_down_timeout:final_desired=2:desired=Some(4)"),
        "{error}"
    );
    assert!(run
        .harness
        .evidence
        .join("capacity-scale-down-timeout.json")
        .is_file());
    let error = prove(faults, &["capacity-scale-down-timeout.json"])
        .result
        .unwrap_err();
    assert!(
        error.starts_with("proof_evidence_write_failed:capacity-scale-down-timeout.json:"),
        "{error}"
    );
}

#[test]
fn a_scale_down_without_a_draining_node_has_no_drain_evidence() {
    let faults = Faults {
        no_draining: true,
        ..Faults::default()
    };
    let run = prove(faults, &[]);
    assert_eq!(run.result, Ok(()));
    assert!(!run.harness.evidence.join("capacity-draining.json").exists());
    assert!(!run.harness.assertions["draining_nodes_became_routing_ineligible"]);
}

#[test]
fn no_managed_container_is_no_committed_operation() {
    let faults = Faults {
        no_managed: true,
        ..Faults::default()
    };
    let run = prove(faults, &[]);
    let error = run.result.unwrap_err();
    assert!(
        error.starts_with("proof_managed_operation_commit_timeout:"),
        "{error}"
    );
    let inspected = fs::read_to_string(
        run.harness
            .evidence
            .join("managed-containers-peak-inspect.json"),
    )
    .unwrap();
    assert_eq!(inspected, "[]");
}

#[test]
fn managed_containers_that_never_come_or_go_time_out() {
    for (running, expected) in [
        ("", "proof_managed_worker_readiness_timeout"),
        (
            "m1\nm2\nm3",
            "proof_managed_worker_exact_count_timeout:expected=1:observed=3",
        ),
    ] {
        let faults = Faults {
            running: Some(running),
            ..Faults::default()
        };
        assert_eq!(prove(faults, &[]).result, Err(expected.to_string()));
    }
}

#[test]
fn a_start_only_run_writes_the_connection_manifest_and_nothing_over_it() {
    let evidence = tempfile::tempdir().unwrap();
    let manifest = evidence.path().join("connection.json");
    let mut start = harness(FakeCluster::new(Faults::default()), evidence.path());
    assert_eq!(run_proof(&mut start, true, Some(&manifest)), Ok(()));
    let written: Value = serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    assert_eq!(written["clusterId"], CLUSTER);
    // The cookie is written first, then the operator key.
    for (existing, expected) in [
        ("cookie", "proof_connection_cookie_open_failed:"),
        ("operator-key", "proof_connection_operator_key_open_failed:"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let manifest = directory.path().join("connection.json");
        fs::write(manifest.with_extension(existing), "").unwrap();
        let mut start = harness(FakeCluster::new(Faults::default()), directory.path());
        let error = run_proof(&mut start, true, Some(&manifest)).unwrap_err();
        assert!(error.starts_with(expected), "{error}");
    }
}

#[test]
fn logs_and_cleanup_follow_what_docker_reports() {
    let evidence = tempfile::tempdir().unwrap();
    // No managed container: nothing to inspect or remove.
    let empty = harness(
        FakeCluster::new(Faults {
            no_managed: true,
            ..Faults::default()
        }),
        evidence.path(),
    );
    empty.collect_logs();
    assert!(evidence.path().join("compose.log").is_file());
    assert!(!evidence
        .path()
        .join("managed-containers-inspect.json")
        .exists());
    assert_eq!(empty.cleanup(), Ok(()));
    // Docker failing to list the containers: no logs of theirs.
    let failing = harness(
        FakeCluster::new(Faults {
            command: Some(2),
            ..Faults::default()
        }),
        evidence.path(),
    );
    failing.collect_logs();
    assert!(!evidence
        .path()
        .join("managed-containers-inspect.json")
        .exists());
    // A removal failing, before `down` and of a container left after it.
    // Listing what is left after `down` failing, too.
    for (command, failed) in [
        (3, "docker rm -f m1 m2 m3"),
        (5, "docker ps -aq"),
        (6, "docker rm -f m1 m2 m3"),
    ] {
        let cleanup = harness(
            FakeCluster::new(Faults {
                command: Some(command),
                ..Faults::default()
            }),
            evidence.path(),
        );
        let error = cleanup.cleanup().unwrap_err();
        assert!(
            error.starts_with(&format!("proof_command_failed:{failed}")),
            "{command}: {error}"
        );
    }
}
