# Operate durable Jobs

## Pick the scope of a queue change

1. Check `Jobs.status(runner)` in the process that owns the runner. The
   `runner_id` is stable for that runner in that process. A restarted process
   issues new local handles, which may reuse numbers.
2. Use `Jobs.local_control(runner, json { action: "pause", queue: "default" })`
   to pause just that runner's queue, or use `stop` to drain and remove that
   queue from local dispatch. Other queues keep running.
3. For a local stop, check the queue's `drained` flag and wait for `stopped`.
   The flag reports whether the runner still tracks local claims or attempts
   for the queue.
   At the end of `grace_ms`, the runner terminates handlers still running on
   that queue and settles their attempts as failures under the current lease.
   Pending claims that return after the stop are released if the runner can
   confirm that they never started. An uncertain claim is recovered by lease.
4. To change admission across the database, call `Jobs.control` with
   `pause` for a queue, `scale` for its concurrency, or `pause_namespace`
   for all queues in the namespace. Existing claims can still run, renew,
   and settle. `resume_namespace` leaves each queue's own state and limit in
   place. Read `namespace_status` on the writable primary after an uncertain
   response before deciding whether to retry the operation.

Admission uses both the local and database queue limits. A scale-down leaves
existing attempts running. The local queue state comes from `Jobs.start` on
each restart. Namespace pause, queue state, and the database queue limit
persist across process and node loss. For a rolling upgrade, run
`Jobs.migrate` from the new binary before using namespace controls. The
installed claim fence survives a replay of the older owner migration.
An older runner may report a claim error in `last_error` while eligible
work is paused. It rolls back the claim before spending an attempt and
retries on its next poll. The last error can still be visible after work
resumes.

## Investigate a job that did not start

1. Read `namespace_status` and `inspect_queue` on the primary. Check whether
   the queue is active, its concurrency is positive, and it has room for
   another executing row. On the intended runner, check local status and
   confirm that its configuration registers the job's worker version.
2. Use `search` for a page of job summaries. Use `get` with an operator
   connection if you need a particular job's payload or error history.
   Compare `scheduled_at_ms` with the primary's clock. Use `run_now` to make
   a scheduled or retryable job eligible for its next normal claim.
3. If the wakeup is missed, allow a fallback poll. Polling is the default.
   Busy database connections or a locked candidate can delay pickup past the
   configured interval. Check the runner's `last_error`, PostgreSQL locks,
   pool capacity, and backing database connectivity before restarting workers.

## Recover after a database outage or failover

1. Fence the old primary and verify that the promoted primary is writable.
   Ensure that it contains the commits required by the application's recovery
   policy. Check its clock before using lease or schedule timestamps.
2. Restore connections and use `get` on the promoted primary. An
   `executing` job can still have a valid lease. If it expired, let recovery
   settle it under the stored attempt budget. A previously committed cancel
   request takes precedence over recovery.
3. If an enqueue response was lost, replay the same spec and `request_key`.
   Prune and delete leave the receipt in place, so that replay can still
   resolve to the original ID even after the job row is gone.
4. When an earlier handler could have committed an external effect, check the
   downstream system by `effect_key`. Automatic retry and `run_now` keep it.
   Operator `retry` on a terminal job starts a new run with a new key.

## Try enqueue, retry, drain, and lease recovery

Use an empty, disposable PostgreSQL database on macOS or Linux. Export
`DATABASE_URL` to its connection URL. You need `meshc`, `psql`, and `jq`.
Run the commands in Bash.

```bash
set -euo pipefail
meshc init jobs_demo
cd jobs_demo
```

Replace `main.mpl` with the following application. It runs one attempt per
drain. The retry job asks to retry on its first attempt. The recovery job
waits on its first attempt when you set `HOLD_JOB=1`.

