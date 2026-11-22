use super::super::store::claim;
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
    assert_eq!(config.event_buffer, 0);
    assert!(!config.event_log);
    assert!(!config.listen);
    assert!(!config.claim_batch);
    assert_eq!(config.poll_ms, 100);
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
        r#"{"queues":{"default":1},"workers":[{"worker":"test","version":1}],"event_buffer":1025}"#,
    ] {
        assert!(Config::parse(config).is_err(), "{config}");
    }
}

#[test]
fn effect_config_admits_only_one_batch_queue_and_a_bounded_integer_session_count() {
    let mut json = json!({
        "queues": {"default": 1},
        "workers": [{"worker": "test", "version": 1}],
        "claim_batch": true
    });
    let default = Config::parse_effects(&json.to_string()).unwrap();
    assert_eq!(default.effect_queue(), Some(("default", 1)));
    assert_eq!(default.effect_sessions, Some(1));
    assert!(Config::parse(&json.to_string())
        .unwrap()
        .effect_queue()
        .is_none());

    json["queues"]["default"] = json!(16);
    for lanes in [1, 4, 8, 16] {
        json["effect_sessions"] = json!(lanes);
        let config = Config::parse_effects(&json.to_string()).unwrap();
        assert_eq!(config.effect_queue(), Some(("default", 16)));
        assert_eq!(config.effect_sessions, Some(lanes));
        assert!(Config::parse(&json.to_string()).is_err());
    }
    for invalid in [
        json!(0),
        json!(-1),
        json!(17),
        json!("8"),
        json!(4.5),
        json!(null),
    ] {
        json["effect_sessions"] = invalid;
        assert!(Config::parse_effects(&json.to_string()).is_err());
    }
    json["effect_sessions"] = json!(4);
    for (key, disallowed) in [
        ("queues", json!({"default": 0})),
        ("queues", json!({"default": 65})),
        ("queues", json!({"default": 3})),
        ("queues", json!({"default": 8, "other": 8})),
        ("claim_batch", json!(false)),
    ] {
        let mut invalid = json.clone();
        invalid[key] = disallowed;
        assert!(Config::parse_effects(&invalid.to_string()).is_err());
    }

    json["queues"]["default"] = json!(64);
    json.as_object_mut().unwrap().remove("effect_sessions");
    let config = Config::parse_effects(&json.to_string()).unwrap();
    assert_eq!(config.effect_queue(), Some(("default", 64)));
    assert_eq!(config.effect_sessions, Some(1));
    for lanes in [8, 16] {
        json["effect_sessions"] = json!(lanes);
        let config = Config::parse_effects(&json.to_string()).unwrap();
        assert_eq!(config.effect_sessions, Some(lanes));
    }
    json["effect_sessions"] = json!(17);
    assert!(Config::parse_effects(&json.to_string()).is_err());
}

#[test]
fn effect_config_listen_admits_one_non_drain_batch_queue() {
    let mut json = json!({
        "queues": {"default": 1},
        "workers": [{"worker": "test", "version": 1}],
        "claim_batch": true,
        "listen": true
    });
    let config = Config::parse_effects(&json.to_string()).unwrap();
    assert_eq!(config.effect_queue(), Some(("default", 1)));
    assert_eq!(config.effect_sessions, Some(1));
    let ordinary = Config::parse(&json.to_string()).unwrap();
    assert!(ordinary.listen);
    assert!(ordinary.effect_queue().is_none());

    json["effect_sessions"] = json!(1);
    assert!(Config::parse_effects(&json.to_string()).is_ok());
    for (key, disallowed) in [
        ("queues", json!({"default": 1, "other": 1})),
        ("effect_sessions", json!(2)),
        ("claim_batch", json!(false)),
        ("drain", json!(true)),
    ] {
        let mut invalid = json.clone();
        invalid[key] = disallowed;
        assert!(Config::parse_effects(&invalid.to_string()).is_err());
    }
    for slots in 2..=64 {
        json["queues"]["default"] = json!(slots);
        let lanes = (slots / 2).min(16);
        json["effect_sessions"] = json!(lanes);
        let config = Config::parse_effects(&json.to_string()).unwrap();
        assert_eq!(config.effect_queue(), Some(("default", slots)));
        assert_eq!(config.effect_sessions, Some(lanes));
    }
    json["effect_sessions"] = json!(8);
    assert!(Config::parse_effects(&json.to_string()).is_ok());
    json["drain"] = json!(true);
    assert!(Config::parse_effects(&json.to_string()).is_err());
}

#[test]
fn shared_batch_reservation_denies_paused_pending_and_active() {
    let config = Config::parse_effects(
        r#"{"queues":{"default":1},"workers":[{"worker":"test","version":1}],"claim_batch":true,"listen":true}"#,
    )
    .unwrap();
    let runner = Runner {
        id: 0,
        state: Mutex::new(State::new(&config)),
        wakeups: Some(Arc::new(Wakeups::new(&config))),
        config,
        check_worker: || Ok(()),
        dispatcher: OnceLock::new(),
        reply_wake: crossbeam_channel::bounded(1).0,
        events: None,
    };
    let set_phase = |phase| runner.state.lock().queues.get_mut("default").unwrap().phase = phase;
    set_phase(QueuePhase::Paused);
    assert_eq!(reserve_claims(&runner, "default", 1), 0);
    set_phase(QueuePhase::Running);
    assert_eq!(reserve_claims(&runner, "default", 1), 1);
    assert_eq!(reserve_claims(&runner, "default", 1), 0);
    {
        let mut state = runner.state.lock();
        state.pending = None;
        state.jobs.insert(
            "lease".into(),
            ActiveJob {
                id: "job".into(),
                queue: "default".into(),
                execution: Execution::acknowledging(Reply::Unstarted(FailureKind::Shutdown), None),
                identity: None,
                reported_lease_loss: false,
            },
        );
    }
    assert_eq!(reserve_claims(&runner, "default", 1), 0);
    assert!(runner.state.lock().pending.is_none());
}

