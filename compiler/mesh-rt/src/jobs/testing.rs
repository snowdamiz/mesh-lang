use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use serde_json::Value;

use super::store::{self, State, MAX_TIMESTAMP_MS};
use super::{runner, worker};
use crate::string::MeshString;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Filter {
    queue: Option<String>,
    worker: Option<String>,
    version: Option<u32>,
    args: Option<Value>,
    state: Option<State>,
    limit: Option<u32>,
}

impl Filter {
    fn parse(namespace: &str, filter: &str) -> Result<Self, String> {
        store::name(namespace, "namespace")?;
        let filter: Self = store::decode(filter)?;
        if let Some(queue) = &filter.queue {
            store::name(queue, "queue")?;
        }
        if let Some(worker) = &filter.worker {
            store::name(worker, "worker")?;
        }
        if filter.version == Some(0) || filter.args.as_ref().is_some_and(|args| !args.is_object()) {
            return Err("job test filters require a positive version and an args object".into());
        }
        if filter
            .limit
            .is_some_and(|limit| !(1..=100).contains(&limit))
        {
            return Err("job test inspection limit must be between 1 and 100".into());
        }
        Ok(filter)
    }

    fn query(
        &self,
        db: &mut impl store::Database,
        namespace: &str,
        inspect: bool,
    ) -> Result<String, String> {
        // Keep the read inside the caller's transaction when given a borrowed connection.
        let (order, projection) = if inspect {
            ("ORDER BY position",
             "SELECT coalesce(jsonb_agg(mesh_jobs.job_json(matches.job) ORDER BY matches.position),
                              '[]')::text FROM matches")
        } else {
            ("", "SELECT EXISTS (SELECT FROM matches)::text")
        };
        let sql = format!(
            "WITH matches AS (
                 SELECT j AS job, position FROM mesh_jobs.jobs j WHERE namespace = $1
                     AND (nullif($2, '') IS NULL OR queue = $2)
                     AND (nullif($3, '') IS NULL OR worker = $3)
                     AND (nullif($4, '') IS NULL OR version = nullif($4, '')::bigint)
                     AND (nullif($5, '') IS NULL OR args @> nullif($5, '')::jsonb)
                     AND (nullif($6, '') IS NULL OR state = $6)
                 {order} LIMIT $7::integer
             ) {projection}"
        );
        let state = serde_json::to_value(self.state).map_err(|error| error.to_string())?;
        store::response(
            db,
            &sql,
            &[
                namespace,
                self.queue.as_deref().unwrap_or_default(),
                self.worker.as_deref().unwrap_or_default(),
                &self.version.map(|v| v.to_string()).unwrap_or_default(),
                &self.args.as_ref().map(Value::to_string).unwrap_or_default(),
                state.as_str().unwrap_or_default(),
                &if inspect { self.limit.unwrap_or(20) } else { 1 }.to_string(),
            ],
        )
    }
}

fn with_scope<T>(
    handle: u64,
    borrowed: bool,
    run: impl FnOnce(&mut super::PgJobDb<'_>) -> Result<T, String>,
) -> Result<T, String> {
    if borrowed {
        unsafe {
            super::jobs_connection::with_connection(handle, |db| {
                if !db.in_transaction() {
                    return Err(
                        "job test inspection requires an active PostgreSQL transaction".into(),
                    );
                }
                run(db)
            })
        }
    } else {
        super::with_pool(handle, run)
    }
}

/// Rust bridge for the separately linked test runtime.
///
/// # Safety
/// The caller must hold a live PoolHandle, or exclusively borrow a live PgConn
/// if `borrowed` is true.
pub unsafe fn inspect(
    handle: u64,
    borrowed: bool,
    namespace: &str,
    filter: &str,
) -> Result<String, String> {
    let filter = Filter::parse(namespace, filter)?;
    with_scope(handle, borrowed, |db| filter.query(db, namespace, true))
}

/// Rust bridge for test assertions. A borrowed scope checks once. Pool checks
/// can poll fresh PostgreSQL snapshots for up to `wait_ms`.
///
/// # Safety
/// Same handle contract as [inspect].
pub unsafe fn assert_match(
    handle: u64,
    borrowed: bool,
    namespace: &str,
    filter: &str,
    wait_ms: i64,
    present: bool,
) -> Result<(), String> {
    if !(0..=60_000).contains(&wait_ms) || (borrowed && wait_ms != 0) {
        return Err("job test wait_ms must be between 0 and 60000 (0 inside a transaction)".into());
    }
    let filter = Filter::parse(namespace, filter)?;
    let deadline = Instant::now() + Duration::from_millis(wait_ms as u64);
    loop {
        let matched =
            with_scope(handle, borrowed, |db| filter.query(db, namespace, false))? == "true";
        if matched {
            return if present {
                Ok(())
            } else {
                Err(format!("unexpected matching job in namespace {namespace}"))
            };
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return if present {
                Err(format!("no matching job in namespace {namespace}"))
            } else {
                Ok(())
            };
        }
        crate::actor::mesh_timer_sleep(remaining.as_millis().clamp(1, 20) as i64);
    }
}

/// Construct a synthetic attempt. A clock of -1 uses the system clock.
pub fn build_job(spec: &str, clock_ms: i64) -> Result<String, String> {
    let now = if clock_ms == -1 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_millis() as u64
    } else {
        u64::try_from(clock_ms).map_err(|_| "invalid job test clock")?
    };
    if now > MAX_TIMESTAMP_MS {
        return Err("invalid job test clock".into());
    }
    store::test_context(spec, now)
}

/// Execute a synthetic attempt and return the production-decoded outcome.
///
/// # Safety
/// The non-null function pointer and its environment must be a live compiled
/// Mesh callback of type fn(String) -> String.
pub unsafe fn perform_job(
    spec: &str,
    clock_ms: i64,
    handler: *mut u8,
    env: *mut u8,
) -> Result<String, String> {
    if handler.is_null() {
        return Err("job test requires a handler".into());
    }
    let context = build_job(spec, clock_ms)?;
    let reply = crate::panic::quietly(|| {
        worker::invoke(&context, |input| {
            crate::callback::call1(handler, env, input as u64) as *const MeshString
        })
    });
    serde_json::to_string(&runner::Reply::from_worker(reply).outcome().0)
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthetic_context_uses_enqueue_defaults_and_its_own_identity() {
        let input = r#"{"worker":"example","delay_ms":5}"#;
        let first: Value = serde_json::from_str(&build_job(input, 11).unwrap()).unwrap();
        let second: Value = serde_json::from_str(&build_job(input, 11).unwrap()).unwrap();
        assert_ne!(first["id"], second["id"]);
        assert_eq!(
            first["effect_key"],
            format!("{}/1", first["id"].as_str().unwrap())
        );
        assert_eq!(first["scheduled_at_ms"], 16);
        assert_eq!(first["queue"], "default");
        assert_eq!(first["attempt"], 1);
        assert_eq!(first["max_attempts"], 20);
        assert!(build_job(r#"{"worker":"example","args":[]}"#, 11).is_err());
        assert!(build_job(input, MAX_TIMESTAMP_MS as i64).is_err());
        assert!(build_job(input, -2).is_err());
    }
}
