use super::*;

#[test]
fn jobs_reopen_preserves_accepted_arguments_and_identity() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("jobs.redb");
    let deployment = DeploymentId::new("example").unwrap();
    let store = JobStore::open(&path, deployment.clone(), StoreLimits::default()).unwrap();
    let queue = QueueId::new("mail").unwrap();
    let configure = Command::new(
        ClientId::new("client").unwrap(),
        RequestId::new("configure").unwrap(),
        Operation::ConfigureQueue {
            queue: queue.clone(),
            policy: QueuePolicy::new(2, 10).unwrap(),
        },
    )
    .unwrap();
    store.apply(1, &configure).unwrap();
    let job = NewJob::new(
        JobId::new("welcome").unwrap(),
        queue,
        WorkerKey::new("send-mail", 1, "schema-v1").unwrap(),
        EncodedArgs::new(vec![0, 1, 255]).unwrap(),
        123,
        RetryPolicy::new(3, 2, 10).unwrap(),
        None,
    );
    let submit = Command::new(
        ClientId::new("client").unwrap(),
        RequestId::new("submit").unwrap(),
        Operation::Enqueue {
            jobs: vec![job.clone()],
        },
    )
    .unwrap();
    let receipt = store.apply(2, &submit).unwrap();
    assert!(receipt.result.is_ok());
    drop(store);
    let store = JobStore::open(&path, deployment, StoreLimits::default()).unwrap();
    let saved = store.job(&job.id).unwrap().unwrap();
    assert_eq!(saved.spec, job);
    assert_eq!(saved.state, JobState::Pending);
    assert_eq!(store.apply(2, &submit).unwrap(), receipt);
}

struct Fixture {
    _directory: tempfile::TempDir,
    store: JobStore,
    index: u64,
}
fn id(s: &str) -> JobId {
    JobId::new(s).unwrap()
}
fn queue(s: &str) -> QueueId {
    QueueId::new(s).unwrap()
}
fn worker() -> WorkerKey {
    WorkerKey::new("send-mail", 1, "schema-v1").unwrap()
}
fn spec(name: &str, q: &str) -> NewJob {
    NewJob::new(
        id(name),
        queue(q),
        worker(),
        EncodedArgs::new(vec![0, 128, 255]).unwrap(),
        0,
        RetryPolicy::new(3, 2, 8).unwrap(),
        None,
    )
}
fn command(request: &str, operation: Operation) -> Command {
    Command::new(
        ClientId::new("client").unwrap(),
        RequestId::new(request).unwrap(),
        operation,
    )
    .unwrap()
}
fn token(job: &Job) -> LeaseToken {
    let JobState::Leased(lease) = &job.state else {
        panic!("expected lease, got {:?}", job.state)
    };
    lease.token.clone()
}
impl Fixture {
    fn new() -> Self {
        Self::with_limits(StoreLimits::default())
    }
    fn with_limits(limits: StoreLimits) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let store = JobStore::open(
            &directory.path().join("jobs.redb"),
            DeploymentId::new("app").unwrap(),
            limits,
        )
        .unwrap();
        Self {
            _directory: directory,
            store,
            index: 0,
        }
    }
    fn apply(&mut self, operation: Operation) -> Receipt {
        self.index += 1;
        self.store
            .apply(self.index, &command(&format!("r{}", self.index), operation))
            .unwrap()
    }
    fn ok(&mut self, operation: Operation) -> Effect {
        self.apply(operation).result.unwrap()
    }
    fn configure(&mut self, name: &str, concurrency: u32) {
        self.ok(Operation::ConfigureQueue {
            queue: queue(name),
            policy: QueuePolicy::new(concurrency, 10).unwrap(),
        });
    }
    fn enqueue(&mut self, jobs: Vec<NewJob>) {
        self.ok(Operation::Enqueue { jobs });
    }
    fn job(&self, name: &str) -> Job {
        self.store.job(&id(name)).unwrap().unwrap()
    }
    fn claim_op(&self, name: &str) -> Operation {
        Operation::Claim {
            id: id(name),
            revision: self.job(name).revision,
            holder: WorkerSession::new("process-incarnation-1").unwrap(),
            worker: Some(worker()),
        }
    }
    fn claim(&mut self, name: &str) -> LeaseToken {
        self.ok(self.claim_op(name));
        token(&self.job(name))
    }
    fn tick(&mut self, tick: u64, utc_ms: u64) {
        self.ok(Operation::Tick {
            tick,
            utc_ms,
            expire_limit: MAX_PAGE,
        });
    }
    fn active(&self, q: &str) -> u32 {
        self.store.queue(&queue(q)).unwrap().unwrap().active_leases
    }
    fn invariant(&self, q: &str) {
        let jobs = self.store.inspect(&queue(q), None, MAX_PAGE).unwrap();
        assert_eq!(
            self.active(q),
            jobs.iter()
                .filter(|j| matches!(j.state, JobState::Leased(_)))
                .count() as u32
        );
        for job in jobs {
            if let JobState::Leased(lease) = job.state {
                assert_eq!(lease.token.job, job.spec.id);
                assert_eq!(lease.token.run, job.run);
                assert!(lease.token.claim_index <= job.revision.0);
            }
        }
    }
}

