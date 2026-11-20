//! Compile a Mesh producer and runner against the public Jobs contract.
//! Receipts expose id, effect_key, and state. Handlers receive the same identity
//! plus the worker, version, queue, args, attempt, max_attempts, and metadata.
//! The proof uses a dedicated PostgreSQL database and real Unix process signals.
//! The proof uses SQL to submit a stale acknowledgement and to make a job due
//! while its runner executable cannot be launched.
//! The crash intentionally repeats a side effect with the same effect key.
#![cfg(unix)]

#[path = "support/test_artifacts.rs"]
mod artifacts;
#[path = "support/jobs_wakeups.rs"]
mod wakeups;

#[path = "support/durable_jobs_testing.rs"]
mod durable_jobs_testing;

use mesh_rt::db::pg::{
    native_pg_close, native_pg_connect, native_pg_execute, native_pg_query, NativePgConn,
};
use serde_json::{json, Value};
use std::fs::{self, File};
use std::os::unix::process::CommandExt;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const LEASE_MS: u64 = 1500;
const WAIT: Duration = Duration::from_secs(30);
const COMMITTED: &str =
    r#"{"worker":"proof","args":{"label":"committed"},"request_key":"commit-once"}"#;

#[test]
fn durable_jobs_survive_process_boundaries() {
    let compiler = artifacts::meshc_bin();
    let scratch = artifacts::repo_root().join(".tmp");
    fs::create_dir_all(&scratch).unwrap();
    let temp = tempfile::tempdir_in(scratch).expect("create durable jobs proof directory");
    let binary = build_fixture(&compiler, &temp.path().join("durable-jobs"), None);

    let Ok(url) = std::env::var("MESH_TEST_DATABASE_URL") else {
        eprintln!("Compiled the Jobs application. Set MESH_TEST_DATABASE_URL to a dedicated PostgreSQL database to exercise it, or run scripts/verify-postgres-jobs.sh.");
        return;
    };
    assert!(
        !url.is_empty(),
        "MESH_TEST_DATABASE_URL must name a dedicated test database"
    );
    let proof = Proof::new(&url, binary, temp.path());
    prove_transaction_and_restart(&proof);
    prove_competing_processes(&proof);
    prove_batch_admission(&proof);
    prove_busy_batch_connection_keeps_lease(&proof);
    prove_crash_and_stale_ack(&proof);
    prove_retry_and_exhaustion(&proof);
    prove_events(&proof);
    prove_native_timeout(&proof);
    prove_native_lease_loss(&proof);
    prove_locked_job_does_not_stall_runner(&proof);
    prove_large_context(&proof);
    #[cfg(target_os = "macos")]
    {
        prove_executable_replacement(&proof);
        prove_launch_failure(&proof);
    }
    prove_signal_drain(&proof);
    prove_join_waits_after_grace(&proof);
    wakeups::prove(&proof);
    assert_eq!(proof.effects("committed").len(), 1);
    assert!(proof.effects("rolled-back").is_empty());
    proof.clean().expect("delete durable jobs proof data");

    if let Some(runtime) = std::env::var_os("MESH_JOBS_REFERENCE_RT_LIB_PATH") {
        let project = temp.path().join("reference-jobs");
        let binary = build_fixture(&compiler, &project, Some(Path::new(&runtime)));
        let reference = Proof::new(&url, binary, &project);
        wakeups::prove_sparse_reference(&reference);
        reference.clean().expect("delete reference proof data");
    }
}

fn build_fixture(compiler: &Path, project: &Path, runtime: Option<&Path>) -> PathBuf {
    fs::create_dir(project).unwrap();
    fs::copy(
        artifacts::repo_root().join("tests/e2e/durable_jobs/main.mpl"),
        project.join("main.mpl"),
    )
    .unwrap();
    let mut command = Command::new(compiler);
    command
        .arg("build")
        .arg(project)
        .env_remove("MESH_RT_LIB_PATH");
    if let Some(runtime) = runtime {
        command.env("MESH_RT_LIB_PATH", runtime);
    }
    let build = command.output().unwrap();
    assert!(
        build.status.success(),
        "{}",
        artifacts::command_output_text(&build)
    );
    project.join(project.file_name().unwrap())
}

fn prove_transaction_and_restart(proof: &Proof) {
    let committed = proof.run("seed", "");
    assert_eq!(committed["state"], "available");
    assert_eq!(
        proof.sql(&format!("SELECT value FROM {}.business", proof.namespace)),
        "committed"
    );
    assert_eq!(proof.job("committed")["id"], committed["id"]);
    assert_eq!(proof.job("rolled-back"), Value::Null);
    assert!(proof.effects("committed").is_empty());
    assert_eq!(proof.run("enqueue", COMMITTED), committed);
    let listed = proof.control(json!({"action": "list", "queue": "default"}));
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert_eq!(listed[0]["id"], committed["id"]);
    assert_eq!(listed[0]["attempt"], 0);

    let mut worker = proof.worker("after-restart", false);
    let ready: Value =
        serde_json::from_slice(&fs::read(worker.markers.join("ready")).unwrap()).unwrap();
    assert!(ready.get("events_dropped").is_none(), "{ready}");
    proof.wait_state("committed", "completed");
    let finished = proof.control(json!({"action": "get", "id": id(&committed)}));
    assert_eq!(finished["state"], "completed");
    assert_eq!(finished["id"], committed["id"]);
    assert_eq!(finished["effect_key"], committed["effect_key"]);
    assert_eq!(finished["attempt"], 1);
    worker.stop();
    let effects = proof.effects("committed");
    assert_eq!(effects.len(), 1);
    assert_context(&effects[0], &committed, "committed", 1);
    assert_eq!(effects[0]["process"], "after-restart");
}

