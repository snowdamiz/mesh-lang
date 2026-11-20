use super::*;

#[test]
fn durable_jobs_test_helpers_and_bounded_drain() {
    let compiler = artifacts::meshc_bin();
    let scratch = artifacts::repo_root().join(".tmp");
    fs::create_dir_all(&scratch).unwrap();
    let temp = tempfile::tempdir_in(scratch).unwrap();
    let fixture = artifacts::repo_root().join("tests/e2e/durable_jobs");
    let project = temp.path().join("jobs-testing");
    fs::create_dir(&project).unwrap();
    fs::copy(fixture.join("main.mpl"), project.join("main.mpl")).unwrap();
    fs::copy(
        fixture.join("testing.test.mpl"),
        project.join("testing.test.mpl"),
    )
    .unwrap();
    fs::write(
        project.join("mesh.toml"),
        "[package]\nname = \"jobs-testing\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();

    let build = Command::new(&compiler)
        .args(["build"])
        .arg(&project)
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "{}",
        artifacts::command_output_text(&build)
    );

    let rejected = temp.path().join("jobs-production-rejection");
    fs::create_dir(&rejected).unwrap();
    fs::write(
        rejected.join("main.mpl"),
        r#"fn main() do
  Test.perform_job("{}", -1, fn(job) do job end)
end
"#,
    )
    .unwrap();
    let build = Command::new(&compiler)
        .args(["build"])
        .arg(&rejected)
        .output()
        .unwrap();
    let diagnostics = artifacts::command_output_text(&build);
    assert!(
        !build.status.success() && diagnostics.contains("perform_job"),
        "{diagnostics}"
    );

    let url = std::env::var("MESH_TEST_DATABASE_URL").unwrap_or_default();
    let test = |namespace: &str, other: &str| {
        let mut command = Command::new(&compiler);
        command
            .arg("test")
            .arg(&project)
            .env("TMPDIR", temp.path())
            .env("MESH_TEST_DATABASE_URL", &url)
            .env("MESH_JOBS_PROOF_NAMESPACE", namespace)
            .env("MESH_JOBS_OTHER_NAMESPACE", other)
            .env_remove("MESH_JOBS_WAIT_LABEL")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command
    };
    if url.is_empty() {
        let child = test("", "").spawn().unwrap();
        check_testing_output(&artifacts::wait_with_timeout(child, WAIT).unwrap());
        eprintln!("Set MESH_TEST_DATABASE_URL for the borrowed transaction, waiting assertion, and drain checks.");
        return;
    }

    let binary = project.join("jobs-testing");
    let _database = lock_proof_database();
    let proof = Proof::new(&url, binary.clone(), temp.path());
    proof.run("migrate", "");
    let other = Proof::new(&url, binary, temp.path());
    let left = proof.namespace.as_str();
    let right = other.namespace.as_str();
    let right_proof = other.on_connection(&url);
    thread::scope(|scope| {
        let test = &test;
        let check = scope.spawn(move || {
            prove_waiting_assertion(&right_proof, "waiting-refute", &mut test(right, left));
        });
        prove_waiting_assertion(&proof, "waiting-assert", &mut test(left, right));
        check.join().unwrap();
    });
    for scope in [&proof, &other] {
        let job = scope.job("manual");
        assert_eq!(job["args"]["scope"], scope.namespace);
        assert_eq!(job["attempt"], 0);
        assert!(scope.effects("manual").is_empty());
    }
    if let Some(runtime) = std::env::var_os("MESH_JOBS_REFERENCE_RT_LIB_PATH") {
        prove_manual_reference(&compiler, &proof, Path::new(&runtime));
    }

    prove_bounded_cutoff(&proof, &other);
    prove_bounded_recursion(&proof);
    prove_bounded_error_policy(&proof);
    prove_bounded_global_capacity(&proof);
    prove_bounded_recovery_budget(&proof);
    prove_bounded_launch_failure(&proof);
    prove_bounded_unconfirmed_settlement(&proof);
    prove_bounded_crash_recovery(&proof);
    proof.clean().unwrap();
    other.clean().unwrap();
}

