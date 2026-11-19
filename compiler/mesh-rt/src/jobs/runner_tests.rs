use super::*;
use serde_json::json;
use std::sync::mpsc;

const TEST_CONFIG: &str = r#"{"queues":{"default":2},"workers":[{"worker":"test","version":1}]}"#;
const TEST_WAIT: Duration = Duration::from_secs(5);

#[test]
fn config_matches_store_names_and_bounds_runner_capacity() {
    let config = Config::parse(TEST_CONFIG).unwrap();
    assert_eq!(config.namespace, "default");
    assert_eq!(config.queues["default"], 2);
    assert!(Config::parse(
        r#"{"namespace":"app jobs","queues":{"batch":1},"workers":[{"worker":"test","version":4294967295}]}"#
    )
    .is_ok());
    for config in [
        r#"{"queues":{"default":0},"workers":[{"worker":"test","version":1}]}"#,
        r#"{"queues":{"default":1024,"other":1},"workers":[{"worker":"test","version":1}]}"#,
        r#"{"queues":{"default":1},"workers":[{"worker":"test","version":0}]}"#,
        r#"{"queues":{"default":1},"workers":[{"worker":"test","version":1},{"worker":"test","version":1}]}"#,
        r#"{"queues":{"default":1},"workers":[{"worker":"test","version":1}],"lease_ms":0}"#,
    ] {
        assert!(Config::parse(config).is_err(), "{config}");
    }
}

#[test]
fn killed_before_start_releases_the_invocation_without_running_it() {
    let scheduler = Scheduler::new(1);
    let captured = Arc::new(());
    let owner = Arc::downgrade(&captured);
    let (ran, seen) = mpsc::channel();
    let (pid, exited) = scheduler.spawn_observed(move || {
        assert_eq!(Arc::strong_count(&captured), 1);
        ran.send(()).unwrap();
    });
    assert!(owner.upgrade().is_some());

    abort(&scheduler, pid, TIMEOUT);
    scheduler.start();
    let exit = exited.recv_timeout(TEST_WAIT).unwrap();
    assert_eq!(exit.reason, ExitReason::Custom(TIMEOUT.into()));
    assert!(scheduler.get_process(pid).is_none());
    assert!(owner.upgrade().is_none());
    assert_eq!(seen.try_recv(), Err(TryRecvError::Disconnected));
    scheduler.signal_shutdown();
    scheduler.wait();
}

#[test]
fn forced_suspended_exit_is_observed_after_unwinding_and_removal() {
    struct OnStack(mpsc::Sender<()>);

    impl Drop for OnStack {
        fn drop(&mut self) {
            self.0.send(()).unwrap();
        }
    }

    let scheduler = Scheduler::new(1);
    let (entered, began) = mpsc::channel();
    let (dropped, unwound) = mpsc::channel();
    let (pid, exited) = scheduler.spawn_observed(move || {
        let _on_stack = OnStack(dropped.clone());
        actor::current_process()
            .unwrap()
            .lock()
            .set_live_state(ProcessState::Waiting);
        entered.send(()).unwrap();
        actor::stack::yield_current();
        panic!("cancelled frame re-entered its body");
    });
    scheduler.start();
    began.recv_timeout(TEST_WAIT).unwrap();
    let process = scheduler.get_process(pid).unwrap();
    scheduler.force_exit(pid, process.lock(), ExitReason::Custom(CANCELLED.into()));
    let exit = exited.recv_timeout(TEST_WAIT).unwrap();
    assert_eq!(exit.reason, ExitReason::Custom(CANCELLED.into()));
    assert_eq!(unwound.try_recv(), Ok(()));
    assert!(scheduler.get_process(pid).is_none());
    scheduler.signal_shutdown();
    scheduler.wait();
}

#[test]
fn an_expired_stop_reports_a_native_callback_until_it_actually_exits() {
    let scheduler = Scheduler::new(1);
    let (entered, began) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let released = Mutex::new(released);
    let (pid, exited) = scheduler.spawn_observed(move || {
        entered.send(()).unwrap();
        released.lock().recv().unwrap();
    });
    scheduler.start();
    began.recv_timeout(TEST_WAIT).unwrap();

    let now = Instant::now();
    let invocation = Arc::new(Invocation {
        clock: Mutex::new(InvocationClock {
            deadline: now + TEST_WAIT,
            lease_until: now + TEST_WAIT,
            forced: None,
        }),
        result: Mutex::new(None),
    });
    let runner = Runner {
        id: 0,
        config: Config::parse(TEST_CONFIG).unwrap(),
        scheduler: &scheduler,
        state: Mutex::new(State {
            phase: Phase::Running,
            jobs: BTreeMap::from([(
                "token".into(),
                ActiveJob {
                    id: "job-id".into(),
                    queue: 0,
                    running: Some((pid, invocation)),
                },
            )]),
            pending_claims: 0,
            counts: Counts::default(),
            last_error: None,
        }),
    };
    runner.request_stop(Duration::ZERO);
    let report: serde_json::Value = serde_json::from_str(&runner.wait()).unwrap();
    assert_eq!(report["drained"], false);
    assert_eq!(report["undrained_jobs"], json!(["job-id"]));
    assert!(matches!(exited.try_recv(), Err(TryRecvError::Empty)));
    assert!(scheduler.get_process(pid).is_some());

    release.send(()).unwrap();
    let exit = exited.recv_timeout(TEST_WAIT).unwrap();
    assert_eq!(exit.reason, ExitReason::Custom(STOPPED.into()));
    assert!(!exit.returned_normally);
    assert!(scheduler.get_process(pid).is_none());
    scheduler.signal_shutdown();
    scheduler.wait();
}