fn prove_events(proof: &Proof) {
    let mut worker = proof.worker_with_config(
        "events",
        true,
        json!({
            "namespace": proof.namespace,
            "queues": {"default": 1},
            "workers": [{"worker": "proof", "version": 1}],
            "lease_ms": 30000,
            "poll_ms": 25,
            "shutdown_grace_ms": 8000,
            "claim_batch": true,
            "event_buffer": 128,
            "event_log": true,
        }),
    );
    let label = "event-sensitive-argument";
    let metadata = "event-sensitive-metadata";
    let completed = proof.enqueue(json!({
        "worker": "proof", "args": {"label": label}, "metadata": {"secret": metadata},
    }));
    let started = worker.wait_event("attempt_started", &completed);
    assert_eq!(started["version"], 1);
    assert_eq!(started["attempt"]["attempt_number"], 1);
    assert!(started["attempt"]["queue_wait_ms"].is_number(), "{started}");
    proof.wait_effects(label, 1);
    assert_eq!(proof.job(label)["state"], "executing");
    assert!(worker
        .events()
        .iter()
        .all(|event| event["event"] != "settlement_committed"));
    let lease = proof.lease(id(&completed));
    worker.release();
    let settlement = worker.wait_event("settlement_committed", &completed);
    let durable = proof.job(label);
    assert_eq!(durable["state"], "completed");
    assert_eq!(settlement["settlement"], "completed");
    assert_eq!(settlement["attempt"], started["attempt"]);
    assert!(
        settlement["handler_duration_ms"].is_number(),
        "{settlement}"
    );

    let exhausted = proof.enqueue(json!({
        "worker": "proof", "args": {"label": "event-exhaustion"}, "max_attempts": 1,
    }));
    let settlement = worker.wait_event("settlement_committed", &exhausted);
    assert_eq!(settlement["settlement"], "discarded");
    assert_eq!(settlement["failure_kind"], "handler_failure");
    assert_eq!(proof.job("event-exhaustion")["state"], "discarded");

    fs::remove_file(worker.markers.join("gate")).unwrap();
    let cancelled = proof.enqueue(spec("event-cancel"));
    proof.wait_effects("event-cancel", 1);
    let mut cancellation = proof.lock_job(&cancelled);
    let rows = native_pg_query(
        &mut cancellation,
        "SELECT mesh_jobs.job_control($1, $2::uuid, 'cancel', NULL, NULL)::text",
        &[&proof.namespace, id(&cancelled)],
    )
    .unwrap();
    let request: Value = serde_json::from_str(&rows[0][0].1).unwrap();
    assert_eq!(request["state"], "executing");
    worker.release();
    eventually(
        "acknowledgement rejected while the cancellation holds the row",
        || {
            let events = worker.events();
            assert!(events
                .iter()
                .all(|event| event["event"] != "settlement_committed"
                    || event["attempt"]["job_id"] != id(&cancelled)));
            events.into_iter().find(|event| {
                event["event"] == "storage_failure"
                    && event["operation"] == "acknowledge"
                    && event["attempt"]["job_id"] == id(&cancelled)
            })
        },
    );
    native_pg_execute(&mut cancellation, "COMMIT", &[]).unwrap();
    native_pg_close(cancellation);
    let settlement = worker.wait_event("settlement_committed", &cancelled);
    assert_eq!(settlement["settlement"], "cancelled");
    assert!(settlement["submission"].as_u64().unwrap() > 1);
    assert_eq!(settlement["failure_kind"], Value::Null);
    assert_eq!(proof.job("event-cancel")["state"], "cancelled");

    // The stderr stream and the independent reader receive the same envelope.
    eventually("opt-in JSON settlement log", || {
        worker
            .log()
            .lines()
            .any(|line| {
                serde_json::from_str::<Value>(line).is_ok_and(|event| {
                    event["event"] == "settlement_committed"
                        && event["attempt"]["job_id"] == id(&completed)
                        && event["settlement"] == "completed"
                })
            })
            .then_some(())
    });
    worker.signal();
    let report = worker.join();
    assert_eq!(report["events_dropped"], 0);
    assert_eq!(report["event_logs_dropped"], 0);
    let batches = worker.event_batches();
    assert_eq!(batches.last().unwrap()["closed"], true);
    assert_eq!(worker.events().last().unwrap()["event"], "runner_stopped");
    let readback = fs::read_to_string(worker.markers.join("events")).unwrap();
    for private in [
        label,
        metadata,
        &lease.token,
        "proof worker requested a retry",
    ] {
        assert!(!readback.contains(private), "{readback}");
        assert!(!worker.log().contains(private));
    }
}

fn prove_competing_processes(proof: &Proof) {
    let scaled = proof.control(json!({"action": "scale", "queue": "default", "concurrency": 2}));
    assert_eq!(scaled["concurrency"], 2);
    let mut left = proof.worker("competitor-left", true);
    let mut right = proof.worker("competitor-right", true);
    let receipts = proof.enqueue(json!([spec("compete-first"), spec("compete-second")]));
    assert_eq!(receipts.as_array().unwrap().len(), 2);
    assert_ne!(id(&receipts[0]), id(&receipts[1]));

    let first_effect = proof.wait_effects("compete-first", 1);
    let second_effect = proof.wait_effects("compete-second", 1);
    assert_ne!(first_effect[0]["process"], second_effect[0]["process"]);
    for label in ["compete-first", "compete-second"] {
        assert_eq!(proof.job(label)["state"], "executing", "{label}");
    }
    left.release();
    right.release();
    for (index, label) in ["compete-first", "compete-second"].into_iter().enumerate() {
        let result = proof.wait_state(label, "completed");
        assert_eq!(result["attempt"], 1);
        let effects = proof.effects(label);
        assert_eq!(effects.len(), 1);
        assert_context(&effects[0], &receipts[index], label, 1);
    }
    left.stop();
    right.stop();
    proof.control(json!({"action": "scale", "queue": "default", "concurrency": 1}));
}