fn check_testing_output(output: &std::process::Output) {
    let report = artifacts::command_output_text(output);
    assert!(output.status.success(), "{report}");
    assert!(report.contains("3 passed"), "{report}");
    assert!(
        !report.contains("invalid synthetic handler ran"),
        "{report}"
    );
}

fn prove_waiting_assertion(proof: &Proof<'_>, label: &str, command: &mut Command) {
    let mut staged = wakeups::stage(proof, "stage_commit", "testing-wait", label);
    assert!(
        proof.job(label).is_null(),
        "{label} became visible before commit"
    );
    let mut observer = native_pg_connect(proof.url).unwrap();
    let mut child = command.env("MESH_JOBS_WAIT_LABEL", label).spawn().unwrap();
    let observed = observe_waiting_read(&mut observer, &mut child, &proof.namespace);
    let pid = match observed {
        Ok(pid) => {
            wakeups::release(&staged);
            pid
        }
        Err(error) => {
            drop(staged);
            let output = artifacts::wait_with_timeout(child, WAIT).unwrap();
            panic!(
                "{label}: {error}\n{}",
                artifacts::command_output_text(&output)
            );
        }
    };
    native_pg_close(observer);
    let output = artifacts::wait_with_timeout(child, WAIT).unwrap();
    check_testing_output(&output);
    let receipt = staged.join();
    assert_eq!(receipt["state"], "available");
    assert_eq!(proof.job(label)["id"], receipt["id"]);
    println!(
        "{label} passed: PostgreSQL session {pid} completed a no-match read before external commit of job {} in {}",
        id(&receipt),
        proof.namespace,
    );
}

fn observe_waiting_read(
    observer: &mut NativePgConn,
    child: &mut Child,
    namespace: &str,
) -> Result<String, String> {
    let deadline = Instant::now() + WAIT + artifacts::LAUNCH_ALLOWANCE;
    loop {
        // The single-connection fixture pool is tagged just before the helper
        // runs. An idle backend with this last query has completed a poll.
        let rows = native_pg_query(
            observer,
            "SELECT pid::text FROM pg_stat_activity
             WHERE datname = current_database() AND application_name = $1 AND state = 'idle'
               AND query LIKE '%SELECT EXISTS (SELECT FROM matches)::text'",
            &[namespace],
        )?;
        if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
            return Err(format!(
                "meshc test exited before the read was observed ({status})"
            ));
        }
        if let Some(row) = rows.into_iter().next() {
            return Ok(row[0].1.clone());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "timed out observing a completed assertion read in {namespace}"
            ));
        }
        thread::sleep(Duration::from_millis(25));
    }
}

fn prove_manual_reference(compiler: &Path, candidate: &Proof, runtime: &Path) {
    let dir = candidate.dir.join("manual-reference");
    let binary = build_reference_fixture(compiler, &dir, runtime);
    let reference = Proof::new(candidate.url, binary, &dir);
    let candidate_job = candidate.job("manual");
    let receipt = reference.enqueue(json!({
        "worker":"proof", "queue":"testing", "version":2, "args":candidate_job["args"],
    }));
    let rows = reference.control(json!({"action":"list", "queue":"testing"}));
    assert_eq!(rows.as_array().unwrap().len(), 1);
    assert_eq!(rows[0]["id"], receipt["id"]);
    let reference_job = reference.control(json!({"action":"get", "id":receipt["id"]}));
    assert_eq!(reference_job["id"], receipt["id"]);
    for field in [
        "worker",
        "queue",
        "version",
        "args",
        "state",
        "attempt",
        "max_attempts",
    ] {
        assert_eq!(reference_job[field], candidate_job[field], "{field}");
    }
    assert!(reference.effects("manual").is_empty());
    reference.clean().unwrap();
    println!("J5 lane 1 passed: main and candidate persisted the same job input and state, and the candidate verified the row with Test.assert_enqueued");
}