#[test]
fn effect_config_caps_local_control_and_preserves_reservations_across_scale_down() {
    let config = Config::parse_effects(
        r#"{"queues":{"default":64},"workers":[{"worker":"test","version":1}],"claim_batch":true,"effect_sessions":8}"#,
    )
    .unwrap();
    let runner = Runner {
        state: Mutex::new(State::new(&config)),
        config,
        id: 0,
        check_worker: || Ok(()),
        dispatcher: OnceLock::new(),
        reply_wake: crossbeam_channel::bounded(1).0,
        events: None,
        wakeups: None,
    };
    let (requests, requested) = mpsc::sync_channel(1);
    let (send, results) = mpsc::sync_channel(1);
    let lane = BatchLane { requests, results };
    lane.request(&runner, "default".into()).unwrap();
    let requested = requested.try_recv().unwrap();
    assert_eq!(requested.slots, 16);
    assert_eq!(reserve_claims(&runner, "default", 16), 0);
    let mut state = runner.state.lock();
    for (queue, action) in [
        ("other", LocalAction::Start { limit: Some(1) }),
        ("default", LocalAction::Start { limit: Some(65) }),
        ("default", LocalAction::Scale { limit: 65 }),
        ("default", LocalAction::Scale { limit: 0 }),
    ] {
        assert!(update_queue(&mut state, &runner.config, queue.into(), action).is_err());
    }
    update_queue(
        &mut state,
        &runner.config,
        "default".into(),
        LocalAction::Scale { limit: 4 },
    )
    .unwrap();
    assert_eq!(state.pending_for("default"), 16);
    drop(state);
    let claims = (0..requested.slots)
        .map(|index| Claim {
            id: format!("job-{index}"),
            token: format!("token-{index}"),
            context: r#"{"args":{"wait_ms":1000}}"#.into(),
            timeout_ms: 2000,
            observed_at_ms: None,
        })
        .collect();
    send.send((
        requested.queue,
        Ok((
            Lease::begin(Duration::from_secs(30)).unwrap(),
            ClaimBatch {
                claims,
                ..ClaimBatch::default()
            },
            None,
        )),
    ))
    .unwrap();
    let mut attempts = Vec::new();
    lane.poll(&runner, &mut attempts, &TimedWorker::start);
    let state = runner.state.lock();
    assert_eq!(state.pending_for("default"), 0);
    assert_eq!(state.counts.claimed, 16);
    assert_eq!(attempts.len(), 16);
    assert_eq!(state.jobs.len(), 16);
    assert_eq!(
        state
            .jobs
            .values()
            .filter(|job| matches!(job.execution, Execution::Running(_)))
            .count(),
        4
    );
    assert_eq!(
        state
            .jobs
            .values()
            .filter(|job| matches!(
                job.execution,
                Execution::Acknowledging {
                    reply: Reply::Unstarted(FailureKind::Shutdown),
                    ..
                }
            ))
            .count(),
        12
    );
    drop(state);
    assert_eq!(reserve_claims(&runner, "default", 16), 0);
    let mut state = runner.state.lock();
    update_queue(
        &mut state,
        &runner.config,
        "default".into(),
        LocalAction::Scale { limit: 64 },
    )
    .unwrap();
    assert_eq!(state.queues["default"].limit, 64);
    assert_eq!(state.jobs.len(), 16);
    assert_eq!(runner.config.effect_sessions, Some(8));
}

#[test]
fn wake_hints_coalesce_to_one_bit_per_configured_queue() {
    let mut config = Config::parse(TEST_CONFIG).unwrap();
    config.queues = (0..64)
        .map(|index| (format!("queue-{index:02}"), 1))
        .collect();
    let wakes = Wakeups::new(&config);
    assert!(!wakes.event(ListenEvent::Hint(&wake_key("unknown"))));
    assert!(wakes.event(ListenEvent::Subscribed));
    assert!(!wakes.event(ListenEvent::Subscribed));
    let first = wake_key("queue-00");
    let last = wake_key("queue-63");
    for _ in 0..1024 {
        assert!(!wakes.event(ListenEvent::Hint(&first)));
        assert!(!wakes.event(ListenEvent::Hint(&last)));
    }
    assert_eq!(
        wakes.take(),
        WakeHints {
            configured: 1 | 1 << 63,
            rescan: true,
            unknown: false,
        }
    );
    assert_eq!(wakes.take(), WakeHints::default());

    let mut queues = BTreeMap::from([
        ("queue-63".into(), LocalQueue::new(1)),
        ("added".into(), LocalQueue::new(1)),
    ]);
    let mut freed = HashSet::new();
    assert!(wakes.event(ListenEvent::Hint(&last)));
    wakes.apply(&mut queues, &mut freed);
    assert!(matches!(queues["queue-63"].source, ClaimWake::Notification));
    assert!(matches!(queues["added"].source, ClaimWake::Ready));
    let added = wake_key("added");
    assert!(wakes.event(ListenEvent::Hint(&added)));
    for _ in 0..1024 {
        assert!(!wakes.event(ListenEvent::Hint(&added)));
    }
    wakes.apply(&mut queues, &mut freed);
    assert!(matches!(queues["added"].source, ClaimWake::Notification));
    assert_eq!(wakes.take(), WakeHints::default());
    wakes.close();
    assert!(!wakes.event(ListenEvent::Subscribed));
    assert_eq!(wakes.report()["subscribed"], false);
    assert_eq!(wakes.report()["subscriptions"], 2);
}