fn prove_batch_admission(proof: &Proof) {
    proof.control(json!({"action": "scale", "queue": "default", "concurrency": 3}));
    let labels = ["batch-first", "batch-second", "batch-third", "batch-fourth"];
    let config = json!({
            "namespace": proof.namespace,
            "queues": {"default": 4},
            "workers": [{"worker": "proof", "version": 1}],
            "lease_ms": LEASE_MS,
            "poll_ms": 25,
            "shutdown_grace_ms": 8000,
            "claim_batch": true,
    });
    let mut workers: Vec<_> = (0..4)
        .map(|index| {
            proof.worker_with_config(&format!("batch-admission-{index}"), true, config.clone())
        })
        .collect();
    proof.enqueue(Value::Array(
        labels.iter().map(|label| spec(label)).collect(),
    ));
    for label in &labels[..3] {
        proof.wait_effects(label, 1);
        assert_eq!(proof.job(label)["state"], "executing");
    }
    assert_eq!(proof.job(labels[3])["attempt"], 0);
    assert!(proof.effects(labels[3]).is_empty());

    for worker in &workers {
        worker.release();
    }
    for label in labels {
        proof.wait_state(label, "completed");
        assert_eq!(proof.job(label)["attempt"], 1);
        assert_eq!(proof.effects(label).len(), 1);
    }
    for worker in &mut workers {
        worker.stop();
    }
    proof.control(json!({"action": "scale", "queue": "default", "concurrency": 1}));
}

fn prove_busy_batch_connection_keeps_lease(proof: &Proof) {
    proof.control(json!({"action": "scale", "queue": "default", "concurrency": 2}));
    let label = "separate-batch-session";
    let receipt = proof.enqueue(json!({
        "worker":"proof", "args":{"label":label}, "max_attempts":1
    }));
    let mut worker = proof.worker_with_config(
        "blocked-batch-claim",
        true,
        json!({
            "namespace": proof.namespace,
            "queues": {"default": 2},
            "workers": [{"worker": "proof", "version": 1}],
            "lease_ms": LEASE_MS,
            "poll_ms": 25,
            "shutdown_grace_ms": 8000,
            "claim_batch": true,
        }),
    );
    proof.wait_effects(label, 1);

    let mut lock = native_pg_connect(proof.url).unwrap();
    native_pg_execute(&mut lock, "BEGIN", &[]).unwrap();
    native_pg_execute(
        &mut lock,
        "LOCK TABLE mesh_jobs.queues IN ACCESS EXCLUSIVE MODE",
        &[],
    )
    .unwrap();
    eventually(
        "the dedicated claim query to wait for its table lock",
        || {
            let waiting = proof.sql(
                "SELECT EXISTS (SELECT FROM pg_stat_activity WHERE datname=current_database()
                AND wait_event_type='Lock' AND query LIKE '%FROM mesh_jobs.claim_batch_v1%')",
            );
            (waiting == "t").then_some(())
        },
    );
    let before = proof.lease(id(&receipt));
    eventually(
        "the active job to renew while the claim query waits",
        || {
            let after = proof.lease(id(&receipt));
            assert_eq!(before.token, after.token);
            (after.expires_at_ms > before.expires_at_ms).then_some(())
        },
    );
    worker.release();
    assert_eq!(proof.wait_state(label, "completed")["attempt"], 1);
    native_pg_close(lock);
    worker.stop();
    assert_eq!(proof.effects(label).len(), 1);
    proof.control(json!({"action": "scale", "queue": "default", "concurrency": 1}));
}

fn prove_crash_and_stale_ack(proof: &Proof) {
    let receipt = proof.enqueue(json!({
        "worker": "proof", "args": {"label": "crash"}, "max_attempts": 3,
    }));
    let mut doomed = proof.worker("before-kill", true);
    proof.wait_effects("crash", 1);
    doomed.kill();
    let old = proof.lease(id(&receipt));
    assert_eq!(proof.job("crash")["state"], "executing");

    let mut recovery = proof.worker("after-kill", true);
    let effects = proof.wait_effects("crash", 2);
    let fresh = proof.lease(id(&receipt));
    assert_ne!(
        old.token, fresh.token,
        "reclaimed lease must have a new token"
    );
    assert!(
        fresh.attempted_at_ms >= old.expires_at_ms,
        "{old:?}, {fresh:?}"
    );
    assert_eq!(effects[0]["process"], "before-kill");
    assert_eq!(effects[1]["process"], "after-kill");
    assert_context(&effects[0], &receipt, "crash", 1);
    assert_context(&effects[1], &receipt, "crash", 2);

    // Submit the old token directly because the public API never exposes it.
    assert_eq!(
        proof.sql(&format!(
            "SELECT mesh_jobs.settle('{}', {}::uuid, {}::uuid, '{{\"action\":\"discard\",\"reason\":\"stale owner\"}}', false)",
            proof.namespace,
            quoted(id(&receipt)),
            quoted(&old.token)
        )),
        "f"
    );
    assert_eq!(proof.lease(id(&receipt)).token, fresh.token);
    assert_eq!(proof.job("crash")["state"], "executing");

    recovery.release();
    let finished = proof.wait_state("crash", "completed");
    assert_eq!(finished["id"], receipt["id"]);
    assert_eq!(finished["attempt"], 2);
    assert_eq!(finished["errors"][0]["attempt"], 1);
    assert_eq!(finished["errors"].as_array().unwrap().len(), 1);
    recovery.stop();
    assert_eq!(proof.effects("crash").len(), 2);
}