fn bounded_request(proof: &Proof, queue: &str, options: Value) -> String {
    bounded_payload(proof, queue, options).to_string()
}

fn bounded_payload(proof: &Proof, queue: &str, options: Value) -> Value {
    json!({
        "config": {
            "namespace": proof.namespace,
            "queues": {queue: 4},
            "workers": [{"worker": "proof", "version": 1}],
            "lease_ms": LEASE_MS,
        },
        "options": options,
    })
}

fn start_drain(proof: &Proof, request: &str) -> Child {
    proof
        .command("drain", request)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

fn drain_error(output: std::process::Output) -> String {
    let report = artifacts::command_output_text(&output);
    assert_eq!(output.status.code(), Some(1), "{report}");
    assert!(report.contains("proof error:"), "{report}");
    report
}

pub(super) fn prove_scoped_drain(admin: &Proof, producer: &Proof, worker: &Proof) {
    const CLAIM: &str = "mesh_jobs.claim_drain_v1(text,text,jsonb,bigint,bigint,jsonb)";
    let label = "scoped-drain";
    let queue = "drain";
    let job = producer.enqueue(json!({"worker":"proof", "queue":queue, "args":{"label":label}}));
    let request = bounded_request(
        worker,
        queue,
        json!({"max_jobs":2,"recursive":false,"on_error":"continue"}),
    );

    let locked = admin.lock_job(&job);
    let idle = worker.run("drain", &request);
    assert_eq!(idle["stop_reason"], "idle", "{idle}");
    assert_eq!(idle["claimed"], 0);
    assert_eq!(idle["remaining"], true);
    native_pg_close(locked);

    // A live caller gets an error when its worker login cannot claim.
    admin.sql(&format!(
        "REVOKE EXECUTE ON FUNCTION {CLAIM} FROM jobs_worker"
    ));
    let child = start_drain(worker, &request);
    let output = artifacts::wait_with_timeout(child, WAIT).unwrap();
    let report = drain_error(output);
    assert!(
        report.contains("permission denied for function claim_drain_v1"),
        "{report}"
    );
    let untouched = admin.job(label);
    assert_eq!(untouched["id"], job["id"]);
    assert_eq!(untouched["state"], "available");
    assert_eq!(untouched["attempt"], 0);
    assert!(admin.effects(label).is_empty());

    admin.sql(&format!("GRANT EXECUTE ON FUNCTION {CLAIM} TO jobs_worker"));
    let completed = worker.run("drain", &request);
    assert_eq!(completed["outcomes"]["completed"], 1, "{completed}");
    assert_eq!(completed["remaining"], false);
    let row = admin.wait_state(label, "completed");
    assert_eq!(row["id"], job["id"]);
    assert_eq!(row["attempt"], 1);
    let effects = admin.wait_effects(label, 1);
    assert_eq!(effects[0]["context"]["effect_key"], job["effect_key"]);
    println!("Scoped Jobs.drain respected a row lock, returned a claim error, and completed under the worker grants");
}

fn prove_bounded_cutoff(proof: &Proof, other: &Proof) {
    let cutoff = proof
        .sql("SELECT mesh_jobs.now_ms() + 600000")
        .parse::<i64>()
        .unwrap();
    proof.enqueue(json!([
        {"worker":"proof", "queue":"cutoff", "args":{"label":"at-cutoff"},
         "scheduled_at_ms":cutoff},
        {"worker":"proof", "queue":"cutoff", "args":{"label":"after-cutoff"},
         "scheduled_at_ms":cutoff + 1},
        {"worker":"proof", "queue":"cutoff", "args":{"label":"other-version"},
         "scheduled_at_ms":cutoff, "version":2},
    ]));
    other.enqueue(json!({"worker":"proof", "queue":"cutoff", "args":{"label":"other-namespace"}}));
    let due_only = proof.run(
        "drain",
        &bounded_request(
            proof,
            "cutoff",
            json!({"max_jobs":4,"recursive":true,"on_error":"continue"}),
        ),
    );
    assert_eq!(due_only["claimed"], 0, "{due_only}");
    assert_eq!(due_only["remaining"], false);
    let scheduled = proof.run(
        "drain",
        &bounded_request(
            proof,
            "cutoff",
            json!({
                "max_jobs": 1, "cutoff_ms": cutoff, "recursive": false, "on_error": "continue"
            }),
        ),
    );
    assert_eq!(scheduled["outcomes"]["completed"], 1, "{scheduled}");
    assert_eq!(scheduled["budget_hit"], true);
    assert_eq!(scheduled["remaining"], false);
    assert_eq!(proof.job("at-cutoff")["state"], "completed");
    assert_eq!(proof.job("after-cutoff")["state"], "scheduled");
    assert_eq!(proof.job("other-version")["attempt"], 0);
    assert_eq!(other.job("other-namespace")["attempt"], 0);
}

fn prove_bounded_recursion(proof: &Proof) {
    proof.enqueue(
        json!({"worker": "proof", "queue": "chain", "args": {"label":"chain", "depth":3}}),
    );
    let once = proof.run(
        "drain",
        &bounded_request(
            proof,
            "chain",
            json!({
                "max_jobs":4, "recursive":false, "on_error":"continue"
            }),
        ),
    );
    assert_eq!(once["outcomes"]["completed"], 1, "{once}");
    assert_eq!(once["remaining"], true);
    assert_eq!(once["budget_hit"], false);
    let recursive = bounded_request(
        proof,
        "chain",
        json!({
            "max_jobs":2,"recursive":true,"on_error":"continue"
        }),
    );
    let first = proof.run("drain", &recursive);
    assert_eq!(first["claimed"], 2, "{first}");
    assert_eq!(first["outcomes"]["completed"], 2);
    assert_eq!(first["stop_reason"], "budget");
    assert_eq!(first["remaining"], true);
    let rest = proof.run("drain", &recursive);
    assert_eq!(rest["outcomes"]["completed"], 1, "{rest}");
    assert_eq!(rest["remaining"], false);
}

fn prove_bounded_error_policy(proof: &Proof) {
    proof.enqueue(json!([
        {"worker":"proof", "queue":"errors", "args":{"label":"poison"}, "max_attempts":2,
         "backoff_base_ms":0, "backoff_max_ms":0},
        {"worker":"proof", "queue":"errors", "args":{"label":"bounded-healthy"}, "priority":1},
    ]));
    let stopped = proof.run(
        "drain",
        &bounded_request(
            proof,
            "errors",
            json!({
                "max_jobs":4,"recursive":true,"on_error":"stop"
            }),
        ),
    );
    assert_eq!(stopped["stop_reason"], "job_error", "{stopped}");
    assert_eq!(stopped["outcomes"]["retryable"], 1);
    assert_eq!(stopped["remaining"], true);
    assert_eq!(proof.job("poison")["state"], "retryable");
    assert_eq!(proof.job("bounded-healthy")["attempt"], 0);

    let continued = proof.run(
        "drain",
        &bounded_request(
            proof,
            "errors",
            json!({
                "max_jobs":4,"recursive":true,"on_error":"continue"
            }),
        ),
    );
    assert_eq!(continued["finished"], 2, "{continued}");
    assert_eq!(continued["outcomes"]["discarded"], 1);
    assert_eq!(continued["outcomes"]["completed"], 1);
    assert_eq!(continued["remaining"], false);
    assert_eq!(proof.job("poison")["state"], "discarded");
    assert_eq!(proof.job("poison")["attempt"], 2);
    assert_eq!(proof.job("bounded-healthy")["state"], "completed");
}

fn prove_bounded_global_capacity(proof: &Proof) {
    let queue = "drain-global-capacity";
    let holder_label = "drain-slot-holder";
    let pending_label = "drain-slot-pending";
    let holder = proof.enqueue(json!({
        "worker":"proof", "queue":queue, "args":{"label":holder_label},
    }));
    let mut peer = proof.worker_with_options(holder_label, true, json!({queue: 1}), 8000, 60_000);
    let effects = proof.wait_effects(holder_label, 1);
    assert_eq!(effects[0]["process"], holder_label);
    assert_job_context(&effects[0], &holder, queue, holder_label, 1);

    let pending = proof.enqueue(json!({
        "worker":"proof", "queue":queue, "args":{"label":pending_label},
    }));
    let before = proof.job(pending_label);
    assert_eq!(before["id"], pending["id"]);
    assert_eq!(before["effect_key"], pending["effect_key"]);
    assert_eq!(before["state"], "available");
    assert_eq!(before["attempt"], 0);

    let occupied_slot = || {
        proof.sql(&format!(
            "SELECT j.lease_token FROM mesh_jobs.jobs j
             JOIN mesh_jobs.queues q ON q.namespace = j.namespace AND q.name = j.queue
             WHERE j.namespace = {} AND j.id = {}::uuid
                 AND q.state = 'active' AND q.concurrency = 1
                 AND j.state = 'executing' AND j.attempt = 1
                 AND j.lease_expires_at_ms > mesh_jobs.now_ms()",
            quoted(&proof.namespace),
            quoted(id(&holder)),
        ))
    };
    let token = occupied_slot();
    assert!(
        !token.is_empty(),
        "peer must hold the sole global queue slot"
    );

    let request = bounded_request(
        proof,
        queue,
        json!({"max_jobs":1,"recursive":false,"on_error":"continue"}),
    );
    let idle = proof.run("drain", &request);
    assert_eq!(idle["stop_reason"], "idle", "{idle}");
    assert_eq!(idle["claimed"], 0, "{idle}");
    assert_eq!(idle["recovered"], 0, "{idle}");
    assert_eq!(idle["remaining"], true, "{idle}");
    peer.assert_running();
    assert_eq!(occupied_slot(), token);
    assert_eq!(proof.job(pending_label), before);
    assert!(proof.effects(pending_label).is_empty());

    peer.signal();
    eventually("the slot holder to receive SIGTERM", || {
        peer.assert_running();
        peer.markers.join("signal").is_file().then_some(())
    });
    peer.release();
    let stopped = peer.join();
    assert_eq!(stopped["drained"], true, "{stopped}");
    assert_eq!(stopped["claimed"], 1, "{stopped}");
    assert_eq!(proof.job(holder_label)["state"], "completed");

    let completed = proof.run("drain", &request);
    assert_eq!(completed["claimed"], 1, "{completed}");
    assert_eq!(completed["outcomes"]["completed"], 1, "{completed}");
    assert_eq!(completed["remaining"], false, "{completed}");
    let job = proof.job(pending_label);
    assert_eq!(job["id"], pending["id"]);
    assert_eq!(job["effect_key"], pending["effect_key"]);
    assert_eq!(job["state"], "completed");
    assert_eq!(job["attempt"], 1);
    let effects = proof.effects(pending_label);
    assert_eq!(effects.len(), 1);
    assert_job_context(&effects[0], &pending, queue, pending_label, 1);
}

fn prove_bounded_recovery_budget(proof: &Proof) {
    let queue = "drain-recovery-budget";
    let first_label = "drain-recover-first";
    let excess_label = "drain-recover-excess";
    let wrong_label = "drain-recover-wrong-version";
    let attempted_at = proof
        .sql("SELECT mesh_jobs.now_ms() - 60000")
        .parse::<i64>()
        .unwrap();
    let specs: Vec<_> = [(first_label, 1), (excess_label, 1), (wrong_label, 2)]
        .into_iter()
        .map(|(label, version)| {
            json!({
                "worker":"proof", "queue":queue, "version":version, "args":{"label":label},
                "scheduled_at_ms":attempted_at, "max_attempts":2,
                "backoff_base_ms":0, "backoff_max_ms":0,
            })
        })
        .collect();
    let receipts = proof.enqueue(json!(specs));
    let [first, excess, wrong] = receipts.as_array().unwrap().as_slice() else {
        panic!("expected three enqueue receipts: {receipts}");
    };
    let scaled = proof.control(json!({"action":"scale", "queue":queue, "concurrency":3}));
    assert_eq!(scaled["concurrency"], 3, "{scaled}");

    // The wrong version expires first, and the second matching lease is beyond the budget.
    proof.sql(&format!(
        "UPDATE mesh_jobs.jobs j SET state = 'executing', attempt = 1,
             attempted_at_ms = {attempted_at}, lease_token = gen_random_uuid(),
             lease_expires_at_ms = {attempted_at} + seed.lease_ms
         FROM (VALUES ({}::uuid, 2000), ({}::uuid, 3000), ({}::uuid, 1000)) AS seed(id, lease_ms)
         WHERE j.namespace = {} AND j.id = seed.id",
        quoted(id(first)),
        quoted(id(excess)),
        quoted(id(wrong)),
        quoted(&proof.namespace),
    ));
    let stored_row = |receipt: &Value| -> Value {
        serde_json::from_str(&proof.sql(&format!(
            "SELECT to_jsonb(j) || jsonb_build_object('effect_key', mesh_jobs.effect_key(j),
                     'expired', j.lease_expires_at_ms <= mesh_jobs.now_ms())
             FROM mesh_jobs.jobs j WHERE j.namespace = {} AND j.id = {}::uuid",
            quoted(&proof.namespace),
            quoted(id(receipt)),
        )))
        .unwrap()
    };
    let seeded = [first, excess, wrong].map(stored_row);
    for ((receipt, version), row) in [(first, 1), (excess, 1), (wrong, 2)]
        .into_iter()
        .zip(&seeded)
    {
        assert_eq!(row["id"], receipt["id"]);
        assert_eq!(row["effect_key"], receipt["effect_key"]);
        assert_eq!(row["version"], version);
        assert_eq!(row["state"], "executing");
        assert_eq!(row["attempt"], 1);
        assert_eq!(row["expired"], true, "{row}");
    }

    let report = proof.run(
        "drain",
        &bounded_request(
            proof,
            queue,
            json!({"max_jobs":1,"recursive":false,"on_error":"continue"}),
        ),
    );
    assert_eq!(report["recovered"], 1, "{report}");
    assert_eq!(report["claimed"], 1, "{report}");
    assert_eq!(report["finished"], 1, "{report}");
    assert_eq!(report["outcomes"]["completed"], 1, "{report}");
    assert_eq!(report["stop_reason"], "budget", "{report}");
    assert_eq!(report["budget_hit"], true, "{report}");
    assert_eq!(report["remaining"], true, "{report}");

    let completed = stored_row(first);
    assert_eq!(completed["id"], first["id"]);
    assert_eq!(completed["effect_key"], first["effect_key"]);
    assert_eq!(completed["state"], "completed");
    assert_eq!(completed["attempt"], 2);
    assert!(completed["lease_token"].is_null());
    let errors = completed["errors"].as_array().unwrap();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0]["reason"], "lease expired");
    assert_eq!(errors[0]["attempt"], 1);
    let effects = proof.effects(first_label);
    assert_eq!(effects.len(), 1);
    assert_job_context(&effects[0], first, queue, first_label, 2);

    assert_eq!(stored_row(excess), seeded[1]);
    assert_eq!(stored_row(wrong), seeded[2]);
    assert!(proof.effects(excess_label).is_empty());
    assert!(proof.effects(wrong_label).is_empty());
}

