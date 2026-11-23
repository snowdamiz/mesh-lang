use super::*;
use crate::db::pg::{native_pg_connect, native_pg_execute, native_pg_query, NativePgConn};
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::Barrier;
use std::thread;
use std::time::{Duration, Instant};

const WORKERS: &str = r#"[{"worker":"Deliver","version":1}]"#;
const TEST_PG_URL: &str = "MESH_JOBS_TEST_PG_URL";
const CHILD_NS: &str = "MESH_JOBS_TEST_CHILD_NAMESPACE";

struct Connection {
    native: NativePgConn,
    transaction: bool,
}

impl Connection {
    fn open(url: &str) -> Self {
        Self {
            native: native_pg_connect(url).unwrap(),
            transaction: false,
        }
    }
}

impl Database for Connection {
    fn query(&mut self, sql: &str, params: &[&str]) -> Result<Vec<Vec<(String, String)>>, String> {
        native_pg_query(&mut self.native, sql, params)
    }

    fn acknowledge(
        &mut self,
        sql: &str,
        params: &[&str],
    ) -> Result<Vec<Vec<(String, String)>>, JobsAckError> {
        self.query(sql, params).map_err(JobsAckError::Unconfirmed)
    }

    fn simple(&mut self, sql: &str) -> Result<(), String> {
        if sql == include_str!("schema.sql") {
            // The public native test driver exposes extended queries only.
            let sql = format!("DO $install$ BEGIN EXECUTE $schema${sql}$schema$; END $install$");
            native_pg_execute(&mut self.native, &sql, &[])?;
        } else {
            native_pg_execute(&mut self.native, sql, &[])?;
        }
        if sql.starts_with("BEGIN") {
            self.transaction = true;
        } else if matches!(sql, "COMMIT" | "ROLLBACK") {
            self.transaction = false;
        }
        Ok(())
    }

    fn in_transaction(&self) -> bool {
        self.transaction
    }
}

struct Fixture {
    db: Connection,
    url: String,
    ns: String,
}

impl Fixture {
    fn new() -> Self {
        let url = std::env::var(TEST_PG_URL)
            .expect("set MESH_JOBS_TEST_PG_URL to a disposable PostgreSQL database");
        let mut db = Connection::open(&url);
        migrate(&mut db).unwrap();
        let ns = response(&mut db, "SELECT gen_random_uuid()::text", &[]).unwrap();
        Self { db, url, ns }
    }

    fn enqueue(&mut self, specs: Value) -> Value {
        self.db.simple("BEGIN").unwrap();
        let result = enqueue(&mut self.db, &self.ns, &specs.to_string()).unwrap();
        self.db.simple("COMMIT").unwrap();
        parse(&result)
    }

    fn ops(&mut self, request: Value) -> Value {
        parse(&control(&mut self.db, &self.ns, &request.to_string()).unwrap())
    }

    fn get(&mut self, id: &str) -> Value {
        self.ops(json!({"action":"get","id":id}))
    }

    fn claim(&mut self) -> Claim {
        self.try_claim().expect("a job should be eligible")
    }

    fn try_claim(&mut self) -> Option<Claim> {
        claim(&mut self.db, &self.ns, "default", WORKERS, 60_000).unwrap()
    }

    fn finish(&mut self, claim: &Claim, outcome: Outcome) -> Settlement {
        finish(&mut self.db, &self.ns, &claim.id, &claim.token, outcome).unwrap()
    }

    fn expire(&mut self, id: &str) {
        self.db
            .query(
                "UPDATE mesh_jobs.jobs SET lease_expires_at_ms = mesh_jobs.now_ms() - 1
             WHERE namespace = $1 AND id = $2::uuid AND state = 'executing'",
                &[&self.ns, id],
            )
            .unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let mut db = Connection::open(&self.url);
        for sql in [
            "DELETE FROM mesh_jobs.receipts WHERE namespace = $1",
            "DELETE FROM mesh_jobs.jobs WHERE namespace = $1",
            "DELETE FROM mesh_jobs.queues WHERE namespace = $1",
            "DELETE FROM mesh_jobs.admissions WHERE namespace = $1",
        ] {
            db.query(sql, &[&self.ns]).unwrap();
        }
    }
}

fn parse(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

fn id(value: &Value) -> &str {
    value["id"].as_str().unwrap()
}

#[test]
#[ignore = "requires MESH_JOBS_TEST_PG_URL pointing at disposable PostgreSQL"]
fn next_due_uses_one_clock_for_the_first_eligible_future_job() {
    let fixture = Fixture::new();
    let mut db = Connection::open(&fixture.url);
    db.simple("BEGIN").unwrap();
    // Roll back the clock replacement before fixture cleanup. Advancing the
    // test clock on every call detects independently sampled cutoffs and delays.
    db.simple("CREATE TEMP SEQUENCE due_clock START WITH 1000")
        .unwrap();
    db.simple(
        "CREATE OR REPLACE FUNCTION mesh_jobs.now_ms() RETURNS bigint
         LANGUAGE sql VOLATILE SET search_path = pg_catalog, pg_temp
         AS $$ SELECT nextval('pg_temp.due_clock') $$",
    )
    .unwrap();
    let ns = fixture.ns.as_str();
    db.query(
        "INSERT INTO mesh_jobs.queues (namespace, name) VALUES ($1, 'default'), ($1, '')",
        &[ns],
    )
    .unwrap();
    let probe = |db: &mut Connection, workers: &str, now_ms: u64, expected: Option<u64>| {
        let now_ms = now_ms.to_string();
        db.query(
            "SELECT setval('pg_temp.due_clock', $1::bigint, false)",
            &[&now_ms],
        )
        .unwrap();
        assert_eq!(next_due_ms(db, ns, "default", workers).unwrap(), expected);
        if expected.is_some() {
            assert_eq!(
                response(db, "SELECT currval('pg_temp.due_clock')::text", &[]).unwrap(),
                now_ms
            );
        }
    };
    probe(&mut db, WORKERS, 1000, None);
    db.query(
        "INSERT INTO mesh_jobs.jobs (namespace, worker, version, queue, args, metadata,
             tags, state, priority, attempt, max_attempts, timeout_ms, backoff_base_ms,
             backoff_max_ms, inserted_at_ms, scheduled_at_ms)
         SELECT $1, 'Deliver', version, 'default', '{}', '{}', '[]', state, 0, attempt,
                3, 1000, 0, 0, 0, scheduled_at_ms
         FROM (VALUES (2, 'scheduled', 0, 1001), (1, 'scheduled', 3, 1002),
                      (1, 'suspended', 0, 1003), (1, 'retryable', 1, 1004),
                      (1, 'scheduled', 0, 1005), (1, 'available', 0, 1006)
               UNION ALL SELECT 1, 'available', 0, n FROM generate_series(0, 1000) n
         ) AS seed(version, state, attempt, scheduled_at_ms)",
        &[ns],
    )
    .unwrap();
    db.query(
        "UPDATE mesh_jobs.jobs SET state = 'executing', attempt = 1,
             lease_token = gen_random_uuid(), lease_expires_at_ms = 5000
         WHERE namespace = $1 AND scheduled_at_ms = 0",
        &[ns],
    )
    .unwrap();
    probe(&mut db, "[]", 1000, None);
    probe(&mut db, WORKERS, 1000, Some(4));
    probe(&mut db, WORKERS, 1004, Some(1));
    probe(&mut db, WORKERS, 1005, Some(1));
    probe(&mut db, WORKERS, 1006, None);
    for queue in ["default", ""] {
        db.query(
            "UPDATE mesh_jobs.queues SET state = 'paused' WHERE namespace = $1 AND name = $2",
            &[ns, queue],
        )
        .unwrap();
        probe(&mut db, WORKERS, 1000, None);
        db.query(
            "UPDATE mesh_jobs.queues SET state = 'active' WHERE namespace = $1 AND name = $2",
            &[ns, queue],
        )
        .unwrap();
        probe(&mut db, WORKERS, 1000, Some(4));
    }
    db.simple("ROLLBACK").unwrap();
}

fn batch_row(fields: [&str; 8]) -> Vec<(String, String)> {
    fields
        .into_iter()
        .map(|value| (String::new(), value.to_owned()))
        .collect()
}

#[test]
fn batch_contention_sentinel_must_be_the_only_row_with_no_payload() {
    let status = ["contention_hint", "", "", "", "", "", "", ""];
    let batch = decode_claim_batch(vec![batch_row(status)], 1, true).unwrap();
    assert!(batch.contention_hint);
    assert!(batch.claims.is_empty());
    assert!(batch.oversized.is_empty());
    assert!(!decode_claim_batch(vec![], 1, true).unwrap().contention_hint);

    for index in 1..status.len() {
        let mut invalid = status;
        invalid[index] = "1";
        assert!(decode_claim_batch(vec![batch_row(invalid)], 1, true).is_err());
    }
    assert!(decode_claim_batch(vec![batch_row(status)[..7].to_vec()], 1, true).is_err());
    for other in [
        status,
        ["claimed", "id", "token", "{}", "1", "2", "1", "1"],
        ["oversized", "id", "", "", "", "16777217", "", "1"],
    ] {
        assert!(decode_claim_batch(vec![batch_row(status), batch_row(other)], 1, true).is_err());
        assert!(decode_claim_batch(vec![batch_row(other), batch_row(status)], 1, true).is_err());
    }
}