fn prove_retry_and_exhaustion(proof: &Proof) {
    let receipts = proof.enqueue(json!([
        {"worker": "proof", "args": {"label": "retry"}, "max_attempts": 2},
        {"worker": "proof", "args": {"label": "poison"}, "max_attempts": 2,
         "backoff_base_ms": 600, "backoff_max_ms": 600},
        {"worker": "proof", "args": {"label": "healthy"}, "delay_ms": 100},
        {"worker": "proof", "args": {"label": "panic"}, "max_attempts": 1},
    ]));
    let mut worker = proof.worker("retry-worker", false);
    let panicked = proof.wait_state("panic", "discarded");
    let poison = proof.wait_state("poison", "discarded");
    let retry = proof.wait_state("retry", "completed");
    let healthy = proof.wait_state("healthy", "completed");
    worker.signal();
    let report = worker.join();
    assert_eq!(report["state"], "stopped", "{report}");
    assert_eq!(report["finished"], 6, "{report}");
    assert_eq!(
        report["outcomes"],
        json!({
            "completed": 2, "cancelled": 0, "discarded": 2,
            "retryable": 2, "scheduled": 0, "released_unstarted": 0
        }),
        "{report}"
    );
    assert_eq!(panicked["id"], receipts[3]["id"]);
    assert_eq!(panicked["attempt"], 1);
    assert!(panicked["errors"][0]["reason"]
        .as_str()
        .unwrap()
        .contains("proof worker panicked"));

    for (index, (label, job, delay, reason)) in [
        ("retry", retry, 400, "proof worker requested a retry"),
        ("poison", poison, 600, "proof worker rejected poison"),
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(job["id"], receipts[index]["id"]);
        assert_eq!(job["attempt"], 2);
        assert_eq!(job["errors"].as_array().unwrap().len(), index + 1);
        assert_eq!(job["errors"][0]["reason"], reason);
        assert_eq!(job["errors"][0]["attempt"], 1);
        let scheduled = job["errors"][0]["at_ms"].as_i64().unwrap() + delay;
        assert_eq!(job["scheduled_at_ms"], scheduled);
        assert!(job["attempted_at_ms"].as_i64().unwrap() >= scheduled);
        let effects = proof.effects(label);
        assert_eq!(effects.len(), 2);
        for (attempt, effect) in effects.iter().enumerate() {
            assert_context(effect, &receipts[index], label, attempt as u64 + 1);
            assert_eq!(effect["context"]["max_attempts"], 2);
        }
    }

    assert_eq!(healthy["id"], receipts[2]["id"]);
    assert_eq!(healthy["attempt"], 1);
    assert_eq!(receipts[2]["state"], "scheduled");
    assert!(
        healthy["attempted_at_ms"].as_i64().unwrap()
            >= healthy["scheduled_at_ms"].as_i64().unwrap()
    );
    let effects = proof.effects("healthy");
    assert_eq!(effects.len(), 1);
    assert_context(&effects[0], &receipts[2], "healthy", 1);
    assert!(
        effects[0]["sequence"].as_u64().unwrap()
            < proof.effects("poison")[1]["sequence"].as_u64().unwrap()
    );
}

fn prove_signal_drain(proof: &Proof) {
    let receipt = proof.enqueue(spec("graceful"));
    let mut worker = proof.worker("signal-worker", true);
    proof.wait_effects("graceful", 1);
    let pending = proof.enqueue(spec("after-signal"));
    worker.signal_group("-TERM");
    eventually("Mesh application received SIGTERM", || {
        worker.assert_running();
        worker.markers.join("signal").is_file().then_some(())
    });
    assert_eq!(proof.job("graceful")["state"], "executing");
    worker.release();
    let report = worker.join();
    assert_eq!(report["state"], "stopped", "{report}");
    assert_eq!(report["drained"], true, "{report}");
    assert_eq!(report["finished"], 1, "{report}");
    assert_eq!(
        proof.wait_state("graceful", "completed")["id"],
        receipt["id"]
    );
    let effects = proof.effects("graceful");
    assert_eq!(effects.len(), 1);
    assert_context(&effects[0], &receipt, "graceful", 1);
    let pending_job = proof.control(json!({"action": "get", "id": id(&pending)}));
    assert_eq!(pending_job["state"], "available");
    assert_eq!(pending_job["attempt"], 0);
    assert!(proof.effects("after-signal").is_empty());
}

fn prove_native_timeout(proof: &Proof) {
    let receipt = proof.enqueue(json!({
        "worker": "proof", "args": {"label": "native-timeout"},
        "timeout_ms": 3000, "max_attempts": 2, "backoff_base_ms": 0,
    }));
    let mut worker = proof.worker("native-timeout", false);
    proof.wait_effects("native-timeout", 1);
    wait_for_native_query(proof, "native-timeout", true);
    let job = proof.wait_state("native-timeout", "completed");
    assert_eq!(job["id"], receipt["id"]);
    assert_eq!(job["attempt"], 2);
    assert_eq!(job["errors"][0]["reason"], "job handler timed out");
    assert_eq!(proof.effects("native-timeout").len(), 2);
    cancel_native_query(proof, "native-timeout");
    wait_for_native_query(proof, "native-timeout", false);
    assert_eq!(
        proof.effects("after-timeout"),
        Vec::<Value>::new(),
        "timed-out handler published a side effect after returning from native I/O"
    );
    worker.stop();
}

