---
title: Concurrency
description: Actors, typed messages, services, supervision, jobs, timers, channels, and process lifecycle in Mesh
---

# Concurrency

Mesh uses the actor model for concurrency, inspired by Erlang/Elixir. Actors are lightweight processes that communicate by message passing. Jobs, scheduler-aware timers, and bounded channels cover common work that does not need a long-lived actor.

> **Autonomous clusters:** This guide teaches the concurrency model. Continue with [Autonomous Clusters](/docs/autonomous-clusters/) for runtime-owned elasticity and [Distributed Proof](/docs/distributed-proof/) for release verification.

## The Actor Model

In Mesh, actors are independent units of computation that do not share memory. Each actor:

- Has its own **mailbox** for receiving messages
- Communicates exclusively via **message passing**
- Runs **concurrently** with other actors
- Is **isolated by default** -- an unlinked actor crashing does not bring down other actors

This model avoids shared-memory data races and shared-state corruption. Application-level waiting cycles are still possible—for example, two services that synchronously call each other—so keep synchronous dependencies acyclic.

## Spawning Actors

Define an actor with the `actor` keyword and start it with `spawn`:

```mesh
actor greeter() do
  receive do
    _ -> println("actor received")
  end
end

fn main() do
  let pid = spawn(greeter)
  send(pid, 1)
  println("main done")
end
```

The `spawn` function returns a **PID** (process identifier) that you use to communicate with the actor. Actors run concurrently with the function that spawned them. Actor parameters become arguments to `spawn`:

```mesh
actor counter(total :: Int) do
  receive do
    amount -> counter(total + amount)
  end
end

fn main() do
  let pid = spawn(counter, 0)
  send(pid, 5)
end
```

Mesh infers `Pid<T>` from the actor's received message type. Sending a value of the wrong type is a compile-time error. Inside an actor, `self()` returns its own PID.

PIDs compare with `==` and `!=`, and show as `<0.12>` in an interpolation (`<node.id.creation>` for a process on another node). No process has PID `0`: it is what a lookup that finds nothing and a remote spawn that fails return, shown as `<0.0>`, and a send to it goes nowhere.

### Keep communicating actors on one worker

Ordinary `spawn` puts normal-priority work in a shared queue. A worker
hosting many actors briefly gives a less occupied worker a chance to start
the next one. Once a coroutine starts, it stays on that worker.

Use `spawn grouped` for actors that exchange frequent messages on one node.
A grouped spawn from `main` or an ordinary actor starts a group on one active
scheduler worker. The scheduler distributes new groups across active workers.
A grouped spawn from inside a group joins the caller's worker. Each actor
keeps its own PID, mailbox, and heap.

```mesh
actor increment(requester :: Pid<Int>) do
  receive do
    value -> send(requester, value + 1)
  end
end

actor pair(requester :: Pid<Int>) do
  let counter = spawn grouped(increment, self())
  send(counter, 41)
  receive do
    result -> send(requester, result)
  end
end

actor coordinator() do
  let _ = spawn grouped(pair, self())
  receive do
    result -> println("#{result}")
  end
end

fn main() do
  spawn(coordinator)
end
```

Grouped actors use operations the compiler can verify will yield or return
without blocking the worker. The compiler rejects native calls, blocking I/O,
and calls through unknown function values in grouped code. Use ordinary
`spawn` for actors that need those operations. Grouped and ordinary actors
share scheduler workers, so an ordinary actor that blocks its thread can
delay a group on that worker.

At optimization level 2 or higher, the compiler can run a grouped actor's
untimed scalar receive loop directly on its worker's stack. Between messages
the worker saves the loop arguments. The actor keeps its PID and mailbox.
Handlers outside the supported subset use the usual coroutine path. To compare
the two implementations, set `MESH_GROUPED_STEP=0` when building the program.

A send from a grouped actor to another node returns `7` without delivering
the message. Existing groups keep their worker when the scheduler reduces its
active worker count. New groups use the remaining active workers.

## Message Passing

Actors communicate by sending and receiving messages. Use `send` to deliver a message to an actor's mailbox, and `receive` to wait for the next message:

```mesh
actor worker() do
  receive do
    _ -> println("worker done")
  end
end

fn main() do
  let w1 = spawn(worker)
  let w2 = spawn(worker)
  let w3 = spawn(worker)
  send(w1, 1)
  send(w2, 2)
  send(w3, 3)
  println("main sent all")
end
```

Key points about message passing:

- Messages are processed **one at a time** from the actor's mailbox
- `send` returns `0` when enqueued or written, `1` when the target is missing, `2` when the mailbox is full, `3` when the message exceeds the mailbox byte limit, `4` when a remote node is unavailable, `5` when a remote write fails, and `6` when a message for another node holds a function or runtime handle, which cannot leave its node
- `receive` blocks until the next message arrives
- `receive` arms match the message like `case` arms, with patterns and `when` guards, and together must cover the actor's message type
- You can spawn multiple actors and send messages to each independently

`send` can take either argument from a pipe: `pid |> send(message)` passes the PID, and the slot pipe `message |2> send(pid)` passes the message.

Receive arms can match on the message's shape. An arm that calls the actor again continues with new arguments; an arm that does not ends the actor:

```mesh
type Command do
  Add(amount :: Int)
  Report
  Stop
end

actor tally(total :: Int) do
  receive do
    Add(amount) when amount > 0 -> tally(total + amount)
    Add(_) -> tally(total)
    Report ->
      println("total: #{total}")
      tally(total)
    Stop -> println("stopped at #{total}")
  end
end

fn main() do
  let pid = spawn(tally, 0)
  send(pid, Add(5))
  send(pid, Add(-2))
  send(pid, Report)
  send(pid, Stop)
  Timer.sleep(50)
end
```

A receive can provide a timeout in milliseconds. The receive expression returns either the message arm's value or the timeout arm's value:

```mesh
actor worker() do
  let result = receive do
    value -> value
    after 1_000 -> 0
  end

  println("#{result}")
end
```

Actors can also perform computation before responding. Here is an actor that runs a function when it receives a message:

