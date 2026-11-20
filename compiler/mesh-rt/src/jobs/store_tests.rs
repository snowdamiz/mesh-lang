use super::*;
use crate::db::pg::{native_pg_connect, native_pg_execute, native_pg_query, NativePgConn};
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::Barrier;
use std::thread;

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
        batch.claims.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
        [id(&jobs[3]), id(&jobs[1])]
    );
    assert_ne!(batch.claims[0].token, batch.claims[1].token);
    let context = parse(&batch.claims[0].context);
    assert_eq!(context["effect_key"], jobs[3]["effect_key"]);
    assert_eq!(context["attempt"], 1);
    assert_eq!(f.get(&batch.claims[0].id)["state"], "executing");
    assert!(f.try_claim().is_none());
    assert_eq!(f.get(id(&jobs[0]))["attempt"], 0);

    assert_eq!(f.finish(&batch.claims[0], Outcome::ReleaseUnstarted), Settlement::ReleasedUnstarted);
    let retried = f.claim();
    assert_eq!(retried.id, batch.claims[0].id);
    assert_ne!(retried.token, batch.claims[0].token);
    assert_eq!(parse(&retried.context), context);
    assert_eq!(f.finish(&batch.claims[0], Outcome::Complete), Settlement::Stale);
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
    assert!(racing_ack.unwrap_err().contains("could not obtain lock"));
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
        assert!(settlement.unwrap_err().contains("could not obtain lock"));
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
