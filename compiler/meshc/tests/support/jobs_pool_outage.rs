use super::wakeups::{await_subscription, wake_status, Relay};
use super::{eventually, id, quoted, Proof, Worker};
use mesh_rt::db::pg::{native_pg_close, native_pg_query};
use serde_json::{json, Value};
use std::fs::{self, File};
use std::time::{Duration, Instant};

const GRACE_MS: u64 = 40;
const JOIN_BOUND: Duration = Duration::from_secs(10); // 5 s checkout plus 5 s Jobs I/O
const ONE_SLOT: &[(&str, &str)] = &[("MESH_JOBS_PROOF_ONE_SLOT", "true")];

pub(super) fn prove(shared: &Proof<'_>) {
    let dir = shared.dir.join("pool-outage");
    fs::create_dir(&dir).unwrap();
    let proof = Proof::new(shared.url, shared.binary.clone(), &dir);
    let relay = Relay::new(proof.url);
    let runner_url = relay.as_ref().map_or(proof.url, |(_, url)| url.as_str());
    let mut worker = proof.worker_with_environment(
        "exhausted-pool",
        true,
        json!({
            "namespace":proof.namespace, "queues":{"exhausted":1},
            "workers":[{"worker":"proof","version":1}], "poll_ms":25,
            "lease_ms":60_000, "shutdown_grace_ms":GRACE_MS,
            "listen":relay.is_some(), "event_buffer":128,
            "recovery_interval_ms":100, "cron_interval_ms":100
        }),
        runner_url,
        ONE_SLOT,
    );
    if relay.is_some() {
        await_subscription(&worker, 0);
    }
    let active = proof.enqueue(json!({
        "worker":"proof", "queue":"exhausted", "args":{"label":"exhausted-active"},
        "max_attempts":1, "timeout_ms":120_000
    }));
    assert_eq!(
        proof.wait_effects("exhausted-active", 1)[0]["context"]["id"],
        active["id"]
    );
    File::create(worker.markers.join("pool-hold")).unwrap();
    let checkout_pid = eventually(
        "fixture to hold its sole pool connection in PostgreSQL",
        || {
            worker.assert_running();
            worker.markers.join("pool-held").exists().then_some(())?;
            held_checkout(&proof)
        },
    );
    let listener = relay.as_ref().map(|(relay, _)| {
        let gate = relay.interrupt_listener(&proof);
        wake_status(&worker, |wake| wake["subscribed"] == false);
        gate
    });

    let pending = proof.enqueue(json!({
        "worker":"proof", "queue":"exhausted", "args":{"label":"exhausted-pending"}
    }));
    let orphan = proof.enqueue(json!({
        "worker":"proof", "queue":"maintenance", "args":{"label":"exhausted-orphan"},
        "max_attempts":3
    }));
    let mut owner =
        proof.worker_with_options("lease-owner", true, json!({"maintenance":1}), 8_000, 3_000);
    proof.wait_effects("exhausted-orphan", 1);
    owner.kill();
    let lease = proof.lease(id(&orphan));
    let occurrence = stage_cron(&proof);

    eventually(
        "recovery and cron to time out waiting on the held checkout",
        || {
            worker.assert_running();
            let events = worker.events();
            ["recover", "cron"]
                .iter()
                .all(|op| {
                    events
                        .iter()
                        .any(|e| e["event"] == "storage_failure" && e["operation"] == *op)
                })
                .then_some(())?;
            (worker.status()?["last_error"] == "pool checkout timeout").then_some(())
        },
    );
    eventually(
        "the orphaned lease to expire on the PostgreSQL clock",
        || {
            (proof.sql(&format!(
                "SELECT mesh_jobs.now_ms() >= {}",
                lease.expires_at_ms
            )) == "t")
                .then_some(())
        },
    );
    assert_eq!(held_checkout(&proof), Some(checkout_pid));
    assert_eq!(proof.job("exhausted-orphan")["state"], "executing");
    assert_eq!(proof.lease(id(&orphan)).token, lease.token);
    assert_eq!(cron_cursor(&proof), occurrence - 1);
    assert!(proof.job("exhausted-cron").is_null());

    // A final maintenance iteration may already be waiting for the checkout.
    // Fence these two rows until that runner joins so the peer proves recovery.
    let mut maintenance = proof.lock_job(&orphan);
    let locks = native_pg_query(
        &mut maintenance,
        "SELECT pg_try_advisory_xact_lock(hashtextextended($1, 1461749648))::text",
        &[&proof.namespace],
    )
    .unwrap();
    assert_eq!(locks[0][0].1, "true");

    let (stop, join) = stop_while_held(&proof, &mut worker, &active);
    drop(listener);
    let settled = proof.wait_state("exhausted-active", "discarded");
    assert_eq!(
        settled["errors"][0]["reason"],
        "job runner shutdown deadline elapsed"
    );
    assert_eq!(proof.effects("exhausted-active").len(), 1);

    assert_eq!(
        proof.control(json!({"action":"pause", "queue":"exhausted"}))["state"],
        "paused"
    );
    // Keep the peer on one checkout as well. With its queue paused, the
    // listener reconnect is the only new socket that can reach the relay.
    let mut peer = proof.worker_with_environment(
        "exhausted-pool-peer",
        false,
        json!({
            "namespace":proof.namespace, "queues":{"exhausted":1},
            "workers":[{"worker":"proof","version":1}], "poll_ms":100,
            "shutdown_grace_ms":8_000, "listen":relay.is_some(),
            "recovery_interval_ms":100, "cron_interval_ms":100
        }),
        runner_url,
        ONE_SLOT,
    );
    let listener = relay.as_ref().map(|(relay, _)| {
        await_subscription(&peer, 0);
        let gate = relay.interrupt_listener(&proof);
        let wakeups = wake_status(&peer, |wake| wake["subscribed"] == false);
        (gate, wakeups["fallback_scans"].as_u64().unwrap())
    });
    let waiting = proof.job("exhausted-pending");
    assert_eq!(waiting["state"], "available");
    assert_eq!(waiting["attempt"], 0);
    assert_eq!(
        proof.control(json!({"action":"resume", "queue":"exhausted"}))["state"],
        "active"
    );
    let polled = proof.wait_state("exhausted-pending", "completed");
    assert_eq!(polled["id"], pending["id"]);
    assert_eq!(polled["attempt"], 1);
    assert_eq!(
        proof.effects("exhausted-pending")[0]["process"],
        "exhausted-pool-peer"
    );
    if let Some((_, scans)) = &listener {
        let wakeups = wake_status(&peer, |wake| {
            wake["fallback_scans"].as_u64().unwrap() > *scans
        });
        assert_eq!(wakeups["subscriptions"], 1);
        assert_eq!(wakeups["subscribed"], false);
        assert_eq!(wakeups["notification_scans"], 0);
    }

    assert_eq!(proof.job("exhausted-orphan")["state"], "executing");
    assert_eq!(cron_cursor(&proof), occurrence - 1);
    native_pg_close(maintenance);
    let reclaimed = eventually("maintenance to recover the expired lease", || {
        let job = proof.job("exhausted-orphan");
        matches!(job["state"].as_str(), Some("retryable" | "available")).then_some(job)
    });
    assert_eq!(reclaimed["attempt"], 1);
    assert_eq!(reclaimed["errors"][0]["reason"], "lease expired");
    assert_eq!(reclaimed["max_attempts"], 3);
    let cron = proof.wait_state("exhausted-cron", "available");
    assert_eq!(cron["scheduled_at_ms"], occurrence * 60_000);
    let cursor = cron_cursor(&proof);
    assert!(cursor >= occurrence, "cron cursor stayed at {cursor}");

    let started = peer.local_control(json!({"action":"start", "queue":"maintenance"}));
    assert_eq!(
        started["queues"]["maintenance"]["state"], "running",
        "{started}"
    );
    let recovered = proof.wait_state("exhausted-orphan", "completed");
    assert_eq!(recovered["id"], orphan["id"]);
    assert_eq!(recovered["attempt"], 2);
    assert!(recovered["attempted_at_ms"].as_i64().unwrap() >= lease.expires_at_ms);
    let effects = proof.effects("exhausted-orphan");
    assert_eq!(effects.len(), 2);
    assert_eq!(effects[0]["process"], "lease-owner");
    assert_eq!(effects[1]["process"], "exhausted-pool-peer");
    for (index, effect) in effects.iter().enumerate() {
        assert_eq!(effect["context"]["id"], orphan["id"]);
        assert_eq!(effect["context"]["effect_key"], orphan["effect_key"]);
        assert_eq!(effect["context"]["attempt"], index + 1);
    }
    let scheduled = proof.wait_state("exhausted-cron", "completed");
    assert_eq!(scheduled["id"], cron["id"]);
    assert_eq!(scheduled["attempt"], 1);
    peer.stop();
    assert_eq!(proof.effects("exhausted-pending").len(), 1);
    assert_eq!(proof.effects("exhausted-cron").len(), 1);
    proof.clean().unwrap();

    if listener.is_some() {
        println!(
            "J7 lane 8 passed on PostgreSQL {}, stop {stop:?} (grace {GRACE_MS} ms), join after checkout release {join:?}, fallback poll attempt 1, expired lease recovered on attempt 2, cron cursor {} -> {cursor}",
            proof.sql("SHOW server_version"),
            occurrence - 1
        );
    } else {
        println!(
            "J7 lane 8 partial: stop {stop:?}, join after checkout release {join:?}, poll-only lease and cron recovery passed. Use a loopback PostgreSQL URL to exercise listener interruption."
        );
    }
}