#[test]
fn jobs_replay_returns_original_receipt_and_conflicting_body_never_mutates() {
    let mut f = Fixture::new();
    f.configure("mail", 2);
    let submit = command(
        "submit",
        Operation::Enqueue {
            jobs: vec![spec("a", "mail")],
        },
    );
    let original = f.store.apply(2, &submit).unwrap();
    assert_eq!(f.store.apply(2, &submit).unwrap(), original);
    assert_eq!(f.store.apply(3, &submit).unwrap(), original);
    let conflict = command(
        "submit",
        Operation::Enqueue {
            jobs: vec![spec("b", "mail")],
        },
    );
    assert_eq!(
        f.store.apply(4, &conflict).unwrap().result,
        Err(Rejection::RequestConflict)
    );
    assert!(matches!(
        f.store.apply(2, &conflict),
        Err(Error::LogConflict)
    ));
    assert!(f.store.job(&id("b")).unwrap().is_none());
    assert_eq!(f.store.receipt(&submit).unwrap(), Some(original));
    assert_eq!(f.store.applied_index().unwrap(), 4);
    assert_eq!(
        f.store
            .queue(&queue("mail"))
            .unwrap()
            .unwrap()
            .retained_jobs,
        1
    );
    let other = Command::new(
        ClientId::new("other-client").unwrap(),
        RequestId::new("submit").unwrap(),
        conflict.operation,
    )
    .unwrap();
    assert!(f.store.apply(5, &other).unwrap().result.is_ok());
}

#[test]
fn jobs_rejected_batches_leave_no_job_indexes_counters_or_uniqueness_claims() {
    let mut f = Fixture::new();
    f.configure("mail", 2);
    f.configure("other", 2);
    let mut a = spec("a", "mail");
    a.unique = Some(UniquenessKey::new("welcome").unwrap());
    let invalid = command(
        "bad-batch",
        Operation::Enqueue {
            jobs: vec![a.clone(), spec("b", "missing")],
        },
    );
    f.index += 1;
    let rejected = f.store.apply(f.index, &invalid).unwrap();
    assert_eq!(rejected.result, Err(Rejection::UnknownQueue));
    assert!(f
        .store
        .inspect(&queue("mail"), None, 10)
        .unwrap()
        .is_empty());
    assert!(f
        .store
        .due(&queue("mail"), None, 10)
        .unwrap()
        .jobs
        .is_empty());
    assert_eq!(
        f.store
            .queue(&queue("mail"))
            .unwrap()
            .unwrap()
            .retained_jobs,
        0
    );
    f.enqueue(vec![a.clone(), spec("b", "other")]);
    assert_eq!(f.store.apply(f.index + 1, &invalid).unwrap(), rejected);
    f.index += 1;
    let mut duplicate = spec("c", "mail");
    duplicate.unique = a.unique;
    assert_eq!(
        f.apply(Operation::Enqueue {
            jobs: vec![spec("d", "other"), duplicate]
        })
        .result,
        Err(Rejection::DuplicateUnique)
    );
    assert!(f.store.job(&id("d")).unwrap().is_none());
    let member = spec("e", "mail");
    assert_eq!(
        f.apply(Operation::Enqueue {
            jobs: vec![member.clone(), member.clone()]
        })
        .result,
        Err(Rejection::DuplicateJob)
    );
    assert!(f.store.job(&member.id).unwrap().is_none());
    let mut unique1 = spec("f", "mail");
    unique1.unique = Some(UniquenessKey::new("another").unwrap());
    let mut unique2 = unique1.clone();
    unique2.id = id("g");
    assert_eq!(
        f.apply(Operation::Enqueue {
            jobs: vec![unique1.clone(), unique2]
        })
        .result,
        Err(Rejection::DuplicateUnique)
    );
    f.enqueue(vec![unique1, member]);
    f.invariant("mail");
    f.invariant("other");
}