```mesh
fn count_loop(n :: Int, target :: Int) -> Int do
  if n >= target do
    n
  else
    count_loop(n + 1, target)
  end
end

actor worker() do
  receive do
    _ -> println("${count_loop(0, 100)}")
  end
end

fn main() do
  let pid = spawn(worker)
  send(pid, 1)
end
```

## Linking and Monitoring

Actors can be linked so that failures propagate between them. If one linked actor crashes, the other is notified:

```mesh
actor linked_worker() do
  receive do
    _ -> println("linked worker done")
  end
end

actor linker() do
  let worker = spawn(linked_worker)
  link(worker)
  receive do
    message -> send(worker, message)
  end
end

fn main() do
  let linker_pid = spawn(linker)
  send(linker_pid, 1)
end
```

- **`link(pid)`** -- bidirectionally links two actors. If one ends abnormally (a panic or runtime error), the other ends too; a normal exit only removes the link. Exit signals never arrive as messages in an actor's own `receive`; only supervisors observe them.
- **`Process.monitor(pid, message)`** -- a one-way monitor: when `pid` ends, for whatever reason, the calling actor is sent `message`, one of its own messages. It is sent at once when there is no such process. Returns the monitor's reference.
- **`Process.demonitor(reference)`** -- removes a monitor, so its message is never sent; returns `0` on success or `1` when the reference is unknown.

```mesh
actor worker() do
  receive do
    n -> println("working on #{n}")
  end
end

actor watcher() do
  let pid :: Pid<Int> = spawn(worker)
  Process.monitor(pid, "worker ended")
  send(pid, 1)
  receive do
    text -> println(text)
  end
end
```

Linking is the foundation for building fault-tolerant systems: supervisors use links to detect and restart failed actors.

An actor can also declare a termination callback for cleanup:

```mesh
actor worker() do
  receive do
    _ -> println("work complete")
  end

  terminate do
    println("cleaning up")
  end
end
```

## Supervision

Supervisors are special actors that monitor and restart child actors when they fail. Define a supervisor with the `supervisor` keyword:

```mesh
actor worker() do
  receive do
    _ -> println("worker got message")
  end
end

supervisor WorkerSup do
  strategy: one_for_one
  max_restarts: 3
  max_seconds: 5

  child w1 do
    start: fn -> spawn(worker) end
    restart: permanent
    shutdown: 5000
  end
end

fn main() do
  let sup = spawn(WorkerSup)
  println("supervisor started")
end
```

### Supervision Strategies

| Strategy | Behavior |
|----------|----------|
| `one_for_one` | Only the failed child is restarted |
| `one_for_all` | All children are restarted when one fails |
| `rest_for_one` | The failed child and all children started after it are restarted |
| `simple_one_for_one` | Accepted template strategy; dynamic child start/terminate operations are not exposed by the current Mesh source API |

### Child Specifications

Each `child` block configures how the supervisor manages that actor:

| Option | Purpose |
|--------|---------|
| `start` | Function that spawns the child actor |
| `restart` | Restart policy: `permanent` (always), `transient` (only on abnormal exit), `temporary` (never) |
| `shutdown` | Positive milliseconds to wait for graceful shutdown, or `brutal_kill` |

### Restart Limits

- **`max_restarts`** -- maximum number of restarts allowed within the time window
- **`max_seconds`** -- the time window in seconds

If a child exceeds the restart limit, the supervisor itself shuts down, escalating the failure to its parent supervisor.

When omitted, the strategy defaults to `one_for_one`, `max_restarts` to `3`, and `max_seconds` to `5`.

### Runtime Errors and Panics

A runtime error, such as `List.get` past the end of a list or `head` of an empty one, and a call to [`panic(message)`](/docs/stdlib/) end only the actor they happen in. The runtime reports it on standard error as an `R0001` diagnostic with the message, other actors keep running, and a supervised actor is restarted according to its `restart` policy:

```mesh
actor worker() do
  receive do
    0 -> panic("cannot handle 0")
    n -> println("handled #{n}")
  end
  worker()
end

fn main() do
  let pid = spawn(worker)
  send(pid, 1)
  send(pid, 0)
  Timer.sleep(100)
  println("main still running")
end
```

This prints `handled 1`, then reports `[R0001] Error: cannot handle 0` on standard error, then prints `main still running`. The same failure in `main` itself ends the program with exit status 101.

## Services (GenServer)

Services are stateful actors that follow the GenServer pattern. They provide a structured way to manage state with synchronous calls and asynchronous casts:

```mesh
service Counter do
  fn init(start_val :: Int) -> Int do
    start_val
  end

  call GetCount() :: Int do |count|
    (count, count)
  end

  call Increment(amount :: Int) :: Int do |count|
    (count + amount, count + amount)
  end

  cast Reset() do |_count|
    0
  end
end

fn main() do
  let pid = Counter.start(10)
  let c1 = Counter.get_count(pid)
  println("${c1}")
  let c2 = Counter.increment(pid, 5)
  println("${c2}")
  Counter.reset(pid)
  let c3 = Counter.get_count(pid)
  println("${c3}")
end
```

### Service Anatomy

- **`init`** -- called when the service starts, returns the initial state
- **`call`** -- synchronous request/response. The handler receives the current state and returns a tuple `(new_state, reply)`
- **`cast`** -- asynchronous fire-and-forget. The handler receives the current state and returns the new state

### Starting and Calling Services

The compiler auto-generates snake_case methods from your PascalCase definitions:

```mesh
service Store do
  fn init(start_val :: Int) -> Int do
    start_val
  end

  call Get() :: Int do |state|
    (state, state)
  end

  call Set(value :: Int) :: Int do |_state|
    (value, value)
  end

  cast Clear() do |_state|
    0
  end
end

fn main() do
  let pid = Store.start(100)
  let v1 = Store.get(pid)
  println("${v1}")
  let v2 = Store.set(pid, 200)
  println("${v2}")
  Store.clear(pid)
  let v3 = Store.get(pid)
  println("${v3}")
end
```

| Definition | Generated method |
|------------|-----------------|
| `Store.start(100)` | Starts the service with initial value |
| `Store.get(pid)` | Calls the `Get` handler |
| `Store.set(pid, 200)` | Calls the `Set` handler |
| `Store.clear(pid)` | Casts the `Clear` handler |