#[test]
fn batch_contention_retries_back_off_and_definitive_empty_resets_them() {
    let mut config = Config::parse(TEST_CONFIG).unwrap();
    config.claim_batch = true;
    config.poll_ms = 12;
    let runner = Runner {
        id: 0,
        state: Mutex::new(State::new(&config)),
        config,
        check_worker: || Ok(()),
        dispatcher: OnceLock::new(),
        reply_wake: crossbeam_channel::bounded(1).0,
        events: None,
        wakeups: None,
    };
    let (requests, _) = mpsc::sync_channel(1);
    let (send, results) = mpsc::sync_channel(1);
    let lane = BatchLane { requests, results };
    let lease = Lease::begin(Duration::from_secs(30)).unwrap();
    let poll = |contention_hint, next_due| {
        runner.state.lock().pending = Some(Reservation {
            queue: "default".into(),
            slots: 1,
        });
        send.send((
            "default".into(),
            Ok((
                lease,
                ClaimBatch {
                    contention_hint,
                    ..ClaimBatch::default()
                },
                next_due,
            )),
        ))
        .unwrap();
        let before = Instant::now();
        lane.poll(&runner, &mut Vec::new(), &|_, _, _, _| {
            panic!("empty batch must not launch a worker")
        });
        let state = runner.state.lock();
        assert!(state.pending.is_none());
        (before, state.queues["default"].next_claim)
    };

    for ceiling_ms in [4, 8, 12, 12] {
        let (before, next) = poll(true, None);
        assert!(next >= before + Duration::from_millis(ceiling_ms / 2));
        assert!(next <= Instant::now() + Duration::from_millis(ceiling_ms));
    }
    let (before, next) = poll(false, None);
    assert!(next >= before + Duration::from_millis(12));
    let (_, next) = poll(true, None);
    assert!(next <= Instant::now() + Duration::from_millis(4));
    let due = Instant::now();
    assert_eq!(poll(true, Some(due)).1, due);
    let state = runner.state.lock();
    assert!(matches!(state.queues["default"].source, ClaimWake::Due));
    let report: serde_json::Value = serde_json::from_str(&runner.report(&state)).unwrap();
    assert_eq!(report["batch_contention_hints"], 6);
    assert_eq!(report["claimed"], 0);

    let mut queue = LocalQueue::new(1);
    let now = Instant::now();
    for _ in 0..2 {
        queue.retry_batch_claim(now, 1, None);
        assert_eq!(queue.next_claim, now + Duration::from_millis(1));
    }
}

#[test]
fn confirmed_batch_launch_failure_stops_later_launches_and_keeps_refunds_tracked() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let runner = Runner {
        id: 0,
        config: Config::parse(TEST_CONFIG).unwrap(),
        check_worker: || Ok(()),
        dispatcher: OnceLock::new(),
        reply_wake: crossbeam_channel::bounded(1).0,
        events: None,
        wakeups: None,
        state: Mutex::new(State {
            phase: Phase::Running,
            pending: Some(Reservation {
                queue: "default".into(),
                slots: 3,
            }),
            ..State::new(&Config::parse(TEST_CONFIG).unwrap())
        }),
    };
    let (requests, _) = mpsc::sync_channel(1);
    let (send, results) = mpsc::sync_channel(1);
    let lane = BatchLane { requests, results };
    let lease = Lease::begin(Duration::from_secs(30)).unwrap();
    runner
        .state
        .lock()
        .queues
        .get_mut("default")
        .unwrap()
        .batch_retry_ms = 8;
    let claims = (1..=3)
        .map(|index| Claim {
            id: format!("job-{index}"),
            token: format!("token-{index}"),
            context: r#"{"args":{"wait_ms":1000}}"#.into(),
            timeout_ms: 2000,
            observed_at_ms: None,
        })
        .collect();
    send.send((
        "default".into(),
        Ok((
            lease,
            ClaimBatch {
                claims,
                ..ClaimBatch::default()
            },
            None,
        )),
    ))
    .unwrap();
    let launches = AtomicUsize::new(0);
    let launch = move |context, deadline, lease_deadline, reply_wake| {
        if launches.fetch_add(1, Ordering::Relaxed) == 0 {
            TimedWorker::start(context, deadline, lease_deadline, reply_wake)
        } else {
            Err("injected launch error".into())
        }
    };
    let mut attempts = Vec::new();
    lane.poll(&runner, &mut attempts, &launch);
    let state = runner.state.lock();
    assert!(matches!(state.phase, Phase::Stopping { .. }));
    assert!(state.pending.is_none());
    assert_eq!(state.queues["default"].batch_retry_ms, 0);
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
                reply: Reply::Unstarted(_),
                ..
            }
        ));
    }
}

