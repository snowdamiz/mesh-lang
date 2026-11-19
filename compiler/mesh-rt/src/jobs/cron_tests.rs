use super::*;
use crate::db::jobs_connection::{with_connection, PgJobDb};
use crate::db::pg::{mesh_pg_close, mesh_pg_connect};
use crate::io::MeshResult;
use crate::string::{mesh_str, text_of, MeshString};
use chrono::DateTime;
use std::sync::Barrier;

#[test]
fn parses_five_fields_and_oban_aliases() {
    let mask = |values: &[u8]| values.iter().fold(0, |bits, value| bits | (1 << value));
    assert_eq!(
        Expression::parse("*/15 9-12/2,14 13 jAN,Dec fri-SaT,7"),
        Ok(Expression::Calendar([
            mask(&[0, 15, 30, 45]),
            mask(&[9, 11, 14]),
            mask(&[13]),
            mask(&[1, 12]),
            mask(&[0, 5, 6]),
        ]))
    );
    for (alias, fields) in [
        ("@hourly", "0 * * * *"),
        ("@daily", "0 0 * * *"),
        ("@midnight", "0 0 * * *"),
        ("@weekly", "0 0 * * SUN"),
        ("@monthly", "0 0 1 * *"),
        ("@yearly", "0 0 1 1 *"),
        ("@annually", "0 0 1 1 *"),
    ] {
        assert_eq!(Expression::parse(alias), Expression::parse(fields));
    }
    assert_eq!(
        Expression::parse("0 0 * * 6/1"),
        Expression::parse("0 0 * * SAT")
    );
    assert_eq!(
        Expression::parse("0 0 * * 7/1"),
        Expression::parse("0 0 * * SUN")
    );
    assert_eq!(Expression::parse("@reboot"), Ok(Expression::Reboot));
}

#[test]
fn rejects_out_of_range_and_impossible_fields() {
    for invalid in [
        "* * * *",
        "* * * * * *",
        "@never",
        "*/0 * * * *",
        "*/+2 * * * *",
        "* 24 * * *",
        "* * 0 * *",
        "* * * 0 *",
        "* * * * 8",
        "* * * * FRI-MON",
        "* * 30 FEB *",
        "* * * JAN, *",
        "* * * * 0/100",
        "+10 * * * *",
        "1--10000000 * * * *",
    ] {
        assert!(Expression::parse(invalid).is_err(), "accepted {invalid}");
    }
    assert!(Expression::parse("0 0 29 FEB *").is_ok());
}

struct Connection(u64);

impl Connection {
    fn open() -> Self {
        let url = std::env::var("MESH_JOBS_TEST_PG_URL").expect("MESH_JOBS_TEST_PG_URL is set");
        let result = unsafe { &*mesh_pg_connect(mesh_str(&url)).cast::<MeshResult>() };
        if result.tag != 0 {
            panic!("{}", unsafe { text_of(result.value.cast::<MeshString>()) });
        }
        Self(unsafe { *result.value.cast::<u64>() })
    }

    fn run<T>(&mut self, f: impl FnOnce(&mut PgJobDb<'_>) -> Result<T, String>) -> T {
        unsafe { with_connection(self.0, f) }.unwrap()
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        mesh_pg_close(self.0);
    }
}

struct Namespace(String);

impl std::ops::Deref for Namespace {
    type Target = str;