#[test]
fn owned_reply_survives_actor_exit_and_invalid_delays_are_failures() {
    unsafe extern "C-unwind" fn echo(input: *const MeshString) -> *const MeshString {
        input
    }

    let scheduler = Scheduler::new(1);
    let input = r#"{"action":"complete"}"#;
    let deadline = Instant::now() + TEST_WAIT;
    let running = Running::start(&scheduler, echo, input.into(), deadline, deadline);
    scheduler.start();
    let reply = loop {
        if let Some(reply) = running.poll() {
            break reply;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    };
    assert!(scheduler.get_process(running.pid).is_none());
    assert!(matches!(reply.outcome(), Outcome::Complete));
    let invalid = Reply::Returned(r#"{"action":"snooze","delay_ms":18446744073709551615}"#.into());
    assert!(matches!(invalid.outcome(), Outcome::Fail { .. }));
    scheduler.signal_shutdown();
    scheduler.wait();
}

#[test]
#[ignore = "requires MESH_TEST_DATABASE_URL"]
fn postgres_runner_executes_schedules_and_settles_renewed_timed_out_and_cancelled_attempts() {
    use crate::db::pool::{mesh_pool_close, mesh_pool_open, unbox_u64_payload};
    use crate::io::MeshResult;
    use crate::string::{mesh_str, text_of};

    unsafe extern "C-unwind" fn handle(input: *const MeshString) -> *const MeshString {
        let context: serde_json::Value = serde_json::from_str(unsafe { text_of(input) }).unwrap();
        let wait_ms = context["args"]["wait_ms"].as_i64().unwrap();
        actor::mesh_timer_sleep(wait_ms);
        mesh_str(r#"{"action":"complete"}"#)
    }

    crate::gc::mesh_rt_init();
    actor::mesh_rt_init_actor(2);
    let url = std::env::var("MESH_TEST_DATABASE_URL").expect("test PostgreSQL URL");
    let pool = unsafe { &*mesh_pool_open(mesh_str(&url), 0, 3, 500).cast::<MeshResult>() };
    assert_eq!(pool.tag, 0);
    let pool = unsafe { unbox_u64_payload(pool.value) };
    with_pool(pool, |db| {
        super::super::store::migrate(db)?;
        db.transaction(|tx| super::super::cron::migrate(tx))
    })
    .unwrap();
    let namespace = format!("runner-test-{}", rand::random::<u64>());
    with_pool(pool, |db| {
        super::super::cron::control(
            db,
            &namespace,
            &json!({
                "action": "schedule", "name": "boot", "expression": "@reboot",
                "job": {"worker": "test", "args": {"wait_ms": 10}}
            })
            .to_string(),
        )
    })
    .unwrap();
    let config = json!({
        "namespace": namespace,
        "queues": {"default": 1},
        "workers": [{"worker": "test", "version": 1}],
        "lease_ms": 300,
        "poll_ms": 10,
    });
    let id = start(
        pool,
        &config.to_string(),
        handle as *const u8,
        std::ptr::null(),
    )
    .unwrap();

    let control = |request: serde_json::Value| -> serde_json::Value {
        let output = with_pool(pool, |db| {
            super::super::store::control(db, &namespace, &request.to_string())
        })
        .unwrap();
        serde_json::from_str(&output).unwrap()
    };

    let enqueue = |wait_ms: u32, timeout_ms: u32| -> String {
        let request = json!({
            "worker": "test",
            "args": {"wait_ms": wait_ms},
            "max_attempts": 1,
            "timeout_ms": timeout_ms,
        });
        let output = with_pool(pool, |db| {
            db.transaction(|tx| super::super::store::enqueue(tx, &namespace, &request.to_string()))
        })
        .unwrap();
        let receipt: serde_json::Value = serde_json::from_str(&output).unwrap();
        receipt["id"].as_str().unwrap().to_string()
    };

    let wait_for = |job_id: &str, expected: &str| -> serde_json::Value {
        let deadline = Instant::now() + TEST_WAIT;
        loop {
            let job = control(json!({"action": "get", "id": job_id}));
            if job["state"] == expected {
                return job;
            }
            assert!(
                Instant::now() < deadline,
                "expected job state {expected}: {job}"
            );
            std::thread::sleep(TICK);
        }
    };

    let long = enqueue(675, 2000);
    assert_eq!(wait_for(&long, "completed")["attempt"], 1);

    let short = enqueue(1000, 80);
    let timed_out = wait_for(&short, "discarded");
    assert_eq!(timed_out["errors"][0]["reason"], TIMEOUT);

    let cancel = enqueue(3000, 5000);
    wait_for(&cancel, "executing");
    control(json!({"action": "cancel", "id": cancel}));
    wait_for(&cancel, "cancelled");

    let report: serde_json::Value = serde_json::from_str(&stop(id, 2000).unwrap()).unwrap();
    assert_eq!(report["state"], "stopped");
    assert_eq!(report["drained"], true);
    assert_eq!(report["finished"], 4);
    assert_eq!(report["scheduled"], 1);
    assert_eq!(join(id).unwrap(), report.to_string());

    let abandoned = enqueue(10, 2000);
    with_pool(pool, |db| {
        let claimed = claim(
            db,
            &namespace,
            "default",
            r#"[{"worker":"test","version":1}]"#,
            50,
        )?
        .unwrap();
        assert_eq!(claimed.id, abandoned);
        Ok(())
    })
    .unwrap();
    std::thread::sleep(Duration::from_millis(75));
    let ready = enqueue(10, 2000);
    let later = enqueue(10, 2000);
    assert_eq!(
        control(json!({"action":"reschedule","id":later,"delay_ms":3_600_000}))["state"],
        "scheduled"
    );
    let mut drain_config = config;
    drain_config["drain"] = json!(true);
    let (locked, lock_acquired) = mpsc::channel();
    let (release_lock, release) = mpsc::channel();
    let lock_ns = namespace.clone();
    let lock_holder = std::thread::spawn(move || {
        with_pool(pool, |db| {
            db.transaction(|tx| {
                tx.query(
                    "SELECT name FROM mesh_jobs.queues WHERE namespace = $1 FOR NO KEY UPDATE",
                    &[&lock_ns],
                )?;
                locked.send(()).unwrap();
                release.recv_timeout(TEST_WAIT).unwrap();
                Ok(())
            })
        })
        .unwrap();
    });
    lock_acquired.recv_timeout(TEST_WAIT).unwrap();
    let one_shot = start(
        pool,
        &drain_config.to_string(),
        handle as *const u8,
        std::ptr::null(),
    )
    .unwrap();
    std::thread::sleep(Duration::from_millis(100));
    assert!(lookup(one_shot).unwrap().report(true).is_none());
    release_lock.send(()).unwrap();
    lock_holder.join().unwrap();
    let deadline = Instant::now() + TEST_WAIT;
    while lookup(one_shot).unwrap().report(true).is_none() {
        assert!(
            Instant::now() < deadline,
            "drain status: {}",
            status(one_shot).unwrap()
        );
        std::thread::sleep(TICK);
    }
    let report: serde_json::Value = serde_json::from_str(&join(one_shot).unwrap()).unwrap();
    assert_eq!(report["finished"], 1);
    assert_eq!(report["scheduled"], 0);
    assert_eq!(report["recovered"], 1);
    let recovered = control(json!({"action":"get","id":abandoned}));
    assert_eq!(recovered["state"], "discarded");
    assert_eq!(recovered["errors"][0]["reason"], "lease expired");
    assert_eq!(
        control(json!({"action":"get","id":ready}))["state"],
        "completed"
    );
    assert_eq!(
        control(json!({"action":"get","id":later}))["state"],
        "scheduled"
    );

    drain_config["drain"] = json!(false);
    drain_config["prune_max_age_ms"] = json!(1);
    let pruner = start(
        pool,
        &drain_config.to_string(),
        handle as *const u8,
        std::ptr::null(),
    )
    .unwrap();
    let deadline = Instant::now() + TEST_WAIT;
    loop {
        let report: serde_json::Value = serde_json::from_str(&status(pruner).unwrap()).unwrap();
        if report["pruned"].as_u64().unwrap() >= 6 {
            break;
        }
        assert!(Instant::now() < deadline, "prune status: {report}");
        std::thread::sleep(TICK);
    }
    stop(pruner, 2000).unwrap();
    assert_eq!(
        control(json!({"action":"get","id":ready})),
        serde_json::Value::Null
    );
    assert_eq!(
        control(json!({"action":"get","id":later}))["state"],
        "scheduled"
    );
    with_pool(pool, |db| {
        db.transaction(|tx| {
            tx.query(
                "DELETE FROM mesh_jobs.jobs WHERE namespace = $1",
                &[&namespace],
            )?;
            tx.query(
                "DELETE FROM mesh_jobs.queues WHERE namespace = $1",
                &[&namespace],
            )?;
            tx.query(
                "DELETE FROM mesh_jobs.admissions WHERE namespace = $1",
                &[&namespace],
            )?;
            tx.query(
                "DELETE FROM mesh_jobs.cron_activation WHERE namespace = $1",
                &[&namespace],
            )?;
            tx.query(
                "DELETE FROM mesh_jobs.cron_schedules WHERE namespace = $1",
                &[&namespace],
            )?;
            tx.query(
                "DELETE FROM mesh_jobs.receipts WHERE namespace = $1",
                &[&namespace],
            )?;
            Ok(())
        })
    })
    .unwrap();
    mesh_pool_close(pool);
}