fn prove_bounded_launch_failure(proof: &Proof) {
    use std::os::unix::fs::PermissionsExt;

    let queue = "drain-launch-failure";
    let receipt = proof.enqueue(json!({
        "worker":"proof", "queue":queue, "args":{"label":queue}, "max_attempts":1
    }));
    let mut payload = bounded_payload(
        proof,
        queue,
        json!({"max_jobs":1,"recursive":true,"on_error":"continue"}),
    );
    payload["config"]["lease_ms"] = json!(8000);
    let request = payload.to_string();
    let (barrier, blocker) = proof.hold_claim(&receipt);
    let mut child = Some(start_drain(proof, &request));
    eventually(
        "the bounded drain claim to reach the launch barrier",
        || {
            if child.as_mut().unwrap().try_wait().unwrap().is_some() {
                let output = child.take().unwrap().wait_with_output().unwrap();
                panic!(
                    "drain caller exited before the claim barrier: {}",
                    artifacts::command_output_text(&output)
                );
            }
            proof.claim_is_waiting(blocker).then_some(())
        },
    );
    let permissions = fs::metadata(&proof.binary).unwrap().permissions();
    fs::set_permissions(
        &proof.binary,
        fs::Permissions::from_mode(permissions.mode() & !0o111),
    )
    .unwrap();
    native_pg_close(barrier);
    let output = artifacts::wait_with_timeout(child.unwrap(), WAIT);
    fs::set_permissions(&proof.binary, permissions).unwrap();
    let error = drain_error(output.unwrap());
    assert!(
        error.contains("could not start job handler process") && error.contains("os error 13"),
        "{error}"
    );
    let refunded = proof.wait_state(queue, "available");
    assert_eq!(refunded["id"], receipt["id"]);
    assert_eq!(refunded["effect_key"], receipt["effect_key"]);
    assert_eq!(refunded["attempt"], 0);
    assert_eq!(refunded["max_attempts"], 1);
    assert!(proof.effects(queue).is_empty());

    proof.sql("DROP TRIGGER hold_racing_claim ON mesh_jobs.jobs");
    let completed = proof.run("drain", &request);
    assert_eq!(completed["outcomes"]["completed"], 1, "{completed}");
    let job = proof.wait_state(queue, "completed");
    assert_eq!(job["id"], receipt["id"]);
    assert_eq!(job["attempt"], 1);
    let effects = proof.effects(queue);
    assert_eq!(effects.len(), 1);
    assert_job_context(&effects[0], &receipt, queue, queue, 1);
    println!(
        "J5 bounded drain returned a launch error and refunded the confirmed unstarted attempt"
    );
}