```mesh
fn handle_job(job :: String) -> String do
  let kind = Json.get_nested(job, "args", "kind")
  let first = Json.get(job, "attempt") == "1"
  if kind == "retry" && first do
    return Json.encode(json { action: "retry_after", delay_ms: 60000, reason: "try again" })
  end
  if kind == "recovery" && first && Env.get("HOLD_JOB", "") == "1" do
    IO.eprintln("handler started")
    Timer.sleep(30000)
  end
  Json.encode(json { action: "complete" })
end

fn run(pool :: PoolHandle, action :: String, value :: String) -> String!String do
  case action do
    "migrate" ->
      let _ = Jobs.migrate(pool)?
      Ok("migrated")
    "enqueue" -> Jobs.enqueue(pool, "jobs-demo",
      json { worker: "demo", args: json { kind: value }, request_key: value,
        max_attempts: 2, backoff_base_ms: 0 })
    "drain" -> Jobs.drain(pool,
      """{"namespace":"jobs-demo","queues":{"default":1},"workers":[{"worker":"demo","version":1}],"lease_ms":1500}""",
      handle_job, json { max_jobs: 1, recursive: false, on_error: "stop" })
    "get" | "run_now" | "retry" -> Jobs.control(pool, "jobs-demo",
      json { action: action, id: value })
    _ -> Err("expected migrate, enqueue, drain, get, run_now, or retry")
  end
end

fn main() do
  let args = Env.args()
  let action = List.get(args, 1)
  let value = if List.length(args) > 2 do List.get(args, 2) else "" end
  case Pool.open(Env.get("DATABASE_URL", ""), 1, 2, 5000) do
    Ok(pool) ->
      let result = run(pool, action, value)
      Pool.close(pool)
      case result do
        Ok(output) -> println(output)
        Err(error) -> panic(error)
      end
    Err(error) -> panic(error)
  end
end
```

Build and migrate, then enqueue a job. The same request key and spec return
the original receipt. The first drain records one retryable outcome. Use
`run_now` to skip the retry delay and drain again. The job completes on
attempt 2 with the same effect key.

```bash
meshc build . --output jobs_demo
./jobs_demo migrate
receipt=$(./jobs_demo enqueue retry)
id=$(jq -er '.id' <<<"$receipt")
key=$(jq -er '.effect_key' <<<"$receipt")
test "$receipt" = "$(./jobs_demo enqueue retry)"
printf '%s\n' "$receipt"
printf 'Request-key replay returned the original receipt.\n'
printf '%s\n' "$receipt" | jq -e '.inserted and .state == "available"'
./jobs_demo drain | jq -ce 'select(.claimed == 1 and .outcomes.retryable == 1
  and .stop_reason == "job_error")'
./jobs_demo get "$id" | jq -ce --arg key "$key" \
  'select(.state == "retryable" and .attempt == 1 and .effect_key == $key
    and .scheduled_at_ms == .errors[0].at_ms + 60000)'
./jobs_demo run_now "$id" | jq -ce --arg key "$key" \
  'select(.state == "available" and .attempt == 1 and .effect_key == $key)'
./jobs_demo drain | jq -ce 'select(.claimed == 1 and .outcomes.completed == 1)'
./jobs_demo get "$id" | jq -ce --arg key "$key" \
  'select(.state == "completed" and .attempt == 2 and .effect_key == $key)'
```

To simulate a crash, start a new drain with `HOLD_JOB=1`. Wait for its handler
to start, then kill the drain process. Read the unfinished claim from a
separate connection and wait for its lease to expire on PostgreSQL's clock.
The replacement drain recovers the claim and completes attempt 2.

```bash
receipt=$(./jobs_demo enqueue recovery)
id=$(jq -er '.id' <<<"$receipt")
key=$(jq -er '.effect_key' <<<"$receipt")
HOLD_JOB=1 ./jobs_demo drain > interrupted.out 2> interrupted.err &
drain_pid=$!
for ((i = 0; i < 100; i++)); do
  if grep -q '^handler started$' interrupted.err; then break; fi
  sleep 0.1
done
kill -KILL "$drain_pid"
wait "$drain_pid" || test "$?" -eq 137
grep '^handler started$' interrupted.err
executing=$(./jobs_demo get "$id")
printf '%s\n' "$executing"
lease=$(jq -er 'select(.state == "executing" and .attempt == 1) | .lease_expires_at_ms' <<<"$executing")
psql -XqAt -v ON_ERROR_STOP=1 -v lease="$lease" "$DATABASE_URL" <<'SQL'
SELECT pg_sleep(greatest(0, :lease - mesh_jobs.now_ms() + 1) / 1000.0);
SQL
./jobs_demo drain | jq -ce 'select(.recovered == 1 and .claimed == 1 and .outcomes.completed == 1)'
./jobs_demo get "$id" | jq -ce --arg key "$key" \
  'select(.state == "completed" and .attempt == 2 and .effect_key == $key
    and .errors[0].reason == "lease expired")'
```

Use the operator `retry` on that completed job to start a new run. It keeps
the job ID, resets the attempt count, and assigns a new effect key. Drain the
new run to completion.

```bash
restarted=$(./jobs_demo retry "$id")
printf '%s\n' "$restarted" | jq -ce --arg id "$id" --arg key "$key" \
  'select(.id == $id and .state == "available" and .attempt == 0 and .effect_key != $key)'
new_key=$(jq -er '.effect_key' <<<"$restarted")
./jobs_demo drain | jq -ce 'select(.claimed == 1 and .outcomes.completed == 1)'
./jobs_demo get "$id" | jq -ce --arg key "$new_key" \
  'select(.state == "completed" and .attempt == 1 and .effect_key == $key)'
```

