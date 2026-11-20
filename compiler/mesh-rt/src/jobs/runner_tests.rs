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
fn confirmed_batch_launch_failure_stops_later_launches_and_keeps_refunds_tracked() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let runner = Runner {
        id: 0,
        config: Config::parse(TEST_CONFIG).unwrap(),
        check_worker: || Ok(()),
        state: Mutex::new(State {
            phase: Phase::Running,
            jobs: BTreeMap::new(),
            pending_claims: 3,
            counts: Counts::default(),
            last_error: None,
        }),
    };
    let (requests, _) = mpsc::sync_channel(1);
    let (send, results) = mpsc::sync_channel(1);
    let mut lane = BatchLane {
        requests,
        results,
        pending: true,
    };
    let mut queues = [Queue {
        name: "default",
        limit: 3,
        next_claim: Instant::now(),
    }];
    let lease = Lease::begin(Duration::from_secs(30)).unwrap();
    let claims = (1..=3)
        .map(|index| Claim {
            id: format!("job-{index}"),
            token: format!("token-{index}"),
            context: r#"{"args":{"wait_ms":1000}}"#.into(),
            timeout_ms: 2000,
        })
        .collect();
    send.send((
        0,
        Ok((
            lease,
            ClaimBatch {
                claims,
                oversized: Vec::new(),
            },
        )),
    ))
    .unwrap();
    let launches = AtomicUsize::new(0);
    let launch = move |context, deadline, lease_deadline| {
        if launches.fetch_add(1, Ordering::Relaxed) == 0 {
            TimedWorker::start(context, deadline, lease_deadline)
        } else {
            Err("injected launch error".into())
        }
    };
    let mut attempts = Vec::new();
    lane.poll(&runner, &mut queues, &mut attempts, &launch);
    let state = runner.state.lock();
    assert!(matches!(state.phase, Phase::Stopping { .. }));
    assert_eq!(state.pending_claims, 0);
    assert_eq!(state.counts.claimed, 3);
    assert_eq!(attempts.len(), 3);
    assert!(matches!(
        state.jobs["token-1"].execution,
        Execution::Running(_)
    ));
    for token in ["token-2", "token-3"] {
        assert!(matches!(
            state.jobs[token].execution,
            Execution::Acknowledging {
                reply: Reply::Unstarted,
                ..
            }
        ));
    }
}