#[test]
fn batch_decoder_restores_mixed_rows_in_numeric_order_without_copying_context() {
    let mut rows = Vec::new();
    for position in (1..=20).rev() {
        let ordinal = position.to_string();
        rows.push(if position % 5 == 0 {
            batch_row(["oversized", &ordinal, "", "", "", "16777217", "", &ordinal])
        } else {
            batch_row([
                "claimed", &ordinal, "token", "{}", "1", "2", &ordinal, &ordinal,
            ])
        });
    }
    let first_context = rows.last().unwrap()[3].1.as_ptr();
    let batch = decode_claim_batch(rows, 16, true).unwrap();
    assert!(!batch.contention_hint);
    assert_eq!(batch.claims[0].context.as_ptr(), first_context);
    let expected = (1..=20).filter(|position| position % 5 != 0);
    assert_eq!(batch.claims.len(), 16);
    for (claim, position) in batch.claims.iter().zip(expected) {
        assert_eq!(claim.id, position.to_string());
        assert_eq!(claim.observed_at_ms, Some(position));
    }
    assert!(batch
        .oversized
        .iter()
        .map(|row| row.id.as_str())
        .eq(["5", "10", "15", "20"]));
}

#[test]
fn batch_decoder_rejects_invalid_ordinals_widths_and_counts() {
    let claim = |ordinal| batch_row(["claimed", "id", "token", "{}", "1", "2", "", ordinal]);
    let oversized = |ordinal| batch_row(["oversized", "id", "", "", "", "16777217", "", ordinal]);
    assert!(decode_claim_batch(vec![claim("1"); 21], 16, false).is_err());
    for ordinal in ["", "0", "-1", "+1", "01", "x", "2", "18446744073709551616"] {
        assert!(
            decode_claim_batch(vec![claim(ordinal)], 1, false).is_err(),
            "{ordinal:?}"
        );
    }
    for rows in [
        vec![claim("1"), oversized("1")],
        vec![oversized("1"), claim("3")],
        vec![claim("1"), claim("2")],
        vec![
            oversized("1"),
            oversized("2"),
            oversized("3"),
            oversized("4"),
            oversized("5"),
        ],
    ] {
        assert!(decode_claim_batch(rows, 1, false).is_err());
    }
    let mut short = claim("1");
    short.remove(6);
    let mut wide = claim("1");
    wide.insert(6, (String::new(), String::new()));
    for row in [short, wide] {
        assert!(decode_claim_batch(vec![row], 1, false).is_err());
    }
}

#[test]
#[ignore = "requires MESH_JOBS_TEST_PG_URL pointing at disposable PostgreSQL"]
fn batch_contention_hint_requires_eligible_backlog_and_open_capacity() {
    let mut f = Fixture::new();
    let job = f.enqueue(json!({"worker":"Deliver","scheduled_at_ms":1}));
    let mut claimant = Connection::open(&f.url);
    claimant.simple("SET statement_timeout = '5s'").unwrap();
    let namespace = f.ns.clone();
    let mut claim = |cutoff| {
        claim_batch(
            &mut claimant,
            &namespace,
            "default",
            WORKERS,
            60_000,
            1,
            cutoff,
        )
        .unwrap()
    };

    let mut locker = Connection::open(&f.url);
    locker.simple("BEGIN").unwrap();
    locker
        .query(
            "SELECT 1 FROM mesh_jobs.queues WHERE namespace = $1 AND name = 'default'
             FOR NO KEY UPDATE",
            &[&f.ns],
        )
        .unwrap();
    assert!(!claim(Some(0)).contention_hint);
    let held = claim(Some(1));
    assert!(held.contention_hint);
    assert!(held.claims.is_empty());
    assert!(held.oversized.is_empty());
    locker.simple("COMMIT").unwrap();

    let claimed = claim(Some(1));
    assert!(!claimed.contention_hint);
    assert_eq!(claimed.claims.len(), 1);
    let first = &claimed.claims[0];
    assert_eq!(first.id, id(&job));
    assert_eq!(parse(&first.context)["attempt"], 1);

    let waiting = f.enqueue(json!({"worker":"Deliver"}));
    let full = claim(None);
    assert!(!full.contention_hint && full.claims.is_empty());
    f.finish(first, Outcome::Complete);
    f.ops(json!({"action":"pause_namespace"}));
    let paused = claim(None);
    assert!(!paused.contention_hint && paused.claims.is_empty());
    f.ops(json!({"action":"resume_namespace"}));
    let claimed = claim(None);
    assert_eq!(claimed.claims[0].id, id(&waiting));
    f.finish(&claimed.claims[0], Outcome::Complete);

    let jobs = f.enqueue(json!([
        {"worker":"Deliver","delay_ms":120000},
        {"worker":"Deliver","version":2},
        {"worker":"Deliver","max_attempts":1}
    ]));
    f.db.query(
        "UPDATE mesh_jobs.jobs SET attempt = max_attempts WHERE namespace = $1 AND id = $2::uuid",
        &[&f.ns, id(&jobs[2])],
    )
    .unwrap();
    let unmatched = claim(None);
    assert!(!unmatched.contention_hint && unmatched.claims.is_empty());
}

#[test]
#[ignore = "requires MESH_JOBS_TEST_PG_URL pointing at disposable PostgreSQL"]
fn batch_claim_samples_live_due_time_after_admission_waits() {
    let mut fixture = Fixture::new();
    let original = fixture.enqueue(json!({"worker":"Deliver","delay_ms":120000}));
    thread::scope(|scope| {
        let mut locker = Connection::open(&fixture.url);
        locker.simple("SET statement_timeout = '10s'").unwrap();
        let (send_pid, receive_pid) = std::sync::mpsc::channel();
        let (start, wait) = std::sync::mpsc::channel();
        let url = fixture.url.clone();
        let namespace = fixture.ns.clone();
        let claimant = scope.spawn(move || {
            let mut db = Connection::open(&url);
            db.simple("SET statement_timeout = '10s'").unwrap();
            send_pid
                .send(response(&mut db, "SELECT pg_backend_pid()::text", &[]).unwrap())
                .unwrap();
            wait.recv_timeout(Duration::from_secs(10)).unwrap();
            claim_batch(&mut db, &namespace, "default", WORKERS, 60_000, 1, None).unwrap()
        });
        let pid = receive_pid.recv_timeout(Duration::from_secs(5)).unwrap();
        // EXCLUSIVE allows the advisory SELECT but holds the admission table lock.
        locker.simple("BEGIN").unwrap();
        locker
            .simple("LOCK TABLE mesh_jobs.queues IN EXCLUSIVE MODE")
            .unwrap();
        start.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while response(
            &mut locker,
            "SELECT (pg_backend_pid() = ANY(pg_blocking_pids($1::integer)))::text",
            &[&pid],
        )
        .unwrap()
            != "true"
        {
            assert!(
                Instant::now() < deadline,
                "the claim did not wait for admission"
            );
            thread::sleep(Duration::from_millis(5));
        }
        // Set the due time after admission blocks and commit after it elapses.
        let due = response(
            &mut locker,
            "UPDATE mesh_jobs.jobs SET scheduled_at_ms = mesh_jobs.now_ms() + 1
             WHERE namespace = $1 AND id = $2::uuid RETURNING scheduled_at_ms::text",
            &[&fixture.ns, id(&original)],
        )
        .unwrap();
        locker
            .query(
                "SELECT pg_sleep(greatest(0, $1::bigint + 1 - mesh_jobs.now_ms()) / 1000.0)",
                &[&due],
            )
            .unwrap();
        locker.simple("COMMIT").unwrap();

        let batch = claimant.join().unwrap();
        assert!(!batch.contention_hint && batch.oversized.is_empty());
        assert_eq!(batch.claims.len(), 1);
        assert_eq!(batch.claims[0].id, id(&original));
        assert_eq!(parse(&batch.claims[0].context)["attempt"], 1);
    });
}