fn prove_bounded_unconfirmed_settlement(proof: &Proof) {
    let queue = "drain-unconfirmed";
    let receipt = proof.enqueue(json!({
        "worker":"proof", "queue":queue, "args":{"label":queue}, "max_attempts":2,
        "backoff_base_ms":0, "backoff_max_ms":0
    }));
    proof.sql(&format!(
        "CREATE FUNCTION {ns}.disconnect_drain_settlement() RETURNS trigger LANGUAGE plpgsql AS $cut$
         BEGIN
             PERFORM pg_terminate_backend(pg_backend_pid());
             PERFORM pg_sleep(1);
             RAISE EXCEPTION 'settlement backend did not exit';
         END; $cut$;
         CREATE TRIGGER disconnect_drain_settlement
         BEFORE UPDATE OF state ON mesh_jobs.jobs FOR EACH ROW
         WHEN (NEW.id = {id}::uuid AND NEW.namespace = {quoted_ns} AND NEW.state = 'completed')
         EXECUTE FUNCTION {ns}.disconnect_drain_settlement()",
        ns = proof.namespace,
        quoted_ns = quoted(&proof.namespace),
        id = quoted(id(&receipt)),
    ));
    let mut payload = bounded_payload(
        proof,
        queue,
        json!({"max_jobs":1,"recursive":true,"on_error":"continue"}),
    );
    payload["config"]["event_log"] = json!(true);
    let request = payload.to_string();
    let child = start_drain(proof, &request);
    let error = drain_error(artifacts::wait_with_timeout(child, WAIT).unwrap());
    let events: Vec<Value> = error
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .filter(|event: &Value| event["attempt"]["job_id"] == receipt["id"])
        .collect();
    assert!(
        events.iter().any(|event| event["event"] == "ack_uncertain"),
        "{error}"
    );
    assert!(
        events
            .iter()
            .all(|event| event["event"] != "settlement_committed"),
        "{error}"
    );

    let pending = proof.wait_state(queue, "executing");
    assert_eq!(pending["id"], receipt["id"]);
    assert_eq!(pending["effect_key"], receipt["effect_key"]);
    assert_eq!(pending["attempt"], 1);
    let effects = proof.effects(queue);
    assert_eq!(effects.len(), 1);
    assert_job_context(&effects[0], &receipt, queue, queue, 1);
    let old = proof.lease(id(&receipt));
    assert!(!old.token.is_empty());
    eventually("the unconfirmed drain attempt's lease to expire", || {
        (proof
            .sql("SELECT mesh_jobs.now_ms()")
            .parse::<i64>()
            .unwrap()
            >= old.expires_at_ms)
            .then_some(())
    });

    proof.sql("DROP TRIGGER disconnect_drain_settlement ON mesh_jobs.jobs");
    let recovered = proof.run("drain", &request);
    assert_eq!(recovered["recovered"], 1, "{recovered}");
    assert_eq!(recovered["outcomes"]["completed"], 1, "{recovered}");
    let job = proof.wait_state(queue, "completed");
    assert_eq!(job["id"], receipt["id"]);
    assert_eq!(job["attempt"], 2);
    let effects = proof.effects(queue);
    assert_eq!(effects.len(), 2);
    assert_job_context(&effects[0], &receipt, queue, queue, 1);
    assert_job_context(&effects[1], &receipt, queue, queue, 2);
    println!("J5 bounded drain returned an unconfirmed settlement error and recovered the same job and effect key");
}