fn prove_native_lease_loss(proof: &Proof) {
    let receipt = proof.enqueue(json!({
        "worker": "proof", "args": {"label": "native-owner-paused"},
        "timeout_ms": 60_000, "max_attempts": 2, "backoff_base_ms": 0,
    }));
    let mut owner = proof.worker("paused-owner", false);
    proof.wait_effects("native-owner-paused", 1);
    wait_for_native_query(proof, "native-owner-paused", true);
    owner.send_signal("-STOP");

    let mut recovery = proof.worker("native-recovery", false);
    let job = proof.wait_state("native-owner-paused", "completed");
    assert_eq!(job["attempt"], 2);
    assert_eq!(job["errors"][0]["reason"], "lease expired");
    let effects = proof.effects("native-owner-paused");
    assert_eq!(effects.len(), 2);
    assert_context(&effects[0], &receipt, "native-owner-paused", 1);
    assert_context(&effects[1], &receipt, "native-owner-paused", 2);
    assert_eq!(effects[1]["process"], "native-recovery");
    owner.send_signal("-CONT");
    cancel_native_query(proof, "native-owner-paused");
    wait_for_native_query(proof, "native-owner-paused", false);
    owner.stop();
    recovery.stop();
    assert!(proof.effects("after-timeout").is_empty());
}

fn wait_for_native_query(proof: &Proof, label: &str, active: bool) {
    eventually(
        &format!("{label} native query to have active={active}"),
        || {
            let result = proof.sql(&format!(
            "SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE application_name = {} AND state = 'active' AND query = 'SELECT pg_sleep(60)')",
            quoted(label),
        ));
            ((result == "t") == active).then_some(())
        },
    );
}

fn prove_locked_job_does_not_stall_runner(proof: &Proof) {
    proof.control(json!({"action":"scale","queue":"default","concurrency":2}));
    let mut worker = proof.worker_with_options(
        "independent-leases",
        true,
        json!({"default":2}),
        8000,
        LEASE_MS,
    );
    let held = proof.enqueue(spec("locked-attempt"));
    proof.wait_effects("locked-attempt", 1);
    let healthy = proof.enqueue(json!({
        "worker": "proof", "args": {"label": "independent-attempt"}, "max_attempts": 1
    }));
    proof.wait_effects("independent-attempt", 1);

    let lock = proof.lock_job(&held);
    let job = proof.job("independent-attempt");
    let before = job["lease_expires_at_ms"].as_i64().unwrap();
    eventually(
        "the independent job to renew while its peer is locked",
        || {
            let job = proof.job("independent-attempt");
            assert_eq!(job["state"], "executing", "{job}");
            assert_eq!(job["id"], healthy["id"], "{job}");
            assert_eq!(job["attempt"], 1, "{job}");
            let after = job["lease_expires_at_ms"].as_i64().unwrap();
            (after >= before + LEASE_MS as i64).then_some(())
        },
    );
    worker.release();
    assert_eq!(
        proof.wait_state("independent-attempt", "completed")["attempt"],
        1
    );
    assert_eq!(proof.effects("independent-attempt").len(), 1);

    native_pg_close(lock);
    proof.wait_state("locked-attempt", "completed");
    worker.stop();
}

fn prove_join_waits_after_grace(proof: &Proof) {
    let jobs = proof.enqueue(json!([
        {"worker":"proof","queue":"shutdown","args":{"label":"join-active"},"max_attempts":1},
        {"worker":"proof","queue":"shutdown","args":{"label":"join-pending"}}
    ]));
    let mut worker = proof.worker_with_options(
        "zero-shutdown-grace",
        true,
        json!({"shutdown":1}),
        0,
        60_000,
    );
    proof.wait_effects("join-active", 1);
    let lock = proof.lock_job(&jobs[0]);

    worker.signal();
    let stop: Value = eventually("zero-grace stop report", || {
        fs::read(worker.markers.join("stop"))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
    });
    assert_eq!(stop["state"], "stopping", "{stop}");
    assert_eq!(stop["undrained_jobs"], json!([jobs[0]["id"]]), "{stop}");
    thread::sleep(Duration::from_millis(100));
    assert!(
        !worker.markers.join("joined").exists(),
        "join returned while its job row was locked"
    );
    worker.assert_running();
    native_pg_close(lock);

    let report = worker.join();
    assert_eq!(report["state"], "stopped", "{report}");
    assert_eq!(report["drained"], true, "{report}");
    assert_eq!(report["outcomes"]["discarded"], 1, "{report}");
    let settled = proof.wait_state("join-active", "discarded");
    assert_eq!(
        settled["errors"][0]["reason"],
        "job runner shutdown deadline elapsed"
    );
    assert_eq!(proof.job("join-pending")["attempt"], 0);
}

fn cancel_native_query(proof: &Proof, label: &str) {
    proof.sql(&format!(
        "SELECT pg_cancel_backend(pid) FROM pg_stat_activity
         WHERE application_name = {} AND state = 'active' AND query = 'SELECT pg_sleep(60)'",
        quoted(label),
    ));
}

fn prove_large_context(proof: &Proof) {
    let request = proof.dir.join("large-enqueue.json");
    let spec = json!({
        "worker": "proof", "args": {"label": "large-context", "numbers": vec![1e308_f64; 30_000]},
        "max_attempts": 1,
    });
    fs::write(&request, spec.to_string()).unwrap();
    let receipt = proof.run("enqueue_file", request.to_str().unwrap());
    let mut worker = proof.worker("large-payload", false);
    eventually("job with large PostgreSQL context completes", || {
        let state = proof.sql(&format!(
            "SELECT state FROM mesh_jobs.jobs WHERE namespace = {} AND id = {}::uuid",
            quoted(&proof.namespace),
            quoted(id(&receipt)),
        ));
        assert_ne!(state, "discarded");
        (state == "completed").then_some(())
    });
    let observed = proof.sql(&format!(
        "SELECT count(*) FROM {}.effects WHERE label = 'large-context' AND octet_length(context::text) > 8 * 1024 * 1024",
        proof.namespace,
    ));
    assert_eq!(observed, "1");
    worker.stop();
}

