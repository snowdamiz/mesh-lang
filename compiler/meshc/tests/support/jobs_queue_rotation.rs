use super::{eventually, id, quoted, Proof};
use mesh_rt::db::pg::{native_pg_close, native_pg_connect, native_pg_query, NativePgConn};
use serde_json::{json, Value};
use std::fs;

const QUANTUM: usize = 16;
const BUSY_SLOTS: usize = QUANTUM + 1;
const BUSY_JOBS: usize = 20;
const SPARSE_LABEL: &str = "rotation-sparse";
const RUNNER: &str = "queue-rotation";

pub(super) fn prove(shared: &Proof<'_>) {
    let dir = shared.dir.join(RUNNER);
    fs::create_dir(&dir).unwrap();
    let proof = Proof::new(shared.url, shared.binary.clone(), &dir);
    let labels: Vec<_> = (0..BUSY_JOBS)
        .map(|index| format!("rotation-busy-{index}"))
        .collect();
    let busy = proof.enqueue(Value::Array(
        labels.iter().map(|label| job("busy", label, 0)).collect(),
    ));
    let busy = busy.as_array().unwrap();
    assert_eq!(busy.len(), BUSY_JOBS);
    let future = proof
        .sql("SELECT mesh_jobs.now_ms() + 3600000")
        .parse()
        .unwrap();
    let sparse = proof.enqueue(job("sparse", SPARSE_LABEL, future));
    assert_eq!(sparse["state"], "scheduled");
    for (queue, slots) in [("busy", BUSY_SLOTS), ("sparse", 1)] {
        let status = proof.control(json!({"action":"scale", "queue":queue, "concurrency":slots}));
        assert_eq!(status["state"], "active", "{status}");
        assert_eq!(status["concurrency"], slots, "{status}");
    }
    assert_eq!(busy_counts(&proof), [BUSY_JOBS, 0]);
    install_admission_trace(&proof, id(&busy[QUANTUM - 1]));

    let mut barrier = native_pg_connect(proof.url).unwrap();
    native_pg_query(
        &mut barrier,
        "SELECT pg_advisory_lock(hashtextextended($1, 0))",
        &[&proof.namespace],
    )
    .unwrap();
    let mut worker = proof.worker_with_config(
        RUNNER,
        true,
        json!({
            "namespace": proof.namespace,
            "queues": {"busy": BUSY_SLOTS, "sparse": 1},
            "workers": [{"worker": "proof", "version": 1}],
            "claim_batch": true, "listen": false, "event_buffer": 128,
            "poll_ms": 25, "lease_ms": 60_000, "shutdown_grace_ms": 8_000,
        }),
    );
    let claim_pid = eventually("first busy batch to reach the PostgreSQL barrier", || {
        worker.assert_running();
        blocked_claim(&mut barrier)
    });

    // The first busy transaction is still open. Commit the sparse schedule
    // change on this connection while it owns the session advisory lock.
    let due = native_pg_query(
        &mut barrier,
        "SELECT mesh_jobs.job_control($1, $2::uuid, 'reschedule', NULL, NULL)::text
         FROM mesh_jobs.jobs
         WHERE namespace = $1 AND id = $2::uuid AND state = 'scheduled'
           AND attempt = 0 AND scheduled_at_ms > mesh_jobs.now_ms()",
        &[&proof.namespace, id(&sparse)],
    )
    .unwrap();
    assert_eq!(
        due.len(),
        1,
        "sparse job must still be scheduled in the future"
    );
    let due: Value = serde_json::from_str(&due[0][0].1).unwrap();
    assert_eq!(due["state"], "available", "{due}");
    assert_eq!(due["attempt"], 0);
    let due_at = due["scheduled_at_ms"].as_i64().unwrap();
    assert_eq!(blocked_claim(&mut barrier), Some(claim_pid));
    native_pg_close(barrier);

    // The first busy quantum leaves one free busy slot. The sparse turn
    // must take place before that slot is filled. Handler completion is gated.
    let mut expected: Vec<_> = labels
        .iter()
        .zip(busy)
        .take(BUSY_SLOTS)
        .map(|(label, receipt)| (label.as_str(), receipt, "busy"))
        .collect();
    expected.insert(QUANTUM, (SPARSE_LABEL, &sparse, "sparse"));
    for &(label, receipt, queue) in &expected {
        let effects = proof.wait_effects(label, 1);
        let effect = &effects[0];
        assert_eq!(effect["process"], RUNNER);
        assert_eq!(effect["context"]["id"], receipt["id"]);
        assert_eq!(effect["context"]["queue"], queue);
        assert_eq!(effect["context"]["attempt"], 1);
        assert_eq!(effect["context"]["effect_key"], receipt["effect_key"]);
        let job = proof.job(label);
        assert_eq!(job["state"], "executing", "{job}");
        assert_eq!(job["attempt"], 1, "{job}");
    }
    let waiting = BUSY_JOBS - BUSY_SLOTS;
    assert_eq!(busy_counts(&proof), [waiting, BUSY_SLOTS]);

    let paused = worker.local_control(json!({"action":"pause", "queue":"busy"}));
    assert_eq!(paused["queues"]["busy"]["state"], "paused", "{paused}");
    assert_eq!(paused["queues"]["busy"]["in_flight"], BUSY_SLOTS);
    assert_eq!(paused["queues"]["sparse"]["in_flight"], 1);
    worker.release();
    for &(label, receipt, _) in &expected {
        let completed = proof.wait_state(label, "completed");
        assert_eq!(completed["id"], receipt["id"]);
        assert_eq!(completed["attempt"], 1);
        assert_eq!(proof.effects(label).len(), 1);
    }
    worker.signal();
    let stopped = worker.join();
    assert_eq!(stopped["state"], "stopped", "{stopped}");
    assert_eq!(stopped["drained"], true, "{stopped}");
    assert_eq!(stopped["events_dropped"], 0, "{stopped}");

    let trace: Vec<Value> = serde_json::from_str(&proof.sql(&format!(
        "SELECT jsonb_agg(to_jsonb(a) ORDER BY seq) FROM {}.rotation_admissions a",
        proof.namespace
    )))
    .unwrap();
    assert_eq!(trace.len(), expected.len(), "{trace:?}");
    for (admission, &(_, receipt, queue)) in trace.iter().zip(&expected) {
        assert_eq!(admission["job_id"], receipt["id"], "{trace:?}");
        assert_eq!(admission["queue"], queue, "{trace:?}");
        assert_eq!(admission["attempt"], 1, "{trace:?}");
    }
    let (first_busy, sparse_turn, next_busy) = (&trace[0], &trace[QUANTUM], &trace[QUANTUM + 1]);
    assert!(trace[..QUANTUM]
        .iter()
        .all(|row| row["claim_xid"] == first_busy["claim_xid"]));
    assert_ne!(first_busy["claim_xid"], sparse_turn["claim_xid"]);
    assert_ne!(first_busy["claim_xid"], next_busy["claim_xid"]);
    assert_ne!(sparse_turn["claim_xid"], next_busy["claim_xid"]);
    let claimed_at = sparse_turn["attempted_at_ms"].as_i64().unwrap();
    assert!(
        claimed_at >= due_at,
        "sparse claim at {claimed_at} precedes schedule {due_at}"
    );

    let starts: Vec<_> = worker
        .events()
        .into_iter()
        .filter(|event| event["event"] == "attempt_started")
        .collect();
    assert_eq!(starts.len(), expected.len(), "{starts:?}");
    for (start, &(_, receipt, _)) in starts.iter().zip(&expected) {
        assert_eq!(start["attempt"]["job_id"], receipt["id"], "{start}");
        assert_eq!(start["attempt"]["attempt_number"], 1, "{start}");
    }
    assert_eq!(starts[QUANTUM]["attempt"]["scheduled_at_ms"], due_at);

    // These same due rows were never admitted, so the busy backlog could
    // not have drained at any point during the rotation.
    for (label, receipt) in labels.iter().zip(busy).skip(BUSY_SLOTS) {
        let pending = proof.job(label);
        assert_eq!(pending["id"], receipt["id"]);
        assert_eq!(pending["state"], "available", "{pending}");
        assert_eq!(pending["attempt"], 0, "{pending}");
        assert!(proof.effects(label).is_empty());
    }
    assert_eq!(busy_counts(&proof), [waiting, 0]);
    proof.clean().unwrap();
    println!(
        "J2 lane 10 passed on PostgreSQL {}. Committed turns: {QUANTUM} busy, 1 sparse, 1 busy. \
         Sparse job {} due at {due_at} and claimed at {claimed_at} (PostgreSQL epoch ms). \
         All admitted jobs started and settled on attempt 1. {waiting} due busy jobs stayed unclaimed.",
        proof.sql("SHOW server_version"),
        id(&sparse),
    );
}

