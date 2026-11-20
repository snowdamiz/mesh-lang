# J1 Oban fixture

Run from this directory with `mix run --no-start fixture.exs -- <action>`.
Install Hex and run `mix deps.get --check-locked` followed by `mix compile` before
starting any measured process. Set `MIX_HOME`, `HEX_HOME`, and `TMPDIR` to
directories under the checkout. Mix places dependencies and compiled files in
the ignored local `deps/` and `_build/` directories by default.

The fixture uses Oban Core 2.24.2 from the `hexpm` registry under Apache-2.0.
It runs `Oban.Engines.Basic` and Oban's PostgreSQL migrations. `mix.lock`
records the dependency versions and package checksums.
[Oban 2.24.2 on Hex](https://hex.pm/packages/oban/2.24.2)

## Driver contract

- `versions` emits one JSON line with `oban`, `erlang`, and `elixir` strings.
  The Erlang version comes from OTP's `OTP_VERSION` file when present and
  otherwise gives the OTP release.
- `migrate` creates the schema named by `JOBS_PARITY_NAMESPACE` and runs
  Oban's migrations there. The benchmark queue is `parity`. For settlement
  evidence, read that schema's `oban_jobs`, with the logical key in
  `args->>'key'`, claim time in `attempted_at`, and successful settlement in
  `completed_at`.
- `enqueue` reads the JSON object at the absolute path in
  `JOBS_PARITY_INPUT`. One transaction writes the product row, calls
  `Oban.insert_all` in groups of up to 128, and captures
  `pg_current_xact_id()::text`. `request_id` defaults to `batch`. The response
  is one JSON line containing the inserted job `count`, the transaction
  `xid`, and `ids`, a map from input job keys to returned Oban IDs. With
  `rollback: true` the receipt describes the inserts made inside
  the rolled-back transaction. The XID has no commit timestamp.
  A repeated request can enqueue another Basic job. The product and effect
  tables deduplicate writes through their unique constraints.
- `run` writes `READY` after connecting to the migrated schema and starting
  its queue. `worker_pid` in attempt rows is the runner's OS PID. PostgreSQL
  sessions identify the fixture as `jobs_parity_oban:<namespace>:<runner>`,
  subject to PostgreSQL's application name length limit.

All database actions take `JOBS_PARITY_DB_URL` and `JOBS_PARITY_NAMESPACE`.
`run` also takes `JOBS_PARITY_RUNNER`. The namespace must be a dedicated
lower snake case schema of at most 63 bytes. The URL accepts `sslmode=disable` or
`sslmode=verify-full` with an optional `sslrootcert`. The driver owns
`public.mesh_jobs_parity_product`, `public.mesh_jobs_parity_attempts`, and
`public.mesh_jobs_parity_effects`, including the unique constraint on effect
identity. Fixture writes use `engine = 'oban'`.

## Dispatch and recovery

- Every runner starts one producer with `JOBS_PARITY_SLOTS` execution slots
  and an Ecto pool of slots plus two connections. Migration and enqueue each
  use two connections. The comparison's database connection limit includes
  both pools. The isolated notifier and peer keep staging local to
  each VM, and enqueue notifications are disabled. The stager runs at
  `JOBS_PARITY_POLL_MS`. The producer keeps Oban's 5 ms jittered dispatch
  cooldown and can also dispatch when an executing job frees a slot.
- A worker captures UTC at entry before its database checkout and commits
  the attempt row. With `fail_first: false`, it sleeps for `hold_ms` before
  committing the effect and completing. With `fail_first: true`, it commits
  the effect before sleeping and returning an error on the first attempt.
  Later attempts skip that hold and try the same effect again.
  Effects use `ON CONFLICT DO NOTHING`, and both tables use the input job key
  as the stable effect key. `JOBS_PARITY_HOLD_MS` is an enqueue-time fallback
  when a job omits `hold_ms`. The job has three attempts total. Oban schedules
  retries one second later because its backoff callback takes whole seconds.
- `dense` and `sparse` leave Lifeline disabled. `fault` uses Core Lifeline
  every 250 ms to move rows in `executing` whose `attempted_at` is older
  than 5,000 ms back to `available`. A row with an exhausted attempt budget
  goes to `discarded`. The next claim increments the attempt and updates
  `attempted_at`. Basic settlement matches the executing state and the
  original attempt timestamp. Lifeline can rescue a live attempt once it
  passes the age threshold. To test a crash after the effect commits, use
  `fail_first: true`. Kill the runner after observing the committed effect
  while the job is still executing, before Lifeline can rescue it.
- The driver sends `stop` on standard input to stop Oban before the Repo,
  using Oban's 15-second queue shutdown grace period. The fixture also
  handles SIGTERM. A hard kill leaves claimed rows for fault-mode Lifeline.
  Cron, pruning, and reindexing are disabled to retain evidence and keep
  background work fixed.