#[test]
fn jobs_stale_finish_cannot_release_successor_and_manual_retry_starts_new_run() {
    let mut f = Fixture::new();
    f.configure("mail", 1);
    f.enqueue(vec![spec("a", "mail")]);
    let old = f.claim("a");
    f.tick(10, 0);
    assert_eq!(f.active("mail"), 0);
    assert_eq!(f.apply(f.claim_op("a")).result, Err(Rejection::NotReady));
    f.tick(12, 0);
    let successor = f.claim("a");
    assert_ne!(old, successor);
    assert_eq!(
        f.apply(Operation::Complete {
            token: old.clone(),
            outcome: Finish::Success
        })
        .result,
        Err(Rejection::StaleLease)
    );
    assert_eq!(f.active("mail"), 1);
    assert_eq!(token(&f.job("a")), successor);
    f.ok(Operation::Complete {
        token: successor.clone(),
        outcome: Finish::Success,
    });
    let completed = f.job("a");
    assert_eq!(
        f.apply(Operation::Retry {
            id: id("a"),
            revision: Revision(completed.revision.0 - 1)
        })
        .result,
        Err(Rejection::StaleRevision)
    );
    f.ok(Operation::Retry {
        id: id("a"),
        revision: completed.revision,
    });
    let next_run = f.claim("a");
    assert_eq!(next_run.run.0, successor.run.0 + 1);
    assert_eq!(f.job("a").attempts, 1);
    assert_eq!(
        f.apply(Operation::Complete {
            token: successor,
            outcome: Finish::Success
        })
        .result,
        Err(Rejection::StaleLease)
    );
    f.invariant("mail");
}

#[test]
fn jobs_renewal_before_expiration_rechecks_latest_deadline_and_reverse_order_fails() {
    for renew_first in [true, false] {
        let mut f = Fixture::new();
        f.configure("mail", 1);
        f.enqueue(vec![spec("a", "mail")]);
        let lease = f.claim("a");
        f.tick(9, 0);
        if renew_first {
            f.ok(Operation::Renew {
                token: lease.clone(),
            });
            f.tick(10, 0);
            assert_eq!(token(&f.job("a")), lease);
            assert_eq!(f.active("mail"), 1);
            f.tick(19, 0);
            assert_eq!(f.job("a").state, JobState::Pending);
        } else {
            f.tick(10, 0);
            assert_eq!(
                f.apply(Operation::Renew { token: lease }).result,
                Err(Rejection::StaleLease)
            );
        }
        assert_eq!(f.active("mail"), 0);
        f.invariant("mail");
    }
}

#[test]
fn jobs_cancel_success_and_claim_orders_preserve_the_active_slot() {
    let mut f = Fixture::new();
    f.configure("mail", 1);
    f.enqueue(vec![
        spec("a", "mail"),
        spec("b", "mail"),
        spec("c", "mail"),
    ]);
    let stale_claim = f.claim_op("a");
    f.ok(Operation::Cancel {
        id: id("a"),
        revision: f.job("a").revision,
    });
    assert_eq!(f.apply(stale_claim).result, Err(Rejection::StaleRevision));
    let lease = f.claim("b");
    f.ok(Operation::Cancel {
        id: id("b"),
        revision: f.job("b").revision,
    });
    assert_eq!(f.active("mail"), 1);
    assert_eq!(
        f.apply(f.claim_op("c")).result,
        Err(Rejection::ConcurrencyLimit)
    );
    assert_eq!(
        f.apply(Operation::Renew {
            token: lease.clone()
        })
        .result,
        Err(Rejection::InvalidState)
    );
    f.ok(Operation::Complete {
        token: lease,
        outcome: Finish::Success,
    });
    assert_eq!(
        f.job("b").state,
        JobState::Terminal(TerminalOutcome::Cancelled)
    );
    let lease = f.claim("c");
    let cancel_before_finish = Operation::Cancel {
        id: id("c"),
        revision: f.job("c").revision,
    };
    f.ok(Operation::Complete {
        token: lease,
        outcome: Finish::Success,
    });
    assert_eq!(
        f.apply(cancel_before_finish).result,
        Err(Rejection::StaleRevision)
    );
    assert_eq!(
        f.job("c").state,
        JobState::Terminal(TerminalOutcome::Completed)
    );
    f.ok(Operation::Retry {
        id: id("b"),
        revision: f.job("b").revision,
    });
    f.claim("b");
    f.ok(Operation::Cancel {
        id: id("b"),
        revision: f.job("b").revision,
    });
    f.tick(10, 0);
    assert_eq!(
        f.job("b").state,
        JobState::Terminal(TerminalOutcome::Cancelled)
    );
    assert_eq!(f.active("mail"), 0);
}