Services with no init arguments use `start()` with no parameters:

```mesh
service Accumulator do
  fn init() -> Int do
    0
  end

  call Add(n :: Int) :: Int do |state|
    (state + n, state + n)
  end
end

fn main() do
  let pid = Accumulator.start()
  Accumulator.add(pid, 1)
  Accumulator.add(pid, 2)
  let result = Accumulator.add(pid, 3)
  println("${result}")
end
```

Arguments and replies are copied between the caller and the service, as actor messages are, so a reply stays valid after the service's state changes or the service stops.

Service calls are synchronous and wait for a reply. Casts are asynchronous. Prefer a cast, a direct actor message, or a job when the caller must remain independent of the service's response time.

A call waits for its own reply only: messages already in the caller's mailbox stay there for its next `receive`. A call to a service that has stopped, or that stops before it replies (a handler that panics, say), panics in the caller, as does a call to a service whose mailbox is full.

## Jobs

`Job` runs short-lived work on actors in the current runtime and returns failures through `Result`.

A runtime restart loses pending jobs and uncollected results. The caller decides whether to start another job after a failure. Use `Jobs` for work that must survive a restart.

```mesh
fn main() do
  let job = Job.async(fn() -> 21 * 2 end)
  case Job.await_timeout(job, 1_000) do
    Ok(value) -> println("#{value}")
    Err(error) -> println("job failed: #{error}")
  end
end
```

| Function | Returns | Description |
|----------|---------|-------------|
| `Job.async(fn)` | `Pid<T>` | Run a zero-argument function in an actor of its own |
| `Job.await(job)` | `Result<T, String>` | Wait without a timeout |
| `Job.await_timeout(job, timeout_ms)` | `Result<T, String>` | Wait up to the given number of milliseconds |
| `Job.map(values, fn)` | `List<Result<U, String>>` | Run one job per list element and return results in input order |

Await a job from the actor that started it. Job replies use separate storage from application messages, so a full mailbox cannot drop them. Each caller can buffer up to 1,024 uncollected `Job.async` replies and 8 MiB of copied reply data. Mesh reserves the reply for any job the caller is actively awaiting. If a completed reply exceeds the caller's buffer limits and the caller is not awaiting it, a later await returns `Err("job result unavailable")`. The job may already have changed external state when you receive that error. Each reply can be read once.

A job panic returns `Err` to its caller. If the caller fails, Mesh stops its linked jobs. A timeout leaves the job running, and a sleeping job can delay process exit after `main` returns. `Job.map` reserves the replies for its own jobs and returns each success or failure in input order.

## Durable jobs

`Jobs` stores work in PostgreSQL. Use the application's database to commit an enqueue together with a business change. Use the same namespace on every process that serves the same queues. A namespace groups work in the database. Processes with write access to the same job tables share authority. Use separate databases for applications that do not trust each other.

