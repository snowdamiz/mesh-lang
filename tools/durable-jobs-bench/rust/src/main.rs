use std::{
    collections::BTreeMap,
    env, fs,
    io::{self, BufRead, Write},
    path::PathBuf,
    str::FromStr,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use apalis::prelude::*;
use apalis_postgres::{Config, PgTask, PostgresStorage, Pubsub, queries::push_tasks};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{
    Connection, PgConnection, PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use ulid::Ulid;

const ENGINE: &str = "apalis";
const QUEUE: &str = "parity";
const CHUNK_SIZE: usize = 128;
const FAIL_FIRST: &str = "fixture_fail_first";

#[derive(Deserialize, Serialize)]
struct Job {
    key: String,
    payload: String,
    hold_ms: u64,
    fail_first: bool,
}

#[derive(Deserialize)]
struct Batch {
    request_id: String,
    jobs: Vec<Job>,
    rollback: bool,
}

#[derive(Deserialize)]
#[serde(tag = "verb", rename_all = "snake_case")]
enum Request {
    Enqueue { path: PathBuf, max_attempts: usize },
    Database,
    Sample,
    Sessions,
    Counters,
    Jobs,
    Writes,
    Stop,
}

async fn enqueue(
    conn: &mut PgConnection,
    batch: Batch,
    max_attempts: usize,
) -> Result<Value, BoxDynError> {
    let mut tx = conn.begin().await?;
    let xid: String = sqlx::query_scalar("SELECT pg_current_xact_id()::text")
        .fetch_one(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO public.mesh_jobs_parity_product VALUES ($1, $2, $3)")
        .bind(ENGINE)
        .bind(QUEUE)
        .bind(&batch.request_id)
        .execute(&mut *tx)
        .await?;

    let mut ids = BTreeMap::new();
    let mut jobs = batch.jobs.into_iter().peekable();
    while jobs.peek().is_some() {
        let tasks = jobs
            .by_ref()
            .take(CHUNK_SIZE)
            .map(|job| {
                let id = TaskId::from_ulid(Ulid::generate());
                if ids.insert(job.key.clone(), id.to_string()).is_some() {
                    return Err("duplicate input job key".into());
                }
                // The transactional API accepts stock Postgres wire-format tasks.
                Ok(TaskBuilder::new(serde_json::to_vec(&job)?)
                    .task_id(id)
                    .max_attempts(max_attempts)
                    .build())
            })
            .collect::<Result<Vec<PgTask>, BoxDynError>>()?;
        push_tasks(&mut *tx, QUEUE, tasks).await?;
    }

    let committed_us = if batch.rollback {
        tx.rollback().await?;
        None
    } else {
        tx.commit().await?;
        sqlx::query_scalar::<_, Option<i64>>(
            "SELECT (extract(epoch FROM pg_xact_commit_timestamp(xmin))*1000000)::bigint
             FROM public.mesh_jobs_parity_product
             WHERE engine = $1 AND namespace = $2 AND request_id = $3 AND xmin = $4::text::xid",
        )
        .bind(ENGINE)
        .bind(QUEUE)
        .bind(&batch.request_id)
        .bind(&xid)
        .fetch_one(conn)
        .await?
    };
    Ok(
        json!({"request_id": batch.request_id, "xid": xid, "enqueued_ids": ids,
              "committed_us": committed_us}),
    )
}

async fn run_session(options: PgConnectOptions) -> Result<(), BoxDynError> {
    let mut conn = PgConnection::connect_with(&options).await?;
    let mut out = io::BufWriter::new(io::stdout().lock());
    writeln!(out, "SESSION_READY")?;
    out.flush()?;
    for line in io::stdin().lock().lines() {
        let query = match serde_json::from_str(&line?)? {
            Request::Enqueue { path, max_attempts } => {
                let batch = serde_json::from_slice(&fs::read(path)?)?;
                let response = enqueue(&mut conn, batch, max_attempts).await?;
                send_line(&mut out, &response)?;
                continue;
            }
            Request::Database => include_str!("../database.sql"),
            Request::Sample => include_str!("../sample.sql"),
            Request::Sessions => include_str!("../sessions.sql"),
            Request::Counters => include_str!("../counters.sql"),
            Request::Jobs => include_str!("../jobs.sql"),
            Request::Writes => include_str!("../writes.sql"),
            Request::Stop => break,
        };
        let response = sqlx::query_scalar::<_, Value>(query)
            .fetch_one(&mut conn)
            .await?;
        send_line(&mut out, &response)?;
    }
    conn.close().await?;
    Ok(())
}

fn send_line(out: &mut impl Write, response: &Value) -> Result<(), BoxDynError> {
    serde_json::to_writer(&mut *out, response)?;
    writeln!(out)?;
    out.flush()?;
    Ok(())
}

async fn handle(
    job: Job,
    id: TaskId,
    attempt: Attempt,
    pool: Data<PgPool>,
) -> Result<(), BoxDynError> {
    let entered_us = i64::try_from(SystemTime::now().duration_since(UNIX_EPOCH)?.as_micros())?;
    let mut conn = pool.acquire().await?;
    let attempt = i32::try_from(attempt.current())?;
    let id = id.to_string();
    // Each statement runs in its own autocommit transaction. Hold the
    // checkout across the sleep, as the frozen Oban handler does.
    sqlx::query(
        "INSERT INTO public.mesh_jobs_parity_attempts
         (engine, namespace, job_key, effect_key, attempt, entered_at, worker_pid, job_id)
         VALUES ('apalis', 'parity', $1, $1, $2,
                 timestamptz 'epoch' + $3::bigint * interval '1 microsecond', $4, $5)",
    )
    .bind(&job.key)
    .bind(attempt)
    .bind(entered_us)
    .bind(i32::try_from(std::process::id())?)
    .bind(&id)
    .execute(&mut *conn)
    .await?;

    if !job.fail_first && job.hold_ms > 0 {
        tokio::time::sleep(Duration::from_millis(job.hold_ms)).await;
    }
    sqlx::query(
        "INSERT INTO public.mesh_jobs_parity_effects
         (engine, namespace, job_key, effect_key, effected_at, job_id)
         VALUES ('apalis', 'parity', $1, $1, clock_timestamp(), $2) ON CONFLICT DO NOTHING",
    )
    .bind(&job.key)
    .bind(&id)
    .execute(&mut *conn)
    .await?;
    drop(conn);

    if job.fail_first && attempt == 1 {
        if job.hold_ms > 0 {
            tokio::time::sleep(Duration::from_millis(job.hold_ms)).await;
        }
        return Err(FAIL_FIRST.into());
    }
    Ok(())
}

async fn run_worker(
    options: PgConnectOptions,
    name: String,
    slots: usize,
    poll: Duration,
) -> Result<(), BoxDynError> {
    let connections = u32::try_from(slots + 2)?;
    let pool = PgPoolOptions::new()
        .min_connections(connections)
        .max_connections(connections)
        .connect_with(options)
        .await?;
    let config = Config::default().queue(QUEUE).batch_size(slots);
    let strategy = Strategy::new()
        .stream(Pubsub::new(pool.clone(), QUEUE.into()))
        .interval(poll);
    let backend = PostgresStorage::new(&pool)
        .with_config(config)
        .poll_with_strategy(strategy);
    let worker = WorkerBuilder::new(name.as_str())
        .backend(backend)
        .concurrency(slots)
        .data(pool.clone())
        .on_event(|_, event| {
            if let Event::Error(error) = event {
                eprintln!("handler: {error}");
            }
        })
        .build(handle);

    let (stop, wait) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let result = io::stdin().read_line(&mut line);
        let _ = stop.send(result.map(|_| line));
    });
    worker
        .run_until(async move {
            let line = wait.await.map_err(io::Error::other)??;
            if !line.is_empty() && line.trim_end() != "stop" {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "expected stop on runner stdin",
                ));
            }
            Ok::<(), io::Error>(())
        })
        .await?;
    pool.close().await;
    Ok(())
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<(), BoxDynError> {
    let args: Vec<_> = env::args().skip(1).collect();
    if args.as_slice() == ["version"] {
        println!("durable-jobs-apalis-bench {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let options = PgConnectOptions::from_str(&env::var("APALIS_BENCH_DB_URL")?)?;
    match args.as_slice() {
        [action] if action == "migrate" => {
            let pool = PgPoolOptions::new()
                .max_connections(1)
                .connect_with(options.application_name("apalis:migrate"))
                .await?;
            PostgresStorage::setup(&pool).await?;
            pool.close().await;
            Ok(())
        }
        [action] if action == "session" => {
            run_session(options.application_name("apalis:producer_observer")).await
        }
        [action] if action == "snapshot" => {
            let mut conn =
                PgConnection::connect_with(&options.application_name("apalis:failure_snapshot"))
                    .await?;
            let jobs = sqlx::query_scalar::<_, Value>(include_str!("../jobs.sql"))
                .fetch_one(&mut conn)
                .await?;
            let writes = sqlx::query_scalar::<_, Value>(include_str!("../writes.sql"))
                .fetch_one(&mut conn)
                .await?;
            send_line(
                &mut io::stdout().lock(),
                &json!({"jobs": jobs, "writes": writes}),
            )?;
            conn.close().await?;
            Ok(())
        }
        [action, runner, slots, poll] if action == "worker" => {
            run_worker(
                options.application_name(&format!("apalis:{runner}")),
                runner.clone(),
                slots.parse()?,
                Duration::from_millis(poll.parse()?),
            )
            .await
        }
        _ => {
            Err("expected version, migrate, session, snapshot, or worker NAME SLOTS POLL_MS".into())
        }
    }
}