fn job(queue: &str, label: &str, scheduled_at_ms: u64) -> Value {
    json!({
        "worker": "proof", "queue": queue, "args": {"label": label},
        "scheduled_at_ms": scheduled_at_ms, "max_attempts": 1, "timeout_ms": 120_000,
    })
}

fn busy_counts(proof: &Proof<'_>) -> [usize; 2] {
    serde_json::from_str(&proof.sql(&format!(
        "SELECT json_build_array(
             count(*) FILTER (WHERE state = 'available' AND attempt = 0
                 AND scheduled_at_ms <= mesh_jobs.now_ms()),
             count(*) FILTER (WHERE state = 'executing'))
         FROM mesh_jobs.jobs WHERE namespace = {} AND queue = 'busy'
           AND worker = 'proof' AND version = 1",
        quoted(&proof.namespace)
    )))
    .unwrap()
}

fn install_admission_trace(proof: &Proof<'_>, last_in_quantum: &str) {
    let ns = &proof.namespace;
    let scope = quoted(ns);
    let last_in_quantum = quoted(last_in_quantum);
    // Writes roll back with the claim. The sequence orders both rows in
    // one claim and successive claims even when their timestamps coincide.
    proof.sql(&format!(
        "CREATE TABLE {ns}.rotation_admissions (
             seq bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
             job_id uuid, queue text, claim_xid text, attempt integer, attempted_at_ms bigint
         );
         CREATE FUNCTION {ns}.trace_admission() RETURNS trigger LANGUAGE plpgsql AS $$
         BEGIN
             IF NEW.id = {last_in_quantum}::uuid THEN
                 PERFORM pg_advisory_xact_lock(hashtextextended({scope}, 0));
             END IF;
             INSERT INTO {ns}.rotation_admissions (job_id, queue, claim_xid, attempt, attempted_at_ms)
             VALUES (NEW.id, NEW.queue, pg_current_xact_id()::text, NEW.attempt, NEW.attempted_at_ms);
             RETURN NEW;
         END;
         $$;
         CREATE TRIGGER {ns} AFTER UPDATE OF state ON mesh_jobs.jobs FOR EACH ROW
             WHEN (NEW.namespace = {scope} AND OLD.state <> 'executing' AND NEW.state = 'executing')
             EXECUTE FUNCTION {ns}.trace_admission();"
    ));
}

fn blocked_claim(barrier: &mut NativePgConn) -> Option<String> {
    let rows = native_pg_query(
        barrier,
        "SELECT pid::text FROM pg_stat_activity
         WHERE wait_event_type = 'Lock' AND wait_event = 'advisory'
           AND pg_backend_pid() = ANY(pg_blocking_pids(pid))
           AND query LIKE '%FROM mesh_jobs.claim_batch_v1(%'",
        &[],
    )
    .unwrap();
    assert!(
        rows.len() <= 1,
        "only one batch claim can be held: {rows:?}"
    );
    rows.into_iter().next().map(|mut row| row.remove(0).1)
}