The PostgreSQL proof command requires PostgreSQL 14 or later. The compatibility workflow targets 17.11, 18.7, and 19.0 with verified TLS and SCRAM on Linux x64 and macOS arm64. Windows is outside the compiled Jobs proof matrix. The [comparison guide](https://github.com/snowdamiz/mesh-lang/blob/main/docs/durable-jobs-parity-evidence.md) describes the workload and recorded results.

Run `Jobs.migrate(pool)` before starting workers on a fresh database or after upgrading Mesh. It creates the job tables and updates the queue's SQL functions. A job stores a worker name, payload version, arguments, queue, and scheduling and retry options. Keep the worker versions needed by outstanding jobs available during a rolling deploy. Workers claim jobs that match their registered names and versions.

If your database revokes default function access, grant the worker role `EXECUTE` on `mesh_jobs.try_renew(text, uuid, uuid, bigint)` and `mesh_jobs.settle_state(text, uuid, uuid, jsonb, boolean)` before starting upgraded workers. Current single-claim workers also need `EXECUTE` on `mesh_jobs.claim_drain_v1(text, text, jsonb, bigint, bigint, jsonb)`. Existing workers retain their legacy function grants during the upgrade.

| Function | Result |
| --- | --- |
| `Jobs.migrate(pool)` | Initialize or migrate the jobs schema |
| `Jobs.enqueue(pool, namespace, spec)` | Commit one job or a JSON array of jobs and return the receipts as JSON |
| `Jobs.enqueue_in(conn, namespace, spec)` | Enqueue on a borrowed `PgConn` in its active transaction |
| `Jobs.start(pool, config, handler)` | Start a runner and return its handle |
| `Jobs.drain(pool, config, handler, options)` | Run a bounded drain and return its JSON report |
| `Jobs.status(runner)` | Read the local runner state as JSON |
| `Jobs.read_events(runner, limit)` | Read and remove up to `limit` events from an enabled runner buffer |
| `Jobs.local_control(runner, request)` | Change one queue on the local runner and return its status |
| `Jobs.stop(runner, grace_ms)` | Request shutdown and report the runner status when the runner stops or the grace period ends |
| `Jobs.join(runner)` | Wait for the runner to stop and return its final status |
| `Jobs.control(pool, namespace, request)` | Inspect jobs and control queues with a JSON operation |

Pass a JSON object to enqueue one job. The default queue is `default` and the default payload version is `1`. A `request_key` lets an application retry an enqueue after losing its response. Reuse the same key and spec until the application knows the commit outcome.

```mesh
fn create_order(conn :: borrow PgConn) -> Int!String do
  Pg.execute(conn, "INSERT INTO orders (id) VALUES ($1::bigint)", ["42"])?
  Jobs.enqueue_in(conn,
    "shop",
    json { worker: "send-receipt", args: json { order_id: 42 }, request_key: "order-42-receipt" })?
  Ok(42)
end

fn submit_order(pool :: PoolHandle) -> Int!String do
  Repo.transaction(pool, create_order)
end
```

The enqueue receipt inside `Repo.transaction` is provisional. Returning `Err` rolls back both writes. `Jobs.enqueue(pool, ...)` uses a separate connection and commits independently.

Use `delay_ms` or `scheduled_at_ms` to defer a job. Absolute times are Unix epoch milliseconds. Priority ranges from `0` (highest) to `9` (lowest). Each job defaults to 20 attempts and a 60-second handler timeout, configurable with `max_attempts` and `timeout_ms`. `backoff_base_ms` and `backoff_max_ms` configure exponential retry delays.

Use `unique` to reuse a matching retained job. A policy can specify `fields` (such as `worker`, `queue`, `args`, or `metadata`), JSON `keys`, `states`, a `period_ms`, and a `timestamp` of `inserted_at` or `scheduled_at`. `replace` can update selected fields of a matching job. Enqueue returns `id`, `effect_key`, `state`, `inserted`, and `conflict`. Request keys preserve the original receipt independently of job retention. Use them for operations whose clients may have to retry an uncertain commit.

### Store large job arguments

PostgreSQL can spend CPU trying to compress a large `args` value even when it ultimately stores the value uncompressed. To try LZ4 for a producer transaction, first check whether the server lists `lz4` in the result of this query.

```sql
SELECT enumvals FROM pg_settings WHERE name = 'default_toast_compression';
```

Inside the transaction, before calling `Jobs.enqueue_in`, run the following on the same connection.

```mesh
let _ = Pg.execute(conn, "SET LOCAL default_toast_compression = 'lz4'", [])?
```

PostgreSQL uses this setting for new values in columns that use the default compression method. Commit or rollback restores the connection's previous setting. [PostgreSQL compression settings](https://www.postgresql.org/docs/17/runtime-config-client.html#GUC-DEFAULT-TOAST-COMPRESSION)

For a deployment that consistently enqueues large, hard-to-compress arguments, the table owner can disable compression for new argument values after `Jobs.migrate`.

```sql
ALTER TABLE mesh_jobs.jobs ALTER COLUMN args SET STORAGE EXTERNAL;
```

This policy applies to every queue sharing that table. Compare enqueue latency and `pg_column_size(args)` on representative payloads. Compressible data can take substantially more disk space under `EXTERNAL`. To restore the original policy for future writes, use `ALTER TABLE mesh_jobs.jobs ALTER COLUMN args SET STORAGE EXTENDED`. PostgreSQL keeps the existing physical representation of stored rows until it rewrites their values. [PostgreSQL column storage](https://www.postgresql.org/docs/17/sql-altertable.html)

### Run workers and handle retries

`Jobs.start` takes a named `fn(String) -> String` handler in a compiled Mesh executable. The default `execution: "fresh_process"` starts each attempt in a fresh process. The handler receives a JSON job with `id`, `effect_key`, `worker`, `version`, `queue`, `args`, `metadata`, `attempt`, and `max_attempts`. In fresh-process mode, open connections and initialize any other dependencies inside the handler. The process inherits the application's environment and has its own actor registry and connection pools. Handler output goes to the runner's standard error. The runner config lists the queues and their concurrency and the worker versions the handler accepts.

```mesh
fn handle_receipt(job :: String) -> String do
  let effect_key = Json.get(job, "effect_key")
  println("processing receipt with key #{effect_key}")
  Json.encode(json { action: "complete" })
end

fn start_receipts(pool :: PoolHandle) -> Int!String do
  Jobs.control(pool, "shop", json { action: "configure", queue: "default", concurrency: 4 })?
  let workers = Json.parse("""[{"worker":"send-receipt","version":1}]""")?
  Jobs.start(pool,
    json {
      namespace: "shop",
      queues: json { default: 4 },
      workers: workers,
      lease_ms: 30000,
      poll_ms: 250,
      shutdown_grace_ms: 15000
    },
    handle_receipt)
end
```

| Handler action | Effect |
| --- | --- |
| `complete` | Record success |
| `fail` with `reason` | Retry with the job's backoff, or discard after the last attempt |
| `retry_after` with `reason` and `delay_ms` | Retry after an explicit delay |
| `snooze` with `delay_ms` | Reschedule and preserve the attempt budget |
| `cancel` with `reason` | End the job as cancelled |
| `discard` with `reason` | End the job and leave the failure available for inspection |

A claim has a lease. A runner renews the lease while its handler runs and records an outcome only for the current unexpired claim. A runner recovers expired attempts until the job spends its attempt budget, then keeps it as `discarded`. Exceptions and timeouts count as failed attempts. Automatic retries keep the same `effect_key`. An operator retry of a terminal job starts a new run with a new effect key.

Set `claim_batch: true` in the runner config to enable opt-in batch dispatch. A batch claims at most 16 jobs before the runner rotates to its next queue. It honors the runner's available slots and the database-wide queue limit. Within a queue, continuous higher-priority arrivals can delay lower-priority jobs.

When an empty batch still sees eligible work and available capacity, the runner retries with a short delay that grows up to `poll_ms`. `Jobs.status` reports these hints in `batch_contention_hints`. PostgreSQL checks admission on every retry.

Batch dispatch uses two dedicated PostgreSQL sessions per runner, one for claims and one for renewal and settlement. Include both in the runner's connection budget. The database limits each job context to 16 MiB and each batch reply to 32 MiB. The runner reports an oversized job in `last_error` and continues claiming later jobs. A batch drain reports oversized jobs and can finish after processing its remaining eligible work. For restricted worker roles, grant `EXECUTE` on `mesh_jobs.claim_batch_v1(text, text, jsonb, bigint, integer, bigint)` and `SELECT`, `INSERT`, and `UPDATE` on `mesh_jobs.batch_context_blocks`. Existing runners can continue with single claims during rollout.

#### Wake runners after commits

Set `listen: true` in a long-running runner's config to subscribe to PostgreSQL wake hints. Run `Jobs.migrate` before enabling publishing. Enable publishing on the sessions that enqueue jobs or resume queues, for example with `ALTER ROLE app_writer IN DATABASE app_db SET mesh_jobs.publish_wakeups = 'on'` for new sessions or `SET LOCAL mesh_jobs.publish_wakeups = 'on'` inside a transaction using `Jobs.enqueue_in`. Publishing and listening each default to off. A runner can listen with publishing disabled.

The listener holds one additional PostgreSQL session per runner, outside the application's pool and in addition to the batch claim session when batching is enabled. Use a direct connection or a session pooler. Set `listen: false` for transaction poolers and to run in poll-only mode. Drain runners never subscribe.

Every successful subscription triggers a scan after `LISTEN` commits, including after reconnect. Wake hints contain fixed-size namespace and queue hashes. PostgreSQL notifications are visible to database users. The runner reads PostgreSQL rows to decide what to claim. It uses the database's clock for known scheduled and retry deadlines and continues fallback polling during listener outages or when hints are missed. With `listen: true`, fallback delays are `poll_ms` plus up to 20% jitter. The default poll-only runner keeps its existing polling cadence. The `wakeups` status reports subscription state and counts claim scans prompted by subscriptions, notifications, due timers, and fallback polls.

Publishing uses PostgreSQL's transactional notification queue. A rollback discards its hints. PostgreSQL can reject the whole transaction at commit if the notification queue is full. Monitor it with `pg_notification_queue_usage()`. A transaction that publishes a hint cannot be prepared for two-phase commit. [PostgreSQL NOTIFY documentation](https://www.postgresql.org/docs/current/sql-notify.html)

If a transaction locks a job row, the runner retries that job's renewal or acknowledgement while it continues processing other active jobs. A claim can expire while its row is locked. Recovery can act once the transaction releases the lock.

Jobs retry unacknowledged claims while attempts remain. A claim can use an attempt before the handler starts, for example if PostgreSQL commits the claim and the runner loses the reply. If a handler sends a receipt and dies before acknowledging it, another attempt can send it again. Pass `effect_key` to a downstream idempotency mechanism or record it in the same transaction as the external effect when both use the same database.

The runtime terminates a fresh attempt process at its handler timeout or when it loses its lease or runner. This includes a handler blocked in a native call while the host can still run its watchdog. A suspended host can miss lease renewals and allow another runner to claim the job before the original host resumes. A service may finish a request it accepted before the attempt was terminated. Use the same `effect_key` to deduplicate these requests across retries.

Deploy compiled executables at versioned paths so old runners can keep starting handlers while they drain. Stop runners from before process isolation to apply hard timeouts to every new attempt. Existing scheduled and retryable jobs keep their stored arguments and deadlines.

On `SIGTERM` or `Jobs.stop`, the runner stops claiming new work and tries to finish in-flight attempts within the grace period. When the grace period ends, it terminates running fresh processes and requests cooperative cancellation of running shared-process actors. `Jobs.stop` can return `stopping` if cleanup is still in progress. The `drained` field reports whether the runner still tracks local claims or attempts. A claim with an unknown commit outcome can remain in PostgreSQL until its lease expires.

`Jobs.join` can wait past the grace period. It returns `stopped` after the runner observes all local attempt exits and finishes its PostgreSQL work, including background maintenance. Before closing the PostgreSQL pool, wait for `Jobs.join` to return for every runner that uses it.

`Jobs.status`, `Jobs.join`, and `Jobs.stop` report cumulative settlement counts for that runner in `outcomes`. The fields are `completed`, `cancelled`, `discarded`, `retryable`, `scheduled`, and `released_unstarted`. Each count reflects writes PostgreSQL confirmed to the runner. Their sum is `finished`. The top-level `scheduled` counts cron jobs enqueued. `stale_acks` counts acknowledgements that applied no write, and `recovered` counts expired attempts handled by this runner's recovery loop.

`peak_queued_context_bytes` records the largest total size of confirmed handler contexts awaiting launch in a single claim response for this runner. It measures the job context strings in memory. PostgreSQL response framing and other process memory have separate limits.

Use a writable PostgreSQL primary with durable commits (`synchronous_commit = on` or stronger) and enough pool connections for handlers, application transactions, and runner I/O. Durability during primary failure depends on the database's storage, replication, and failover configuration. Schedules and lease expiration use the primary's clock.

#### Run trusted handlers in the application process

Set `execution: "trusted_shared_process"` in an ordinary long-running `Jobs.start` runner. Each attempt runs as a Mesh actor in the application process. It uses the application's scheduler, environment, credentials, and output streams. Deploy one trust domain per process. Use fresh-process execution for `Jobs.start_effects`, `Jobs.drain`, and `Jobs.start` with `drain: true`.

This example records one receipt per business key in a tenant's application database. Provision a separate application login per tenant. Create `tenant_app.receipts (receipt_key text PRIMARY KEY)` as the application schema owner and grant the handler login schema usage and insert privileges on `receipt_key`. Keep the queue login separate and keep other tenants' credentials out of this process. Set `statement_timeout` and `lock_timeout` on the application login. The business key comes from the producer and stays the same on retries, including an operator retry that changes the Jobs `effect_key`.

```mesh
fn record_receipt(key :: String) -> Int!String do
  let params = [key]
  let app = Pg.connect(Env.get("TENANT_APP_DATABASE_URL", ""))?
  let written = Pg.execute(app,
    "INSERT INTO tenant_app.receipts(receipt_key) VALUES ($1) ON CONFLICT DO NOTHING",
    params)
  Pg.close(app)
  written
end

fn receipt_key(job :: String) -> String!String do
  let args = Json.object_get(Json.parse(job)?, "args")?
  let key = Json.as_string(Json.object_get(args, "receipt_key")?)?
  if key == "" || String.length(key) > 128 do
    Err("invalid receipt key")
  else
    Ok(key)
  end
end

fn handle_trusted_receipt(job :: String) -> String do
  case receipt_key(job) do
    Err(_) -> Json.encode(json { action: "discard", reason: "invalid receipt key" })
    Ok(key) ->
      case record_receipt(key) do
        Ok(_) -> Json.encode(json { action: "complete" })
        Err(_) -> Json.encode(json { action: "fail", reason: "receipt write unconfirmed" })
      end
  end
end

fn start_trusted_receipts(queue_pool :: PoolHandle) -> Int!String do
  let workers = Json.parse("""[{"worker":"record-receipt","version":1}]""")?
  Jobs.start(queue_pool,
    json {
      namespace: "tenant-a",
      queues: json { default: 1 },
      workers: workers,
      execution: "trusted_shared_process"
    },
    handle_trusted_receipt)
end
```

Run `Jobs.migrate` with the queue schema owner before starting the runner. Enqueue worker `record-receipt` with an argument such as `{"receipt_key":"order-47"}`.

Keep each attempt bounded. Set a job timeout, limit database and network waits, and use cooperative Mesh operations such as `Timer.sleep` between units of work. Keep resources that need explicit shutdown in application-owned code and finish synchronous connection use before yielding. Keep work in the handler actor. Actors or external work started by a handler have their own lifetimes and can outlast the attempt.

Cancellation, timeout, lease loss, and stop can end the actor at a Mesh scheduling boundary. A synchronous database or native call occupies its scheduler thread until it returns. The runner keeps the local attempt slot until actor exit. `Jobs.stop` can report undrained work after its grace period, and `Jobs.join` waits for that work to exit. Wait for `Jobs.join` before closing application and queue pools. A Mesh panic fails the attempt. A native abort or process exit takes down the runner and its in-flight actors. Other runners recover the abandoned leases.

PostgreSQL still checks the Jobs lease token at queue settlement. The example's application write uses a separate transaction with the tenant login's ordinary SQL privileges. It can commit after cancellation or lease expiry, including when a replacement attempt is already running. Reconcile an unknown commit by replaying the same business key. On supported GNU/Linux targets, `Jobs.start_effects` runs a fresh worker process and checks the current lease at the application write through the configured fence.

### Read runner events

Set `event_buffer` in the runner config to a capacity between 1 and 1024 to enable the reader. Set `event_log: true` to stream the same events as JSON lines on standard error. Both options default to disabled and work independently.

`Jobs.read_events(runner, limit)` returns `Result<String, String>`. The JSON result has an `events` array, a cumulative `dropped` count, and a `closed` flag. Use a limit between 1 and 128. Each call removes the events it returns from the runner's shared buffer. An empty batch with `closed: false` means you can poll again later. After the runner stops and you drain the buffer, the reader returns `closed: true`. Calls return an error for an unknown runner or a disabled reader.

Each event has `version: 1`, `runner_id`, an `event` name, and `observed_at_ms` from the runner's clock. Attempt events include an `attempt` object with the job ID, an opaque attempt ID, the attempt number, and timing. The same attempt ID identifies every event for one claim. It changes on a new claim, even when a snooze preserves the attempt number. `queue_wait_ms` is the time from the scheduled time to the claim observation, measured on PostgreSQL's clock. `handler_duration_ms` measures elapsed time from local launch to observation of the worker result or termination. Unavailable timings are JSON `null`.

| Event | Meaning |
| --- | --- |
| `runner_started`, `runner_stopped` | The local dispatcher started or finished its work. |
| `attempt_started` | PostgreSQL confirmed the claim and the runner launched a worker. |
| `settlement_committed` | PostgreSQL confirmed the new disposition. A retry at exhaustion reports `discarded`, and a committed operator cancellation takes precedence over a handler outcome. |
| `lease_lost` | The local lease deadline elapsed or the database reported that the runner no longer owned the lease. |
| `ack_stale` | PostgreSQL confirmed that this acknowledgement applied no write. |
| `ack_uncertain` | The runner could not confirm the result of this acknowledgement. A subsequent retry uses the same lease token. A stale reply to that retry leaves the earlier result unknown to the runner. |
| `storage_failure` | A storage operation failed. The event includes an operation name with a fixed set of possible values. |
| `maintenance` | PostgreSQL confirmed a positive count from `recover`, `cron`, or `prune`. The count uses the same definition as the corresponding runner status field. |

Acknowledgement events include a `submission` number, `handler_duration_ms`, and a nullable `failure_kind` that describes the local outcome. The failure kinds are `handler_failure`, `invalid_outcome`, `worker_failure`, `timeout`, `shutdown`, and `lease_lost`. Events omit arguments, metadata, credentials, lease tokens, and raw error text. Keep job IDs and attempt IDs out of metric labels.

Each sink drops new events when its buffer is full. The JSON logger buffers up to 128 events per runner plus the write in progress, and each process supports up to 16 active logger threads. Runner status includes `events_dropped` and `event_logs_dropped` when either option is enabled. The logger counts events lost to a write failure and events skipped after that failure. Its counter can advance after `Jobs.join` returns. The runner never waits for either sink. An overflowing sink can miss the final event, and process exit can interrupt pending log writes.

With either sink enabled, runner status includes `event_buffer_bytes` with a `reader` and a `logger` entry. A disabled sink has a `null` entry. Each occupied slot is charged for the in-memory event and a full attempt identity, including the job and attempt ID strings. The charge applies even when the identity is absent or shared. `upper_bound_bytes` is the charge for a full channel. `sampled_peak_bytes` is the largest observed occupancy in those units and can miss a peak that drains between samples. The counters cover queued records and exclude unused slots, channel and allocation overhead, and records already taken for reading or logging.

### Inspect and operate queues

Use `Jobs.local_control` and `Jobs.status` from the process that owns the runner handle. Local queue actions take `queue` and one of the following actions.

| Local action | Behavior |
| --- | --- |
| `start` | Add a queue or start it again. An optional `limit` defaults to the prior limit or one for a new queue |
| `pause`, `resume` | Change admission on this runner while existing attempts continue |
| `scale` | Set the local `limit`. Existing attempts may finish before the new limit takes effect |
| `stop` | Begin draining this queue with optional `grace_ms`. Check its status for `stopped` before starting it again |

Each queue in the status response has a `state`, `limit`, `in_flight`, `pending_claims`, and `drained`. Local settings come from the runner config on every process start. A runner in drain mode uses the queues selected at startup.

Pass a JSON object to `Jobs.control` with an `action` and the fields required for that action. For example, use `json { action: "pause", queue: "default" }` to stop new claims without stopping active attempts. A new queue allows one concurrent attempt across all runners. Change that limit with `configure` or `scale`. The per-runner limits in `Jobs.start` still apply.

| Action | Fields |
| --- | --- |
| `configure`, `scale` | `queue`, `concurrency` |
| `pause`, `stop` | `queue` |
| `start`, `resume` | `queue` |
| `get` | `id` for a job or `queue` for a queue |
| `list` | Optional `queue`, `worker`, `state`, `after_id`, `limit` |
| `search` | Optional `filter` with `queue`, `worker`, `state`, and `limit`, plus an optional top-level `after_id` |
| `list_queues` | Optional `after`, `limit` |
| `queue_stats` | `queue`. Returns counts by state and the age of the oldest due job |
| `inspect_queue` | `queue`. Returns the queue limit, state, number of executing rows, and namespace pause flag |
| `namespace_status`, `pause_namespace`, `resume_namespace` | Apply to the namespace passed to `Jobs.control` |
| `cancel`, `retry`, `discard`, `suspend`, `resume`, `delete` | `id` |
| `cancel_all`, `retry_all` | Optional `filter` object with `queue`, `worker`, `state`, `limit` |
| `delete_all` | `filter` with a queue, worker, or terminal state, and optional `limit`. Deletes terminal jobs only |
| `update` | `id` and a `patch` with any of `args`, `metadata`, `tags`, `priority`, `timeout_ms`, `scheduled_at_ms`, or `delay_ms` |
| `run_now` | `id` of a scheduled or retryable job. Makes that job due on the primary's clock |
| `reschedule` | `id` and either `delay_ms` or a UTC `scheduled_at_ms` |
| `prune` | `before_ms` or `max_age_ms`, with optional `queue` and `limit` |
| `schedule` | `name`, `expression`, `job`, and optional `timezone`, `misfire`, `lookback_minutes` |
| `unschedule` | `name` |
| `list_schedules` | Optional `after`, `limit` |

`get` returns the persisted job and its error history. `list` and `search` return summaries ordered by ID. Pass the last ID as `after_id` for the next page. A cancellation of an executing job records intent. The current runner observes the intent and stops the attempt, or the lease expires and recovery settles the cancellation.

`pause_namespace` covers every queue, including queues created while the namespace is paused. It leaves any existing attempt and queue setting in place. `resume_namespace` restores admission for active queues. `update` and `run_now` return an error for locked, missing, or ineligible jobs. Both keep the current effect key and attempt budget. An update that supplies a new schedule leaves suspended jobs suspended.

Set `prune_max_age_ms` in the runner config to remove old terminal jobs in bounded batches. Without it, jobs remain available for inspection until an operator prunes or deletes them. Use `drain: true` with `Jobs.start` and `Jobs.join` to recover expired claims and process eligible, due work. This drain mode starts without cron or pruning tasks and leaves future jobs scheduled.

### Run a bounded drain

`Jobs.drain` takes the same named handler and runner config as `Jobs.start`. Give the config an explicit `namespace` and pass the drain options separately.

```mesh
fn drain_receipts(pool :: PoolHandle, config :: String) -> String!String do
  Jobs.drain(pool, config, handle_receipt,
    json { max_jobs: 25, recursive: false, on_error: "stop" })
end
```

The drain recovers up to `max_jobs` expired attempts in the selected queues and worker versions before claiming new work. It runs each claimed attempt in a separate process and waits for settlement before claiming another job. Queue state and database-wide concurrency limits apply.

| Option | Meaning |
|--------|---------|
| `max_jobs` | Required. Limits the call to 1–1000 claims, including retries. Recovery counts separately. |
| `cutoff_ms` | Optional inclusive UTC epoch timestamp in milliseconds. The drain may claim jobs scheduled through this time, including future jobs. When omitted, each claim uses PostgreSQL's current time. |
| `recursive` | Required. With `false`, the drain takes up to `max_jobs` eligible IDs in priority, schedule, and insertion order at the start and attempts each at most once. With `true`, it can also claim newly eligible jobs until it reaches the budget. |
| `on_error` | Required. `"stop"` stops after a committed retryable or discarded result. `"continue"` allows the drain to claim more jobs. Both return settlement counts. A claim, launch, or unconfirmed settlement error returns `Err`. |

The report includes `claimed`, `finished`, `outcomes`, and `recovered`. The `finished` count sums the committed outcomes of attempts claimed by this call. `budget_hit` reports whether the call used all its claims. `remaining` reports eligible or executing work in the selected active queues and cutoff at the time of the final PostgreSQL query. It can be true when another runner holds capacity, a transaction holds a row lock, or the call drained only its initial IDs. The `stop_reason` is `budget`, `job_error`, `idle`, or `shutdown`. An idle drain returns after a pass with no claim and leaves locked or otherwise unavailable work for another call.

Use a dedicated namespace for each test. See [Testing durable jobs](/docs/testing/#testing-durable-jobs) for enqueue assertions and direct handler tests.

For maintenance settings, role grants, recovery, and concurrent reindex, use the [Jobs operations runbook](jobs-operations.md).

### Recurring jobs

The `schedule` action stores a five-field cron expression or an alias such as `@daily`. It accepts an IANA time zone such as `America/New_York` and defaults to `Etc/UTC`. Runners enqueue each occurrence and advance its cursor in one transaction. Nonexistent local minutes during the spring clock change are skipped. Repeated local minutes in the fall can each enqueue a job.

`misfire` controls missed occurrences within `lookback_minutes` (up to 1,440). `skip` checks the current minute, `coalesce` picks the most recent match, and `catch_up` enqueues all matches in order. `@reboot` enqueues once per namespace activation. A new activation begins after 60 seconds without a committed scheduler heartbeat.

## Timers

Timers use monotonic deadlines. Sleeping an actor yields its scheduler worker so other actors can run.

| Function | Returns | Description |
|----------|---------|-------------|
| `Timer.sleep(milliseconds)` | `Unit` | Suspend the current actor until the delay expires |
| `Timer.send_after(pid, milliseconds, message)` | `Unit` | Deliver a typed message after a delay |
| `Timer.apply_after(milliseconds, function)` | `Unit` | Call a zero-argument function after a delay, in an actor of its own |

```mesh
actor reminder() do
  receive do
    message -> println(message)
    after 5_000 -> println("no reminder received")
  end
end

fn main() do
  let pid = spawn(reminder)
  Timer.send_after(pid, 100, "time to stretch")
  Timer.sleep(200)
end
```

`Timer.send_after` delivers a plain message, which an actor's `receive` sees but a service's `cast` handler does not: a service dispatches on a tag that only its generated functions know. To reach a service after a delay, schedule a function that casts:

```mesh
service Writer do
  fn init(start :: Int) -> Int do
    start
  end

  cast Flush(reason :: String) do |flushed|
    println("flush: #{reason}")
    flushed + 1
  end
end

fn main() do
  let writer = Writer.start(0)
  Timer.apply_after(100, fn () -> Writer.flush(writer, "timer") end)
  Timer.sleep(200)
end
```

The function runs in its own actor, which is not linked to the caller: a failing callback does not take the caller down. Values it captures stay alive until it has run. As with `send_after`, the program does not wait for a pending timer once `main` has returned and every remaining actor is idle.

## Bounded Channels

`Channel` provides bounded, in-process queues. A `Channel<T>` carries values of one type `T`, of any type; creation and queue operations return `Result`, and producers use `try_send` and never wait for space.

```mesh
fn round_trip(value :: Int) -> Int!String do
  let channel = Channel.bounded_bytes(128, 1_024, :reject_newest)?
  Channel.try_send(channel, value)?
  let timeout = Duration.millis(10)?
  Channel.recv(channel, timeout)
end

fn main() do
  case round_trip(42) do
    Ok(value) -> println("#{value}")
    Err(error) -> println(error)
  end
end
```

| Function | Returns | Description |
|----------|---------|-------------|
| `Channel.bounded(item_capacity, policy)` | `Result<Channel<T>, String>` | Create a queue bounded by item count |
| `Channel.bounded_bytes(item_capacity, byte_capacity, policy)` | `Result<Channel<T>, String>` | Apply both item and byte bounds |
| `Channel.try_send(channel, value)` | `Result<Int, String>` | Enqueue a value immediately or report backpressure |
| `Channel.recv(channel, timeout_nanos)` | `Result<T, String>` | Dequeue, waiting for at most that many nanoseconds; `0` polls |
| `Channel.depth(channel)` | `Int` | Current item count, or `-1` for an unknown handle |
| `Channel.byte_depth(channel)` | `Int` | Current queued bytes, or `-1` for an unknown handle |
| `Channel.dropped(channel)` | `Int` | Values rejected or replaced, or `-1` for an unknown handle |

Overflow policies are:

- `:reject_newest` — keep queued values and return `Err("channel full")` for the new value.
- `:drop_oldest` — discard the oldest value and enqueue the new value.
- `:latest_only` — discard every queued value and retain only the newest.

`Channel.recv` takes nanoseconds. Use `Duration.millis` or `Duration.seconds` to make the unit explicit and to detect overflow.

A queued value takes eight bytes plus the size of whatever it references: a `String`, a list, or a struct is copied out of the sender's heap as it is sent, and into the receiver's as it is received, so actors can share a channel. `bounded_bytes` bounds that total too; a value larger than the whole byte capacity is rejected, whatever the policy, with `Err("value exceeds the channel byte capacity")`. `try_send` returns `Ok(0)` on acceptance and may return `"channel busy"` instead of waiting for the shared registry lock. An actor waiting in `recv` is suspended until a value arrives or the timeout passes, so its scheduler worker runs other actors meanwhile.

## Process Names and Shutdown

`Process` manages actors within the current runtime and coordinates graceful shutdown.

| Function | Returns | Description |
|----------|---------|-------------|
| `Process.register(name, pid)` | `Int` | Register a local name; `0` is success and `1` is failure |
| `Process.whereis(name)` | `Pid` | Resolve a local name; PID `0` (`<0.0>`) means not found |
| `Process.monitor(pid, message)` | `Int` | Send the calling actor `message` when `pid` ends; returns the monitor's reference |
| `Process.demonitor(reference)` | `Int` | Remove a monitor; `0` is success and `1` is failure |
| `Process.install_shutdown_signals()` | `Unit` | Treat native `SIGINT` and `SIGTERM` as shutdown requests |
| `Process.shutdown_requested()` | `Bool` | Read the process-wide shutdown flag |
| `Process.request_shutdown()` | `Unit` | Set the shutdown flag programmatically |
| `Process.exit(status)` | `Unit` | Exit the native process with a status code, 0 to 255 (any other status exits with 1) |

`Process.whereis` returns an untyped `Pid`, which accepts any message, so a
send through it does not tell the compiler what the actor receives. When an
actor is reached only that way, give its pid a message type where it is
spawned; otherwise the actor is error E0079:

```mesh
actor counter() do
  receive do
    n -> println("got #{n}")
  end
end

fn main() do
  let pid :: Pid<Int> = spawn(counter)
  Process.register("counter", pid)
  send(Process.whereis("counter"), 5)
  Timer.sleep(50)
end
```

HTTP servers observe the same shutdown flag: after shutdown is requested they stop accepting new connections and drain connections already accepted.

Create and remove monitors from the actor that owns them: a monitor's
message is one of that actor's messages, so `Process.monitor` outside an
actor is a compile-time error (E0086). `Process.demonitor` returns `1` for a
reference the calling actor does not hold.

## Deterministic Random Values

`Random` is a deterministic, explicitly state-threaded pseudo-random generator. It is useful for simulations, scheduling choices, and reproducible tests; it is not a cryptographic random source.

```mesh
fn main() do
  let state = Random.seed(42)
  let (next_state, value) = Random.next_int(state, 1, 100)
  println("state=#{next_state}, value=#{value}")
end
```

| Function | Returns | Description |
|----------|---------|-------------|
| `Random.seed(seed)` | `Int` | Normalize a deterministic generator state |
| `Random.next_int(state, min, max)` | `(Int, Int)` | Return `(next_state, value)` with `min <= value <= max`; panics when `min > max` |
| `Random.next_unit_ppm(state)` | `(Int, Int)` | Return `(next_state, value)` where `0 <= value < 1_000_000` |

## Next Steps

- [Type System](/docs/type-system/) -- structs, generics, traits, and deriving
- [Distributed Mesh](/docs/distributed/) -- remote actors and global process names
- [Syntax Cheatsheet](/docs/cheatsheet/) -- quick reference for all Mesh syntax