    fn deref(&self) -> &str {
        &self.0
    }
}

impl Drop for Namespace {
    fn drop(&mut self) {
        Connection::open().run(|db| {
            for table in [
                "cron_schedules",
                "cron_activation",
                "receipts",
                "jobs",
                "queues",
                "admissions",
            ] {
                db.query(
                    &format!("DELETE FROM mesh_jobs.{table} WHERE namespace = $1"),
                    &[self],
                )?;
            }
            Ok(())
        });
    }
}

fn setup(db: &mut impl Database, name: &str) -> Result<Namespace, String> {
    super::super::store::migrate(db)?;
    migrate(db)?;
    migrate(db)?;
    let rows = db.query("SELECT pg_catalog.gen_random_uuid()::text", &[])?;
    Ok(Namespace(format!(
        "cron-test-{name}-{}",
        only_value(&rows)?
    )))
}

fn ms(utc: &str) -> i64 {
    DateTime::parse_from_rfc3339(utc)
        .unwrap()
        .timestamp_millis()
}

fn register(
    db: &mut impl Database,
    namespace: &str,
    name: &str,
    expression: &str,
    timezone: &str,
    misfire: &str,
    lookback_minutes: u32,
) -> Result<Value, String> {
    let req = json!({
        "action": "schedule", "name": name, "expression": expression, "timezone": timezone,
        "misfire": misfire, "lookback_minutes": lookback_minutes,
        "job": {"worker": name, "version": 3, "args": {"x": 9}, "queue": "cron", "max_attempts": 2}
    });
    Ok(serde_json::from_str(&control(db, namespace, &req.to_string())?).unwrap())
}

fn position_cursor(db: &mut impl Database, namespace: &str, utc_ms: i64) -> Result<(), String> {
    db.query(
        "UPDATE mesh_jobs.cron_schedules SET cursor_minute = $2::bigint WHERE namespace = $1",
        &[namespace, &(utc_ms / MINUTE_MS).to_string()],
    )?;
    Ok(())
}

fn times(db: &mut impl Database, namespace: &str, worker: &str) -> Result<Vec<i64>, String> {
    let rows = db.query(
        "SELECT scheduled_at_ms::text FROM mesh_jobs.jobs
         WHERE namespace = $1 AND worker = $2 ORDER BY scheduled_at_ms",
        &[namespace, worker],
    )?;
    rows.iter()
        .map(|row| Ok(column(row)?.parse().unwrap()))
        .collect()
}

#[test]
#[ignore = "requires MESH_JOBS_TEST_PG_URL"]
fn cron_control_is_validated_paginated_and_namespace_scoped() {
    Connection::open().run(|db| {
        let ns = setup(db, "control")?;
        let monday = register(db, &ns, "a'quoted", "@weekly", "Etc/UTC", "skip", 60)?;
        assert_eq!(
            register(db, &ns, "a'quoted", "@weekly", "Etc/UTC", "skip", 60)?,
            monday
        );
        register(db, &ns, "b", "@midnight", "Asia/Kathmandu", "skip", 60)?;
        assert!(register(db, &ns, "invalid", "@hourly", "+02:30", "skip", 60).is_err());
        assert!(register(db, &ns, "invalid", "@hourly", "Etc/UTC", "skip", 0).is_err());
        assert!(register(db, &ns, "invalid", "@hourly", "Etc/UTC", "skip", 1441).is_err());
        for job in [
            json!({"worker":"x", "args": []}),
            json!({"worker":"x", "request_key":"mine"}),
        ] {
            let req =
                json!({"action":"schedule", "name":"invalid", "expression":"@hourly", "job":job});
            assert!(control(db, &ns, &req.to_string()).is_err());
        }
        let first = r#"{"action":"list_schedules","limit":1}"#;
        assert_eq!(
            serde_json::from_str::<Value>(&control(db, &ns, first)?).unwrap(),
            json!([monday])
        );
        let next = r#"{"action":"list_schedules","after":"a'quoted","limit":1}"#;
        assert_eq!(
            serde_json::from_str::<Value>(&control(db, &ns, next)?).unwrap()[0]["name"],
            "b"
        );
        let other_ns = format!("{}-other", &*ns);
        assert_eq!(control(db, &other_ns, first)?, "[]");
        let delete = r#"{"action":"unschedule","name":"a'quoted"}"#;
        assert_eq!(control(db, &other_ns, delete)?, r#"{"removed":false}"#);
        assert_eq!(control(db, &ns, delete)?, r#"{"removed":true}"#);
        let replacement = register(db, &ns, "a'quoted", "@weekly", "Etc/UTC", "skip", 60)?;
        assert_ne!(replacement["id"], monday["id"]);
        let large = " ".repeat(MAX_REQUEST_BYTES + 1);
        assert!(control(db, &ns, &large).is_err());
        Ok(())
    });
}

#[test]
#[ignore = "requires MESH_JOBS_TEST_PG_URL"]
fn concurrent_producers_enqueue_one_job_per_minute_and_activation_epoch() {
    let mut connection = Connection::open();
    let now = ms("2025-05-01T12:00:00Z");
    let ns = connection.run(|db| {
        let ns = setup(db, "concurrent")?;
        register(db, &ns, "every-minute", "* * * * *", "Etc/UTC", "skip", 60)?;
        register(db, &ns, "reboot", "@reboot", "Etc/UTC", "skip", 60)?;
        position_cursor(db, &ns, now - MINUTE_MS)?;
        Ok(ns)
    });
    let barrier = Barrier::new(2);
    let enqueued = std::thread::scope(|scope| {
        let barrier = &barrier;
        let ns = &ns;
        let start = |mut connection: Connection, owner: &'static str| {
            scope.spawn(move || {
                barrier.wait();
                connection.run(|db| tick_with_clock(db, ns, owner, 8, Some(now)))
            })
        };
        let a = Connection::open();
        let b = Connection::open();
        let first = start(a, "a");
        let second = start(b, "b");
        first.join().unwrap() + second.join().unwrap()
    });
    assert_eq!(enqueued, 2);
    Connection::open().run(|db| {
        assert_eq!(
            tick_with_clock(db, &ns, "restarted-owner", 8, Some(now + 30_000))?,
            0
        );
        assert_eq!(times(db, &ns, "reboot")?, [now]);
        assert_eq!(times(db, &ns, "every-minute")?, [now]);
        let restart = now + 90_000;
        assert_eq!(tick_with_clock(db, &ns, "new-owner", 8, Some(restart))?, 2);
        assert_eq!(
            tick_with_clock(db, &ns, "another-owner", 8, Some(restart))?,
            0
        );
        assert_eq!(times(db, &ns, "reboot")?, [now, restart]);
        assert_eq!(times(db, &ns, "every-minute")?, [now, now + MINUTE_MS]);
        let rows = db.query(
            "SELECT count(*)::text FROM mesh_jobs.jobs WHERE namespace = $1 AND version = 3
             AND queue = 'cron' AND max_attempts = 2 AND args = '{\"x\":9}'::jsonb",
            &[&ns],
        )?;
        assert_eq!(only_value(&rows)?, "4");
        Ok(())
    });
}

#[test]
#[ignore = "requires MESH_JOBS_TEST_PG_URL"]
fn evaluates_dst_gaps_repeated_minutes_and_combined_day_fields() {
    Connection::open().run(|db| {
        let ns = setup(db, "dst")?;
        let zone = "America/New_York";
        register(db, &ns, "gap", "30 2 * * *", zone, "catch_up", 180)?;
        register(db, &ns, "after-gap", "30 3 * * *", zone, "catch_up", 180)?;
        register(db, &ns, "repeat", "30 1 * * *", zone, "catch_up", 180)?;
        position_cursor(db, &ns, ms("2025-03-09T06:59:00Z"))?;
        assert_eq!(
            tick_with_clock(db, &ns, "owner", 8, Some(ms("2025-03-09T08:00:00Z")))?,
            1
        );
        assert!(times(db, &ns, "gap")?.is_empty());
        assert_eq!(times(db, &ns, "after-gap")?, [ms("2025-03-09T07:30:00Z")]);
        position_cursor(db, &ns, ms("2025-11-02T04:59:00Z"))?;
        assert_eq!(
            tick_with_clock(db, &ns, "owner", 8, Some(ms("2025-11-02T07:00:00Z")))?,
            2
        );
        assert_eq!(
            times(db, &ns, "repeat")?,
            [ms("2025-11-02T05:30:00Z"), ms("2025-11-02T06:30:00Z")]
        );

        register(
            db,
            &ns,
            "friday-13th",
            "0 12 13 * FRI",
            "Etc/UTC",
            "skip",
            60,
        )?;
        position_cursor(db, &ns, ms("2025-06-06T11:59:00Z"))?;
        tick_with_clock(db, &ns, "owner", 8, Some(ms("2025-06-06T12:00:00Z")))?;
        tick_with_clock(db, &ns, "owner", 8, Some(ms("2025-06-13T12:00:00Z")))?;
        tick_with_clock(db, &ns, "owner", 8, Some(ms("2025-07-13T12:00:00Z")))?;
        assert_eq!(times(db, &ns, "friday-13th")?, [ms("2025-06-13T12:00:00Z")]);
        Ok(())
    });
}

#[test]
#[ignore = "requires MESH_JOBS_TEST_PG_URL"]
fn resumes_bounded_catch_up_and_skips_or_coalesces_missed_matches() {
    Connection::open().run(|db| {
        let ns = setup(db, "downtime")?;
        let now = ms("2025-05-01T12:09:00Z");
        register(db, &ns, "catch", "*/2 * * * *", "Etc/UTC", "catch_up", 5)?;
        register(db, &ns, "coalesce", "*/2 * * * *", "Etc/UTC", "coalesce", 5)?;
        register(db, &ns, "skip", "*/2 * * * *", "Etc/UTC", "skip", 5)?;
        position_cursor(db, &ns, ms("2024-01-01T00:00:00Z"))?;
        assert_eq!(tick_with_clock(db, &ns, "owner", 0, Some(now))?, 0);
        assert!(times(db, &ns, "catch")?.is_empty());
        assert_eq!(tick_with_clock(db, &ns, "owner", 1, Some(now))?, 1);
        assert_eq!(times(db, &ns, "catch")?, [now - 3 * MINUTE_MS]);
        assert_eq!(tick_with_clock(db, &ns, "owner", 8, Some(now))?, 2);
        assert_eq!(tick_with_clock(db, &ns, "owner", 8, Some(now))?, 0);
        assert_eq!(times(db, &ns, "coalesce")?, [now - MINUTE_MS]);
        assert!(times(db, &ns, "skip")?.is_empty());
        assert_eq!(
            times(db, &ns, "catch")?,
            [now - 3 * MINUTE_MS, now - MINUTE_MS]
        );
        assert_eq!(
            tick_with_clock(db, &ns, "owner", 8, Some(now - 10 * MINUTE_MS))?,
            0
        );
        assert_eq!(
            tick_with_clock(db, &ns, "owner", 8, Some(now + MINUTE_MS))?,
            3
        );
        assert_eq!(times(db, &ns, "skip")?, [now + MINUTE_MS]);
        Ok(())
    });
}

#[test]
#[ignore = "requires MESH_JOBS_TEST_PG_URL"]
fn failed_enqueue_rolls_back_the_occurrence_and_its_progress() {
    Connection::open().run(|db| {
        let ns = setup(db, "rollback")?;
        let now = ms("2025-05-01T12:00:00Z");
        register(db, &ns, "catch", "* * * * *", "Etc/UTC", "catch_up", 2)?;
        position_cursor(db, &ns, now - 2 * MINUTE_MS)?;
        struct FailSecondEnqueue<'a, D>(&'a mut D, usize);
        impl<D: Database> Database for FailSecondEnqueue<'_, D> {
            fn query(&mut self, sql: &str, params: &[&str]) -> Result<Rows, String> {
                if sql.contains("mesh_jobs.enqueue(") {
                    self.1 += 1;
                    if self.1 == 2 {
                        return self.0.query("SELECT 1 / 0", &[]);
                    }
                }
                self.0.query(sql, params)
            }

            fn simple(&mut self, sql: &str) -> Result<(), String> {
                self.0.simple(sql)
            }

            fn in_transaction(&self) -> bool {
                self.0.in_transaction()
            }
        }
        let mut failing = FailSecondEnqueue(db, 0);
        assert!(tick_with_clock(&mut failing, &ns, "owner", 8, Some(now)).is_err());
        assert_eq!(failing.1, 2);
        assert!(times(db, &ns, "catch")?.is_empty());
        assert_eq!(tick_with_clock(db, &ns, "owner", 8, Some(now))?, 2);
        assert_eq!(times(db, &ns, "catch")?, [now - MINUTE_MS, now]);
        assert_eq!(tick_with_clock(db, &ns, "owner", 8, Some(now))?, 0);
        assert_eq!(
            only_value(&db.query(
                "SELECT count(*)::text FROM mesh_jobs.receipts WHERE namespace = $1",
                &[&ns]
            )?)?,
            "0"
        );
        Ok(())
    });
}

#[test]
#[ignore = "requires MESH_JOBS_TEST_PG_URL"]
fn production_tick_uses_database_clock() {
    Connection::open().run(|db| {
        let ns = setup(db, "clock")?;
        register(db, &ns, "same-minute", "* * * * *", "Etc/UTC", "skip", 60)?;
        assert_eq!(tick(db, &ns, "owner", 8)?, 1);
        let rows = db.query(
            "SELECT (scheduled_at_ms % 60000 = 0 AND scheduled_at_ms <= inserted_at_ms
                     AND inserted_at_ms - scheduled_at_ms < 120000)::text
             FROM mesh_jobs.jobs WHERE namespace = $1",
            &[&ns],
        )?;
        assert_eq!(only_value(&rows)?, "true");
        Ok(())
    });
}
