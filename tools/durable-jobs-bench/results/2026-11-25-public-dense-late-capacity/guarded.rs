
#[test]
#[ignore = "requires MESH_TEST_DATABASE_URL and the canonical bounded grant body"]
fn postgres_grouped_receipts_free_seats_while_one_checked_claim_waits() {
    use crate::db::pool::{mesh_pool_close, mesh_pool_open, unbox_u64_payload};
    use crate::io::MeshResult;
    use crate::string::mesh_str;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct ControlledExit {
        index: u64,
        exit_before: Arc<AtomicU64>,
        killed: bool,
    }

    impl JobProcess for ControlledExit {
        fn reply_pending(&self) -> bool {
            self.killed || self.index < self.exit_before.load(Ordering::Acquire)
        }

        fn poll(&mut self, _: Instant) -> Result<WorkerPoll, String> {
            Ok(if self.reply_pending() {
                WorkerPoll::Exited(Ok(r#"{"action":"complete"}"#.into()))
            } else {
                WorkerPoll::Running
            })
        }

        fn renew(&mut self, _: u64) -> Result<(), String> {
            Ok(())
        }

        fn kill(&mut self) -> Result<(), String> {
            self.killed = true;
            Ok(())
        }
    }

    fn until(mut check: impl FnMut() -> Result<bool, String>) -> Result<(), String> {
        let deadline = Instant::now() + TEST_WAIT;
        while !check()? {
            if Instant::now() >= deadline {
                return Err("guarded runner checkpoint was not reached".into());
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        Ok(())
    }

    crate::gc::mesh_rt_init();
    actor::mesh_rt_init_actor(2);
    let url = std::env::var("MESH_TEST_DATABASE_URL").unwrap();
    let pool = unsafe { &*mesh_pool_open(mesh_str(&url), 0, 6, 500).cast::<MeshResult>() };
    assert_eq!(pool.tag, 0);
    let pool = unsafe { unbox_u64_payload(pool.value) };
    let namespace = format!("runner-late-capacity-{}", rand::random::<u64>());
    with_pool(pool, |db| {
        super::super::store::migrate(db)?;
        db.transaction(|tx| super::super::cron::migrate(tx))?;
        let overlay =
            std::fs::read_to_string(std::env::var("MESH_TEST_LATE_CAPACITY_D388").unwrap())
                .map_err(|error| error.to_string())?;
        db.simple(&overlay)?;
        let jobs: Vec<_> = (0..16)
            .map(|index| json!({"worker": "test", "args": {"index": index}, "timeout_ms": 30000}))
            .collect();
        db.transaction(|tx| {
            super::super::store::enqueue(tx, &namespace, &json!(jobs).to_string())
        })?;
        super::super::store::control(
            db,
            &namespace,
            r#"{"action":"configure","queue":"default","concurrency":64}"#,
        )?;
        Ok(())
    })
    .unwrap();

    let config = json!({
        "namespace": namespace,
        "queues": {"default": 8},
        "workers": [{"worker": "test", "version": 1}],
        "lease_ms": 30000,
        "poll_ms": 10,
        "execution": "trusted_shared_process",
        "claim_batch": true,
        "claim_batch_strategy": "set_based",
        "group_success": true,
        "listen": true
    });
    let exit_before = Arc::new(AtomicU64::new(0));
    let exits = exit_before.clone();
    let id = start_runner(
        pool,
        &config.to_string(),
        || Ok(()),
        Box::new(move |context, _, _, _| {
            let context: serde_json::Value = serde_json::from_str(&context).unwrap();
            Ok(Box::new(ControlledExit {
                index: context["args"]["index"].as_u64().unwrap(),
                exit_before: exits.clone(),
                killed: false,
            }))
        }),
    )
    .unwrap();
    let runner = lookup(id).unwrap();
    let result = (|| {
        until(|| {
            let state = runner.state.lock();
            Ok(state.counts.claimed == 8 && state.jobs.len() == 8 && state.pending.is_none())
        })?;
        with_pool(pool, |db| {
            db.transaction(|tx| {
                let locked = tx.query(
                    "SELECT name FROM mesh_jobs.queues WHERE namespace = $1
                     AND name = 'default' FOR NO KEY UPDATE",
                    &[&namespace],
                )?;
                assert_eq!(locked.len(), 1);
                let blocked_claim = |tx: &mut PgJobDb<'_>| {
                    tx.query("SELECT pg_stat_clear_snapshot()", &[])?;
                    tx.query(
                        "SELECT pid::text FROM pg_stat_activity
                         WHERE pg_backend_pid() = ANY(pg_blocking_pids(pid))
                         AND query LIKE '%FROM mesh_jobs.claim_batch_bounded_v1%'",
                        &[],
                    )
                };
                exit_before.store(2, Ordering::Release);
                let mut waiting_pid = String::new();
                until(|| {
                    let blocked = blocked_claim(tx)?;
                    if blocked.len() == 1 {
                        waiting_pid.clone_from(&blocked[0][0].1);
                        return Ok(true);
                    }
                    Ok(false)
                })?;
                {
                    let state = runner.state.lock();
                    assert_eq!(state.counts.outcomes.completed, 2);
                    assert_eq!(state.jobs.len(), 6);
                    assert_eq!(state.pending_for("default"), 2);
                }
                eprintln!("late_capacity_checkpoint: two confirmed releases, six tracked jobs, one blocked two-seat request");
                exit_before.store(5, Ordering::Release);
                until(|| Ok(runner.state.lock().jobs.len() == 3))?;
                let blocked = blocked_claim(tx)?;
                assert_eq!(blocked.len(), 1);
                assert_eq!(blocked[0][0].1, waiting_pid);
                {
                    let state = runner.state.lock();
                    assert_eq!(state.counts.outcomes.completed, 5);
                    assert_eq!(state.counts.claimed, 8);
                    assert_eq!(state.pending_for("default"), 2);
                    assert_eq!(state.queues["default"].limit - state.jobs.len(), 5);
                    assert!(state.last_error.is_none());
                }
                eprintln!("late_capacity_checkpoint: five confirmed releases, three tracked jobs, same blocked two-seat request");
                Ok(())
            })
        })?;
        until(|| Ok(runner.state.lock().counts.claimed >= 10))?;
        with_pool(pool, |db| {
            let widths = db.query(
                "SELECT count(*)::text FROM mesh_jobs.jobs
                 WHERE namespace = $1 AND attempt = 1 GROUP BY attempted_at_ms
                 ORDER BY attempted_at_ms LIMIT 2",
                &[&namespace],
            )?;
            assert_eq!(widths.len(), 2);
            assert_eq!(widths[0][0].1, "8");
            assert_eq!(widths[1][0].1, "2");
            Ok(())
        })?;
        Ok::<_, String>(())
    })();
    exit_before.store(16, Ordering::Release);
    stop(id, 0).unwrap();
    join(id).unwrap();
    unsafe { mesh_pool_close(pool) };
    result.unwrap();
}
