# Rust durable Jobs adapter

**SETUP complete for Apalis.** The Linux x86-64 release passed the
functional and trace-off `sparse` smokes on the Linode with PostgreSQL
17.11. The smoke and all five frozen profiles also passed on macOS
arm64. The matrix owner controls official timing.

## Linode matrix handoff

Use this command prefix on the Linode. Append the profile
(`noop`, `slow`, `sparse`, `short`, or `large`) and the absolute evidence
directory. Use `nobody` as `run_as` and the `--scratch` value as
`postgres_storage`.

```sh
/var/lib/durable-jobs-bench/apalis/source-v1/tools/durable-jobs-bench/rust/run \
  --pg-bin /var/lib/durable-jobs-bench/native/toolchain/pgsql-17.11/bin \
  --scratch /var/lib/durable-jobs-bench/apalis/scratch
```

The native build manifest is under
`/var/lib/durable-jobs-bench/apalis/source-v1/target/linode-apalis/build/release/`.
Set `version` to `apalis 1.2.0 / apalis-postgres 1.2.0` and
`artifact_sha256` to `b3c2fb2b472d7c8b2706e13c38281a46958b669f9d4cf22ca252636d083e7e66`.
The native lane can reuse the verified Rust 1.99.0 installation at
`/var/lib/durable-jobs-bench/native/toolchain/rust-1.99.0/bin`.
The shared PostgreSQL 17.11 installation includes `pg_stat_statements`,
`pgcrypto`, and `hstore`.
The functional smoke verified the producer rollback, a first-attempt
success, a retry that reached native `Done`, and an exhausted job that
retained native `Failed` and its error. The trace-off `sparse` smoke
reconciled 24 enqueue calls and 24 completed jobs against the shared
ledger verifier. Each job had two distinct, ordered application XIDs.
The peak was four connections against the cap of five.

On the Linode, the raw evidence is under
`/var/lib/durable-jobs-bench/apalis/runs/{functional-v1,sparse-trace-off-v1}/`.
The original archives, staged source hashes, and an offline replay are
tracked with the receipt. [Replay the native evidence](../results/2026-11-23-apalis-smoke/README.md)

## Build during setup

The adapter pins the published `apalis` and `apalis-postgres` crates at
1.2.0. Cargo.lock also pins `apalis-core` 1.2.0 and SQLx 0.9.1. Build using
Rust and Cargo 1.99.0, Python 3.11 or newer, OpenSSL, and PostgreSQL 17.11
with the matching `pg_stat_statements`, `pgcrypto`, and `hstore` extensions
available. The build keeps Cargo state under `target/linode-apalis/`.

From the checkout root, stage only this adapter and its shared dependencies
when preparing the native host. The shared files are
`tools/durable-jobs-bench/verify.py`, `scripts/jobs-parity.py`, and
`scripts/jobs-parity-profiles.json`. Keep their relative paths intact.

```sh
tools/durable-jobs-bench/rust/build
```

If Rust is installed outside rustup, set `APALIS_RUST_TOOLCHAIN_BIN` to
the absolute directory containing the pinned `rustc` and `cargo`.

The release executable and its `build.json` and `dependencies.txt` are in
`target/linode-apalis/build/release/`. The matrix inventory uses engine
`rust`, version `apalis 1.2.0 / apalis-postgres 1.2.0`, and the native
`artifact_sha256` from that build's manifest. The adapter verifies the
binary, build inputs, and dependency tree against that manifest each run.

## Run one arm

Run as an unprivileged user with write access to the evidence and scratch
directories. The matrix controller can use its
`run_as` entry. Set `postgres_storage` in the inventory to the owned
directory supplied as `--scratch`. The controller pins the adapter and its
children to the shared CPUs.

```sh
root=$PWD
pg_bin=/absolute/path/to/postgresql-17.11/bin
scratch="$root/target/linode-apalis/scratch"
DURABLE_JOBS_PHASE=diagnostic "$root/tools/durable-jobs-bench/rust/run" --pg-bin "$pg_bin" --scratch "$scratch" \
  smoke "$root/target/linode-apalis/native-smoke-01"
DURABLE_JOBS_PHASE=diagnostic "$root/tools/durable-jobs-bench/rust/run" --pg-bin "$pg_bin" --scratch "$scratch" \
  sparse "$root/target/linode-apalis/native-sparse-01"
```

For `smoke`, check `smoke.json` for two completions, one retry, one exhausted
failure, and one rollback. For `sparse`, check `metrics.json` for 24 completions,
zero failures, and 24 jobs with ordered independent writes. Each successful
`sparse` job has one attempt XID and one effect XID.

Use any of `noop`, `slow`, `sparse`, `short`, or `large`. Supply a new or
empty evidence directory each time. One invocation creates one fresh,
capped PostgreSQL cluster and runs one arm. Build, migrations, worker startup,
and the three-second ready idle are outside the durable timing window.
The matrix controller supplies `DURABLE_JOBS_PHASE` and schedules warmups.
The adapter removes its disposable cluster and TLS material after each arm.

The [published transactional enqueue API](https://docs.rs/apalis-postgres/1.2.0/apalis_postgres/queries/fn.push_tasks.html)
inserts stock wire-format tasks in the same SQLx transaction as the product
row. The controller uses one warm connection for producer and observer SQL.
Each runner uses [stock PostgreSQL storage and pub/sub](https://docs.rs/apalis-postgres/1.2.0/apalis_postgres/struct.PostgresStorage.html),
two Tokio worker threads, a pool of slots plus two, and Apalis's concurrency
layer set to its assigned slots. Fetch batch size is the assigned slots.
The stock listener wakes a runner on a job insert, with a 250 ms fallback
poll. Apalis may reserve work as `Queued` before starting a handler.

The handler keeps the application connection checked out across the hold.
It records the attempt and the effect as distinct autocommit statements,
matching the frozen Oban fixture. The parity runs require exactly one
successful attempt and one effect per job. The smoke requires a rolled-back
producer transaction, a successful job, a retry that reaches native `Done`,
and an exhausted attempt that retains native `Failed` and its error.

## Evidence for the matrix

A successful arm emits `ledger.json` in the shared schema along with
`metrics.json`, raw PostgreSQL job and write exports, the enqueue receipts,
connection and worker samples, statement counters, build evidence, and logs.
Use `*.json`, `*.log`, and `dependencies.txt` as the adapter evidence globs
in the matrix inventory. If an arm fails, it preserves available evidence
and tries another connection to export the state after runner shutdown.

The ledger takes timestamps from the producer product row, the PostgreSQL
application tuples, and the native terminal tuple. Producer submission is
recorded before the warm session receives the request. Pickup is handler
entry. Apalis refreshes its `lock_at` during the `Queued` to `Running`
transition, so the raw job export includes `last_lock_us` and the ledger
omits the optional original claim timestamp. Runner CPU comes from
`wait4` and includes idle and shutdown. Resource snapshots and
`pg_stat_statements` are retained separately from the commit windows.