#[test]
fn effect_retirement_refunds_a_pending_batch_and_drains_the_active_attempt() {
    struct HeldEffectReply(Receiver<()>);

    impl JobProcess for HeldEffectReply {
        fn reply_pending(&self) -> bool {
            false
        }

        fn poll(&mut self, _: Instant) -> Result<WorkerPoll, String> {
            if self.0.try_recv().is_ok() {
                return Ok(WorkerPoll::Exited(Err(
                    "Jobs effect outcome is unconfirmed".into(),
                )));
            }
            Ok(WorkerPoll::Running)
        }

        fn limit_effect_deadline(&mut self, _: Instant) {
            panic!("retirement must preserve the handler and lease deadlines");
        }

        fn kill(&mut self) -> Result<(), String> {
            panic!("retirement must let the active worker drain");
        }

        fn renew(&mut self, _: u64) -> Result<(), String> {
            unreachable!()
        }
    }

    let config = Config::parse_effects(
        r#"{"queues":{"default":3},"workers":[{"worker":"test","version":1}],"claim_batch":true,"shutdown_grace_ms":0}"#,
    )
    .unwrap();
    #[allow(unused_mut)]
    let mut runner = Runner {
        id: 0,
        state: Mutex::new(State::new(&config)),
        config,
        check_worker: || Ok(()),
        dispatcher: OnceLock::new(),
        reply_wake: crossbeam_channel::bounded(1).0,
        events: None,
        wakeups: None,
    };
    let claim = |name: &str| Claim {
        id: name.into(),
        token: name.into(),
        context: "{}".into(),
        timeout_ms: 60_000,
        observed_at_ms: None,
    };
    let lease = Lease::begin(Duration::from_secs(60)).unwrap();
    let (finish_active, reply) = mpsc::channel();
    let reply = Mutex::new(Some(reply));
    let active = launch_confirmed(
        &runner,
        &mut runner.state.lock(),
        "default",
        lease,
        claim("active"),
        &move |_, _, _, _| Ok(Box::new(HeldEffectReply(reply.lock().take().unwrap()))),
    );

    let (requests, requested) = mpsc::sync_channel(1);
    let (send, results) = mpsc::sync_channel(1);
    let lane = BatchLane { requests, results };
    lane.request(&runner, "default".into()).unwrap();
    assert_eq!(requested.try_recv().unwrap().slots, 2);

    // On the production target the pending reply observes pool exhaustion.
    // Other targets exercise the same runner transition without a Linux pool.
    #[cfg(all(target_os = "linux", target_arch = "aarch64", target_env = "gnu"))]
    {
        runner.config.effects = Some(super::super::effects::Effects::retired_fixture());
    }
    #[cfg(not(all(target_os = "linux", target_arch = "aarch64", target_env = "gnu")))]
    runner.state.lock().drain_retired_effects();

    send.send((
        "default".into(),
        Ok((
            lease,
            ClaimBatch {
                claims: vec![claim("pending-1"), claim("pending-2")],
                ..ClaimBatch::default()
            },
            None,
        )),
    ))
    .unwrap();
    let mut attempts = vec![active];
    lane.poll(&runner, &mut attempts, &|_, _, _, _| {
        panic!("a claim returned after final retirement cannot launch")
    });
    assert_eq!(reserve_claims(&runner, "default", 1), 0);
    {
        let state = runner.state.lock();
        assert!(matches!(state.phase, Phase::DrainingRetiredEffects));
        assert_eq!(
            state.last_error.as_deref(),
            Some("Jobs effect session retired")
        );
        assert!(state.pending.is_none());
        assert_eq!(state.counts.claimed, 3);
    }
    assert_eq!(attempts.len(), 3);
    runner.supervise(true);
    {
        let state = runner.state.lock();
        let Execution::Running(worker) = &state.jobs["active"].execution else {
            panic!("the active worker is still running");
        };
        assert!(worker.stop.is_none());
    }

    let mut io = LifecycleIo {
        pool: 0,
        session: None,
    };
    for attempt in &mut attempts[1..] {
        assert!(
            !attempt.poll_with_ack(&runner, &mut io, |_, id, token, outcome| {
                assert_eq!(id, token);
                assert!(matches!(outcome, Outcome::ReleaseUnstarted));
                Ok(Settlement::ReleasedUnstarted)
            })
        );
        runner.state.lock().jobs.remove(&attempt.token);
    }
    assert_eq!(runner.state.lock().counts.outcomes.released_unstarted, 2);

    finish_active.send(()).unwrap();
    runner.supervise(true);
    assert!(
        !attempts[0].poll_with_ack(&runner, &mut io, |_, id, token, outcome| {
            assert_eq!((id, token), ("active", "active"));
            assert!(matches!(
                outcome,
                Outcome::Fail { reason } if reason == "Jobs effect outcome is unconfirmed"
            ));
            Ok(Settlement::Discarded)
        })
    );
    assert_eq!(runner.state.lock().counts.outcomes.released_unstarted, 2);
}

#[test]
fn explicit_queue_and_runner_stop_can_shorten_an_effect_retirement_drain() {
    let config = Config::parse_effects(
        r#"{"queues":{"default":1},"workers":[{"worker":"test","version":1}],"claim_batch":true}"#,
    )
    .unwrap();
    let mut state = State::new(&config);
    state.drain_retired_effects();
    assert!(!state.phase.stop_wait_complete());
    assert!(update_queue(&mut state, &config, "default".into(), LocalAction::Resume).is_err());

    let now = Instant::now();
    state.stop_at(now + TEST_WAIT);
    state.stop_at(now + TEST_WAIT * 2);
    assert!(matches!(state.phase, Phase::Stopping { deadline } if deadline == now + TEST_WAIT));

    // Keep the queue undrained so the stop cutoff remains observable.
    state.pending = Some(Reservation {
        queue: "default".into(),
        slots: 1,
    });
    for grace_ms in [0, 1_000] {
        update_queue(
            &mut state,
            &config,
            "default".into(),
            LocalAction::Stop {
                grace_ms: Some(grace_ms),
            },
        )
        .unwrap();
        assert!(matches!(
            state.queues["default"].phase,
            QueuePhase::Stopping(cutoff) if cutoff <= Instant::now()
        ));
    }
    state.stop_at(now);
    assert!(state.phase.stop_wait_complete());
}

#[test]
fn polling_and_kill_errors_keep_an_attempt_undrained_until_reap() {
    struct Worker {
        polls: std::collections::VecDeque<Result<WorkerPoll, String>>,
        kills: usize,
    }

    impl JobProcess for Worker {
        fn reply_pending(&self) -> bool {
            false
        }

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
        queue: "default".into(),
        lease: None,
        submissions: 0,
    };
    let runner = Runner {
        id: 0,
        config: Config::parse(TEST_CONFIG).unwrap(),
        check_worker: || Ok(()),
        dispatcher: OnceLock::new(),
        reply_wake: crossbeam_channel::bounded(1).0,
        events: None,
        wakeups: None,
        state: Mutex::new(State {
            phase: Phase::Running,
            jobs: BTreeMap::from([(
                "token".into(),
                ActiveJob {
                    id: "job-id".into(),
                    queue: "default".into(),
                    identity: None,
                    reported_lease_loss: false,
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
                        started_at: None,
                    }),
                },
            )]),
            ..State::new(&Config::parse(TEST_CONFIG).unwrap())
        }),
    };

    runner.supervise(false);
    assert!(runner.state.lock().last_error.is_none());
    runner.supervise(true);
    runner.request_stop(Duration::ZERO);
    let mut io = LifecycleIo {
        pool: 0,
        session: None,
    };
    assert!(attempt.poll(&runner, &mut io));
    let report: serde_json::Value =
        serde_json::from_str(&runner.report(&runner.state.lock())).unwrap();
    assert_eq!(report["state"], "stopping");
    assert_eq!(report["undrained_jobs"], json!(["job-id"]));
    assert_eq!(report["finished"], 0);

    runner.supervise(true);
    assert!(attempt.poll(&runner, &mut io));

    runner.supervise(true);
    assert!(!attempt.poll(&runner, &mut io));
    let state = runner.state.lock();
    let Execution::Acknowledging { reply, .. } = &state.jobs["token"].execution else {
        panic!("reaped worker must be eligible for settlement");
    };
    assert!(
        matches!(reply.outcome().0, Outcome::Fail { reason } if reason == "could not poll worker")
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
            started_at: None,
        };
        assert!(running
            .poll(now, shutting_down.then_some(STOPPED), &mut None)
            .is_none());
        publish.send(reply).unwrap();
        match running.poll(now, shutting_down.then_some(STOPPED), &mut None) {
            Some(Reply::Returned(output)) => assert_eq!(output, "done"),
            Some(Reply::Failed(_, reason)) => panic!("reaped worker failed: {reason}"),
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
            started_at: None,
        };
        if observed_exit {
            assert!(running.poll(now, None, &mut None).is_none());
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
            Execution::Running(mut worker) => worker.poll(now, None, &mut None),
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
        started_at: None,
    };
    assert!(matches!(
        running.poll(now, Some(STOPPED), &mut None),
        Some(Reply::Cancelled)
    ));
}

