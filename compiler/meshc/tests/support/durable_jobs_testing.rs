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
        let child = Command::new(&compiler)
            .arg("test")
            .arg(&project)
            .env("MESH_TEST_DATABASE_URL", &url)
            .env("MESH_JOBS_PROOF_NAMESPACE", namespace)
            .env("MESH_JOBS_OTHER_NAMESPACE", other)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let output = artifacts::wait_with_timeout(child, WAIT).unwrap();
        let report = artifacts::command_output_text(&output);
        assert!(output.status.success(), "{report}");
        assert!(report.contains("2 passed"), "{report}");
        assert!(
            !report.contains("invalid synthetic handler ran"),
            "{report}"
        );
    };
    if url.is_empty() {
        test("", "");
        eprintln!("Set MESH_TEST_DATABASE_URL for the borrowed transaction and drain checks.");
        return;
    }

    let binary = project.join("jobs-testing");
    let _database = lock_proof_database();
    let proof = Proof::new(&url, binary.clone(), temp.path());
    proof.run("migrate", "");
    let other = Proof::new(&url, binary, temp.path());
    let left = proof.namespace.as_str();
    let right = other.namespace.as_str();
    thread::scope(|scope| {
        let check = scope.spawn(|| test(left, right));
        test(right, left);
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
    prove_bounded_crash_recovery(&proof);
    proof.clean().unwrap();
    other.clean().unwrap();
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
    json!({
        "config": {
            "namespace": proof.namespace,
            "queues": {queue: 4},
            "workers": [{"worker": "proof", "version": 1}],
            "lease_ms": LEASE_MS,
        },
        "options": options,
    })
    .to_string()
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
    let child = worker
        .command("drain", &request)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let output = artifacts::wait_with_timeout(child, WAIT).unwrap();
    let report = artifacts::command_output_text(&output);
    assert_eq!(output.status.code(), Some(1), "{report}");
    assert!(
        report.contains("proof error:")
            && report.contains("permission denied for function claim_drain_v1"),
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
