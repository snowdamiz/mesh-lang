# Dense application commit boundary

The retained [handler-pool run](../2026-11-24-trusted-handler-app-pool/timing-clean-identity-retry)
records each app write's `xmin`, PostgreSQL commit timestamp, and handler return.
This packet adds a PostgreSQL callback timestamp between those endpoints. It
uses the qualified Linux x86-64 `direct-observed` runner for every arm, including
both inactive controls. The app, queue runner, producer, workload, and observer
are the frozen ordinary trusted shared-process fixture. Public queue results and
River, Oban, and Apalis peer results stay in their own evidence sets.

## Interpret the clocks

In PostgreSQL 17.11, ordinary `PRE_COMMIT` runs before `RecordTransactionCommit`.
The callback reads `GetTopFullTransactionIdIfAny()`, which returns the original
epoch and does not allocate an XID. `RecordTransactionCommit` takes its stop
timestamp before inserting the commit WAL record. With no replication origin,
the commit timestamp lookup returns that same timestamp. For a WAL-writing
transaction with synchronous commit, PostgreSQL flushes WAL, marks the XID
committed, and handles any synchronous replication wait. It then calls
`ProcArrayEndTransaction` and invokes `XACT_EVENT_COMMIT` before resource and
lock release. Parallel workers have separate events and leave durable commit
to the leader. Abort and prepare never reach the ordinary commit callback.
The upstream transaction-commit trace point carries a virtual XID.
[PostgreSQL 17.11 transaction source](https://github.com/postgres/postgres/blob/REL_17_11/src/backend/access/transam/xact.c#L1284)

The hook captures `CLOCK_REALTIME` in the commit callback. The retained handler
uses the same clock at microsecond resolution. PostgreSQL finishes an implicit
extended-protocol transaction before it sends ReadyForQuery on Sync.
[PostgreSQL 17.11 protocol source](https://github.com/postgres/postgres/blob/REL_17_11/src/backend/tcop/postgres.c#L4664)

- Record to callback includes WAL insertion and flush, commit status, ProcArray,
  and any backend scheduling gaps before the callback.
- Callback to return includes PostgreSQL cleanup and reply handling, socket and
  TLS work, client processing, and any scheduling gaps after the callback.

The split locates time on either side of the callback. Assigning individual
causes such as `fdatasync` or network latency requires finer instrumentation.

The setup canary compares `pg_waldump` with `pg_xact_commit_timestamp` for a
synchronous WAL-logged write. It checks the callback's position in the client
clock window and uses a nonzero XID epoch. It also verifies role filtering,
inactive preload, and read-only, rollback, and prepare handling. The timing
arms bracket their wall clocks with monotonic samples and PostgreSQL
`clock_timestamp()` outside the workload. Replay rejects endpoint offset
changes over 100 µs and out-of-order samples. Replay also checks the existing
spare-CPU host observer samples for clock drift during the workload. Transient
clock steps that unwind between observations remain possible.

The module preloads only for `compiled_caller` in the private `mesh` database.
Each active backend keeps up to 4,096 records in a fixed process-local buffer.
The callback takes one realtime sample per committed transaction with an XID.
Both modes write exit receipts only when the backend exits. Replay requires
32 matching backend receipts with orderly exit codes and complete trailers.
It lifts each ledger XID into a strictly bounded interval of full XIDs from
the same cluster. All 2,048 application writes must join uniquely. Producer and
settlement XIDs must be distinct. Missing or extra samples, reused XIDs, clock
errors, overflow, and a changed synchronous-commit setting invalidate the arm.
The local correctness canary and offline tests exercise the module and replay
without running a benchmark.

## Controller handoff

Use the existing Linode booking controller. The packet sets two minutes for
setup, five minutes for the first group, and nine minutes for the two
confirmation groups. It reserves 45 seconds at the end of each timing window.
Each adapter invocation has a 35-second timeout and five seconds for forced
termination. Keep the existing idle checks, execution reservation, and release
receipts. Stop on the first failed or interrupted arm and retain its raw output.

1. Run `python3 -B packet.py check`. Stage the sealed setup files during the setup
   booking and run `bash native-setup.sh <12-hex-attempt>` through the controller.
   Retain stdout as `setup.json`. The setup holds both existing host locks and
   checks the pinned 17.11 server executable. The toolchain's
   `/var/lib/durable-jobs-bench/native/toolchain/pgsql-17.11/bin/pg_config` gives the
   server headers, `pkglibdir`, and the build configuration. The setup records
   those paths. It publishes the module under the booking's private
   `postflush-pg17-<attempt>` directory by absolute path.
2. Stage the sealed arm files and generate each inventory with
   `python3 -B packet.py inventory setup.json <remote-stage> <lane>`. Use
   `/var/tmp/mesh-durable-jobs-matrix/postflush-<12-hex-attempt>-<lane>` as the
   stage path. Save a JSON map from lane to stage in `stages.json`. The
   existing matrix worker checks the module and runner bytes before launch.
3. Run group 1 within its timing booking. Follow the packet's order for
   each phase and archive each arm with the existing matrix worker.
   Record the group entry verbatim as `01-noop/order.json` and preserve the
   standard `<phase>-<lane>/<phase>-mesh` receipt directories. Run
   `python3 -B score.py <timing-evidence> setup.json stages.json 1`.
4. If the screen passes, book groups 2 and 3 and replay all three with the
   same command and `1 2 3`. Retain the signed active-vs-inactive deltas, control
   spread, raw archives, per-write bindings, and release receipts. After
   release and export, remove the booking-owned stage, module, and scratch
   through the controller's fenced cleanup.

For drain and each write's record-to-return p95, the inactive pair must differ
by at most 5% of its mean. The active arm must be within the larger of 3% and
the inactive spread. These are screening thresholds. Only three passing
groups qualify the split. Each group's cumulative share must clear 55% on the
same side of the callback, including a microsecond rounding allowance, to
classify that side as dominant. Return separate first-write and second-write
distributions alongside the cumulative split.

No Linode setup or timing ran in this implementation turn. Native headers,
`pkglibdir`, the Linux module hash, and its canary result become live evidence
when the controller captures `setup.json` under its setup booking.
