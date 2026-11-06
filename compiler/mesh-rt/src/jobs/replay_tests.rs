use super::*;

fn next(store: &JobStore, command: &Command) -> Receipt {
    let index = store.applied_index().unwrap() + 1;
    let receipt = store.apply(index, command).unwrap();
    assert_eq!(store.applied_index().unwrap(), index);
    receipt
}
fn enqueue(sequence: u64, name: &str) -> Command {
    Command::enqueue(client(), sequence, vec![spec(name, "mail")]).unwrap()
}
fn refused(receipt: Receipt, reason: AdmissionRejection) {
    assert_eq!(receipt.result, Err(Rejection::NotAdmitted(reason)));
}

#[test]
fn jobs_window_acknowledgement_lost_response_and_restart() {
    let limits = StoreLimits {
        max_streams: 1,
        receipts_per_stream: 1,
        ..StoreLimits::default()
    };
    let mut f = Fixture::with_limits(limits.clone());
    f.configure("mail", 1);
    let first = enqueue(1, "a");
    let second = enqueue(2, "b");
    let receipt = next(&f.store, &first);
    assert_eq!(next(&f.store, &first), receipt);
    refused(next(&f.store, &second), AdmissionRejection::WindowFull);
    assert_eq!(
        f.store.receipt(&second).unwrap(),
        ReceiptLookup::NotAdmitted
    );
    assert_eq!(
        next(&f.store, &Command::Close { stream: client() }).result,
        Err(Rejection::UnacknowledgedRequests)
    );
    assert_eq!(
        next(
            &f.store,
            &Command::Retire {
                stream: client(),
                through: 2
            }
        )
        .result,
        Err(Rejection::InvalidRetirement)
    );
    let path = f._directory.path().join("jobs.redb");
    drop(f.store);
    let store = JobStore::open(&path, DeploymentId::new("app").unwrap(), limits.clone()).unwrap();
    assert_eq!(
        store.receipt(&first).unwrap(),
        ReceiptLookup::Recorded(receipt.clone())
    );
    assert_eq!(next(&store, &first), receipt);
    refused(next(&store, &second), AdmissionRejection::WindowFull);
    let retire = Command::Retire {
        stream: client(),
        through: 1,
    };
    assert_eq!(
        next(&store, &retire).result,
        Ok(Effect::Retired { through: 1 })
    );
    assert_eq!(
        next(&store, &retire).result,
        Ok(Effect::Retired { through: 1 })
    );
    assert_eq!(
        next(
            &store,
            &Command::Retire {
                stream: client(),
                through: 0
            }
        )
        .result,
        Ok(Effect::Retired { through: 1 })
    );
    drop(store);
    let store = JobStore::open(&path, DeploymentId::new("app").unwrap(), limits).unwrap();
    assert_eq!(store.receipt(&first).unwrap(), ReceiptLookup::Retired);
    refused(next(&store, &first), AdmissionRejection::RetiredRequest);
    refused(
        next(&store, &enqueue(1, "resurrected")),
        AdmissionRejection::RetiredRequest,
    );
    assert!(store.job(&id("resurrected")).unwrap().is_none());
    assert!(next(&store, &second).result.is_ok());
    assert_eq!(store.status().unwrap().retained_receipts, 1);
}