#[cfg(target_os = "macos")]
fn prove_executable_replacement(proof: &Proof) {
    let mut old = proof.worker("replaced-runner", false);
    let replacement = proof.binary.with_extension("replacement");
    fs::copy(&proof.binary, &replacement).unwrap();
    fs::rename(replacement, &proof.binary).unwrap();
    proof.enqueue(spec("after-replacement"));
    eventually("runner stops after its executable was replaced", || {
        let report: Value =
            serde_json::from_slice(&fs::read(old.markers.join("status")).ok()?).ok()?;
        (report["state"] == "stopped").then_some(())
    });
    old.signal();
    let report = old.join();
    assert_eq!(report["claimed"], 0);
    assert!(report["last_error"]
        .as_str()
        .unwrap()
        .contains("executable changed"));
    let pending = proof.job("after-replacement");
    assert_eq!(pending["attempt"], 0);
    assert_eq!(pending["state"], "available");
    let mut new = proof.worker("replacement-runner", false);
    assert_eq!(
        proof.wait_state("after-replacement", "completed")["attempt"],
        1
    );
    new.stop();
}

#[cfg(target_os = "macos")]
fn prove_launch_failure(proof: &Proof) {
    use std::os::unix::fs::PermissionsExt;

    let spec = json!({
        "worker": "proof", "args": {"label": "launch-failure"},
        "metadata": {"reference": "launch-proof"},
        "unique": {"fields": ["worker", "metadata"], "states": "incomplete"},
        "delay_ms": 120_000, "max_attempts": 1,
    });
    let receipt = proof.enqueue(spec.clone());
    let mut old = proof.worker("launch-failure", false);
    let mode = fs::metadata(&proof.binary).unwrap().permissions().mode();
    fs::set_permissions(&proof.binary, fs::Permissions::from_mode(mode & !0o111)).unwrap();
    proof.sql(&format!(
        "UPDATE mesh_jobs.jobs SET scheduled_at_ms = mesh_jobs.now_ms() WHERE namespace = {} AND id = {}::uuid",
        quoted(&proof.namespace), quoted(id(&receipt)),
    ));
    eventually("runner stops after the denied launch", || {
        let report: Value =
            serde_json::from_slice(&fs::read(old.markers.join("status")).ok()?).ok()?;
        (report["state"] == "stopped").then_some(())
    });
    fs::set_permissions(&proof.binary, fs::Permissions::from_mode(mode)).unwrap();
    old.signal();
    let report = old.join();
    assert_eq!(report["claimed"], 1);
    assert_eq!(report["outcomes"]["released_unstarted"], 1, "{report}");
    assert!(report["last_error"]
        .as_str()
        .unwrap()
        .contains("could not start job handler"));
    let pending = proof.job("launch-failure");
    assert_eq!(pending["attempt"], 0);
    assert_eq!(pending["metadata"], spec["metadata"]);
    let replay = proof.enqueue(spec);
    assert_eq!(replay["id"], receipt["id"]);
    assert_eq!(replay["conflict"], true);

    let mut worker = proof.worker("launch-recovery", false);
    assert_eq!(
        proof.wait_state("launch-failure", "completed")["attempt"],
        1
    );
    worker.stop();
    assert_eq!(proof.effects("launch-failure").len(), 1);
}

fn spec(label: &str) -> Value {
    json!({"worker": "proof", "args": {"label": label}})
}

fn id(receipt: &Value) -> &str {
    receipt["id"]
        .as_str()
        .expect("enqueue receipt must have a job ID")
}

fn assert_context(effect: &Value, receipt: &Value, label: &str, attempt: u64) {
    let context = &effect["context"];
    assert_eq!(context["id"], id(receipt), "{context}");
    assert!(receipt["effect_key"]
        .as_str()
        .is_some_and(|key| !key.is_empty()));
    assert_eq!(context["effect_key"], receipt["effect_key"], "{context}");
    assert_eq!(context["worker"], "proof");
    assert_eq!(context["version"], 1);
    assert_eq!(context["queue"], "default");
    assert_eq!(context["args"], json!({"label": label}));
    assert_eq!(context["attempt"], attempt);
    assert!(context["max_attempts"]
        .as_u64()
        .is_some_and(|max| max >= attempt));
    assert!(context["metadata"].is_object());
}

fn quoted(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn eventually<T>(description: &str, mut probe: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + WAIT;
    loop {
        if let Some(result) = probe() {
            return result;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {description}"
        );
        thread::sleep(Duration::from_millis(25));
    }
}

#[derive(Debug, serde::Deserialize)]
struct Lease {
    token: String,
    expires_at_ms: i64,
    attempted_at_ms: i64,
}

struct Proof<'a> {
    url: &'a str,
    binary: PathBuf,
    dir: &'a Path,
    namespace: String,
    cleaned: std::cell::Cell<bool>,
}