#[test]
fn jobs_retry_backoff_exhaustion_and_discard_are_revision_fenced() {
    let mut f = Fixture::new();
    f.configure("mail", 1);
    f.enqueue(vec![spec("a", "mail")]);
    for (attempt, tick, next_tick) in [(1, 0, 2), (2, 2, 6), (3, 6, 6)] {
        f.tick(tick, 0);
        let lease = f.claim("a");
        assert_eq!(f.job("a").attempts, attempt);
        f.ok(Operation::Complete {
            token: lease,
            outcome: Finish::Failure,
        });
        assert_eq!(f.active("mail"), 0);
        if attempt < 3 {
            assert_eq!(f.job("a").ready_at_tick, next_tick);
            assert_eq!(f.apply(f.claim_op("a")).result, Err(Rejection::NotReady));
        }
    }
    assert_eq!(
        f.job("a").state,
        JobState::Terminal(TerminalOutcome::Exhausted)
    );
    let exhausted = f.job("a").revision;
    f.ok(Operation::Retry {
        id: id("a"),
        revision: exhausted,
    });
    assert_eq!(
        f.apply(Operation::Discard {
            id: id("a"),
            revision: exhausted
        })
        .result,
        Err(Rejection::StaleRevision)
    );
    f.ok(Operation::Discard {
        id: id("a"),
        revision: f.job("a").revision,
    });
    assert_eq!(
        f.job("a").state,
        JobState::Terminal(TerminalOutcome::Discarded)
    );
    assert_eq!(
        f.apply(Operation::Enqueue {
            jobs: vec![spec("a", "mail")]
        })
        .result,
        Err(Rejection::DuplicateJob)
    );
}

#[test]
fn jobs_pause_and_limit_reduction_drain_existing_leases() {
    let mut f = Fixture::new();
    f.configure("mail", 2);
    f.enqueue(vec![
        spec("a", "mail"),
        spec("b", "mail"),
        spec("c", "mail"),
    ]);
    let a = f.claim("a");
    let b = f.claim("b");
    f.configure("mail", 1);
    assert_eq!(f.active("mail"), 2);
    assert_eq!(
        f.apply(f.claim_op("c")).result,
        Err(Rejection::ConcurrencyLimit)
    );
    f.ok(Operation::SetPaused {
        queue: queue("mail"),
        paused: true,
    });
    f.ok(Operation::Renew { token: a.clone() });
    f.ok(Operation::Complete {
        token: a,
        outcome: Finish::Success,
    });
    f.ok(Operation::Complete {
        token: b,
        outcome: Finish::Success,
    });
    assert_eq!(f.apply(f.claim_op("c")).result, Err(Rejection::Paused));
    f.ok(Operation::SetPaused {
        queue: queue("mail"),
        paused: false,
    });
    f.claim("c");
    assert_eq!(f.active("mail"), 1);
}

#[test]
fn jobs_missing_incompatible_worker_blocks_without_using_attempt_or_slot() {
    let mut f = Fixture::new();
    f.configure("mail", 1);
    f.enqueue(vec![spec("a", "mail")]);
    for (worker, reason) in [
        (None, BlockReason::MissingWorker),
        (
            Some(WorkerKey::new("send-mail", 2, "schema-v1").unwrap()),
            BlockReason::IncompatibleWorker,
        ),
        (
            Some(WorkerKey::new("send-mail", 1, "schema-v2").unwrap()),
            BlockReason::IncompatibleWorker,
        ),
    ] {
        f.ok(Operation::Claim {
            id: id("a"),
            revision: f.job("a").revision,
            holder: WorkerSession::new("session").unwrap(),
            worker,
        });
        assert_eq!(f.job("a").state, JobState::Blocked(reason));
        assert_eq!(f.job("a").attempts, 0);
        assert_eq!(f.active("mail"), 0);
        assert_eq!(f.store.due(&queue("mail"), None, 10).unwrap().jobs.len(), 1);
    }
    f.claim("a");
    assert_eq!(f.active("mail"), 1);
}

