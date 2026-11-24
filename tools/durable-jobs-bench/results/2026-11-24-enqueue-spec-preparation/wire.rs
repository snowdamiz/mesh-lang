use super::super::{enqueue, enqueue_candidate, migrate, response, Database, JobsAckError};
use crate::db::pg::{native_pg_connect, native_pg_execute, native_pg_query, NativePgConn};
use serde_json::Value;

struct Connection {
    inner: NativePgConn,
    transaction: bool,
}

impl Connection {
    fn open(url: &str) -> Self {
        Self { inner: native_pg_connect(url).unwrap(), transaction: false }
    }
}

impl Database for Connection {
    fn query(&mut self, sql: &str, params: &[&str]) -> Result<Vec<Vec<(String, String)>>, String> {
        native_pg_query(&mut self.inner, sql, params)
    }

    fn acknowledge(&mut self, sql: &str, params: &[&str]) -> Result<Vec<Vec<(String, String)>>, JobsAckError> {
        self.query(sql, params).map_err(JobsAckError::Unconfirmed)
    }

    fn simple(&mut self, sql: &str) -> Result<(), String> {
        // The native Rust test connection sends extended queries. Install the
        // multi-statement migration batches with one PostgreSQL DO statement.
        let wrapped;
        let statement = if sql.contains("CREATE ") && sql.contains(';') {
            wrapped = format!("DO $proof$ BEGIN EXECUTE $migration$ {sql} $migration$; END $proof$");
            wrapped.as_str()
        } else {
            sql
        };
        native_pg_execute(&mut self.inner, statement, &[])?;
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

type Writer = fn(&mut Connection, &str, &str) -> Result<String, String>;

fn count(db: &mut Connection, namespace: &str, table: &str) -> usize {
    response(db, &format!("SELECT count(*)::text FROM mesh_jobs.{table} WHERE namespace = $1"), &[namespace])
        .unwrap().parse().unwrap()
}

pub(crate) fn run() {
    let url = std::env::var("ENQUEUE_SPEC_TEST_PG_URL").unwrap();
    let mut writer = Connection::open(&url);
    let mut observer = Connection::open(&url);
    migrate(&mut writer).unwrap();
    let pg = response(&mut observer, "SHOW server_version", &[]).unwrap();
    let batch = |prefix| format!("[{}]", (0..16).map(|i| format!(
        r#"{{"worker":"deliver","args":{{"dup":3,"dup":4,"text":"α\u001f🙂","number":-0.0}},"meta":{{"说明":[1e-22,null]}},"request_key":"{prefix}-{i}"}}"#
    )).collect::<Vec<_>>().join(","));

    let mut idempotent_rows = 0;
    for (ns, new_first) in [("enqueue-old-new", false), ("enqueue-new-old", true)] {
        let submit: Writer = if new_first { enqueue_candidate } else { enqueue };
        let retry: Writer = if new_first { enqueue } else { enqueue_candidate };
        let input = batch(ns);
        assert_eq!(submit(&mut writer, ns, &input).unwrap_err(),
                   "mesh jobs enqueue requires an outer transaction");
        writer.simple("BEGIN").unwrap();
        let first: Value = serde_json::from_str(&submit(&mut writer, ns, &input).unwrap()).unwrap();
        assert_eq!(first.as_array().unwrap().len(), 16);
        assert_eq!(count(&mut observer, ns, "jobs"), 0);
        assert_eq!(count(&mut observer, ns, "receipts"), 0);
        writer.simple("COMMIT").unwrap();
        assert_eq!(count(&mut observer, ns, "jobs"), 16);
        assert_eq!(count(&mut observer, ns, "receipts"), 16);
        writer.simple("BEGIN").unwrap();
        let repeated: Value = serde_json::from_str(&retry(&mut writer, ns, &input).unwrap()).unwrap();
        assert!(first == repeated);
        let changed = input.replacen("dup\":4", "dup\":5", 1);
        assert!(retry(&mut writer, ns, &changed).unwrap_err().contains("already bound"));
        writer.simple("COMMIT").unwrap();
        assert_eq!(count(&mut observer, ns, "jobs"), 16);
        assert_eq!(count(&mut observer, ns, "receipts"), 16);
        idempotent_rows += 16;
    }
    let ns = "enqueue-rollback";
    writer.simple("BEGIN").unwrap();
    writer.simple("CREATE TEMP TABLE app_proof (x integer)").unwrap();
    writer.simple("INSERT INTO app_proof VALUES (7)").unwrap();
    let invalid = r#"[{"worker":"deliver","request_key":"good"},{"worker":"deliver","args":{"invalid":"\u0000"}}]"#;
    let a = enqueue(&mut writer, ns, invalid).unwrap_err();
    let b = enqueue_candidate(&mut writer, ns, invalid).unwrap_err();
    assert_eq!(a, b);
    assert!(a.contains("Unicode escape"));
    assert_eq!(count(&mut writer, ns, "jobs"), 0);
    assert_eq!(count(&mut writer, ns, "receipts"), 0);
    assert_eq!(response(&mut writer, "SELECT x::text FROM app_proof", &[]).unwrap(), "7");
    enqueue_candidate(&mut writer, ns, &batch(ns)).unwrap();
    assert_eq!(count(&mut writer, ns, "jobs"), 16);
    assert_eq!(count(&mut observer, ns, "jobs"), 0);
    writer.simple("ROLLBACK").unwrap();
    assert_eq!(count(&mut observer, ns, "jobs"), 0);
    assert_eq!(count(&mut observer, ns, "receipts"), 0);
    let bulk_enabled = std::env::var("MESH_JOBS_SET_BASED_ENQUEUE").as_deref() == Ok("1");
    let bulk_calls = if bulk_enabled {
        let calls = response(&mut observer,
            "SELECT coalesce(sum(calls),0)::text FROM pg_stat_statements WHERE query LIKE 'SELECT ordinal, receipt FROM mesh_jobs.enqueue_batch_v1(%' AND query NOT LIKE '%pg_stat_statements%'", &[])
            .unwrap().parse::<usize>().unwrap();
        assert!(calls >= 5, "the opted-in writer did not run");
        calls
    } else {
        0
    };
    println!("{}", serde_json::json!({"postgres": pg, "accepted_cross_version_replays": idempotent_rows,
              "outer_rollback": true, "savepoint_recovery": true, "bulk_calls": bulk_calls}));
}