#[test]
fn jobs_closed_generation_never_registers_or_recreates_work() {
    let mut f = Fixture::with_limits(StoreLimits {
        max_streams: 1,
        receipts_per_stream: 1,
        ..StoreLimits::default()
    });
    f.configure("mail", 1);
    let original = enqueue(1, "a");
    next(&f.store, &original).result.unwrap();
    next(
        &f.store,
        &Command::Retire {
            stream: client(),
            through: 1,
        },
    )
    .result
    .unwrap();
    let close = Command::Close { stream: client() };
    next(&f.store, &close).result.unwrap();
    next(&f.store, &close).result.unwrap();
    refused(
        next(&f.store, &Command::Register { stream: client() }),
        AdmissionRejection::RetiredStream,
    );
    refused(
        next(&f.store, &enqueue(2, "late")),
        AdmissionRejection::RetiredStream,
    );
    let second = ClientStream::new(1, 2).unwrap();
    let third = ClientStream::new(1, 3).unwrap();
    assert_eq!(
        next(&f.store, &Command::Register { stream: third }).result,
        Err(Rejection::GenerationGap)
    );
    next(&f.store, &Command::Register { stream: second })
        .result
        .unwrap();
    next(&f.store, &Command::Register { stream: second })
        .result
        .unwrap();
    assert_eq!(
        next(&f.store, &Command::Register { stream: third }).result,
        Err(Rejection::StreamInUse)
    );
    let outside = ClientStream::new(2, 1).unwrap();
    assert_eq!(
        next(&f.store, &Command::Register { stream: outside }).result,
        Err(Rejection::StreamLimit)
    );
    assert_eq!(f.store.receipt(&original).unwrap(), ReceiptLookup::Retired);
    refused(
        next(&f.store, &enqueue(1, "late")),
        AdmissionRejection::RetiredStream,
    );
    refused(next(&f.store, &close), AdmissionRejection::RetiredStream);
    assert!(f.store.job(&id("late")).unwrap().is_none());
    let new = Command::enqueue(second, 1, vec![spec("b", "mail")]).unwrap();
    next(&f.store, &new).result.unwrap();
    let path = f._directory.path().join("jobs.redb");
    drop(f.store);
    let store = JobStore::open(
        &path,
        DeploymentId::new("app").unwrap(),
        StoreLimits {
            max_streams: 1,
            receipts_per_stream: 1,
            ..StoreLimits::default()
        },
    )
    .unwrap();
    refused(
        next(&store, &Command::Register { stream: client() }),
        AdmissionRejection::RetiredStream,
    );
    refused(
        next(&store, &enqueue(1, "late")),
        AdmissionRejection::RetiredStream,
    );
    assert!(matches!(
        store.receipt(&new).unwrap(),
        ReceiptLookup::Recorded(_)
    ));
}

#[test]
fn jobs_concurrent_out_of_order_requests_retry_without_duplicate_work() {
    let mut f = Fixture::new();
    f.configure("mail", 1);
    let barrier = std::sync::Barrier::new(3);
    let (tx, rx) = std::sync::mpsc::channel();
    let store = &f.store;
    std::thread::scope(|scope| {
        let gap = scope.spawn(|| {
            barrier.wait();
            let result = store.apply(3, &enqueue(2, "b")).unwrap();
            tx.send(()).unwrap();
            result
        });
        let barrier = &barrier;
        let first = scope.spawn(move || {
            barrier.wait();
            rx.recv().unwrap();
            store.apply(4, &enqueue(1, "a")).unwrap()
        });
        barrier.wait();
        refused(
            gap.join().unwrap(),
            AdmissionRejection::SequenceGap { next: 1 },
        );
        first.join().unwrap().result.unwrap();
    });
    let accepted = next(store, &enqueue(2, "b"));
    accepted.result.as_ref().unwrap();
    assert_eq!(next(store, &enqueue(2, "b")), accepted);
    assert_eq!(
        next(store, &enqueue(2, "changed")).result,
        Err(Rejection::RequestConflict)
    );
    assert!(store.job(&id("changed")).unwrap().is_none());
    assert_eq!(
        store.queue(&queue("mail")).unwrap().unwrap().retained_jobs,
        2
    );
}