#[test]
fn jobs_inspection_schedule_and_history_are_bounded_and_persistent() {
    let mut f = Fixture::new();
    f.configure("mail", 3);
    let mut a = spec("a", "mail");
    a.scheduled_at_ms = 100;
    f.enqueue(vec![a, spec("b", "mail"), spec("c", "mail")]);
    assert_eq!(f.apply(f.claim_op("a")).result, Err(Rejection::NotReady));
    let page = f.store.due(&queue("mail"), None, 1).unwrap();
    assert_eq!(page.jobs[0].spec.id, id("b"));
    let next = f.store.due(&queue("mail"), page.next.as_ref(), 1).unwrap();
    assert_eq!(next.jobs[0].spec.id, id("c"));
    assert!(f.store.due(&queue("other"), page.next.as_ref(), 1).is_err());
    f.tick(5, 100);
    let lease = f.claim("a");
    for _ in 0..40 {
        f.ok(Operation::Renew {
            token: lease.clone(),
        });
    }
    let history = f.store.history(&id("a"), Revision(0), MAX_PAGE).unwrap();
    assert_eq!(history.len(), HISTORY_LIMIT);
    assert_eq!(history.last().unwrap().revision, f.job("a").revision);
    assert_eq!(
        f.store.inspect(&queue("mail"), Some(&id("a")), 1).unwrap()[0]
            .spec
            .id,
        id("b")
    );
    assert!(f.store.inspect(&queue("mail"), None, MAX_PAGE + 1).is_err());
    f.tick(5, 50);
    assert_eq!(f.store.clock().unwrap(), (5, 100));
    assert_eq!(
        f.apply(Operation::Tick {
            tick: 4,
            utc_ms: 150,
            expire_limit: 1
        })
        .result,
        Err(Rejection::ClockRegression)
    );
    let path = f._directory.path().join("jobs.redb");
    drop(f.store);
    let store = JobStore::open(
        &path,
        DeploymentId::new("app").unwrap(),
        StoreLimits::default(),
    )
    .unwrap();
    assert_eq!(
        store.history(&id("a"), Revision(0), MAX_PAGE).unwrap(),
        history
    );
    assert_eq!(
        store.job(&id("a")).unwrap().unwrap().spec.scheduled_at_ms,
        100
    );
    assert_eq!(store.clock().unwrap(), (5, 100));
}

#[test]
fn jobs_validates_wire_values_metadata_and_explicit_retention_bounds() {
    assert!(JobId::new("bad/id").is_err());
    assert!(RequestId::new("").is_err());
    assert!(WorkerKey::new("mail", 0, "schema").is_err());
    assert!(QueuePolicy::new(0, 1).is_err());
    assert!(RetryPolicy::new(101, 1, 2).is_err());
    assert!(EncodedArgs::new(vec![0; MAX_ARGS + 1]).is_err());
    assert!(serde_json::from_str::<QueueId>("\"../bad\"").is_err());
    assert!(serde_json::from_str::<QueuePolicy>("[1,0]").is_err());
    assert!(serde_json::from_str::<RetryPolicy>("[1,5,2]").is_err());
    let mut f = Fixture::with_limits(StoreLimits {
        max_jobs: 1,
        max_commands: 4,
        max_queues: 1,
    });
    f.configure("mail", 1);
    f.enqueue(vec![spec("a", "mail")]);
    assert_eq!(
        f.apply(Operation::Enqueue {
            jobs: vec![spec("b", "mail")]
        })
        .result,
        Err(Rejection::JobLimit)
    );
    assert_eq!(
        f.apply(Operation::ConfigureQueue {
            queue: queue("other"),
            policy: QueuePolicy::new(1, 1).unwrap()
        })
        .result,
        Err(Rejection::QueueLimit)
    );
    assert!(matches!(
        f.store.apply(5, &command("full", f.claim_op("a"))),
        Err(Error::AdmissionFull)
    ));
    assert_eq!(f.store.applied_index().unwrap(), 4);
    let path = f._directory.path().join("jobs.redb");
    drop(f.store);
    assert!(matches!(
        JobStore::open(
            &path,
            DeploymentId::new("wrong-app").unwrap(),
            StoreLimits::default()
        ),
        Err(Error::IncompatibleStore)
    ));
    assert!(JobStore::open(
        &f._directory.path().join("missing").join("jobs.redb"),
        DeploymentId::new("app").unwrap(),
        StoreLimits::default()
    )
    .is_err());
}