fn held_checkout(proof: &Proof<'_>) -> Option<u32> {
    proof
        .sql(&format!(
            "SELECT pid FROM pg_stat_activity WHERE datname = current_database()
             AND application_name = {} AND state = 'idle in transaction'",
            quoted(&proof.namespace)
        ))
        .parse()
        .ok()
}

fn stage_cron(proof: &Proof<'_>) -> i64 {
    // Use a single past calendar occurrence with a 24-hour cron hour, even
    // if the wall clock crosses a minute boundary during the proof.
    let (occurrence, expression): (i64, String) = serde_json::from_str(&proof.sql(
        "WITH target(minute) AS (SELECT mesh_jobs.now_ms() / 60000 - 1)
         SELECT json_build_array(minute,
           to_char(to_timestamp(minute * 60) AT TIME ZONE 'UTC', 'MI HH24 DD MM') || ' *')
         FROM target",
    ))
    .unwrap();
    proof.control(json!({
        "action":"schedule", "name":"outage", "expression":expression,
        "timezone":"Etc/UTC", "misfire":"catch_up",
        "job":{"worker":"proof", "queue":"maintenance", "args":{"label":"exhausted-cron"}}
    }));
    proof.sql(&format!(
        "UPDATE mesh_jobs.cron_schedules SET cursor_minute = {}
         WHERE namespace = {} AND name = 'outage'",
        occurrence - 1,
        quoted(&proof.namespace)
    ));
    occurrence
}

