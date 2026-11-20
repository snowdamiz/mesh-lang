use super::{assert_job_context, eventually, Proof, Worker};
use mesh_rt::db::pg::native_pg_close;
use serde_json::{json, Value};
use std::fs;

const TARGET: &str = "target";
const WITNESS: &str = "witness";

pub(super) fn prove(shared: &Proof<'_>) {
    for (mode, batch) in [("single", false), ("batch", true)] {
        let dir = shared.dir.join(format!("queue-local-grace-{mode}"));
        fs::create_dir(&dir).unwrap();
        let proof = Proof::new(shared.url, shared.binary.clone(), &dir);
        prove_grace_expiry(&proof, batch);
        proof.clean().unwrap();
        println!("J6 queue-local grace expiry passed ({mode})");

        let dir = shared.dir.join(format!("queue-local-claim-{mode}"));
        fs::create_dir(&dir).unwrap();
        let proof = Proof::new(shared.url, shared.binary.clone(), &dir);
        prove_late_claim(&proof, batch);
        proof.clean().unwrap();
        println!("J6 queue-local late claim refund passed ({mode})");

        let dir = shared.dir.join(format!("queue-local-replacement-{mode}"));
        fs::create_dir(&dir).unwrap();
        let proof = Proof::new(shared.url, shared.binary.clone(), &dir);
        prove_replacement(&proof, batch);
        proof.clean().unwrap();
        println!("J6 queue-local process replacement passed ({mode})");
    }
}

fn config(proof: &Proof<'_>, queues: Value, batch: bool) -> Value {
    json!({
        "namespace": proof.namespace,
        "queues": queues,
        "workers": [{"worker":"proof", "version":1}],
        "claim_batch": batch, "event_buffer": 128, "listen": false,
        "poll_ms": 25, "lease_ms": 60_000, "shutdown_grace_ms": 8_000,
        "cron_interval_ms": 0,
    })
}

fn enqueue(proof: &Proof<'_>, queue: &str, label: &str, priority: u8) -> Value {
    proof.enqueue(json!({
        "worker":"proof", "queue":queue, "args":{"label":label},
        "priority":priority, "max_attempts":1, "timeout_ms":120_000,
    }))
}

fn stopped_queue(worker: &mut Worker) -> Value {
    eventually("the target queue to stop and drain", || {
        worker.assert_running();
        let report = worker.status()?;
        (report["queues"][TARGET]["state"] == "stopped").then(|| {
            assert_eq!(report["state"], "running", "{report}");
            assert_eq!(report["queues"][TARGET]["drained"], true, "{report}");
            assert_eq!(report["queues"][TARGET]["in_flight"], 0, "{report}");
            assert_eq!(report["queues"][TARGET]["pending_claims"], 0, "{report}");
            report
        })
    })
}

fn prove_grace_expiry(proof: &Proof<'_>, batch: bool) {
    let held = "grace-held";
    let pending = "grace-pending";
    let receipt = enqueue(proof, TARGET, held, 0);
    let mut worker = proof.worker_with_environment(
        "grace",
        true,
        config(proof, json!({(TARGET):1, (WITNESS):1}), batch),
        proof.url,
        &[("MESH_JOBS_PROOF_GATE_LABEL", held)],
    );
    let effects = proof.wait_effects(held, 1);
    assert_job_context(&effects[0], &receipt, TARGET, held, 1);
    assert_eq!(proof.job(held)["state"], "executing");
    enqueue(proof, TARGET, pending, 1);
    let queued = proof.job(pending);
    assert_eq!(queued["state"], "available", "{queued}");
    assert_eq!(queued["attempt"], 0, "{queued}");

    let stopping = worker.local_control(json!({"action":"stop", "queue":TARGET, "grace_ms":250}));
    assert_eq!(
        stopping["queues"][TARGET]["state"], "stopping",
        "{stopping}"
    );
    assert_eq!(stopping["queues"][TARGET]["in_flight"], 1, "{stopping}");
    let stopped = stopped_queue(&mut worker);
    assert_eq!(stopped["outcomes"]["discarded"], 1, "{stopped}");
    assert_eq!(stopped["outcomes"]["released_unstarted"], 0, "{stopped}");
    assert_eq!(stopped["queues"][WITNESS]["state"], "running", "{stopped}");
    let discarded = proof.wait_state(held, "discarded");
    assert_eq!(discarded["id"], receipt["id"], "{discarded}");
    assert_eq!(
        discarded["effect_key"], receipt["effect_key"],
        "{discarded}"
    );
    assert_eq!(discarded["attempt"], 1, "{discarded}");
    assert_eq!(
        discarded["errors"][0]["reason"],
        "job queue shutdown deadline elapsed"
    );
    assert!(!worker.markers.join("gate").exists());

    let witness = enqueue(proof, WITNESS, "grace-witness", 0);
    assert_completed(proof, &witness, WITNESS, "grace-witness", "grace");
    worker.stop();
    assert_eq!(proof.job(pending), queued);
    assert!(proof.effects(pending).is_empty());
    assert_eq!(proof.effects(held), effects);
}