#[test]
fn jobs_tick_expires_a_bounded_page_and_rejects_elapsed_leases_even_before_sweep() {
    let mut f = Fixture::new();
    f.configure("mail", 3);
    f.enqueue(vec![
        spec("a", "mail"),
        spec("b", "mail"),
        spec("c", "mail"),
    ]);
    f.claim("a");
    f.claim("b");
    let c = f.claim("c");
    f.ok(Operation::Tick {
        tick: 10,
        utc_ms: 0,
        expire_limit: 1,
    });
    assert_eq!(f.active("mail"), 2);
    assert_eq!(
        f.apply(Operation::Complete {
            token: c,
            outcome: Finish::Success
        })
        .result,
        Err(Rejection::StaleLease)
    );
    f.ok(Operation::Tick {
        tick: 10,
        utc_ms: 0,
        expire_limit: 1,
    });
    assert_eq!(f.active("mail"), 1);
    f.ok(Operation::Tick {
        tick: 10,
        utc_ms: 0,
        expire_limit: 1,
    });
    assert_eq!(f.active("mail"), 0);
}

#[test]
fn jobs_competing_claims_use_committed_revision_and_one_slot() {
    let mut f = Fixture::new();
    f.configure("mail", 1);
    f.enqueue(vec![spec("a", "mail"), spec("b", "mail")]);
    let first = f.claim_op("a");
    let competing_same = f.claim_op("a");
    let competing_other = f.claim_op("b");
    f.ok(first);
    assert_eq!(
        f.apply(competing_same).result,
        Err(Rejection::StaleRevision)
    );
    assert_eq!(
        f.apply(competing_other).result,
        Err(Rejection::ConcurrencyLimit)
    );
    f.invariant("mail");
}

#[test]
fn jobs_exhaustive_three_step_sequences_preserve_counters_and_lease_authority() {
    for sequence in 0..343 {
        let mut f = Fixture::new();
        f.configure("mail", 1);
        f.enqueue(vec![spec("a", "mail"), spec("b", "mail")]);
        let mut digits = sequence;
        let mut last_token = None;
        for _ in 0..3 {
            let action = digits % 7;
            digits /= 7;
            let job = f.job("a");
            if let JobState::Leased(lease) = &job.state {
                last_token = Some(lease.token.clone());
            }
            let operation = match action {
                0 => f.claim_op("a"),
                1 => f.claim_op("b"),
                2 => Operation::Cancel {
                    id: id("a"),
                    revision: job.revision,
                },
                3 => Operation::Tick {
                    tick: f.store.clock().unwrap().0 + 10,
                    utc_ms: 0,
                    expire_limit: 1,
                },
                4 => Operation::Retry {
                    id: id("a"),
                    revision: job.revision,
                },
                5 | 6 => {
                    let Some(token) = last_token.clone() else {
                        continue;
                    };
                    if action == 5 {
                        Operation::Complete {
                            token,
                            outcome: Finish::Success,
                        }
                    } else {
                        Operation::Renew { token }
                    }
                }
                _ => unreachable!(),
            };
            let before = f.store.inspect(&queue("mail"), None, 10).unwrap();
            let receipt = f.apply(operation);
            if receipt.result.is_err() {
                assert_eq!(
                    f.store.inspect(&queue("mail"), None, 10).unwrap(),
                    before,
                    "sequence {sequence}"
                );
            }
            f.invariant("mail");
            assert!(f.active("mail") <= 1, "sequence {sequence}");
        }
    }
}

#[test]
fn jobs_subprocess_commit_helper() {
    let Some(path) = std::env::var_os("MESH_JOBS_DEATH_TEST_PATH") else {
        return;
    };
    let store = JobStore::open(
        std::path::Path::new(&path),
        DeploymentId::new("death-test").unwrap(),
        StoreLimits::default(),
    )
    .unwrap();
    store
        .apply(
            1,
            &command(
                "configure",
                Operation::ConfigureQueue {
                    queue: queue("mail"),
                    policy: QueuePolicy::new(1, 10).unwrap(),
                },
            ),
        )
        .unwrap()
        .result
        .unwrap();
    let accepted = command(
        "submit",
        Operation::Enqueue {
            jobs: vec![spec("survived", "mail")],
        },
    );
    store.apply(2, &accepted).unwrap().result.unwrap();
    let claim = command(
        "claim",
        Operation::Claim {
            id: id("survived"),
            revision: Revision(2),
            holder: WorkerSession::new("dead-process").unwrap(),
            worker: Some(worker()),
        },
    );
    store.apply(3, &claim).unwrap().result.unwrap();
    use std::io::{Read, Write};
    let mut out = std::io::stdout().lock();
    writeln!(out, "JOBS_COMMIT_ACCEPTED").unwrap();
    out.flush().unwrap();
    let mut barrier = [0u8; 1];
    std::io::stdin().read_exact(&mut barrier).unwrap();
    panic!("parent must kill this process after the accepted commit");
}