#[test]
#[ignore = "requires MESH_JOBS_TEST_PG_URL pointing at disposable PostgreSQL"]
fn batch_claim_preserves_order_and_uses_the_shared_capacity() {
    let mut f = Fixture::new();
    f.ops(json!({"action":"configure","queue":"default","limit":3}));
    let jobs = f.enqueue(json!([
        {"worker":"Deliver","priority":3,"scheduled_at_ms":0},
        {"worker":"Deliver","priority":1,"scheduled_at_ms":2},
        {"worker":"Deliver","priority":1,"scheduled_at_ms":1},
        {"worker":"Deliver","priority":1,"scheduled_at_ms":1},
        {"worker":"Deliver","version":2,"priority":0,"scheduled_at_ms":0},
        {"worker":"Other","priority":0,"scheduled_at_ms":0}
    ]));
    assert_eq!(f.claim().id, id(&jobs[2]));

    let mut db = Connection::open(&f.url);
    db.simple("SET default_transaction_isolation TO 'repeatable read'")
        .unwrap();
    let batch = claim_batch(&mut db, &f.ns, "default", WORKERS, 60_000, 16, None).unwrap();
    assert!(batch.oversized.is_empty());
    assert_eq!(
        batch
            .claims
            .iter()
            .map(|c| c.id.as_str())
            .collect::<Vec<_>>(),
        [id(&jobs[3]), id(&jobs[1])]
    );
    assert_ne!(batch.claims[0].token, batch.claims[1].token);
    let context = parse(&batch.claims[0].context);
    assert_eq!(context["effect_key"], jobs[3]["effect_key"]);
    assert_eq!(context["attempt"], 1);
    assert_eq!(f.get(&batch.claims[0].id)["state"], "executing");
    assert!(f.try_claim().is_none());
    assert_eq!(f.get(id(&jobs[0]))["attempt"], 0);

    assert_eq!(
        f.finish(&batch.claims[0], Outcome::ReleaseUnstarted),
        Settlement::ReleasedUnstarted
    );
    let retried = f.claim();
    assert_eq!(retried.id, batch.claims[0].id);
    assert_ne!(retried.token, batch.claims[0].token);
    assert_eq!(parse(&retried.context), context);
    assert_eq!(
        f.finish(&batch.claims[0], Outcome::Complete),
        Settlement::Stale
    );
}

#[test]
#[ignore = "requires MESH_JOBS_TEST_PG_URL pointing at disposable PostgreSQL"]
fn legacy_and_batch_workers_keep_distinct_executes_after_schema_reapply() {
    let mut fixture = Fixture::new();
    fixture.ops(json!({"action":"configure","queue":"default","limit":2}));
    let jobs = fixture.enqueue(json!([{"worker":"Deliver"}, {"worker":"Deliver"}]));
    let suffix = rand::random::<u64>();
    let old = format!("mesh_batch_legacy_{suffix}");
    let new = format!("mesh_batch_v1_{suffix}");
    let batch_signature = "mesh_jobs.claim_batch_v1(text,text,jsonb,bigint,integer,bigint)";
    let mut db = Connection::open(&fixture.url);
    db.simple("BEGIN ISOLATION LEVEL READ COMMITTED").unwrap();
    for statement in [
        format!("CREATE ROLE {old}"),
        format!("CREATE ROLE {new} IN ROLE {old}"),
        format!("REVOKE EXECUTE ON FUNCTION {batch_signature} FROM PUBLIC"),
        format!("GRANT USAGE ON SCHEMA mesh_jobs TO {old}"),
        format!("GRANT SELECT, UPDATE ON mesh_jobs.jobs, mesh_jobs.queues TO {old}"),
        format!(
            "GRANT EXECUTE ON FUNCTION mesh_jobs.claim(text,text,jsonb,bigint),
             mesh_jobs.now_ms(), mesh_jobs.effect_key(mesh_jobs.jobs) TO {old}"
        ),
        format!("GRANT EXECUTE ON FUNCTION {batch_signature} TO {new}"),
        format!("GRANT SELECT, INSERT, UPDATE ON mesh_jobs.batch_context_blocks TO {new}"),
    ] {
        db.simple(&statement).unwrap();
    }
    db.simple(include_str!("schema.sql")).unwrap();

    db.simple(&format!("SET LOCAL ROLE {old}")).unwrap();
    assert_eq!(
        response(
            &mut db,
            "SELECT has_function_privilege($1, 'EXECUTE')::text",
            &[batch_signature]
        )
        .unwrap(),
        "false",
    );
    let legacy = response(
        &mut db,
        "SELECT id FROM mesh_jobs.claim($1, 'default', $2::jsonb, 60000)",
        &[&fixture.ns, WORKERS],
    )
    .unwrap();
    assert_eq!(legacy, id(&jobs[0]));

    db.simple("RESET ROLE").unwrap();
    db.simple(&format!("SET LOCAL ROLE {new}")).unwrap();
    let claimed = response(
        &mut db,
        "SELECT id FROM mesh_jobs.claim_batch_v1($1, 'default', $2::jsonb, 60000, 16, NULL)",
        &[&fixture.ns, WORKERS],
    )
    .unwrap();
    assert_eq!(claimed, id(&jobs[1]));
    db.simple("ROLLBACK").unwrap();
}

#[test]
#[ignore = "requires MESH_JOBS_TEST_PG_URL pointing at disposable PostgreSQL"]
fn batch_claim_keeps_the_ordered_prefix_with_large_contexts() {
    const MIB: usize = 1024 * 1024;
    let mut fixture = Fixture::new();
    fixture.ops(json!({"action":"configure","queue":"default","concurrency":5}));
    let jobs = fixture.enqueue(json!(vec![json!({"worker":"Deliver"}); 5]));
    let jobs = jobs.as_array().unwrap();
    for (job, payload_bytes) in jobs.iter().zip([16 * MIB, 12 * MIB, 12 * MIB, 12 * MIB]) {
        fixture
            .db
            .query(
                "UPDATE mesh_jobs.jobs SET args = jsonb_build_object('large', repeat('x', $3::integer))
                 WHERE namespace = $1 AND id = $2::uuid",
                &[&fixture.ns, id(job), &payload_bytes.to_string()],
            )
            .unwrap();
    }

    let first = claim_batch(
        &mut fixture.db,
        &fixture.ns,
        "default",
        WORKERS,
        60_000,
        5,
        None,
    )
    .unwrap();
    assert_eq!(first.oversized.len(), 1);
    assert_eq!(first.oversized[0].id, id(&jobs[0]));
    let ids: Vec<_> = first.claims.iter().map(|claim| claim.id.as_str()).collect();
    assert_eq!(ids, [id(&jobs[1]), id(&jobs[2])]);
    let bytes: usize = first.claims.iter().map(|claim| claim.context.len()).sum();
    assert!(
        (24 * MIB..32 * MIB).contains(&bytes),
        "returned {bytes} context bytes"
    );
    let following = claim_batch(
        &mut fixture.db,
        &fixture.ns,
        "default",
        WORKERS,
        60_000,
        5,
        None,
    )
    .unwrap();
    assert!(following.oversized.is_empty());
    let ids: Vec<_> = following
        .claims
        .iter()
        .map(|claim| claim.id.as_str())
        .collect();
    assert_eq!(ids, [id(&jobs[3]), id(&jobs[4])]);
}

#[test]
#[ignore = "requires MESH_JOBS_TEST_PG_URL pointing at disposable PostgreSQL"]
fn batch_drain_steps_past_an_oversized_prefix_and_rechecks_replacements() {
    let mut fixture = Fixture::new();
    let jobs = fixture.enqueue(json!(vec![json!({"worker":"Deliver"}); 6]));
    for job in &jobs.as_array().unwrap()[..5] {
        fixture
            .db
            .query(
                "UPDATE mesh_jobs.jobs SET args = jsonb_build_object('large', repeat('x', 16777216))
                 WHERE namespace = $1 AND id = $2::uuid",
                &[&fixture.ns, id(job)],
            )
            .unwrap();
    }
    let selected = r#"{"default":1}"#;
    let first = claim_batch(
        &mut fixture.db,
        &fixture.ns,
        "default",
        WORKERS,
        60_000,
        1,
        None,
    )
    .unwrap();
    assert!(!first.contention_hint);
    assert_eq!(first.oversized.len(), 4);
    assert!(first.claims.is_empty());
    assert!(
        drain_work(&mut fixture.db, &fixture.ns, selected, WORKERS, true)
            .unwrap()
            .pending
    );
    let second = claim_batch(
        &mut fixture.db,
        &fixture.ns,
        "default",
        WORKERS,
        60_000,
        1,
        None,
    )
    .unwrap();
    assert_eq!(second.oversized.len(), 1);
    assert_eq!(second.claims[0].id, id(&jobs[5]));
    assert_eq!(
        fixture.finish(&second.claims[0], Outcome::Complete),
        Settlement::Completed
    );
    let remaining = drain_work(&mut fixture.db, &fixture.ns, selected, WORKERS, true).unwrap();
    assert!(!remaining.pending);
    assert!(remaining.oversized.is_some());
    let blocked = claim_batch(
        &mut fixture.db,
        &fixture.ns,
        "default",
        WORKERS,
        60_000,
        1,
        None,
    )
    .unwrap();
    assert!(!blocked.contention_hint);
    assert!(blocked.claims.is_empty() && blocked.oversized.is_empty());

    fixture
        .db
        .query(
            "UPDATE mesh_jobs.jobs SET args = '{}' WHERE namespace = $1 AND id = $2::uuid",
            &[&fixture.ns, id(&jobs[0])],
        )
        .unwrap();
    assert!(
        drain_work(&mut fixture.db, &fixture.ns, selected, WORKERS, true)
            .unwrap()
            .pending
    );
    let fixed = claim_batch(
        &mut fixture.db,
        &fixture.ns,
        "default",
        WORKERS,
        60_000,
        1,
        None,
    )
    .unwrap();
    assert_eq!(fixed.claims[0].id, id(&jobs[0]));
    assert_eq!(parse(&fixed.claims[0].context)["attempt"], 1);
}