impl<'a> Proof<'a> {
    fn new(url: &'a str, binary: PathBuf, dir: &'a Path) -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let namespace = format!("mesh_jobs_proof_{}_{stamp}", std::process::id());
        let proof = Self {
            url,
            binary,
            dir,
            namespace,
            cleaned: std::cell::Cell::new(true),
        };
        let database = proof.sql("SELECT current_database()");
        assert!(
            !matches!(database.as_str(), "postgres" | "template0" | "template1"),
            "MESH_TEST_DATABASE_URL must point to a dedicated test database"
        );
        assert!(proof.sql("SHOW server_version_num").parse::<u32>().unwrap() >= 140_000);
        let ns = &proof.namespace;
        proof.sql(&format!(
            "CREATE SCHEMA {ns};
             CREATE TABLE {ns}.business (value text PRIMARY KEY);
             CREATE TABLE {ns}.effects (
                 seq bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
                 worker text NOT NULL, label text NOT NULL, context jsonb NOT NULL
             );"
        ));
        proof.cleaned.set(false);
        proof
    }

    fn sql_result(&self, sql: &str) -> Result<String, String> {
        let output = Command::new("psql")
            .args(["-XAtw", "-v", "ON_ERROR_STOP=1", "-c", sql])
            .arg("--dbname")
            .arg(self.url)
            .env_remove("PGHOSTADDR")
            .env_remove("PGSERVICE")
            .env("PGCONNECT_TIMEOUT", "5")
            .env(
                "PGOPTIONS",
                "-c statement_timeout=5000 -c lock_timeout=3000",
            )
            .output()
            .map_err(|error| format!("cannot invoke psql: {error}"))?;
        if output.status.success() {
            Ok(String::from_utf8(output.stdout).unwrap().trim().to_owned())
        } else {
            Err(format!(
                "psql failed for {sql}:\n{}",
                artifacts::command_output_text(&output)
            ))
        }
    }

    fn sql(&self, sql: &str) -> String {
        self.sql_result(sql)
            .unwrap_or_else(|error| panic!("{error}"))
    }

    fn job(&self, label: &str) -> Value {
        serde_json::from_str(&self.sql(&format!(
            "SELECT COALESCE((SELECT mesh_jobs.job_json(j) FROM mesh_jobs.jobs j
                WHERE namespace='{}' AND args->>'label'={}), 'null'::jsonb)",
            self.namespace,
            quoted(label)
        )))
        .unwrap()
    }

    fn wait_state(&self, label: &str, state: &str) -> Value {
        eventually(&format!("{label} to become {state}"), || {
            let job = self.job(label);
            assert!(
                !matches!(
                    job["state"].as_str(),
                    Some("completed" | "discarded" | "cancelled")
                ) || job["state"] == state,
                "{label} unexpectedly reached {job}"
            );
            (job["state"] == state).then_some(job)
        })
    }

    fn lease(&self, id: &str) -> Lease {
        serde_json::from_str(&self.sql(&format!(
            "SELECT jsonb_build_object('token', lease_token, 'expires_at_ms', lease_expires_at_ms,
                'attempted_at_ms', attempted_at_ms)
             FROM mesh_jobs.jobs WHERE namespace='{}' AND id={}::uuid",
            self.namespace,
            quoted(id)
        )))
        .unwrap()
    }

    fn effects(&self, label: &str) -> Vec<Value> {
        serde_json::from_str(&self.sql(&format!(
            "SELECT COALESCE(jsonb_agg(jsonb_build_object('sequence', seq, 'process', worker, 'context', context)
                ORDER BY seq), '[]'::jsonb) FROM {}.effects WHERE label={}",
            self.namespace,
            quoted(label)
        ))).unwrap()
    }

    fn wait_effects(&self, label: &str, count: usize) -> Vec<Value> {
        eventually(&format!("{count} application effects for {label}"), || {
            let effects = self.effects(label);
            assert!(
                effects.len() <= count,
                "extra effects for {label}: {effects:?}"
            );
            (effects.len() == count).then_some(effects)
        })
    }

    fn command(&self, operation: &str, payload: &str) -> Command {
        let mut command = Command::new(&self.binary);
        command
            .env("MESH_TEST_DATABASE_URL", self.url)
            .env("MESH_JOBS_PROOF_NAMESPACE", &self.namespace)
            .env("MESH_JOBS_PROOF_COMMAND", operation)
            .env("MESH_JOBS_PROOF_PAYLOAD", payload)
            .env_remove("MESH_JOBS_PROOF_GATE")
            .env_remove("MESH_JOBS_PROOF_EVENTS")
            .env_remove("MESH_JOBS_PROOF_PUBLISH");
        command
    }

    fn run(&self, operation: &str, payload: &str) -> Value {
        self.run_command(operation, &mut self.command(operation, payload))
    }

    fn run_command(&self, operation: &str, command: &mut Command) -> Value {
        let child = command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let output = artifacts::wait_with_timeout(child, WAIT)
            .unwrap_or_else(|error| panic!("{operation}: {error}"));
        assert!(
            output.status.success(),
            "{operation}: {}",
            artifacts::command_output_text(&output)
        );
        serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
            panic!(
                "{operation}: invalid JSON ({error}): {}",
                artifacts::command_output_text(&output)
            )
        })
    }

    fn enqueue(&self, spec: Value) -> Value {
        self.run("enqueue", &spec.to_string())
    }

    fn control(&self, action: Value) -> Value {
        self.run("control", &action.to_string())
    }

    fn lock_job(&self, receipt: &Value) -> NativePgConn {
        let mut lock = native_pg_connect(self.url).unwrap();
        native_pg_execute(&mut lock, "BEGIN", &[]).unwrap();
        let rows = native_pg_query(
            &mut lock,
            "SELECT 1 FROM mesh_jobs.jobs WHERE namespace = $1 AND id = $2::uuid FOR NO KEY UPDATE",
            &[&self.namespace, id(receipt)],
        )
        .unwrap();
        assert_eq!(rows.len(), 1);
        lock
    }

    fn worker(&self, name: &str, blocked: bool) -> Worker {
        self.worker_with_options(name, blocked, json!({"default":1}), 8000, LEASE_MS)
    }

    fn worker_with_options(
        &self,
        name: &str,
        blocked: bool,
        queues: Value,
        grace_ms: u32,
        lease_ms: u64,
    ) -> Worker {
        let config = json!({
            "namespace": self.namespace,
            "queues": queues,
            "workers": [{"worker": "proof", "version": 1}],
            "lease_ms": lease_ms,
            "poll_ms": 25,
            "shutdown_grace_ms": grace_ms,
        });
        self.worker_with_config(name, blocked, config)
    }

    fn worker_with_config(&self, name: &str, blocked: bool, config: Value) -> Worker {
        self.worker_with_url(name, blocked, config, self.url)
    }

    fn worker_with_url(&self, name: &str, blocked: bool, config: Value, url: &str) -> Worker {
        let markers = self.dir.join(name);
        fs::create_dir(&markers).unwrap();
        let output = File::create(markers.join("output")).unwrap();
        let mut command = self.command("serve", &config.to_string());
        command
            .process_group(0)
            .env("MESH_TEST_DATABASE_URL", url)
            .env("MESH_JOBS_PROOF_WORKER", name)
            .env("MESH_JOBS_PROOF_MARKERS", &markers)
            .stdout(output.try_clone().unwrap())
            .stderr(output);
        if config["event_buffer"].is_number() {
            command.env("MESH_JOBS_PROOF_EVENTS", "true");
        }
        if blocked {
            command.env("MESH_JOBS_PROOF_GATE", markers.join("gate"));
        }
        let mut worker = Worker {
            child: Some(command.spawn().unwrap()),
            markers,
        };
        eventually(&format!("{name} to report ready"), || {
            worker.assert_running();
            fs::read(worker.markers.join("ready"))
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                .map(|report| {
                    assert_eq!(report["state"], "running");
                    assert_eq!(report["namespace"], self.namespace);
                })
        });
        worker
    }

    fn clean(&self) -> Result<(), String> {
        if self.cleaned.get() {
            return Ok(());
        }
        let ns = &self.namespace;
        self.sql_result(&format!(
            "DO $$ BEGIN
               IF to_regclass('mesh_jobs.jobs') IS NOT NULL THEN
                 DELETE FROM mesh_jobs.jobs WHERE namespace='{ns}';
                 DELETE FROM mesh_jobs.receipts WHERE namespace='{ns}';
                 DELETE FROM mesh_jobs.queues WHERE namespace='{ns}';
                 DELETE FROM mesh_jobs.admissions WHERE namespace='{ns}';
               END IF;
             END $$;
             DROP SCHEMA {ns} CASCADE;"
        ))?;
        self.cleaned.set(true);
        Ok(())
    }
}