fn prove_bounded_crash_recovery(proof: &Proof) {
    let receipt = proof.enqueue(json!({
        "worker":"proof", "queue":"crash", "args":{"label":"bounded-crash"},
        "max_attempts":2, "backoff_base_ms":0, "backoff_max_ms":0
    }));
    let drain = bounded_request(
        proof,
        "crash",
        json!({
            "max_jobs":1,"recursive":true,"on_error":"continue"
        }),
    );
    let blocked = |name: &str| {
        let markers = proof.dir.join(name);
        fs::create_dir(&markers).unwrap();
        let log = File::create(markers.join("output")).unwrap();
        let child = proof
            .command("drain", &drain)
            .env("MESH_JOBS_PROOF_GATE", markers.join("gate"))
            .env("MESH_JOBS_PROOF_WORKER", name)
            .stdout(log.try_clone().unwrap())
            .stderr(log)
            .spawn()
            .unwrap();
        Worker {
            child: Some(child),
            markers,
        }
    };

    let mut first = blocked("bounded-crash-owner");
    proof.wait_effects("bounded-crash", 1);
    first.kill();
    let old = proof.lease(id(&receipt));
    assert_eq!(proof.job("bounded-crash")["state"], "executing");
    eventually("the bounded drain's lease to expire", || {
        (proof
            .sql("SELECT mesh_jobs.now_ms()")
            .parse::<i64>()
            .unwrap()
            >= old.expires_at_ms)
            .then_some(())
    });
    let mut recovery = blocked("bounded-crash-replacement");
    let effects = proof.wait_effects("bounded-crash", 2);
    let fresh = proof.lease(id(&receipt));
    assert_ne!(old.token, fresh.token);
    assert_eq!(
        proof.sql(&format!(
            "SELECT mesh_jobs.settle({}, {}::uuid, {}::uuid, '{{\"action\":\"discard\"}}', false)",
            quoted(&proof.namespace),
            quoted(id(&receipt)),
            quoted(&old.token)
        )),
        "f"
    );
    recovery.release();
    let report = recovery.join_report("stop_reason");
    assert_eq!(report["recovered"], 1, "{report}");
    assert_eq!(report["outcomes"]["completed"], 1);
    assert_eq!(proof.job("bounded-crash")["state"], "completed");
    for (index, effect) in effects.iter().enumerate() {
        let context = &effect["context"];
        assert_eq!(context["id"], id(&receipt));
        assert_eq!(context["effect_key"], receipt["effect_key"]);
        assert_eq!(context["queue"], "crash");
        assert_eq!(context["attempt"], index + 1);
    }
}