#[test]
fn invalid_outcomes_are_failures() {
    assert!(matches!(
        Reply::Returned(r#"{"action":"complete"}"#.into())
            .outcome()
            .0,
        Outcome::Complete
    ));
    for invalid in [
        "not JSON",
        r#"{"action":"snooze","delay_ms":18446744073709551615}"#,
    ] {
        assert!(matches!(
            Reply::Returned(invalid.into()).outcome().0,
            Outcome::Fail { .. }
        ));
    }
}

#[test]
fn acknowledgement_retry_preserves_token_without_reconstructing_a_commit() {
    for confirmed_retry in [Settlement::Stale, Settlement::Completed] {
        let token = "private-test-token";
        let job_id = "9f1ae775-5c0d-407e-a322-f04d33db8608";
        let claim = Claim {
            id: job_id.into(),
            token: token.into(),
            context: r#"{"attempt":1,"scheduled_at_ms":10}"#.into(),
            timeout_ms: 1000,
            observed_at_ms: Some(50),
        };
        let mut attempt = Attempt {
            id: job_id.into(),
            token: token.into(),
            queue: "default".into(),
            lease: Some(Lease::begin(TEST_WAIT).unwrap()),
            submissions: 0,
        };
        let config = Config::parse(TEST_CONFIG).unwrap();
        let state = State::new(&config);
        let runner = Runner {
            id: 15,
            config,
            check_worker: || Ok(()),
            dispatcher: OnceLock::new(),
            reply_wake: crossbeam_channel::bounded(1).0,
            events: Events::new(8, false).unwrap(),
            wakeups: None,
            state: Mutex::new(State {
                phase: Phase::Running,
                jobs: BTreeMap::from([(
                    token.into(),
                    ActiveJob {
                        id: job_id.into(),
                        queue: "default".into(),
                        identity: Some(Identity::new(&claim)),
                        reported_lease_loss: false,
                        execution: Execution::acknowledging(
                            Reply::Returned(r#"{"action":"complete"}"#.into()),
                            Some(42),
                        ),
                    },
                )]),
                ..state
            }),
        };
        let mut io = LifecycleIo {
            pool: 0,
            session: None,
        };
        assert!(
            attempt.poll_with_ack(&runner, &mut io, |_, id, sent_token, outcome| {
                assert_eq!((id, sent_token), (job_id, token));
                assert!(matches!(outcome, Outcome::Complete));
                Err(JobsAckError::Unconfirmed(
                    "sensitive connection error".into(),
                ))
            })
        );
        let read = || {
            let batch = runner.events.as_ref().unwrap().read(8).unwrap();
            assert!(!batch.contains(token));
            assert!(!batch.contains("sensitive connection error"));
            serde_json::from_str::<serde_json::Value>(&batch).unwrap()
        };
        let first = read();
        let events = first["events"].as_array().unwrap();
        assert_eq!(events.len(), 1);
        let first = &events[0];
        assert_eq!(first["event"], "ack_uncertain");
        assert_eq!(first["submission"], 1);
        assert_eq!(first["handler_duration_ms"], 42);
        assert!(first.get("settlement").is_none());
        assert_eq!(runner.state.lock().counts.outcomes.total(), 0);
        assert_eq!(runner.state.lock().counts.stale_acks, 0);

        // Run the next scheduled poll without depending on wall-clock sleeps.
        let mut state = runner.state.lock();
        let Execution::Acknowledging { retry_at, .. } =
            &mut state.jobs.get_mut(token).unwrap().execution
        else {
            panic!("uncertain acknowledgement must remain eligible for retry");
        };
        *retry_at = Some(Instant::now());
        drop(state);
        assert!(
            !attempt.poll_with_ack(&runner, &mut io, |_, id, sent_token, outcome| {
                assert_eq!((id, sent_token), (job_id, token));
                assert!(matches!(outcome, Outcome::Complete));
                Ok(confirmed_retry)
            })
        );
        let retry = read();
        let events = retry["events"].as_array().unwrap();
        assert_eq!(events.len(), 1);
        let retry = &events[0];
        assert_eq!(retry["submission"], 2);
        assert_eq!(retry["attempt"], first["attempt"]);
        let state = runner.state.lock();
        if confirmed_retry == Settlement::Stale {
            assert_eq!(retry["event"], "ack_stale");
            assert!(retry.get("settlement").is_none());
            assert_eq!(state.counts.outcomes.total(), 0);
            assert_eq!(state.counts.stale_acks, 1);
        } else {
            assert_eq!(retry["event"], "settlement_committed");
            assert_eq!(retry["settlement"], "completed");
            assert_eq!(state.counts.outcomes.total(), 1);
            assert_eq!(state.counts.stale_acks, 0);
        }
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
    fn start(
        context: String,
        _: Instant,
        _: u64,
        _: Sender<()>,
    ) -> Result<Box<dyn JobProcess>, String> {
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
    fn reply_pending(&self) -> bool {
        false
    }

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
        process: TimedWorker::start(
            r#"{"args":{"wait_ms":2000}}"#.into(),
            now,
            0,
            crossbeam_channel::bounded(1).0,
        )
        .unwrap(),
        deadline: now + TEST_WAIT,
        lease_deadline: now,
        reaped: false,
        stop: None,
        started_at: None,
    });
    assert!(!execution.apply_renewal(&Lease::begin(TEST_WAIT).unwrap(), &mut None));
    let Execution::Running(mut worker) = execution else {
        panic!("worker must remain tracked until reaped");
    };
    assert_eq!(worker.lease_deadline, now);
    assert!(matches!(
        worker.poll(now, None, &mut None),
        Some(Reply::Failed(_, reason)) if reason == LEASE_LOST
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
            // Isolate maintenance from the dispatcher's idle claim reservations.
            control(id, r#"{"queue":"default","action":"pause"}"#)?;
            let runner = lookup(id)?;
            let deadline = Instant::now() + TEST_WAIT;
            loop {
                let rows = tx.query(
                    "SELECT EXISTS (SELECT 1 FROM pg_locks
                     WHERE relation = 'mesh_jobs.cron_activation'::regclass AND NOT granted
                     AND pg_backend_pid() = ANY(pg_blocking_pids(pid)))::text",
                    &[],
                )?;
                if rows[0][0].1 == "true" && runner.state.lock().pending.is_none() {
                    break;
                }
                if Instant::now() >= deadline {
                    return Err("runner did not become claim-idle with maintenance blocked".into());
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
fn batch_lifecycle_survives_an_exhausted_application_pool() {
    use crate::db::pool::{mesh_pool_close, mesh_pool_open, unbox_u64_payload};
    use crate::io::MeshResult;
    use crate::string::mesh_str;

    crate::gc::mesh_rt_init();
    actor::mesh_rt_init_actor(2);
    let url = std::env::var("MESH_TEST_DATABASE_URL").expect("test PostgreSQL URL");
    let open_pool = || {
        let result = unsafe { &*mesh_pool_open(mesh_str(&url), 0, 1, 100).cast::<MeshResult>() };
        assert_eq!(result.tag, 0);
        unsafe { unbox_u64_payload(result.value) }
    };
    let pool = open_pool();
    let observer = open_pool();
    with_pool(observer, |db| {
        super::super::store::migrate(db)?;
        db.transaction(|tx| super::super::cron::migrate(tx))
    })
    .unwrap();
    let namespace = format!("batch-pool-{}", rand::random::<u64>());
    let config = json!({
        "namespace": namespace,
        "queues": {"default": 1},
        "workers": [{"worker":"test", "version":1}],
        "poll_ms": 10,
        "lease_ms": 500,
        "claim_batch": true,
    });
    let runner = start_runner(
        pool,
        &config.to_string(),
        || Ok(()),
        Box::new(TimedWorker::start),
    )
    .unwrap();
    with_pool(pool, |_| {
        let receipt = with_pool(observer, |db| {
            db.transaction(|tx| super::super::store::enqueue(
                tx, &namespace,
                r#"{"worker":"test","args":{"wait_ms":1100},"timeout_ms":3000,"max_attempts":1}"#,
            ))
        })?;
        let receipt: serde_json::Value = serde_json::from_str(&receipt).unwrap();
        let get = json!({"action":"get", "id": receipt["id"]}).to_string();
        let deadline = Instant::now() + TEST_WAIT;
        loop {
            let job = with_pool(observer, |db| {
                super::super::store::control(db, &namespace, &get)
            })?;
            let job: serde_json::Value = serde_json::from_str(&job).unwrap();
            if job["state"] == "completed" {
                assert_eq!(job["attempt"], 1);
                break;
            }
            assert!(
                Instant::now() < deadline && job["state"] != "discarded",
                "{job}"
            );
            std::thread::sleep(TICK);
        }
        Ok(())
    })
    .unwrap();
    stop(runner, 2000).unwrap();
    let report: serde_json::Value = serde_json::from_str(&join(runner).unwrap()).unwrap();
    assert_eq!(report["outcomes"]["completed"], 1);
    assert_eq!(report["stale_acks"], 0);
    mesh_pool_close(observer);
    mesh_pool_close(pool);
}

#[test]
#[ignore = "requires MESH_TEST_DATABASE_URL with CREATEDB on a disposable server"]
fn effect_listen_64_slots_wakes_and_settles_on_separate_sessions() {
    use crate::db::pool::{mesh_pool_close, mesh_pool_open, unbox_u64_payload};
    use crate::io::MeshResult;
    use crate::string::mesh_str;
    use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};

    crate::gc::mesh_rt_init();
    actor::mesh_rt_init_actor(2);
    let url = std::env::var("MESH_TEST_DATABASE_URL").expect("test PostgreSQL URL");
    let mut runner_url = url::Url::parse(&url).expect("valid test PostgreSQL URL");
    let database = format!("mesh_effect_listen_{:016x}", rand::random::<u64>());
    runner_url.set_path(&database);
    let open_pool = |url: &str| {
        let result = unsafe { &*mesh_pool_open(mesh_str(url), 1, 1, 500).cast::<MeshResult>() };
        assert_eq!(result.tag, 0);
        unsafe { unbox_u64_payload(result.value) }
    };
    let admin = open_pool(&url);
    let created = with_pool(admin, |db| {
        db.simple(&format!("CREATE DATABASE {database} TEMPLATE template0"))
    });
    if let Err(error) = created {
        mesh_pool_close(admin);
        panic!("could not create the isolated LISTEN test database: {error}");
    }

    let mut pool = None;
    let mut runner_id = None;
    let checked = catch_unwind(AssertUnwindSafe(|| {
        let isolated = open_pool(runner_url.as_str());
        pool = Some(isolated);
        check_effect_listen_64_slots(isolated, &mut runner_id);
    }));
    let joined = runner_id.map(|id| stop(id, 1000).and_then(|_| join(id)));
    if let Some(pool) = pool {
        mesh_pool_close(pool);
    }
    let dropped = with_pool(admin, |db| {
        db.simple(&format!("DROP DATABASE {database} WITH (FORCE)"))
    });
    mesh_pool_close(admin);
    if let Err(panic) = checked {
        if let Err(error) = dropped {
            eprintln!("could not drop LISTEN test database {database}: {error}");
        }
        resume_unwind(panic);
    }
    dropped.unwrap();
    let joined: serde_json::Value = serde_json::from_str(&joined.unwrap().unwrap()).unwrap();
    assert_eq!(joined["drained"], true);
    assert_eq!(joined["last_error"], serde_json::Value::Null);
}

fn check_effect_listen_64_slots(pool: u64, runner_id: &mut Option<u64>) {
    use std::sync::atomic::{AtomicBool, Ordering};

    struct HeldWorker(Option<Arc<AtomicBool>>);

    impl JobProcess for HeldWorker {
        fn reply_pending(&self) -> bool {
            false
        }

        fn poll(&mut self, _: Instant) -> Result<WorkerPoll, String> {
            Ok(match &self.0 {
                Some(release) if release.load(Ordering::Acquire) => {
                    WorkerPoll::Exited(Ok(r#"{"action":"complete"}"#.into()))
                }
                Some(_) => WorkerPoll::Running,
                None => WorkerPoll::Exited(Err("worker killed".into())),
            })
        }

        fn kill(&mut self) -> Result<(), String> {
            self.0 = None;
            Ok(())
        }

        fn renew(&mut self, _: u64) -> Result<(), String> {
            Ok(())
        }
    }

    with_pool(pool, |db| {
        super::super::store::migrate(db)?;
        db.transaction(|tx| super::super::cron::migrate(tx))
    })
    .unwrap();

    let namespace = format!("effect-listen-{}", rand::random::<u64>());
    let config = Config::parse_effects(
        &json!({
            "namespace": namespace,
            "queues": {"default": 64},
            "workers": [{"worker": "test", "version": 1}],
            "claim_batch": true,
            "listen": true,
            "effect_sessions": 8,
            "poll_ms": 60_000,
            "cron_interval_ms": 0,
        })
        .to_string(),
    )
    .unwrap();
    // Exercise real runner sessions with the test worker on hosts without Jobs.effect.
    let release = Arc::new(AtomicBool::new(false));
    let start_worker: Box<StartWorker> = Box::new({
        let release = Arc::clone(&release);
        move |_, _, _, _| Ok(Box::new(HeldWorker(Some(Arc::clone(&release)))))
    });
    let runner = start_configured(pool, config, || Ok(()), start_worker, None).unwrap();
    *runner_id = Some(runner.id);
    let report =
        || -> serde_json::Value { serde_json::from_str(&status(runner.id).unwrap()).unwrap() };
    let wait_for = |ready: &dyn Fn(&serde_json::Value) -> bool| {
        let deadline = Instant::now() + TEST_WAIT;
        loop {
            let snapshot = report();
            if ready(&snapshot) {
                return snapshot;
            }
            assert!(Instant::now() < deadline, "{snapshot}");
            std::thread::sleep(TICK);
        }
    };
    wait_for(&|snapshot| {
        snapshot["wakeups"]["subscription_scans"] == 1 && snapshot["pending_claims"] == 0
    });
    with_pool(pool, |db| {
        db.transaction(|tx| {
            tx.simple("SET LOCAL mesh_jobs.publish_wakeups = on")?;
            tx.query(
                "SELECT mesh_jobs.queue_control($1, 'default', 'configure', 64)",
                &[&namespace],
            )?;
            for _ in 0..64 {
                super::super::store::enqueue(
                    tx,
                    &namespace,
                    r#"{"worker":"test","max_attempts":1}"#,
                )?;
            }
            Ok(())
        })
    })
    .unwrap();
    let occupied = wait_for(&|snapshot| snapshot["queues"]["default"]["in_flight"] == 64);
    assert_eq!(occupied["claimed"], 64);
    assert_eq!(occupied["pending_claims"], 0);
    assert_eq!(occupied["finished"], 0);
    with_pool(pool, |db| {
        let rows = db.query(
            "SELECT count(*)::text FROM mesh_jobs.jobs
             WHERE namespace = $1 AND queue = 'default' AND state = 'executing' AND attempt = 1",
            &[&namespace],
        )?;
        assert_eq!(rows[0][0].1, "64");
        Ok(())
    })
    .unwrap();
    release.store(true, Ordering::Release);
    let settled = wait_for(&|snapshot| snapshot["outcomes"]["completed"] == 64);
    assert_eq!(settled["wakeups"]["notification_scans"], 1);
    assert_eq!(settled["wakeups"]["fallback_scans"], 0);

    let listen = format!("LISTEN \"mesh_jobs_wake_{}\"", wake_key(&namespace));
    wait_for(&|_| {
        with_pool(pool, |db| {
            let rows = db.query(
                "SELECT (count(*) = 3 AND bool_and(state = 'idle')
                     AND count(*) FILTER (WHERE query = 'COMMIT') = 1
                     AND count(*) FILTER (WHERE query LIKE 'SELECT coalesce(mesh_jobs.settle_state(%') = 1
                     AND count(*) FILTER (WHERE query = $1) = 1)::text
                 FROM pg_stat_activity
                 WHERE datname = current_database() AND backend_type = 'client backend'
                   AND pid <> pg_backend_pid()",
                &[&listen],
            )?;
            Ok(rows[0][0].1 == "true")
        })
        .unwrap()
    });
}

#[test]
#[ignore = "requires MESH_TEST_DATABASE_URL"]
fn committed_claim_without_confirmation_is_not_refunded_during_queue_stop() {
    use crate::db::pool::{mesh_pool_close, mesh_pool_open, unbox_u64_payload};
    use crate::io::MeshResult;
    use crate::string::mesh_str;

    crate::gc::mesh_rt_init();
    actor::mesh_rt_init_actor(2);
    let url = std::env::var("MESH_TEST_DATABASE_URL").expect("test PostgreSQL URL");
    let pool = unsafe { &*mesh_pool_open(mesh_str(&url), 0, 3, 500).cast::<MeshResult>() };
    assert_eq!(pool.tag, 0);
    let pool = unsafe { unbox_u64_payload(pool.value) };
    with_pool(pool, |db| super::super::store::migrate(db)).unwrap();
    let namespace = format!("queue-stop-uncertain-{}", rand::random::<u64>());
    let config = Config::parse(
        &json!({"namespace":namespace,"queues":{"default":1,"sibling":1},
                "workers":[{"worker":"test","version":1}],"lease_ms":60000})
        .to_string(),
    )
    .unwrap();
    let workers = serde_json::to_string(&config.workers).unwrap();
    let runner = Runner {
        state: Mutex::new(State::new(&config)),
        config,
        id: 0,
        check_worker: || Ok(()),
        dispatcher: OnceLock::new(),
        reply_wake: crossbeam_channel::bounded(1).0,
        events: None,
        wakeups: None,
    };
    let enqueue = |queue| {
        let spec = json!({"worker":"test","queue":queue,"args":{"wait_ms":0},
                          "max_attempts":2,"backoff_base_ms":0,"backoff_max_ms":0});
        let receipt = with_pool(pool, |db| {
            db.transaction(|tx| super::super::store::enqueue(tx, &namespace, &spec.to_string()))
        })
        .unwrap();
        serde_json::from_str::<serde_json::Value>(&receipt).unwrap()
    };
    let pending = enqueue("default");
    let sibling = enqueue("sibling");
    let id = pending["id"].as_str().unwrap();
    let lost = dispatch_single_claim(
        &runner,
        "default",
        &|_, _, _, _| panic!("a claim without a confirmation cannot start a worker"),
        || {
            let mut state = runner.state.lock();
            assert_eq!(state.pending_for("default"), 1);
            update_queue(
                &mut state,
                &runner.config,
                "default".into(),
                LocalAction::Stop { grace_ms: Some(0) },
            )?;
            assert!(matches!(
                state.queues["default"].phase,
                QueuePhase::Stopping(_)
            ));
            drop(state);
            // Commit through the real store, then simulate failure at the caller boundary.
            let committed = with_pool(pool, |db| {
                claim(db, &namespace, "default", &workers, 60_000)
            })?
            .expect("the transaction must have claimed the original job");
            assert_eq!(committed.id, id);
            Err("claim confirmation unavailable".into())
        },
    );
    assert!(matches!(lost, Err(error) if error == "claim confirmation unavailable"));
    {
        let mut state = runner.state.lock();
        state.refresh_stopping_queues();
        assert!(state.pending.is_none());
        assert!(state.jobs.is_empty());
        assert!(matches!(state.queues["default"].phase, QueuePhase::Stopped));
        assert!(matches!(state.queues["sibling"].phase, QueuePhase::Running));
        assert_eq!(state.counts.claimed, 0);
    }
    let first_token = with_pool(pool, |db| {
        db.query("SELECT lease_token::text FROM mesh_jobs.jobs WHERE namespace = $1 AND id = $2::uuid
                  AND state = 'executing' AND attempt = 1 AND lease_expires_at_ms > mesh_jobs.now_ms()",
                 &[&namespace, id])
    }).unwrap()[0][0].1.clone();
    assert!(!first_token.is_empty());

    let (sibling_attempt, _) =
        claim_one(&runner, pool, &workers, "sibling", &TimedWorker::start).unwrap();
    let mut sibling_attempt = sibling_attempt.expect("sibling queue should still claim");
    assert_eq!(sibling_attempt.id, sibling["id"].as_str().unwrap());
    runner.supervise(true);
    assert!(!sibling_attempt.poll(
        &runner,
        &mut LifecycleIo {
            pool,
            session: None
        }
    ));
    let sibling_state = with_pool(pool, |db| {
        super::super::store::control(
            db,
            &namespace,
            &json!({"action":"get","id":sibling["id"]}).to_string(),
        )
    })
    .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&sibling_state).unwrap()["state"],
        "completed"
    );

    with_pool(pool, |db| {
        db.query(
            "UPDATE mesh_jobs.jobs SET lease_expires_at_ms = mesh_jobs.now_ms() - 1
                  WHERE namespace = $1 AND id = $2::uuid AND lease_token = $3::uuid",
            &[&namespace, id, &first_token],
        )?;
        assert_eq!(recover(db, &namespace, 1, Some(r#"{"default":1}"#))?, 1);
        let replacement = claim(db, &namespace, "default", &workers, 60_000)?.unwrap();
        assert_eq!(replacement.id, id);
        assert_ne!(replacement.token, first_token);
        assert_eq!(
            finish(db, &namespace, id, &first_token, Outcome::Complete).unwrap(),
            Settlement::Stale
        );
        let context: serde_json::Value = serde_json::from_str(&replacement.context).unwrap();
        assert_eq!(context["attempt"], 2);
        assert_eq!(context["effect_key"], pending["effect_key"]);
        assert_eq!(
            finish(db, &namespace, id, &replacement.token, Outcome::Complete).unwrap(),
            Settlement::Completed
        );
        Ok(())
    })
    .unwrap();
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
        "event_buffer": 64,
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
    let batch: serde_json::Value = serde_json::from_str(&read_events(id, 64).unwrap()).unwrap();
    let events = batch["events"].as_array().unwrap();
    assert_eq!(batch["closed"], true);
    assert_eq!(events.first().unwrap()["event"], "runner_started");
    assert_eq!(events.last().unwrap()["event"], "runner_stopped");
    assert!(events.iter().any(|event| event["event"] == "maintenance"
        && event["operation"] == "cron"
        && event["count"] == 1));
    assert!(events
        .iter()
        .any(|event| event["event"] == "settlement_committed"
            && event["attempt"]["job_id"] == short
            && event["failure_kind"] == "timeout"));

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
    let batch: serde_json::Value =
        serde_json::from_str(&read_events(one_shot, 64).unwrap()).unwrap();
    assert!(batch["events"].as_array().unwrap().iter().any(|event| {
        event["event"] == "maintenance" && event["operation"] == "recover" && event["count"] == 1
    }));
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
        let id = submit(
            json!({"worker":"test", "queue":queue, "args":{"wait_ms":675},
            "max_attempts":max_attempts, "backoff_base_ms":0}),
        );
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
    two_queues["event_buffer"] = json!(1);
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
    assert!(report["events_dropped"].as_u64().unwrap() >= 3);
    let reader = &report["event_buffer_bytes"]["reader"];
    assert!(reader["upper_bound_bytes"].as_u64().unwrap() > 0);
    assert_eq!(reader["sampled_peak_bytes"], reader["upper_bound_bytes"]);
    assert_eq!(
        report["event_buffer_bytes"]["logger"],
        serde_json::Value::Null
    );
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
    let batch: serde_json::Value = serde_json::from_str(&read_events(pruner, 64).unwrap()).unwrap();
    assert!(batch["events"].as_array().unwrap().iter().any(|event| {
        event["event"] == "maintenance"
            && event["operation"] == "prune"
            && event["count"].as_u64().unwrap() >= 6
    }));
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