impl Drop for Proof<'_> {
    fn drop(&mut self) {
        if let Err(error) = self.clean() {
            eprintln!(
                "could not clean proof namespace {}: {error}",
                self.namespace
            );
        }
    }
}

struct Worker {
    child: Option<Child>,
    markers: PathBuf,
}

impl Worker {
    fn log(&self) -> String {
        fs::read_to_string(self.markers.join("output")).unwrap_or_default()
    }

    fn event_batches(&self) -> Vec<Value> {
        fs::read_to_string(self.markers.join("events"))
            .unwrap_or_default()
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect()
    }

    fn events(&self) -> Vec<Value> {
        self.event_batches()
            .into_iter()
            .flat_map(|mut batch| batch["events"].take().as_array().cloned().unwrap())
            .collect()
    }

    fn wait_event(&self, kind: &str, receipt: &Value) -> Value {
        eventually(kind, || {
            self.events()
                .into_iter()
                .find(|event| event["event"] == kind && event["attempt"]["job_id"] == id(receipt))
        })
    }

    fn assert_running(&mut self) {
        if let Some(status) = self.child.as_mut().unwrap().try_wait().unwrap() {
            panic!(
                "worker {:?} exited early ({status}):\n{}",
                self.markers,
                self.log()
            );
        }
    }

    fn release(&self) {
        File::create(self.markers.join("gate")).unwrap();
    }

    fn signal(&mut self) {
        self.send_signal("-TERM");
    }

    fn send_signal(&mut self, signal: &str) {
        self.assert_running();
        let pid = self.child.as_ref().unwrap().id();
        assert!(
            Command::new("kill")
                .args([signal, &pid.to_string()])
                .status()
                .unwrap()
                .success(),
            "could not signal worker {pid}"
        );
    }

    fn signal_group(&mut self, signal: &str) {
        self.assert_running();
        let pid = self.child.as_ref().unwrap().id();
        assert!(Command::new("kill")
            .args([signal, "--", &format!("-{pid}")])
            .status()
            .unwrap()
            .success());
    }

    fn join(&mut self) -> Value {
        let status: ExitStatus = eventually(&format!("{:?} to stop", self.markers), || {
            self.child.as_mut().unwrap().try_wait().unwrap()
        });
        self.child.take();
        let log = self.log();
        assert!(
            status.success(),
            "worker {:?} failed ({status}):\n{log}",
            self.markers
        );
        log.lines()
            .rev()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .find(|value| value.get("state").is_some())
            .unwrap_or_else(|| panic!("worker did not print a JSON shutdown report:\n{log}"))
    }

    fn stop(&mut self) {
        self.signal();
        let report = self.join();
        assert_eq!(report["state"], "stopped", "{report}");
        assert_eq!(report["drained"], true, "{report}");
    }

    fn kill(&mut self) {
        let mut child = self.child.take().unwrap();
        child.kill().unwrap();
        assert_eq!(child.wait().unwrap().signal(), Some(9));
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
            if thread::panicking() {
                eprintln!("worker {:?}:\n{}", self.markers, self.log());
            }
        }
    }
}