## Set maintenance limits

Set these keys in the `Jobs.start` runner config. Each interval starts at the
end of the previous pass. The first eligible pass starts when the runner
starts.

| Key | Default | Range or condition |
| --- | --- | --- |
| `recovery_interval_ms` | `poll_ms` clamped to 100–1,000 ms, or 10–1,000 ms in drain mode | 1–60,000 ms |
| `recovery_page_size` | 64 | 1–1,000 expired leases and up to the same number of due rows per pass |
| `cron_interval_ms` | 1,000 ms | 1–30,000 ms. Set 0 to disable cron on this runner |
| `cron_page_size` | 64 | 1–128 occurrences per tick |
| `cron_schedule_page_size` | 128 | 1–128 candidate schedules per tick |
| `prune_interval_ms` | 60,000 ms | 1–86,400,000 ms |
| `prune_page_size` | 1,000 | 1–1,000 terminal rows per pass |
| `prune_max_age_ms` | Pruning disabled | 0–365 days in milliseconds to enable pruning |

Keep a cron runner available for namespaces with schedules. Cron commits each
page's enqueues and cursor changes in the same transaction. If the scheduling
connection dies before commit, the next owner can process the same page. If it
dies after commit, the next owner continues from the committed cursor. After a
60-second heartbeat gap, `@reboot` uses a new namespace activation. Calendar
schedules apply their stored misfire policy and lookback window.

Use `delete_all` for filtered terminal-row cleanup, or `prune` for age-based
cleanup. Both operations skip locked rows and cap the number of rows changed
per call. Run another page if more rows remain. Request receipts have separate
retention and are preserved by both operations.

## Reindex one Jobs index

Use a dedicated administration connection to the writable primary. Configure
the connection through libpq, for example via `PGSERVICE`. Connect as the
Jobs table owner or, on PostgreSQL 17 or later, a role with `USAGE` on
`mesh_jobs` and `MAINTAIN` on the indexed table. The worker and the ordinary
operator roles need no reindex authority.

```sh
PGSERVICE=jobs_index_admin bash scripts/jobs-reindex.sh mesh_jobs_ready 30000
```

The script accepts one allowlisted Jobs or cron index per invocation. The
timeout is 30 seconds by default, with a maximum of 300 seconds, and the
lock timeout is two seconds. A session advisory lock covers the concurrent
swap. A second invocation against the same database fails while the first
owns that lock. Watch `pg_stat_progress_create_index` and settlement progress
from a separate connection. If a concurrent reindex fails, inspect
`pg_index.indisvalid` and the indexes on the table before retrying. PostgreSQL
can retain an invalid index with a `_ccnew` or `_ccold` suffix after failure.

## Assign database roles

1. Set the migrator as the Jobs object owner. To remove default execution
   access for its future functions, run
   `ALTER DEFAULT PRIVILEGES REVOKE EXECUTE ON FUNCTIONS FROM PUBLIC` as
   the migrator before running `Jobs.migrate`.
2. Apply the checked-in grants as the object owner after migrating. Grant the
   producer its own application-table privileges separately. The worker
   grants include `claim_drain_v1` for current single-claim runners and
   bounded `Jobs.drain`. [Jobs grants](https://github.com/snowdamiz/mesh-lang/blob/main/scripts/fixtures/jobs-role-grants.sql)

   ```sh
   psql -X -v ON_ERROR_STOP=1 -v producer=app_producer -v worker=app_worker \
     -v operator=app_operator -v inspector=app_inspector \
     -f scripts/fixtures/jobs-role-grants.sql
   ```
3. Use `cron_interval_ms: 0` for a worker that only claims and settles.
   For a cron worker, grant producer authority for transactional enqueue,
   `SELECT, INSERT, UPDATE` on `mesh_jobs.cron_activation`, and
   `SELECT` and `UPDATE (cursor_minute, reboot_epoch)` on
   `mesh_jobs.cron_schedules`. Grant `DELETE` on `mesh_jobs.jobs` to a
   worker that prunes.
4. Give inspectors access to `search`, `inspect_queue`, and
   `namespace_status`. The summary views exclude arguments, metadata, tags,
   errors, and lease tokens. Producer and worker grants include the underlying
   SQL privileges used by invoker functions and row locks. Use these roles in
   a trusted application database.

Run the disposable fresh and upgrade role proof with a local checkout that
contains the pinned main revision `7c162056`.

```sh
env -u MESH_TEST_DATABASE_URL -u MESH_JOBS_TEST_PG_URL \
  bash scripts/verify-postgres-jobs.sh --j6-roles
```