#[test]
fn jobs_reopens_after_real_process_death_following_accepted_commit() {
    use std::io::{BufRead, BufReader};
    use std::process::{Command as ProcessCommand, Stdio};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("jobs.redb");
    let mut child = ProcessCommand::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "jobs::tests::jobs_subprocess_commit_helper",
            "--nocapture",
        ])
        .env("MESH_JOBS_DEATH_TEST_PATH", &path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if line.unwrap().contains("JOBS_COMMIT_ACCEPTED") {
                tx.send(()).unwrap();
                return;
            }
        }
    });
    let accepted = rx.recv_timeout(std::time::Duration::from_secs(30));
    child.kill().unwrap();
    let status = child.wait().unwrap();
    reader.join().unwrap();
    accepted.expect("child must acknowledge its durable commit before death");
    assert!(!status.success());
    let store = JobStore::open(
        &path,
        DeploymentId::new("death-test").unwrap(),
        StoreLimits::default(),
    )
    .unwrap();
    let saved = store.job(&id("survived")).unwrap().unwrap();
    assert_eq!(saved.spec, spec("survived", "mail"));
    assert_eq!(saved.revision, Revision(3));
    assert_eq!(
        token(&saved).holder,
        WorkerSession::new("dead-process").unwrap()
    );
    assert_eq!(
        store.queue(&queue("mail")).unwrap().unwrap().active_leases,
        1
    );
    let accepted = command(
        "submit",
        Operation::Enqueue {
            jobs: vec![spec("survived", "mail")],
        },
    );
    assert_eq!(
        store.apply(2, &accepted).unwrap(),
        store.receipt(&accepted).unwrap().unwrap()
    );
    store
        .apply(
            4,
            &command(
                "expire",
                Operation::Tick {
                    tick: 10,
                    utc_ms: 0,
                    expire_limit: 1,
                },
            ),
        )
        .unwrap()
        .result
        .unwrap();
    assert_eq!(
        store.queue(&queue("mail")).unwrap().unwrap().active_leases,
        0
    );
}

#[test]
fn jobs_barrier_holds_stale_expiration_observation_across_committed_renewal() {
    let mut f = Fixture::new();
    f.configure("mail", 1);
    f.enqueue(vec![spec("a", "mail")]);
    let lease = f.claim("a");
    f.tick(9, 0);
    let observed = std::sync::Barrier::new(2);
    let renewed = std::sync::Barrier::new(2);
    let store = &f.store;
    std::thread::scope(|scope| {
        let expire = scope.spawn(|| {
            let saved = store.job(&id("a")).unwrap().unwrap();
            let JobState::Leased(old) = saved.state else {
                panic!("lease")
            };
            observed.wait();
            renewed.wait();
            store
                .apply(
                    6,
                    &command(
                        "expire-after-observation",
                        Operation::Tick {
                            tick: old.expires_at_tick,
                            utc_ms: 0,
                            expire_limit: 1,
                        },
                    ),
                )
                .unwrap()
        });
        observed.wait();
        let receipt = store
            .apply(
                5,
                &command(
                    "renew-after-observation",
                    Operation::Renew {
                        token: lease.clone(),
                    },
                ),
            )
            .unwrap();
        renewed.wait();
        receipt.result.unwrap();
        assert_eq!(
            expire.join().unwrap().result,
            Ok(Effect::Ticked {
                expired: Vec::new()
            })
        );
    });
    assert_eq!(token(&f.job("a")), lease);
    assert_eq!(f.active("mail"), 1);
    let JobState::Leased(lease) = f.job("a").state else {
        panic!("lease")
    };
    assert_eq!(lease.expires_at_tick, 19);
}