fn prove_late_claim(proof: &Proof<'_>, batch: bool) {
    let held = "claim-held";
    let pending = "claim-pending";
    let receipt = enqueue(proof, TARGET, held, 0);
    let pending_receipt = enqueue(proof, TARGET, pending, 1);
    let before = proof.job(held);
    let queued = proof.job(pending);
    assert_eq!(before["state"], "available", "{before}");
    assert_eq!(before["attempt"], 0, "{before}");
    assert_eq!(queued["state"], "available", "{queued}");
    assert_eq!(queued["attempt"], 0, "{queued}");
    let config = config(proof, json!({(TARGET):2}), batch);
    // The batch path reserves two local slots. Only one row can be admitted.
    assert_eq!(
        proof.control(json!({"action":"scale", "queue":TARGET, "concurrency":1}))["concurrency"],
        1
    );
    let (barrier, blocker) = proof.hold_claim(&receipt);
    let mut worker = proof.worker_with_config("claim", false, config.clone());
    eventually("the compiled claim to reach the PostgreSQL gate", || {
        worker.assert_running();
        proof.claim_is_waiting(blocker).then_some(())
    });

    let stopping = worker.local_control(json!({"action":"stop", "queue":TARGET, "grace_ms":0}));
    assert_eq!(stopping["state"], "running", "{stopping}");
    assert_eq!(
        stopping["queues"][TARGET]["state"], "stopping",
        "{stopping}"
    );
    assert_eq!(stopping["queues"][TARGET]["in_flight"], 0, "{stopping}");
    assert_eq!(
        stopping["queues"][TARGET]["pending_claims"],
        if batch { 2 } else { 1 },
        "{stopping}"
    );
    assert_eq!(stopping["queues"][TARGET]["drained"], false, "{stopping}");
    assert_eq!(stopping["claimed"], 0, "{stopping}");
    assert_eq!(proof.job(held), before);
    assert!(proof.claim_is_waiting(blocker));
    native_pg_close(barrier);

    let stopped = stopped_queue(&mut worker);
    assert_eq!(stopped["claimed"], 1, "{stopped}");
    assert_eq!(stopped["finished"], 1, "{stopped}");
    assert_eq!(stopped["outcomes"]["released_unstarted"], 1, "{stopped}");
    worker.signal();
    let joined = worker.join();
    assert_eq!(joined["drained"], true, "{joined}");
    assert_eq!(joined["events_dropped"], 0, "{joined}");
    let events = worker.events();
    assert!(
        events
            .iter()
            .all(|event| event["event"] != "attempt_started"),
        "{events:?}"
    );
    let settlements: Vec<_> = events
        .iter()
        .filter(|event| event["event"] == "settlement_committed")
        .collect();
    assert_eq!(settlements.len(), 1, "{events:?}");
    let refund = settlements[0];
    assert_eq!(refund["attempt"]["job_id"], receipt["id"], "{refund}");
    assert_eq!(refund["attempt"]["attempt_number"], 1, "{refund}");
    assert_eq!(refund["settlement"], "released_unstarted", "{refund}");
    assert_eq!(refund["failure_kind"], "shutdown", "{refund}");
    let refunded = proof.job(held);
    for field in [
        "id",
        "effect_key",
        "state",
        "attempt",
        "max_attempts",
        "errors",
        "lease_expires_at_ms",
    ] {
        assert_eq!(refunded[field], before[field], "{field}: {refunded}");
    }
    assert_eq!(proof.job(pending), queued);
    assert!(proof.effects(held).is_empty());
    assert!(proof.effects(pending).is_empty());

    let mut replay = proof.worker_with_config("claim-replay", false, config);
    assert_completed(proof, &receipt, TARGET, held, "claim-replay");
    assert_completed(proof, &pending_receipt, TARGET, pending, "claim-replay");
    replay.stop();
}