#[test]
fn jobs_committed_invalid_and_policy_rejections_advance() {
    let mut f = Fixture::new();
    f.configure("mail", 1);
    let invalid = Command::Request {
        stream: client(),
        sequence: 1,
        operation: Operation::Enqueue { jobs: Vec::new() },
    };
    let negative = next(&f.store, &invalid);
    assert_eq!(negative.result, Err(Rejection::InvalidCommand));
    assert_eq!(next(&f.store, &invalid), negative);
    assert_eq!(
        f.store.receipt(&invalid).unwrap(),
        ReceiptLookup::Recorded(negative)
    );
    assert_eq!(
        next(&f.store, &enqueue(1, "changed")).result,
        Err(Rejection::RequestConflict)
    );
    for invalid in [
        Command::Internal(Operation::Enqueue {
            jobs: vec![spec("a", "mail")],
        }),
        Command::Internal(Operation::Tick {
            tick: MAX_TIME + 1,
            utc_ms: 0,
            expire_limit: 1,
        }),
    ] {
        assert_eq!(
            next(&f.store, &invalid).result,
            Err(Rejection::InvalidCommand)
        );
    }
    next(&f.store, &enqueue(2, "a")).result.unwrap();
    let negative = next(&f.store, &enqueue(3, "a"));
    assert_eq!(negative.result, Err(Rejection::DuplicateJob));
    assert_eq!(next(&f.store, &enqueue(3, "a")), negative);
    assert_eq!(
        next(&f.store, &enqueue(3, "b")).result,
        Err(Rejection::RequestConflict)
    );
    assert!(f.store.job(&id("b")).unwrap().is_none());
    let checkpoint = f.store.applied_index().unwrap();
    assert_eq!(
        f.store.apply(2, &enqueue(4, "old-log")).unwrap().result,
        Ok(Effect::AlreadyApplied {
            through: checkpoint
        })
    );
    assert_eq!(f.store.applied_index().unwrap(), checkpoint);
    assert!(f.store.job(&id("old-log")).unwrap().is_none());
    assert!(matches!(
        f.store.apply(0, &enqueue(4, "zero")),
        Err(Error::LogConflict)
    ));
}

#[test]
fn jobs_exhaustive_replay_traces_preserve_retirement_and_receipt_bounds() {
    for trace in 0..512u32 {
        let mut f = Fixture::with_limits(StoreLimits {
            max_streams: 1,
            receipts_per_stream: 1,
            ..StoreLimits::default()
        });
        f.configure("mail", 1);
        let second = ClientStream::new(1, 2).unwrap();
        let commands = [
            enqueue(1, "a"),
            enqueue(2, "b"),
            enqueue(1, "changed"),
            Command::Retire {
                stream: client(),
                through: 1,
            },
            Command::Close { stream: client() },
            Command::Register { stream: second },
            Command::enqueue(second, 1, vec![spec("second-generation", "mail")]).unwrap(),
            Command::Internal(Operation::Tick {
                tick: 1,
                utc_ms: 1,
                expire_limit: 1,
            }),
        ];
        let mut code = trace;
        let mut retired = false;
        let mut closed = false;
        let mut first_result = None;
        for _ in 0..3 {
            let choice = (code % 8) as usize;
            code /= 8;
            let receipt = next(&f.store, &commands[choice]);
            if choice == 0 || choice == 2 {
                if retired || closed {
                    assert!(
                        matches!(
                            receipt.result,
                            Err(Rejection::NotAdmitted(
                                AdmissionRejection::RetiredRequest
                                    | AdmissionRejection::RetiredStream
                            ))
                        ),
                        "trace {trace}"
                    );
                } else if let Ok(Effect::Enqueued(_)) = &receipt.result {
                    match &first_result {
                        None => first_result = Some(receipt.clone()),
                        Some(first) => assert_eq!(&receipt, first, "trace {trace}"),
                    }
                }
            }
            if matches!(receipt.result, Ok(Effect::Retired { through: 1 })) {
                retired = true;
            }
            if matches!(receipt.result, Ok(Effect::StreamClosed)) {
                closed = true;
            }
            assert!(
                f.store.status().unwrap().retained_receipts <= 1,
                "trace {trace}"
            );
            let jobs = f.store.inspect(&queue("mail"), None, MAX_PAGE).unwrap();
            assert_eq!(
                jobs.len() as u64,
                f.store
                    .queue(&queue("mail"))
                    .unwrap()
                    .unwrap()
                    .retained_jobs
            );
            assert!(
                !(jobs.iter().any(|j| j.spec.id == id("a"))
                    && jobs.iter().any(|j| j.spec.id == id("changed"))),
                "trace {trace}"
            );
        }
    }
}