#[test]
fn jobs_barrier_orders_two_claims_that_observed_the_same_revision() {
    let mut f = Fixture::new();
    f.configure("mail", 1);
    f.enqueue(vec![spec("a", "mail")]);
    let observed = std::sync::Barrier::new(3);
    let (committed_tx, committed_rx) = std::sync::mpsc::channel();
    let store = &f.store;
    std::thread::scope(|scope| {
        let first = scope.spawn(|| {
            let job = store.job(&id("a")).unwrap().unwrap();
            observed.wait();
            let result = store.apply(
                3,
                &command(
                    "first-claim",
                    Operation::Claim {
                        id: job.spec.id,
                        revision: job.revision,
                        holder: WorkerSession::new("session-1").unwrap(),
                        worker: Some(worker()),
                    },
                ),
            );
            committed_tx.send(()).unwrap();
            result.unwrap()
        });
        let barrier = &observed;
        let second = scope.spawn(move || {
            let job = store.job(&id("a")).unwrap().unwrap();
            barrier.wait();
            committed_rx.recv().unwrap();
            store
                .apply(
                    4,
                    &command(
                        "second-claim",
                        Operation::Claim {
                            id: job.spec.id,
                            revision: job.revision,
                            holder: WorkerSession::new("session-2").unwrap(),
                            worker: Some(worker()),
                        },
                    ),
                )
                .unwrap()
        });
        observed.wait();
        assert!(first.join().unwrap().result.is_ok());
        assert_eq!(second.join().unwrap().result, Err(Rejection::StaleRevision));
    });
    assert_eq!(
        token(&f.job("a")).holder,
        WorkerSession::new("session-1").unwrap()
    );
    assert_eq!(f.active("mail"), 1);
}

#[test]
fn jobs_reopen_preserves_blocked_terminal_and_cancel_intent_with_indexes() {
    let mut f = Fixture::new();
    f.configure("mail", 2);
    f.enqueue(vec![
        spec("blocked", "mail"),
        spec("cancelled", "mail"),
        spec("leased", "mail"),
    ]);
    f.ok(Operation::Claim {
        id: id("blocked"),
        revision: f.job("blocked").revision,
        holder: WorkerSession::new("session").unwrap(),
        worker: None,
    });
    f.ok(Operation::Cancel {
        id: id("cancelled"),
        revision: f.job("cancelled").revision,
    });
    f.claim("leased");
    f.ok(Operation::Cancel {
        id: id("leased"),
        revision: f.job("leased").revision,
    });
    let jobs = f.store.inspect(&queue("mail"), None, 10).unwrap();
    let status = f.store.status().unwrap();
    let path = f._directory.path().join("jobs.redb");
    drop(f.store);
    let store = JobStore::open(
        &path,
        DeploymentId::new("app").unwrap(),
        StoreLimits::default(),
    )
    .unwrap();
    assert_eq!(store.inspect(&queue("mail"), None, 10).unwrap(), jobs);
    assert_eq!(store.status().unwrap(), status);
    assert_eq!(
        store.queue(&queue("mail")).unwrap().unwrap().active_leases,
        1
    );
    assert_eq!(
        store
            .due(&queue("mail"), None, 10)
            .unwrap()
            .jobs
            .iter()
            .map(|j| j.spec.id.clone())
            .collect::<Vec<_>>(),
        vec![id("blocked")]
    );
}

#[test]
fn jobs_capped_backoff_and_checked_deadline_reject_overflow_without_claim() {
    let mut f = Fixture::new();
    f.configure("mail", 1);
    let mut a = spec("a", "mail");
    a.retry = RetryPolicy::new(5, 2, 3).unwrap();
    f.enqueue(vec![a]);
    for (tick, delay) in [(0, 2), (2, 3), (5, 3), (8, 3)] {
        f.tick(tick, 0);
        let lease = f.claim("a");
        f.ok(Operation::Complete {
            token: lease,
            outcome: Finish::Failure,
        });
        assert_eq!(f.job("a").ready_at_tick, tick + delay);
    }
    f.tick(MAX_TIME, 0);
    let before = f.job("a");
    assert_eq!(f.apply(f.claim_op("a")).result, Err(Rejection::Arithmetic));
    assert_eq!(f.job("a"), before);
    assert_eq!(f.active("mail"), 0);
}

#[test]
fn jobs_cross_queue_expiration_updates_each_counter_in_one_commit() {
    let mut f = Fixture::new();
    f.configure("mail", 1);
    f.configure("other", 1);
    f.enqueue(vec![spec("a", "mail"), spec("b", "other")]);
    f.claim("a");
    f.claim("b");
    f.tick(10, 0);
    assert_eq!(f.active("mail"), 0);
    assert_eq!(f.active("other"), 0);
    assert_eq!(f.job("a").revision, f.job("b").revision);
    assert_eq!(f.job("a").state, JobState::Pending);
    assert_eq!(f.job("b").state, JobState::Pending);
}