fn cron_cursor(proof: &Proof<'_>) -> i64 {
    proof
        .sql(&format!(
            "SELECT cursor_minute FROM mesh_jobs.cron_schedules
             WHERE namespace = {} AND name = 'outage'",
            quoted(&proof.namespace)
        ))
        .parse()
        .unwrap()
}

fn stop_while_held(proof: &Proof<'_>, worker: &mut Worker, active: &Value) -> (Duration, Duration) {
    worker.signal();
    let stop = eventually("Jobs.stop to return with the pool exhausted", || {
        worker.assert_running();
        let nanos = fs::read_to_string(worker.markers.join("stop-ns")).ok()?;
        nanos
            .strip_suffix('\n')?
            .parse()
            .ok()
            .map(Duration::from_nanos)
    });
    assert!(
        stop <= Duration::from_millis(GRACE_MS + 100),
        "Jobs.stop took {stop:?}"
    );
    let report: Value =
        serde_json::from_slice(&fs::read(worker.markers.join("stop")).unwrap()).unwrap();
    assert_eq!(report["state"], "stopping", "{report}");
    assert_eq!(report["undrained_jobs"], json!([active["id"]]), "{report}");
    assert!(!worker.markers.join("joined").exists());
    assert_eq!(proof.job("exhausted-active")["state"], "executing");
    assert_eq!(proof.job("exhausted-pending")["attempt"], 0);

    let released = Instant::now();
    File::create(worker.markers.join("pool-release")).unwrap();
    let report: Value = eventually("Jobs.join to return after releasing the checkout", || {
        serde_json::from_slice(&fs::read(worker.markers.join("joined")).ok()?).ok()
    });
    let join = released.elapsed();
    assert!(join <= JOIN_BOUND, "Jobs.join took {join:?} after release");
    assert_eq!(report["state"], "stopped", "{report}");
    assert_eq!(report["drained"], true, "{report}");
    assert_eq!(report["outcomes"]["discarded"], 1, "{report}");
    assert_eq!(worker.join(), report);
    (stop, join)
}