#[test]
#[ignore = "requires MESH_JOBS_TEST_PG_URL pointing at disposable PostgreSQL"]
fn release_before_execution_preserves_uniqueness_and_honors_cancellation() {
    let mut fixture = Fixture::new();
    let spec = json!({"worker":"Deliver", "metadata":{"reference":"launch-proof"},
        "unique":{"fields":["worker", "metadata"], "states":"incomplete"}});
    let admitted = fixture.enqueue(spec.clone());
    let first = fixture.claim();
    assert_eq!(
        fixture.finish(&first, Outcome::ReleaseUnstarted),
        Settlement::ReleasedUnstarted
    );
    let released = fixture.get(id(&admitted));
    assert_eq!(released["state"], "available");
    assert_eq!(released["attempt"], 0);
    assert_eq!(released["metadata"], spec["metadata"]);
    assert_eq!(
        released["scheduled_at_ms"],
        parse(&first.context)["scheduled_at_ms"]
    );
    assert_eq!(id(&fixture.enqueue(spec)), id(&admitted));

    let replacement = fixture.claim();
    assert_eq!(
        fixture.finish(&first, Outcome::ReleaseUnstarted),
        Settlement::Stale
    );
    fixture.ops(json!({"action":"cancel","id":replacement.id}));
    assert_eq!(
        fixture.finish(&replacement, Outcome::ReleaseUnstarted),
        Settlement::Cancelled
    );
    let cancelled = fixture.get(id(&admitted));
    assert_eq!(cancelled["state"], "cancelled");
    assert_eq!(cancelled["attempt"], 1);
}

#[test]
#[ignore = "requires MESH_JOBS_TEST_PG_URL pointing at disposable PostgreSQL"]
fn migration_preserves_existing_attempts_and_legacy_boolean_settlement() {
    let mut f = Fixture::new();
    f.enqueue(json!({"worker":"Deliver","args":{"reference":"durable"}}));
    let claim = f.claim();
    let executing = f.get(&claim.id);

    let mut legacy = Connection::open(&f.url);
    migrate(&mut legacy).unwrap();
    assert_eq!(
        response(
            &mut legacy,
            "SELECT version::text FROM mesh_jobs.schema_version WHERE singleton",
            &[]
        )
        .unwrap(),
        "1"
    );
    assert_eq!(f.get(&claim.id), executing);

    let settle = "SELECT mesh_jobs.settle($1, $2::uuid, $3::uuid,
        '{\"action\":\"complete\"}'::jsonb, false)::text";
    let params = [f.ns.as_str(), claim.id.as_str(), claim.token.as_str()];
    assert_eq!(response(&mut legacy, settle, &params).unwrap(), "true");
    assert_eq!(response(&mut legacy, settle, &params).unwrap(), "false");
    assert_eq!(f.finish(&claim, Outcome::Complete), Settlement::Stale);
    let completed = f.get(&claim.id);
    assert_eq!(completed["state"], "completed");
    assert_eq!(completed["args"], executing["args"]);
    assert_eq!(completed["attempt"], executing["attempt"]);
}

#[test]
#[ignore = "requires MESH_JOBS_TEST_PG_URL pointing at disposable PostgreSQL"]
fn bulk_actions_respect_filters_and_retry_opens_a_new_run() {
    let mut fixture = Fixture::new();
    let jobs = fixture.enqueue(json!([
        {"worker":"Deliver"},
        {"worker":"Deliver"},
        {"worker":"Keep", "delay_ms": 3600000}
    ]));
    let cancelled =
        fixture.ops(json!({"action":"cancel_all", "filter":{"worker":"Deliver", "limit":1}}));
    assert_eq!(cancelled["changed"], 1);
    let cancelled_id = cancelled["ids"][0].as_str().unwrap();
    assert_eq!(fixture.get(cancelled_id)["state"], "cancelled");
    assert_eq!(
        fixture.ops(json!({"action":"cancel_all","filter":{"worker":"Deliver"}}))["changed"],
        1
    );
    assert_eq!(
        fixture.ops(json!({"action":"cancel_all","filter":{"worker":"Deliver"}}))["changed"],
        0
    );
    assert_eq!(fixture.get(id(&jobs[2]))["state"], "scheduled");
    let retried = fixture
        .ops(json!({"action":"retry_all", "filter":{"state":"cancelled", "queue":"default"}}));
    assert_eq!(retried["changed"], 2);
    let job = fixture.get(cancelled_id);
    assert_eq!(job["state"], "available");
    assert_eq!(job["effect_key"], format!("{cancelled_id}/2"));
}

#[test]
#[ignore = "requires MESH_JOBS_TEST_PG_URL pointing at disposable PostgreSQL"]
fn edits_and_terminal_deletion_respect_claim_locks_and_keep_receipts() {
    let mut f = Fixture::new();
    let spec = json!({"worker":"Deliver", "request_key":"ops-key"});
    let receipt = f.enqueue(spec.clone());
    let id = id(&receipt);
    for patch in [
        json!({"args":null,"priority":1}),
        json!({"priority":10}),
        json!({"delay_ms":0,"scheduled_at_ms":0}),
        json!({"lease_token":"fake"}),
    ] {
        let edit = json!({"action":"update","id":id,"patch":patch}).to_string();
        assert!(control(&mut f.db, &f.ns, &edit).is_err(), "{edit}");
    }
    let edit = json!({
        "action":"update","id":id,
        "patch":{"args":{"task":"updated"},"priority":3,"delay_ms":1000}
    })
    .to_string();
    let mut owner = Connection::open(&f.url);
    owner.simple("BEGIN").unwrap();
    let claimed = owner
        .query(
            "SELECT id, token FROM mesh_jobs.claim($1, 'default', $2::jsonb, 60000)",
            &[&f.ns, WORKERS],
        )
        .unwrap();
    assert_eq!(claimed[0][0].1, id);
    assert!(control(&mut f.db, &f.ns, &edit)
        .unwrap_err()
        .contains("locked"));
    assert_eq!(
        f.ops(json!({"action":"delete_all","filter":{"queue":"default"}}))["deleted"],
        0
    );
    owner.simple("ROLLBACK").unwrap();
    let updated = parse(&control(&mut f.db, &f.ns, &edit).unwrap());
    assert_eq!(updated["state"], "scheduled");
    assert_eq!(updated["effect_key"], receipt["effect_key"]);
    assert_eq!(updated["attempt"], 0);
    assert_eq!(f.get(id)["args"], json!({"task":"updated"}));
    f.ops(json!({"action":"run_now","id":id}));
    let claim = f.claim();
    assert!(control(&mut f.db, &f.ns, &edit).is_err());
    assert_eq!(f.finish(&claim, Outcome::Complete), Settlement::Completed);
    let second = f.enqueue(json!({"worker":"Unchanged"}));
    let summaries = f.ops(json!({"action":"search","filter":{"limit":1}}));
    let cursor = summaries[0]["id"].as_str().unwrap();
    assert!(summaries[0].get("args").is_none());
    let next = f.ops(json!({"action":"search","filter":{"limit":1},"after_id":cursor}));
    assert_ne!(next[0]["id"], summaries[0]["id"]);
    assert!(f
        .ops(json!({"action":"search","after_id":next[0]["id"]}))
        .as_array()
        .unwrap()
        .is_empty());
    let removed = f.ops(json!({"action":"delete_all","filter":{"queue":"default","limit":1}}));
    assert_eq!(removed, json!({"deleted":1,"ids":[id]}));
    assert_eq!(f.get(second["id"].as_str().unwrap())["state"], "available");
    assert_eq!(f.enqueue(spec), receipt);
}