fn prove_replacement(proof: &Proof<'_>, batch: bool) {
    let config = config(proof, json!({(TARGET):3, (WITNESS):2}), batch);
    for (queue, limit) in [(TARGET, 3), (WITNESS, 2)] {
        let status =
            proof.control(json!({"action":"configure", "queue":queue, "concurrency":limit}));
        assert_eq!(status["concurrency"], limit, "{status}");
    }
    let mut original = proof.worker_with_config("original", false, config.clone());
    let paused = proof.control(json!({"action":"pause", "queue":TARGET}));
    assert_eq!(paused["state"], "paused", "{paused}");
    let scaled = proof.control(json!({"action":"scale", "queue":TARGET, "concurrency":2}));
    assert_eq!(scaled["state"], "paused", "{scaled}");
    assert_eq!(scaled["concurrency"], 2, "{scaled}");
    let local_scale = original.local_control(json!({"action":"scale", "queue":TARGET, "limit":1}));
    assert_eq!(local_scale["queues"][TARGET]["limit"], 1, "{local_scale}");
    original.local_control(json!({"action":"pause", "queue":WITNESS}));
    let stopped = original.local_control(json!({"action":"stop", "queue":TARGET}));
    assert_eq!(stopped["queues"][WITNESS]["state"], "paused", "{stopped}");
    assert_eq!(stopped_queue(&mut original)["queues"][TARGET]["limit"], 1);
    let jobs = [
        (0, "replacement-first"),
        (1, "replacement-second"),
        (2, "replacement-pending"),
    ]
    .map(|(priority, label)| (label, enqueue(proof, TARGET, label, priority)));
    original.stop();

    let mut replacement = proof.worker_with_config("replacement", true, config);
    let ready: Value =
        serde_json::from_slice(&fs::read(replacement.markers.join("ready")).unwrap()).unwrap();
    assert_eq!(ready["queues"][TARGET]["state"], "running", "{ready}");
    assert_eq!(ready["queues"][TARGET]["limit"], 3, "{ready}");
    assert_eq!(ready["queues"][WITNESS]["state"], "running", "{ready}");
    let persisted = proof.control(json!({"action":"get", "queue":TARGET}));
    assert_eq!(persisted["state"], "paused", "{persisted}");
    assert_eq!(persisted["concurrency"], 2, "{persisted}");
    let paused_witness = "replacement-paused-witness";
    let paused_receipt = enqueue(proof, WITNESS, paused_witness, 0);
    proof.wait_effects(paused_witness, 1);
    for (label, receipt) in &jobs {
        let job = proof.job(label);
        assert_eq!(job["id"], receipt["id"], "{job}");
        assert_eq!(job["state"], "available", "{job}");
        assert_eq!(job["attempt"], 0, "{job}");
        assert!(proof.effects(label).is_empty());
    }

    let resumed = proof.control(json!({"action":"resume", "queue":TARGET}));
    assert_eq!(resumed["state"], "active", "{resumed}");
    assert_eq!(resumed["concurrency"], 2, "{resumed}");
    for (label, _) in &jobs[..2] {
        proof.wait_effects(label, 1);
    }
    let capped_witness = "replacement-capped-witness";
    let capped_receipt = enqueue(proof, WITNESS, capped_witness, 0);
    proof.wait_effects(capped_witness, 1);
    let counts = proof.control(json!({"action":"queue_stats", "queue":TARGET}));
    assert_eq!(
        counts["counts"],
        json!({"executing":2, "available":1}),
        "{counts}"
    );
    let (pending, pending_receipt) = &jobs[2];
    assert_eq!(proof.job(pending)["attempt"], 0);
    assert!(proof.effects(pending).is_empty());

    replacement.release();
    for (label, receipt) in &jobs {
        assert_completed(proof, receipt, TARGET, label, "replacement");
    }
    for (label, receipt) in [
        (paused_witness, paused_receipt),
        (capped_witness, capped_receipt),
    ] {
        assert_completed(proof, &receipt, WITNESS, label, "replacement");
    }
    replacement.signal();
    let joined = replacement.join();
    assert_eq!(joined["drained"], true, "{joined}");
    assert_eq!(joined["events_dropped"], 0, "{joined}");
    let events = replacement.events();
    let next_start = events
        .iter()
        .position(|event| {
            event["event"] == "attempt_started"
                && event["attempt"]["job_id"] == pending_receipt["id"]
        })
        .expect("the pending job must start after capacity frees up");
    assert!(
        events[..next_start].iter().any(|event| {
            event["event"] == "settlement_committed"
                && event["settlement"] == "completed"
                && jobs[..2]
                    .iter()
                    .any(|(_, receipt)| event["attempt"]["job_id"] == receipt["id"])
        }),
        "{events:?}"
    );
}

fn assert_completed(proof: &Proof<'_>, receipt: &Value, queue: &str, label: &str, process: &str) {
    let completed = proof.wait_state(label, "completed");
    assert_eq!(completed["id"], receipt["id"], "{completed}");
    assert_eq!(completed["attempt"], 1, "{completed}");
    let effects = proof.effects(label);
    assert_eq!(effects.len(), 1, "{effects:?}");
    assert_eq!(effects[0]["process"], process);
    assert_job_context(&effects[0], receipt, queue, label, 1);
}