#[test]
fn polling_and_kill_errors_keep_an_attempt_undrained_until_reap() {
    struct Worker {
        polls: std::collections::VecDeque<Result<WorkerPoll, String>>,
        kills: usize,
    }

    impl JobProcess for Worker {
        fn poll(&mut self, _: Instant) -> Result<WorkerPoll, String> {
            self.polls.pop_front().unwrap()
        }

        fn kill(&mut self) -> Result<(), String> {
            self.kills += 1;
            if self.kills == 1 {
                Err("could not signal worker".into())
            } else {
                Ok(())
            }
        }

        fn renew(&mut self, _: u64) -> Result<(), String> {
            unreachable!()
        }
    }

    let now = Instant::now();
    let mut attempt = Attempt {
        id: "job-id".into(),
        token: "token".into(),
        queue: 0,
        lease: None,
    };
    let runner = Runner {
        id: 0,
        config: Config::parse(TEST_CONFIG).unwrap(),
        check_worker: || Ok(()),
        state: Mutex::new(State {
            phase: Phase::Running,
            jobs: BTreeMap::from([(
                "token".into(),
                ActiveJob {
                    id: "job-id".into(),
                    queue: 0,
                    execution: Execution::Running(Running {
                        process: Box::new(Worker {
                            polls: [
                                Err("could not poll worker".into()),
                                Ok(WorkerPoll::Running),
                                Ok(WorkerPoll::Exited(Ok(r#"{"action":"complete"}"#.into()))),
                            ]
                            .into(),
                            kills: 0,
                        }),
                        deadline: now + TEST_WAIT,
                        lease_deadline: now + TEST_WAIT,
                        reaped: false,
                        stop: None,
                    }),
                },
            )]),
            pending_claims: 0,
            counts: Counts::default(),
            last_error: None,
        }),
    };

    runner.supervise();
    runner.request_stop(Duration::ZERO);
    assert!(attempt.poll(&runner, 0));
    let report: serde_json::Value =
        serde_json::from_str(&runner.report(&runner.state.lock())).unwrap();
    assert_eq!(report["state"], "stopping");
    assert_eq!(report["undrained_jobs"], json!(["job-id"]));
    assert_eq!(report["finished"], 0);

    runner.supervise();
    assert!(attempt.poll(&runner, 0));

    runner.supervise();
    assert!(!attempt.poll(&runner, 0));
    let state = runner.state.lock();
    let Execution::Acknowledging { reply, .. } = &state.jobs["token"].execution else {
        panic!("reaped worker must be eligible for settlement");
    };
    assert!(
        matches!(reply.outcome(), Outcome::Fail { reason } if reason == "could not poll worker")
    );
}

#[test]
#[cfg(unix)]
fn reaped_worker_reply_survives_timeout_and_shutdown() {
    let now = Instant::now();
    for (deadline, shutting_down) in [(now, false), (now + TEST_WAIT, true)] {
        let (process, publish, reply) = WorkerProcess::reaped_with_held_reply();
        let mut running = Running {
            process: Box::new(process),
            deadline,
            lease_deadline: now + TEST_WAIT,
            reaped: false,
            stop: None,
        };
        assert!(running.poll(now, shutting_down, &mut None).is_none());
        publish.send(reply).unwrap();
        match running.poll(now, shutting_down, &mut None) {
            Some(Reply::Returned(output)) => assert_eq!(output, "done"),
            Some(Reply::Failed(reason)) => panic!("reaped worker failed: {reason}"),
            _ => panic!("reaped worker did not return its reply"),
        }
    }
}

#[test]
#[cfg(unix)]
fn reaped_worker_reply_survives_a_confirmed_late_renewal() {
    for (observed_exit, published_reply) in [(true, false), (false, false), (false, true)] {
        let now = Instant::now();
        let (process, publish, reply) = WorkerProcess::reaped_with_held_reply();
        let mut running = Running {
            process: Box::new(process),
            deadline: now + TEST_WAIT,
            lease_deadline: now,
            reaped: false,
            stop: None,
        };
        if observed_exit {
            assert!(running.poll(now, false, &mut None).is_none());
        }
        if published_reply {
            publish.send(reply.clone()).unwrap();
        }

        let mut execution = Execution::Running(running);
        assert!(execution.apply_renewal(&Lease::begin(TEST_WAIT).unwrap(), &mut None));
        if !published_reply {
            publish.send(reply).unwrap();
        }
        let result = match execution {
            Execution::Running(mut worker) => worker.poll(now, false, &mut None),
            Execution::Acknowledging { reply, .. } => Some(reply),
        };
        assert!(matches!(result, Some(Reply::Returned(output)) if output == "done"));
    }
}

#[test]
#[cfg(unix)]
fn reaped_worker_keeps_a_previous_stop_while_awaiting_reply() {
    let now = Instant::now();
    let (process, _publish, _) = WorkerProcess::reaped_with_held_reply();
    let mut running = Running {
        process: Box::new(process),
        deadline: now,
        lease_deadline: now + TEST_WAIT,
        reaped: false,
        stop: Some(Reply::Cancelled),
    };
    assert!(matches!(
        running.poll(now, true, &mut None),
        Some(Reply::Cancelled)
    ));
}

#[test]
fn invalid_outcomes_are_failures() {
    assert!(matches!(
        Reply::Returned(r#"{"action":"complete"}"#.into()).outcome(),
        Outcome::Complete
    ));
    for invalid in [
        "not JSON",
        r#"{"action":"snooze","delay_ms":18446744073709551615}"#,
    ] {
        assert!(matches!(
            Reply::Returned(invalid.into()).outcome(),
            Outcome::Fail { .. }
        ));
    }
}

// The native proof exercises actual child processes. This worker holds a
// killed attempt past its first lease to test the runner's durable renewal.
struct TimedWorker {
    completes_at: Instant,
    kill_delay: Duration,
    reaps_at: Option<Instant>,
}

impl TimedWorker {
    fn start(context: String, _: Instant, _: u64) -> Result<Box<dyn JobProcess>, String> {
        let context: serde_json::Value = serde_json::from_str(&context).unwrap();
        let wait_ms = context["args"]["wait_ms"].as_u64().unwrap();
        let kill_delay = context["args"]["kill_delay_ms"].as_u64().unwrap_or(0);
        Ok(Box::new(Self {
            completes_at: Instant::now() + Duration::from_millis(wait_ms),
            kill_delay: Duration::from_millis(kill_delay),
            reaps_at: None,
        }))
    }
}

impl JobProcess for TimedWorker {
    fn poll(&mut self, _: Instant) -> Result<WorkerPoll, String> {
        let now = Instant::now();
        Ok(match self.reaps_at {
            Some(reaps_at) if now >= reaps_at => WorkerPoll::Exited(Err("worker killed".into())),
            None if now >= self.completes_at => {
                WorkerPoll::Exited(Ok(r#"{"action":"complete"}"#.into()))
            }
            _ => WorkerPoll::Running,
        })
    }

    fn kill(&mut self) -> Result<(), String> {
        self.reaps_at
            .get_or_insert(Instant::now() + self.kill_delay);
        Ok(())
    }

    fn renew(&mut self, _: u64) -> Result<(), String> {
        Ok(())
    }
}

#[test]
fn a_late_renewal_cannot_extend_an_expired_local_lease() {
    let now = Instant::now();
    let mut execution = Execution::Running(Running {
        process: TimedWorker::start(r#"{"args":{"wait_ms":2000}}"#.into(), now, 0).unwrap(),
        deadline: now + TEST_WAIT,
        lease_deadline: now,
        reaped: false,
        stop: None,
    });
    assert!(!execution.apply_renewal(&Lease::begin(TEST_WAIT).unwrap(), &mut None));
    let Execution::Running(mut worker) = execution else {
        panic!("worker must remain tracked until reaped");
    };
    assert_eq!(worker.lease_deadline, now);
    assert!(matches!(
        worker.poll(now, false, &mut None),
        Some(Reply::Failed(reason)) if reason == LEASE_LOST
    ));
}

#[test]
#[ignore = "requires MESH_TEST_DATABASE_URL"]
fn join_waits_for_maintenance_after_stop_grace() {
    use crate::db::pool::{mesh_pool_close, mesh_pool_open, unbox_u64_payload};
    use crate::io::MeshResult;
    use crate::string::mesh_str;

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

    let namespace = format!("runner-join-test-{}", rand::random::<u64>());
    let mut config: serde_json::Value = serde_json::from_str(TEST_CONFIG).unwrap();
    config["namespace"] = json!(namespace);
    let (joined, join_result) = mpsc::channel();
    let (stop_report, early_join) = with_pool(pool, |db| {
        db.transaction(|tx| {
            tx.simple("LOCK TABLE mesh_jobs.cron_activation IN ACCESS EXCLUSIVE MODE")?;
            let id = start_runner(
                pool,
                &config.to_string(),
                || Ok(()),
                Box::new(TimedWorker::start),
            )?;
            let deadline = Instant::now() + TEST_WAIT;
            loop {
                let rows = tx.query(
                    "SELECT EXISTS (SELECT 1 FROM pg_locks
                     WHERE relation = 'mesh_jobs.cron_activation'::regclass AND NOT granted
                     AND pg_backend_pid() = ANY(pg_blocking_pids(pid)))::text",
                    &[],
                )?;
                if rows[0][0].1 == "true" {
                    break;
                }
                if Instant::now() >= deadline {
                    return Err("runner maintenance did not wait on the table lock".into());
                }
                std::thread::sleep(TICK);
            }

            let stop_report = stop(id, 0)?;
            let (joining, started) = mpsc::channel();
            std::thread::spawn(move || {
                joining.send(()).unwrap();
                joined.send(join(id)).unwrap();
            });
            started
                .recv_timeout(TEST_WAIT)
                .map_err(|error| error.to_string())?;
            Ok((
                stop_report,
                join_result.recv_timeout(Duration::from_millis(100)),
            ))
        })
    })
    .unwrap();

    let report: serde_json::Value = serde_json::from_str(&stop_report).unwrap();
    assert_eq!(report["state"], "stopping");
    assert_eq!(report["drained"], true);
    assert!(
        matches!(early_join, Err(mpsc::RecvTimeoutError::Timeout)),
        "join returned while maintenance was blocked: {early_join:?}"
    );

    let report = join_result.recv_timeout(TEST_WAIT).unwrap().unwrap();
    let report: serde_json::Value = serde_json::from_str(&report).unwrap();
    assert_eq!(report["state"], "stopped");
    assert_eq!(report["drained"], true);
    assert_eq!(report["last_error"], serde_json::Value::Null);
    let activation = with_pool(pool, |db| {
        db.query(
            "DELETE FROM mesh_jobs.cron_activation WHERE namespace = $1 RETURNING namespace",
            &[&namespace],
        )
    })
    .unwrap();
    assert_eq!(activation.len(), 1);
    mesh_pool_close(pool);
}

#[test]
#[ignore = "requires MESH_TEST_DATABASE_URL"]
fn postgres_runner_executes_schedules_and_settles_renewed_timed_out_and_cancelled_attempts() {
    use crate::db::pool::{mesh_pool_close, mesh_pool_open, unbox_u64_payload};
    use crate::io::MeshResult;
    use crate::string::mesh_str;

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
    let id = start_runner(
        pool,
        &config.to_string(),
        || Ok(()),
        Box::new(TimedWorker::start),
    )
    .unwrap();

    let control = |request: serde_json::Value| -> serde_json::Value {
        let output = with_pool(pool, |db| {
            super::super::store::control(db, &namespace, &request.to_string())
        })
        .unwrap();
        serde_json::from_str(&output).unwrap()
    };

    let submit = |request: serde_json::Value| -> String {
        let output = with_pool(pool, |db| {
            db.transaction(|tx| super::super::store::enqueue(tx, &namespace, &request.to_string()))
        })
        .unwrap();
        let receipt: serde_json::Value = serde_json::from_str(&output).unwrap();
        receipt["id"].as_str().unwrap().to_string()
    };
    let enqueue = |wait_ms: u32, timeout_ms: u32| -> String {
        submit(json!({
            "worker": "test",
            "args": {"wait_ms": wait_ms},
            "max_attempts": 1,
            "timeout_ms": timeout_ms,
        }))
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

    let short = submit(json!({
        "worker": "test",
        "args": {"wait_ms": 2000, "kill_delay_ms": 400},
        "max_attempts": 1,
        "timeout_ms": 80,
    }));
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
    assert_eq!(
        report["outcomes"],
        json!({
            "completed": 2, "cancelled": 1, "discarded": 1, "retryable": 0,
            "scheduled": 0, "released_unstarted": 0,
        })
    );
    assert_eq!(report["scheduled"], 1);
    assert_eq!(join(id).unwrap(), report.to_string());
    assert_eq!(status(id).unwrap(), report.to_string());

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
    let one_shot = start_runner(
        pool,
        &drain_config.to_string(),
        || Ok(()),
        Box::new(TimedWorker::start),
    )
    .unwrap();
    std::thread::sleep(Duration::from_millis(100));
    let runner = lookup(one_shot).unwrap();
    assert!(!runner.state.lock().phase.is_stopped());
    release_lock.send(()).unwrap();
    lock_holder.join().unwrap();
    let deadline = Instant::now() + TEST_WAIT;
    while !runner.state.lock().phase.is_stopped() {
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
    assert_eq!(
        report["outcomes"],
        json!({
            "completed": 1, "cancelled": 0, "discarded": 0, "retryable": 0,
            "scheduled": 0, "released_unstarted": 0,
        })
    );
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

    let abandon = |queue: &str, max_attempts: u32| -> String {
        let id = submit(json!({"worker":"test", "queue":queue, "args":{"wait_ms":1},
            "max_attempts":max_attempts, "backoff_base_ms":0}));
        with_pool(pool, |db| {
            let claimed = claim(
                db,
                &namespace,
                queue,
                r#"[{"worker":"test","version":1}]"#,
                60_000,
            )?
            .unwrap();
            assert_eq!(claimed.id, id);
            Ok(())
        })
        .unwrap();
        id
    };
    let expires_later = abandon("default", 2);
    let locked_job = abandon("blocked", 1);
    let outside = abandon("outside", 1);
    let expire = |id: &str| {
        with_pool(pool, |db| {
            db.query(
                "UPDATE mesh_jobs.jobs SET lease_expires_at_ms = mesh_jobs.now_ms() - 1
             WHERE namespace = $1 AND id = $2::uuid",
                &[&namespace, id],
            )
        })
        .unwrap();
    };
    expire(&locked_job);
    expire(&outside);
    let (locked, lock_acquired) = mpsc::channel();
    let (release_lock, release) = mpsc::channel();
    let lock_ns = namespace.clone();
    let job_to_lock = locked_job.clone();
    let lock_holder = std::thread::spawn(move || {
        with_pool(pool, |db| db.transaction(|tx| {
            tx.query("SELECT id FROM mesh_jobs.jobs WHERE namespace = $1 AND id = $2::uuid FOR NO KEY UPDATE",
                &[&lock_ns, &job_to_lock])?;
            locked.send(()).unwrap();
            release.recv_timeout(TEST_WAIT).unwrap();
            Ok(())
        })).unwrap();
    });
    lock_acquired.recv_timeout(TEST_WAIT).unwrap();
    let mut two_queues = drain_config.clone();
    two_queues["queues"]["blocked"] = json!(1);
    let one_shot = start_runner(
        pool,
        &two_queues.to_string(),
        || Ok(()),
        Box::new(TimedWorker::start),
    )
    .unwrap();
    std::thread::sleep(Duration::from_millis(200));
    let runner = lookup(one_shot).unwrap();
    assert!(!runner.state.lock().phase.is_stopped());
    expire(&expires_later);
    assert_eq!(wait_for(&expires_later, "completed")["attempt"], 2);
    release_lock.send(()).unwrap();
    lock_holder.join().unwrap();
    let deadline = Instant::now() + TEST_WAIT;
    while !runner.state.lock().phase.is_stopped() {
        assert!(
            Instant::now() < deadline,
            "drain status: {}",
            status(one_shot).unwrap()
        );
        std::thread::sleep(TICK);
    }
    let report: serde_json::Value = serde_json::from_str(&join(one_shot).unwrap()).unwrap();
    assert_eq!(report["recovered"], 2);
    assert_eq!(report["finished"], 1);
    assert_eq!(report["outcomes"]["completed"], 1);
    assert_eq!(report["outcomes"]["discarded"], 0);
    assert_eq!(
        control(json!({"action":"get","id":locked_job}))["state"],
        "discarded"
    );
    assert_eq!(
        control(json!({"action":"get","id":outside}))["state"],
        "executing"
    );

    drain_config["drain"] = json!(false);
    drain_config["prune_max_age_ms"] = json!(1);
    let pruner = start_runner(
        pool,
        &drain_config.to_string(),
        || Ok(()),
        Box::new(TimedWorker::start),
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