#[test]
#[ignore = "requires MESH_JOBS_TEST_PG_URL pointing at disposable PostgreSQL"]
fn namespace_pause_fences_pending_transactions_and_respects_queue_state() {
    let mut f = Fixture::new();
    f.ops(json!({"action":"configure","queue":"other","concurrency":3}));
    f.ops(json!({"action":"pause","queue":"other"}));
    let original = f.enqueue(json!({"worker":"Deliver"}));
    let legacy_claim = "SELECT id FROM mesh_jobs.claim($1, 'default', $2::jsonb, 60000)";
    let mut owner = Connection::open(&f.url);
    owner.simple("SET statement_timeout = '5s'").unwrap();
    owner.simple("BEGIN").unwrap();
    assert_eq!(
        response(&mut owner, legacy_claim, &[&f.ns, WORKERS]).unwrap(),
        id(&original)
    );
    f.db.simple("SET lock_timeout = '50ms'").unwrap();
    assert!(control(&mut f.db, &f.ns, r#"{"action":"pause_namespace"}"#)
        .unwrap_err()
        .contains("lock timeout"));
    assert_eq!(f.ops(json!({"action":"namespace_status"}))["paused"], false);
    owner.simple("ROLLBACK").unwrap();
    f.db.simple("BEGIN").unwrap();
    assert_eq!(f.ops(json!({"action":"pause_namespace"}))["paused"], true);
    let pending_pause = response(&mut owner, legacy_claim, &[&f.ns, WORKERS]).unwrap_err();
    assert!(
        pending_pause.contains("namespace admission is paused"),
        "{pending_pause}"
    );
    f.db.simple("COMMIT").unwrap();
    let another = f.enqueue(json!({"worker":"Deliver","queue":"another"}));
    assert!(f.try_claim().is_none());
    let legacy_error = response(&mut owner, legacy_claim, &[&f.ns, WORKERS]).unwrap_err();
    assert!(
        legacy_error.contains("namespace admission is paused"),
        "{legacy_error}"
    );
    let untouched = f.get(id(&original));
    assert_eq!(untouched["state"], "available");
    assert_eq!(untouched["attempt"], 0);
    assert!(
        claim_batch(&mut f.db, &f.ns, "another", WORKERS, 60_000, 5, None)
            .unwrap()
            .claims
            .is_empty()
    );
    assert_eq!(f.ops(json!({"action":"resume_namespace"}))["paused"], false);
    assert_eq!(
        response(&mut owner, legacy_claim, &[&f.ns, WORKERS]).unwrap(),
        id(&original)
    );
    assert_eq!(
        f.ops(json!({"action":"inspect_queue","queue":"other"}))["state"],
        "paused"
    );
    let batch = claim_batch(&mut f.db, &f.ns, "another", WORKERS, 60_000, 5, None).unwrap();
    assert_eq!(batch.claims[0].id, another["id"]);
}

#[test]
#[ignore = "requires MESH_JOBS_TEST_PG_URL pointing at disposable PostgreSQL"]
fn legacy_claim_sees_pause_committed_after_its_statement_started() {
    let mut f = Fixture::new();
    f.db.simple("SET statement_timeout = '5s'").unwrap();
    let job = f.enqueue(json!({"worker":"Deliver"}));
    let trigger = format!("hold_claim_{}", f.ns.replace('-', ""));
    f.db.simple(&format!(
        "CREATE FUNCTION mesh_jobs.{trigger}() RETURNS trigger LANGUAGE plpgsql AS $$
         BEGIN
             PERFORM pg_advisory_xact_lock(hashtextextended(NEW.namespace, 1782495114));
             RETURN NEW;
         END $$"
    ))
    .unwrap();
    // PostgreSQL runs this trigger before mesh_jobs_fence_claim by name.
    f.db.simple(&format!(
        "CREATE TRIGGER {trigger} BEFORE UPDATE OF state ON mesh_jobs.jobs FOR EACH ROW
         WHEN (NEW.namespace = '{}' AND NEW.state = 'executing')
         EXECUTE FUNCTION mesh_jobs.{trigger}()",
        f.ns,
    ))
    .unwrap();
    f.db.query(
        "SELECT pg_advisory_lock(hashtextextended($1, 1782495114))",
        &[&f.ns],
    )
    .unwrap();

    let (pid_tx, pid_rx) = std::sync::mpsc::channel();
    let url = f.url.clone();
    let namespace = f.ns.clone();
    let claimant = thread::spawn(move || {
        let mut db = Connection::open(&url);
        db.simple("SET statement_timeout = '10s'").unwrap();
        pid_tx
            .send(response(&mut db, "SELECT pg_backend_pid()::text", &[]).unwrap())
            .unwrap();
        response(
            &mut db,
            "SELECT id FROM mesh_jobs.claim($1, 'default', $2::jsonb, 60000)",
            &[&namespace, WORKERS],
        )
    });
    let pid = pid_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while response(
        &mut f.db,
        "SELECT (pg_backend_pid() = ANY(pg_blocking_pids($1::integer)))::text",
        &[&pid],
    )
    .unwrap()
        != "true"
    {
        assert!(
            Instant::now() < deadline,
            "legacy claim did not reach the transition"
        );
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(f.ops(json!({"action":"pause_namespace"}))["paused"], true);
    f.db.query(
        "SELECT pg_advisory_unlock(hashtextextended($1, 1782495114))",
        &[&f.ns],
    )
    .unwrap();
    let result = claimant.join();
    f.db.simple(&format!("DROP TRIGGER {trigger} ON mesh_jobs.jobs"))
        .unwrap();
    f.db.simple(&format!("DROP FUNCTION mesh_jobs.{trigger}()"))
        .unwrap();
    let error = result.unwrap().unwrap_err();
    assert!(error.contains("namespace admission is paused"), "{error}");
    let row = f.get(id(&job));
    assert_eq!(row["state"], "available");
    assert_eq!(row["attempt"], 0);
}

#[test]
#[ignore = "requires MESH_JOBS_TEST_PG_URL pointing at disposable PostgreSQL"]
fn enqueue_is_atomic_with_the_application_transaction_and_replays_after_pruning() {
    let mut f = Fixture::new();
    let mut observer = Connection::open(&f.url);
    let key = json!({"worker":"Deliver","request_key":"abc","args":{"order":"x'); SELECT 1; --"}});
    let key = key.to_string();
    assert!(enqueue(&mut f.db, &f.ns, &key).is_err());

    f.db.simple("BEGIN").unwrap();
    f.db.simple("CREATE TEMP TABLE app_state (committed boolean)")
        .unwrap();
    f.db.simple("INSERT INTO app_state VALUES (true)").unwrap();
    let rolled_back = parse(&enqueue(&mut f.db, &f.ns, &key).unwrap());
    let request = json!({"action":"get","id":id(&rolled_back)}).to_string();
    assert_eq!(control(&mut observer, &f.ns, &request).unwrap(), "null");
    f.db.simple("ROLLBACK").unwrap();
    assert_eq!(f.get(id(&rolled_back)), Value::Null);

    f.db.simple("BEGIN").unwrap();
    f.db.simple("CREATE TEMP TABLE app_state (committed boolean)")
        .unwrap();
    f.db.simple("INSERT INTO app_state VALUES (true)").unwrap();
    let invalid_batch = json!([
        {"worker":"Deliver","request_key":"abc"},
        {"worker":"Deliver","args":{"invalid":"\0"}}
    ]);
    let error = enqueue(&mut f.db, &f.ns, &invalid_batch.to_string()).unwrap_err();
    assert!(error.contains("Unicode escape"), "{error}");
    let receipt = parse(&enqueue(&mut f.db, &f.ns, &key).unwrap());
    assert_eq!(
        response(&mut f.db, "SELECT committed::text FROM app_state", &[]).unwrap(),
        "true"
    );
    f.db.simple("COMMIT").unwrap();

    let claim = f.claim();
    assert_eq!(claim.id, id(&receipt));
    assert_eq!(parse(&claim.context)["effect_key"], receipt["effect_key"]);
    assert_eq!(parse(&claim.context)["args"]["order"], "x'); SELECT 1; --");
    assert_eq!(f.finish(&claim, Outcome::Complete), Settlement::Completed);
    assert_eq!(
        f.ops(json!({"action":"prune","max_age_ms":0})),
        json!({"deleted":1})
    );
    assert_eq!(f.get(&claim.id), Value::Null);

    assert_eq!(f.enqueue(parse(&key)), receipt);
    f.db.simple("BEGIN").unwrap();
    let changed = enqueue(
        &mut f.db,
        &f.ns,
        r#"{"worker":"Deliver","request_key":"abc"}"#,
    );
    assert!(changed.unwrap_err().contains("already bound"));
    f.db.simple("COMMIT").unwrap();
    assert_eq!(f.ops(json!({"op":"list"})), json!([]));
}

#[test]
#[ignore = "requires MESH_JOBS_TEST_PG_URL pointing at disposable PostgreSQL"]
fn broad_uniqueness_and_receipts_survive_concurrent_writers_and_stale_snapshots() {
    let mut f = Fixture::new();
    let mut stale = Connection::open(&f.url);
    stale
        .simple("BEGIN ISOLATION LEVEL REPEATABLE READ")
        .unwrap();
    stale
        .query("SELECT count(*) FROM mesh_jobs.jobs", &[])
        .unwrap();

    let initial = f.enqueue(json!({"worker":"Deliver","args":{"group":1,"other":2}}));
    let matching = json!({
        "worker":"Deliver","queue":"other","args":{"group":1,"other":3},
        "unique":{"fields":["worker","args"],"keys":["group"],"states":"incomplete"},
        "replace":{"available":["args"]},
        "request_key":"copy",
    });
    let error = enqueue(&mut stale, &f.ns, &matching.to_string()).unwrap_err();
    assert!(error.contains("could not serialize"), "{error}");
    stale.simple("ROLLBACK").unwrap();
    let conflict = f.enqueue(matching.clone());
    assert_eq!(id(&conflict), id(&initial));
    assert_eq!(conflict["conflict"], true);
    assert_eq!(f.get(id(&initial))["args"]["other"], 3);

    let workers = 5;
    let barrier = Barrier::new(workers);
    let receipts = thread::scope(|scope| {
        let tasks: Vec<_> = (0..workers).map(|_| scope.spawn(|| {
            let mut db = Connection::open(&f.url);
            db.simple("BEGIN").unwrap();
            barrier.wait();
            let receipt = enqueue(
                &mut db, &f.ns,
                r#"{"worker":"Deliver","request_key":"concurrent","args":{"group":2},"unique":true}"#,
            ).unwrap();
            db.simple("COMMIT").unwrap();
            receipt
        })).collect();
        tasks
            .into_iter()
            .map(|task| task.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert!(receipts.iter().all(|receipt| receipt == &receipts[0]));
    assert_eq!(f.enqueue(matching), conflict);
    assert_eq!(f.ops(json!({"action":"list"})).as_array().unwrap().len(), 2);

    let future = f.enqueue(json!({
        "worker":"Deliver","delay_ms":60000,
        "unique":{"key":"scheduled","states":"scheduled","timestamp":"scheduled_at","period_ms":1000}
    }));
    let same_schedule = f.enqueue(json!({
        "worker":"Deliver","delay_ms":90000,
        "unique":{"key":"scheduled","states":"scheduled","timestamp":"scheduled_at","period_ms":1000},
        "replace":["scheduled_at_ms"]
    }));
    assert_eq!(id(&future), id(&same_schedule));
    assert_eq!(same_schedule["conflict"], true);
}

#[test]
#[ignore = "requires MESH_JOBS_TEST_PG_URL pointing at disposable PostgreSQL"]
fn competing_claims_obey_capacity_pause_stop_and_drain() {
    let mut f = Fixture::new();
    f.ops(json!({"action":"configure","queue":"default","limit":2}));
    let specs = Value::Array(
        (0..6)
            .map(|priority| json!({"worker":"Deliver","priority":priority,"scheduled_at_ms":0}))
            .collect(),
    );
    let jobs = f.enqueue(specs);
    let workers = 8;
    let barrier = Barrier::new(workers);
    let mut claimed = thread::scope(|scope| {
        let tasks: Vec<_> = (0..workers)
            .map(|_| {
                scope.spawn(|| {
                    let mut db = Connection::open(&f.url);
                    db.simple("SET default_transaction_isolation TO 'repeatable read'")
                        .unwrap();
                    barrier.wait();
                    claim(&mut db, &f.ns, "default", WORKERS, 60_000).unwrap()
                })
            })
            .collect();
        tasks
            .into_iter()
            .filter_map(|task| task.join().unwrap())
            .collect::<Vec<_>>()
    });
    // A contender may skip the briefly locked queue even with capacity left.
    assert!((1..=2).contains(&claimed.len()));
    if claimed.len() == 1 {
        claimed.push(f.claim());
    }
    claimed.sort_by(|a, b| a.id.cmp(&b.id));
    let mut expected = [id(&jobs[0]), id(&jobs[1])];
    expected.sort_unstable();
    assert_eq!(
        claimed
            .iter()
            .map(|job| job.id.as_str())
            .collect::<Vec<_>>(),
        expected
    );
    assert!(f.try_claim().is_none());

    let paused = f.ops(json!({"action":"pause","queue":"default"}));
    assert_eq!(paused["running"], 2);
    assert_eq!(paused["state"], "paused");
    f.ops(json!({"action":"scale","queue":"default","concurrency":1}));
    f.ops(json!({"action":"resume","queue":"default"}));
    assert!(f.try_claim().is_none());
    assert_eq!(
        f.finish(&claimed[0], Outcome::Complete),
        Settlement::Completed
    );
    assert!(f.try_claim().is_none());
    f.ops(json!({"action":"stop","queue":"default"}));
    f.ops(json!({"action":"resume","queue":"default"}));
    assert_eq!(
        f.finish(&claimed[1], Outcome::Complete),
        Settlement::Completed
    );
    assert!(f.try_claim().is_none());
    f.ops(json!({"action":"start","queue":"default"}));
    assert_eq!(f.claim().id, id(&jobs[2]));
}

#[test]
#[ignore = "requires MESH_JOBS_TEST_PG_URL pointing at disposable PostgreSQL"]
fn cancel_holds_capacity_until_ack_or_recovery_and_expired_tokens_are_fenced() {
    let mut f = Fixture::new();
    f.enqueue(json!([
        {"worker":"Deliver","max_attempts":2,"backoff_base_ms":0,"priority":0},
        {"worker":"Deliver","priority":1},
        {"worker":"Deliver","priority":2}
    ]));
    let mut new_conn = Connection::open(&f.url);

    let first = f.claim();
    assert_eq!(
        renew(&mut new_conn, &f.ns, &first.id, &first.token, 90_000).unwrap(),
        LeaseStatus::Active
    );
    f.expire(&first.id);
    assert_eq!(
        renew(&mut new_conn, &f.ns, &first.id, &first.token, 90_000).unwrap(),
        LeaseStatus::Lost
    );
    assert_eq!(f.finish(&first, Outcome::Complete), Settlement::Stale);
    assert!(f.try_claim().is_none());
    assert_eq!(recover(&mut f.db, &f.ns, 1, None).unwrap(), 1);
    let replacement = f.claim();
    assert_eq!(first.id, replacement.id);
    assert_ne!(first.token, replacement.token);
    assert_eq!(
        parse(&first.context)["effect_key"],
        parse(&replacement.context)["effect_key"]
    );
    assert_eq!(parse(&replacement.context)["attempt"], 2);
    assert_eq!(f.finish(&first, Outcome::Complete), Settlement::Stale);
    assert_eq!(
        renew(&mut new_conn, &f.ns, &first.id, &first.token, 90_000).unwrap(),
        LeaseStatus::Lost
    );

    new_conn.simple("SET statement_timeout = 1000").unwrap();
    f.db.simple("BEGIN").unwrap();
    let requested = f.ops(json!({"action":"cancel","id":replacement.id}));
    let racing_ack = finish(
        &mut new_conn,
        &f.ns,
        &replacement.id,
        &replacement.token,
        Outcome::Complete,
    );
    f.db.simple("COMMIT").unwrap();
    assert_eq!(requested["state"], "executing");
    assert!(requested["cancel_requested_at_ms"].is_number());
    assert!(racing_ack
        .unwrap_err()
        .message()
        .contains("could not obtain lock"));
    assert_eq!(
        renew(
            &mut new_conn,
            &f.ns,
            &replacement.id,
            &replacement.token,
            90_000
        )
        .unwrap(),
        LeaseStatus::Cancelled
    );
    assert!(f.try_claim().is_none());
    assert_eq!(
        f.ops(json!({"action":"delete","id":replacement.id}))["deleted"],
        false
    );
    assert_eq!(
        finish(
            &mut new_conn,
            &f.ns,
            &replacement.id,
            &replacement.token,
            Outcome::Complete,
        )
        .unwrap(),
        Settlement::Cancelled
    );
    assert_eq!(f.get(&replacement.id)["state"], "cancelled");

    let second = f.claim();
    f.ops(json!({"action":"cancel","id":second.id}));
    f.expire(&second.id);
    assert_eq!(recover(&mut f.db, &f.ns, 1, None).unwrap(), 1);
    assert_eq!(f.get(&second.id)["state"], "cancelled");
    assert_ne!(f.claim().id, second.id);
}

#[test]
#[ignore = "requires MESH_JOBS_TEST_PG_URL pointing at disposable PostgreSQL"]
fn schedules_versions_snoozes_and_retry_budget_use_primary_time() {
    let mut f = Fixture::new();
    let jobs = f.enqueue(json!([
        {"worker":"Deliver","version":2},
        {"worker":"Deliver","delay_ms":120000},
        {"worker":"Deliver","max_attempts":2,"backoff_base_ms":25000,"backoff_max_ms":25000,"suspended":true,
         "metadata":{"snoozed":"not a count"},"timeout_ms":1234}
    ]));
    assert!(f.try_claim().is_none());
    let job_id = id(&jobs[2]);
    f.ops(json!({"action":"resume","id":job_id}));
    let first = f.claim();
    assert_eq!(first.id, job_id);
    assert_eq!(first.timeout_ms, 1234);
    assert_eq!(
        f.finish(&first, Outcome::Snooze { delay_ms: 0 }),
        Settlement::Scheduled
    );
    let again = f.claim();
    let context = parse(&again.context);
    assert_eq!(context["metadata"]["snoozed"], 1);
    assert_eq!(context["attempt"], 1);

    assert_eq!(
        f.finish(
            &again,
            Outcome::Fail {
                reason: "retry me".into()
            }
        ),
        Settlement::Retryable
    );
    let retryable = f.get(job_id);
    assert_eq!(retryable["state"], "retryable");
    let metrics = f.ops(json!({"action":"queue_stats","queue":"default"}));
    assert_eq!(
        metrics["counts"],
        json!({"available":1,"scheduled":1,"retryable":1})
    );
    assert!(metrics["oldest_due_age_ms"].as_u64().is_some());
    let scheduled = f.get(id(&jobs[1]));
    let mut reopened = Connection::open(&f.url);
    migrate(&mut reopened).unwrap();
    assert_eq!(f.get(id(&jobs[1])), scheduled);
    assert_eq!(f.get(job_id), retryable);
    let now = response(&mut f.db, "SELECT mesh_jobs.now_ms()::text", &[])
        .unwrap()
        .parse::<u64>()
        .unwrap();
    assert!(retryable["scheduled_at_ms"].as_u64().unwrap() > now + 20_000);
    assert!(f.try_claim().is_none());
    f.ops(json!({"action":"reschedule","id":job_id,"scheduled_at_ms":0}));
    let last = f.claim();
    assert_eq!(parse(&last.context)["attempt"], 2);
    assert_eq!(
        f.finish(
            &last,
            Outcome::RetryAfter {
                delay_ms: 1,
                reason: "exhaust".into()
            }
        ),
        Settlement::Discarded
    );
    let discarded = f.get(job_id);
    assert_eq!(discarded["state"], "discarded");
    assert_eq!(discarded["errors"].as_array().unwrap().len(), 2);
    let restarted = f.ops(json!({"action":"retry","id":job_id}));
    assert_eq!(restarted["attempt"], 0);
    assert_ne!(restarted["effect_key"], discarded["effect_key"]);
    assert_ne!(restarted["effect_key"], jobs[2]["effect_key"]);
    assert_eq!(
        f.ops(json!({"action":"discard","id":job_id}))["state"],
        "discarded"
    );

    let other = claim(
        &mut f.db,
        &f.ns,
        "default",
        r#"[{"worker":"Deliver","version":2}]"#,
        60_000,
    )
    .unwrap()
    .unwrap();
    assert_eq!(other.id, id(&jobs[0]));
    assert_eq!(f.finish(&other, Outcome::Complete), Settlement::Completed);
    assert_eq!(f.get(id(&jobs[1]))["state"], "scheduled");
    assert_eq!(
        f.ops(json!({"action":"list_queues","limit":1}))[0]["queue"],
        "default"
    );
    let page = f.ops(json!({"action":"list","limit":1}));
    let after = f.ops(json!({"action":"list","after_id":id(&page[0])}));
    assert_eq!(after.as_array().unwrap().len(), 2);
}

struct WorkerProcess(Child);

impl WorkerProcess {
    fn start(f: &Fixture) -> (Self, Option<Value>) {
        let module = module_path!().split_once("::").unwrap().1;
        let mut process = Self(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--ignored",
                    "--exact",
                    &format!("{module}::worker_process"),
                    "--nocapture",
                ])
                .env(TEST_PG_URL, &f.url)
                .env(CHILD_NS, &f.ns)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap(),
        );
        let mut reader = BufReader::new(process.0.stdout.take().unwrap());
        let value = reader
            .by_ref()
            .lines()
            .map(|line| line.unwrap())
            .find_map(|line| line.strip_prefix("claim=").map(parse))
            .expect("worker process exited without reporting a claim");
        process.0.stdout = Some(reader.into_inner());
        (process, (!value.is_null()).then_some(value))
    }
}

impl Drop for WorkerProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
#[ignore = "requires MESH_JOBS_TEST_PG_URL pointing at disposable PostgreSQL"]
fn worker_process() {
    let Ok(ns) = std::env::var(CHILD_NS) else {
        return;
    };
    let mut db = Connection::open(&std::env::var(TEST_PG_URL).unwrap());
    let claim = claim(&mut db, &ns, "default", WORKERS, 1_000).unwrap();
    let receipt = claim
        .as_ref()
        .map(|job| json!({"id":job.id,"token":job.token}));
    println!("claim={}", json!(receipt));
    std::io::stdout().flush().unwrap();
    if claim.is_some() {
        let _ = std::io::stdin().read(&mut [0_u8; 1]);
    }
}

#[test]
#[ignore = "requires MESH_JOBS_TEST_PG_URL pointing at disposable PostgreSQL"]
fn process_crash_retains_lease_and_recovery_enforces_exhaustion() {
    let mut f = Fixture::new();
    let original = f.enqueue(json!([
        {"worker":"Deliver","priority":0,"max_attempts":1},
        {"worker":"Deliver","priority":1}
    ]));
    let (mut worker, receipt) = WorkerProcess::start(&f);
    let receipt = receipt.unwrap();
    assert_eq!(id(&receipt), id(&original[0]));
    let (mut competitor, missed) = WorkerProcess::start(&f);
    assert!(missed.is_none());
    assert!(competitor.0.wait().unwrap().success());
    worker.0.kill().unwrap();
    assert!(!worker.0.wait().unwrap().success());
    assert!(f.try_claim().is_none());
    let job_id = id(&receipt);
    f.db.query(
        "SELECT pg_sleep(greatest(0, lease_expires_at_ms - mesh_jobs.now_ms() + 1) / 1000.0)
         FROM mesh_jobs.jobs WHERE namespace = $1 AND id = $2::uuid",
        &[&f.ns, job_id],
    )
    .unwrap();
    assert_eq!(recover(&mut f.db, &f.ns, 1, None).unwrap(), 1);
    assert_eq!(f.get(job_id)["state"], "discarded");
    assert_eq!(f.get(job_id)["errors"][0]["reason"], "lease expired");
    assert_eq!(
        finish(
            &mut f.db,
            &f.ns,
            job_id,
            receipt["token"].as_str().unwrap(),
            Outcome::Complete
        )
        .unwrap(),
        Settlement::Stale
    );
    assert_eq!(f.claim().id, id(&original[1]));
}

#[test]
#[ignore = "requires MESH_JOBS_TEST_PG_URL pointing at disposable PostgreSQL"]
fn locked_job_operations_leave_the_claim_for_retry() {
    let mut f = Fixture::new();
    f.enqueue(json!({"worker":"Deliver"}));
    let mut worker = Connection::open(&f.url);
    worker.simple("SET statement_timeout = 1000").unwrap();
    let claim = claim(&mut worker, &f.ns, "default", WORKERS, 60_000)
        .unwrap()
        .unwrap();

    f.db.simple("BEGIN").unwrap();
    f.db.query(
        "SELECT 1 FROM mesh_jobs.jobs WHERE namespace = $1 AND id = $2::uuid FOR NO KEY UPDATE",
        &[&f.ns, &claim.id],
    )
    .unwrap();
    let renewal = renew(&mut worker, &f.ns, &claim.id, &claim.token, 60_000);
    assert!(renewal.unwrap_err().contains("could not obtain lock"));
    for outcome in [Outcome::Complete, Outcome::ReleaseUnstarted] {
        let settlement = finish(&mut worker, &f.ns, &claim.id, &claim.token, outcome);
        assert!(settlement
            .unwrap_err()
            .message()
            .contains("could not obtain lock"));
    }
    f.db.simple("ROLLBACK").unwrap();
    assert_eq!(
        renew(&mut worker, &f.ns, &claim.id, &claim.token, 60_000).unwrap(),
        LeaseStatus::Active
    );
    f.db.query(
        "UPDATE mesh_jobs.jobs SET lease_expires_at_ms = mesh_jobs.now_ms() - 1
         WHERE namespace = $1 AND id = $2::uuid",
        &[&f.ns, &claim.id],
    )
    .unwrap();
    assert_eq!(
        finish(
            &mut worker,
            &f.ns,
            &claim.id,
            &claim.token,
            Outcome::Complete,
        )
        .unwrap(),
        Settlement::Stale
    );
    assert_eq!(f.get(&claim.id)["state"], "executing");
    assert_eq!(recover(&mut f.db, &f.ns, 1, None).unwrap(), 1);
}

struct CheckedSession {
    pool: u64,
    session: crate::db::jobs_connection::JobSession,
}

impl CheckedSession {
    fn open(url: &str) -> Self {
        use crate::db::pool::{mesh_pool_open, unbox_u64_payload};
        use crate::io::MeshResult;
        use crate::string::mesh_str;

        crate::gc::mesh_rt_init();
        let pool = unsafe { &*mesh_pool_open(mesh_str(url), 0, 1, 500).cast::<MeshResult>() };
        assert_eq!(pool.tag, 0);
        let pool = unsafe { unbox_u64_payload(pool.value) };
        let session = unsafe { crate::db::jobs_connection::JobSession::from_pool(pool) }.unwrap();
        Self { pool, session }
    }

    fn set(&mut self, ns: &str) -> SetClaimBatch {
        self.session
            .with(|db| claim_batch_set(db, ns, "default", WORKERS, 60_000, 16, true))
            .unwrap()
    }
}

impl Drop for CheckedSession {
    fn drop(&mut self) {
        crate::db::pool::mesh_pool_close(self.pool);
    }
}

#[test]
#[ignore = "requires MESH_JOBS_TEST_PG_URL pointing at disposable PostgreSQL"]
fn checked_set_batch_shares_capacity_with_existing_claimers_and_controls() {
    let mut fixture = Fixture::new();
    fixture.ops(json!({"action":"configure","queue":"default","concurrency":3}));
    let jobs = fixture.enqueue(json!([
        {"worker":"Deliver","priority":0},{"worker":"Deliver","priority":1},
        {"worker":"Deliver","priority":2},{"worker":"Deliver","priority":3},
        {"worker":"Deliver","priority":4},{"worker":"Deliver","priority":5}
    ]));
    let ns = fixture.ns.clone();
    let ready = |db: SetClaimBatch| match db {
        SetClaimBatch::Batch(batch) => batch.claims,
        SetClaimBatch::Empty(_) => Vec::new(),
        SetClaimBatch::Fallback => panic!("small grants should not need the cursor"),
    };
    let mut set = CheckedSession::open(&fixture.url);
    let mut legacy = Connection::open(&fixture.url);
    let legacy_grant = claim(&mut legacy, &ns, "default", WORKERS, 60_000)
        .unwrap()
        .unwrap();
    assert_eq!(legacy_grant.id, id(&jobs[0]));

    // A producer's foreign-key check must remain compatible with the
    // admission lock held while a batch waits for its own commit.
    let mut precommit = Connection::open(&fixture.url);
    precommit.simple("BEGIN").unwrap();
    assert_eq!(
        response(
            &mut precommit,
            "SELECT id FROM mesh_jobs.claim_batch_set_v1($1,'default',$2::jsonb,60000,1,NULL)",
            &[&ns, WORKERS]
        )
        .unwrap(),
        id(&jobs[1])
    );
    fixture.db.simple("SET lock_timeout = '250ms'").unwrap();
    fixture.enqueue(json!({"worker":"Deliver","priority":9}));
    fixture.db.simple("RESET lock_timeout").unwrap();
    precommit.simple("ROLLBACK").unwrap();

    let mut admin = Connection::open(&fixture.url);
    admin.simple("BEGIN").unwrap();
    admin
        .query(
            "SELECT mesh_jobs.queue_control($1,'default','scale',2)",
            &[&ns],
        )
        .unwrap();
    assert!(
        claim_batch(&mut legacy, &ns, "default", WORKERS, 60_000, 16, None)
            .unwrap()
            .claims
            .is_empty()
    );
    assert!(matches!(
        set.set(&ns),
        SetClaimBatch::Batch(ClaimBatch {
            contention_hint: true,
            ..
        })
    ));
    admin.simple("COMMIT").unwrap();

    let grant = ready(set.set(&ns)).pop().unwrap();
    assert_eq!(grant.id, id(&jobs[1]));
    assert!(grant.observed_at_ms.is_some());
    assert!(ready(set.set(&ns)).is_empty());
    assert!(fixture.try_claim().is_none());

    // Namespace control may take its advisory lock before touching queues.
    // Admission must return while that exclusive lock is still held.
    admin.simple("BEGIN").unwrap();
    admin
        .query("SELECT mesh_jobs.set_namespace_paused($1,true)", &[&ns])
        .unwrap();
    assert_eq!(
        fixture.finish(&legacy_grant, Outcome::Complete),
        Settlement::Completed
    );
    assert!(ready(set.set(&ns)).is_empty());
    admin
        .query(
            "SELECT mesh_jobs.queue_control($1,'default','pause',NULL)",
            &[&ns],
        )
        .unwrap();
    admin.simple("COMMIT").unwrap();
    assert!(ready(set.set(&ns)).is_empty());

    fixture.ops(json!({"action":"cancel","id":grant.id}));
    assert_eq!(
        renew(&mut legacy, &ns, &grant.id, &grant.token, 60_000).unwrap(),
        LeaseStatus::Cancelled
    );
    fixture.expire(&grant.id);
    assert_eq!(fixture.finish(&grant, Outcome::Complete), Settlement::Stale);
    assert_eq!(recover(&mut legacy, &ns, 1, None).unwrap(), 1);
    assert_eq!(fixture.get(&grant.id)["state"], "cancelled");
    admin
        .query("SELECT mesh_jobs.set_namespace_paused($1,false)", &[&ns])
        .unwrap();
    fixture.ops(json!({"action":"resume","queue":"default"}));

    admin.simple("BEGIN").unwrap();
    admin
        .query(
            "SELECT id FROM mesh_jobs.jobs WHERE namespace=$1 AND id=$2::uuid FOR NO KEY UPDATE",
            &[&ns, id(&jobs[2])],
        )
        .unwrap();
    assert_eq!(
        ready(set.set(&ns))
            .iter()
            .map(|grant| grant.id.as_str())
            .collect::<Vec<_>>(),
        [id(&jobs[3]), id(&jobs[4])]
    );
    admin.simple("ROLLBACK").unwrap();
}

#[test]
#[ignore = "requires the claim admission wire proof relay"]
fn checked_grant_never_escapes_an_invalid_or_uncertain_reply() {
    let Ok(url) = std::env::var("MESH_TEST_CLAIM_RELAY_URL") else {
        return;
    };
    let ns = std::env::var("MESH_TEST_CLAIM_RELAY_NAMESPACE").expect("wire proof namespace");
    let mut session = CheckedSession::open(&url);
    let result = session.session.with(|db| {
        if std::env::var_os("MESH_TEST_CLAIM_RELAY_CURSOR").is_some() {
            claim_batch_cursor_checked(db, &ns, "default", WORKERS, 60_000, 2, false)
                .map(SetClaimBatch::Batch)
        } else {
            claim_batch_set(db, &ns, "default", WORKERS, 60_000, 2, false)
        }
    });
    if std::env::var("MESH_TEST_CLAIM_RELAY_CASE").as_deref() == Ok("reply_budget") {
        assert!(matches!(&result, Err(error) if error.contains("32 MiB reply limit")));
    }
    assert!(
        result.is_err(),
        "the checked client returned a launchable grant"
    );
}

#[test]
#[ignore = "requires MESH_JOBS_TEST_PG_URL pointing at disposable PostgreSQL"]
fn checked_batch_keeps_the_ordered_prefix_when_the_set_reply_needs_cursor_sizing() {
    let mut fixture = Fixture::new();
    fixture.ops(json!({"action":"configure","queue":"default","concurrency":2}));
    let jobs = fixture.enqueue(json!([
        {"worker":"Deliver","priority":0},
        {"worker":"Deliver","priority":1},
        {"worker":"Deliver","priority":2}
    ]));
    let large = id(&jobs[1]);
    fixture
        .db
        .query(
            "UPDATE mesh_jobs.jobs SET args = jsonb_build_object(
           'blob',(SELECT string_agg(md5(x::text), '') FROM generate_series(1,4096) x))
         WHERE namespace=$1 AND id=$2::uuid",
            &[&fixture.ns, large],
        )
        .unwrap();
    let mut set = CheckedSession::open(&fixture.url);
    assert!(matches!(set.set(&fixture.ns), SetClaimBatch::Fallback));
    assert_eq!(fixture.get(id(&jobs[0]))["attempt"], 0);
    assert_eq!(fixture.get(large)["attempt"], 0);
    let batch = set
        .session
        .with(|db| {
            claim_batch_cursor_checked(db, &fixture.ns, "default", WORKERS, 60_000, 2, false)
        })
        .unwrap();
    assert_eq!(
        batch
            .claims
            .iter()
            .map(|job| job.id.as_str())
            .collect::<Vec<_>>(),
        [id(&jobs[0]), large]
    );
    assert_eq!(
        serde_json::from_str::<Value>(&batch.claims[1].context).unwrap()["args"]["blob"]
            .as_str()
            .unwrap()
            .len(),
        131_072
    );
    assert!(fixture.try_claim().is_none());
}
