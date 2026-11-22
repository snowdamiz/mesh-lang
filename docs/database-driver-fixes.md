# Database driver fixes plan

This plan removes known waste from the PostgreSQL and SQLite drivers in `compiler/mesh-rt/src/db/`. It serves people who write Mesh services that query a database. It adds one connection URL option and no other public API. It adds no new database, no replication, and no durable actor.

The program enforces one rule. A fix lands only when the benchmark from D1 shows its gain on the base branch and on the PR head, with the same results returned.

The PRs in order are D1 (measure), D2 (pool health check), D3 (statement reuse), D4 (pool waits), D5 (SQLite writes off workers), D6 (SQLite defaults), D7 (row decoding), and D8 (PostgreSQL socket waits).

## How to read this

One box is one unit of work. Every box names the evidence that checks it. A nested box is a sub-step of the box above it. Check a box only when its evidence exists, a file, a log line, a test run, or a SHA. The body is a how-to. The appendices explain and record.

The program runs `<plugin root>/skills/poteto-mode/playbooks/autopilot-stack.md`. Owners build and verify each PR. The root keeps one linear stack in the order D1 through D8. The operator lands the stack. This plan does not authorize a merge.

Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked.

A second plan, `docs/durable-jobs-oban-parity.md`, runs at the same time. Its phases J2 and J3 edit files in `compiler/mesh-rt/src/db/`. The owner rules below keep the two plans off the same file.

## Program checklist

### Arm the program

- [ ] State the protocol and this plan to the operator, then stop. Start execution only on the operator's explicit go.
- [ ] Read these at program start. Re-read them at every tick.
  - [ ] `cat <plugin root>/skills/poteto-mode/playbooks/autopilot-stack.md`
  - [ ] `cat <plugin root>/skills/poteto-mode/playbooks/autopilot-full.md`
  - [ ] `cat <plugin root>/skills/swarm/SKILL.md`
  - [ ] `cat <plugin root>/skills/benchmark-checklist/SKILL.md`
  - [ ] `cat <plugin root>/skills/poteto-mode/playbooks/opening-a-pr.md`
  - [ ] `cat AGENTS.md docs/durable-jobs-oban-parity.md`
- [ ] Record the current main SHA, the output of `pg_config --version`, and every open PR that touches `compiler/mesh-rt/src/db/`. List the PRs with `gh pr list --state open`.
- [ ] On the operator's go, arm the audit tick as an hourly schedule_task bound to this thread with the tick prompt below. Never leave the cadence to memory.
- [ ] Use this tick prompt, verbatim. "Re-read the execution playbook from disk. Audit the operation against it and fix drift in this tick. Probe every active lane and judge progress by side effects only. Stand down a stuck lane and dispatch its replacement now. Then post a short status message to the operator in chat only when the audit found a tracked change that no earlier status message reported, such as a PR opened, a code-ready head, a round launched or closed, a verdict, a merge, a stuck agent and the action taken, a blocker added or cleared, or a decision only the operator can make. Name every such change and nothing else. Do not repeat a table, the merged list, or an unchanged blocker. If the audit found none, end the turn with no reply text. Either way, log this tick's row in your decision trail. The row names the items reported, or none."
- [ ] On the operator's hold or stand-down, send every owner a zero-writes order at once.

### Spawn owners

- [ ] Spawn one owner per PR with the full lifecycle the execution playbook names. Run every owner and every lane on `claudeAgent/claude-opus-5-5 effort=xhigh`.
- [ ] Follow this dependency graph. Base each PR on its stack parent.
  - [ ] D1 is first and branches from `main`.
  - [ ] D2 after D1. D3 after D2. D4 after D3.
  - [ ] D5 needs only D1. Build it while D2 through D4 are in review. The root appends it after D4.
  - [ ] D6 after D5. D7 after D3 and D6. D8 after D4, D5, and D7.
- [ ] Hold the file boundaries.
  - [ ] The stack order settles overlap inside this plan. D2, D3, D7, and D8 edit `compiler/mesh-rt/src/db/pg.rs`. D2, D4, and D8 edit `pool.rs`. D5, D6, and D7 edit `sqlite.rs`. Only D8 edits `jobs_connection.rs`. An owner edits a shared file only on top of its stack parent.
  - [ ] No PR in this plan edits `scripts/verify-postgres-jobs.sh` or any file under `compiler/mesh-rt/src/jobs/`. Jobs phase J1 extends that script.
  - [ ] Before an owner starts D2, D3, D4, D7, or D8, list the open Jobs parity PRs. J3 edits `db/pg.rs` and `db/jobs_connection.rs`. J2 can edit `db/pool.rs`. If a Jobs PR is open on a file the owner needs, wait for it to merge and branch from the new main.
- [ ] Hold the operator decision. D6 changes the journal mode of every SQLite file that a Mesh program opens. Start D6 only after the operator approves that default in chat.
- [ ] Hold the review gate. No PR changes a graphical interaction. No PR is review-gated.
- [ ] Apply the stop rules.
  - [ ] Skip a PR when D1 shows that its target cost is inside the noise floor. Record the numbers in Appendix A.
  - [ ] When a PR fails its perf rule after one redesign, stop that PR, record the numbers in Appendix A, and continue with the PRs that do not depend on it.

### PR mechanics, for every PR

- [ ] Use `gh` for every PR operation. Never require `gt`.
- [ ] Open the PR ready, never draft, per **Opening a PR**. Use `gh pr create --base <parent-branch>`. A stack child targets its parent branch.
- [ ] Install the hooks once with `bash scripts/install-git-hooks.sh`. Run `cargo fmt --all -- --check` before the PR-facing push. Push with hooks on.
- [ ] Run `/deslop` before each commit and `/no-comments` before review.
- [ ] Triage every automated review comment per `<plugin root>/skills/poteto-mode/references/bugbot-triage.md`.
- [ ] Each push starts an Authoritative verification run of about 33 minutes and a Performance smoke run of about 14 minutes. Cancel a superseded run with `gh run cancel`.
- [ ] Rebase onto the stack parent before the code-ready report. Keep that merge base in fix rounds. Only the root rebases or retargets a link.

### Verdict and merge, for every PR

- [ ] A full swarm runs once, at the code-ready head. Another full swarm runs only for new behavior, a redesign, or a large new diff. A rebase, a merge-conflict resolution, or a minor fix gets a single verifier at the new head. Run each full swarm per `<plugin root>/skills/swarm/SKILL.md`. One gates lane. The ten live lanes from the PR's **Verify, live** block. The perf lane from its **Verify, perf** block. Two audit lanes, each with its own focus, that read the diff and the receipts and distrust the PR body.
- [ ] The gates lane runs `cargo fmt --all -- --check`, `cargo test --locked -p mesh-rt --lib --test library --test library_memory`, `bash scripts/verify-db-drivers.sh`, and `bash scripts/verify-postgres-jobs.sh`.
- [ ] Vet every reported number with `<plugin root>/skills/benchmark-checklist/SKILL.md`. Name the limiter, count errors, and confirm that the timed work ran.
- [ ] Clean only when every lane is `PASS`. Findings go back to the owner, including a defect that a lane filed as a note. A new head gets a fresh verdict, except for results that stay valid under the patch-id rule in `playbooks/shipping.md`.
- [ ] The root appends each clean PR to the stack with its base set to its parent branch. The operator lands the stack bottom-up.

### Boot recipe, for every live lane

Each live lane runs in its own worktree at the PR head. The lanes drive the command line. There is no project control skill for it.

- [ ] `git fetch origin <head-branch> && git checkout <head SHA>`.
- [ ] Build with `cargo build --workspace --locked`.
- [ ] Start PostgreSQL through the lane's own command. `scripts/verify-db-drivers.sh` and `tools/db-performance/bench.py` each create a disposable cluster under `.tmp/` when `MESH_TEST_DATABASE_URL` is unset, and each removes it on exit.
- [ ] Use PostgreSQL 17 locally. This machine has no 18 or 19 binaries, so a lane that names 18.7 or 19.0 reads the hosted compatibility jobs for the PR head.
- [ ] Run one-worker scenarios with `MESH_SCHEDULER_MIN_WORKERS=1 MESH_SCHEDULER_MAX_WORKERS=1`.
- [ ] Save every lane's output to `target/db-driver-fixes/<head SHA>/<lane>.log` and return the paths with the report.
- [ ] Remove the worktree and every scratch directory outside the checkout when the lane ends.

## Measure the drivers and gate their live tests (D1)

**Depends on.** None.

**Files.**

- [ ] Create `tools/db-performance/bench.py`, `tools/db-performance/mesh/main.mpl`, and `tools/db-performance/README.md`.
- [ ] Create `scripts/verify-db-drivers.sh`.
- [ ] Edit `.github/workflows/authoritative-verification.yml`.

**Build.**

- [ ] Write `scripts/verify-db-drivers.sh`. Use `MESH_TEST_DATABASE_URL` when it is set. Otherwise create a disposable cluster under `.tmp/verify-db-drivers.XXXXXX`, listen on 127.0.0.1 only, and remove the cluster in an EXIT trap.
- [ ] In that script, run the live unit tests that no gate runs today, with `cargo test --locked -p mesh-rt --lib db:: -- --include-ignored --test-threads=1`.
- [ ] In that script, run `e2e_db_values`, `e2e_pg_conn_ownership`, and `e2e_migrate` with `--ignored`. Then run `e2e_sqlite_db_values`, `e2e_sqlite_todo`, `e2e_sqlite_built_package`, and `cargo test --locked -p meshc --test e2e_stdlib e2e_sqlite`.
- [ ] Add one step to the Rust core job in `authoritative-verification.yml`, after the step named `Prove durable jobs on PostgreSQL`, that runs the script against the same `postgres:17` service.
- [ ] Write the Mesh fixture with these modes. `pg_point` reads one row by primary key with `Pool.query`. `pg_typed` does the same with `Pool.query_values`. `pg_write` inserts one row with `Pool.execute`. `pg_rows` reads 10,000 rows of 8 columns with `Pool.query_as`. `pg_sleep` runs `SELECT pg_sleep(0.05)`. `sqlite_point`, `sqlite_write`, and `sqlite_rows` mirror the first, third, and fourth modes.
- [ ] Run each mode from 1, 16, and 64 actors. Print a checksum of every result so that a wrong answer fails the run.
- [ ] Add a ticker actor to every mode. The ticker sleeps 1 ms in a loop and records how late each wake is, with the monotonic clock that `tools/performance/mesh/main.mpl` uses. Report the p99 and the maximum.
- [ ] In `bench.py`, put a counting TCP proxy between the fixture and PostgreSQL, with `sslmode=disable`. Count frontend `Sync` and `Query` messages as round trips. Count frontend `Parse` messages separately.
- [ ] In `bench.py`, accept `--baseline` and `--candidate` binaries. Shuffle the two inside each block, run at least five blocks, and write `run.json` and `comparison.csv` with the median, the range, p50, p99, the binary SHA-256, the host, and the PostgreSQL version. Follow the pattern in `tools/performance/quick.py`.
- [ ] Mark a trial invalid when any call fails or a checksum differs. Exit nonzero when a side has fewer than five valid blocks.

**You see.**

- [ ] `python3 tools/db-performance/bench.py --baseline <binary> --candidate <binary>` prints one table per mode and writes both files.
- [ ] `bash scripts/verify-db-drivers.sh` reports the live `db::` tests as run, not ignored.

**Verify, unit.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked.

- [ ] Add `python3 tools/db-performance/bench.py --self-test`. It checks the percentile math, the proxy counts on a scripted exchange, and the rejection of a missing sample. Run it.
- [ ] Run `bash scripts/verify-db-drivers.sh`.

**Verify, live.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked. Ten lanes on `claudeAgent/claude-opus-5-5 effort=xhigh` at the PR head, per the boot recipe.

- [ ] Lane 1. Regression lane against main. Run `bash scripts/verify-postgres-jobs.sh` on main and on head. Main has no `verify-db-drivers.sh`, so record that fact. Save `d1-lane-1.log`. Pass when both runs report the same passes.
- [ ] Lane 2. Run `verify-db-drivers.sh` with `MESH_TEST_DATABASE_URL` unset. Save `d1-lane-2.log`. Pass when the live `db::` tests ran and `.tmp/` holds no scratch directory afterward.
- [ ] Lane 3. Run the script with `MESH_TEST_DATABASE_URL` set to a running server. Save `d1-lane-3.log`. Pass when the script creates no cluster and the tests pass.
- [ ] Lane 4. Read the hosted Rust core job for the head. Save `d1-lane-4.log`. Pass when the new step ran and passed on Linux.
- [ ] Lane 5. Send SIGINT to the script while the tests run. Save `d1-lane-5.log`. Pass when no `postgres` process and no scratch directory remain.
- [ ] Lane 6. Run `bench.py` with the same binary on both sides, for `pg_point` at 1, 16, and 64 actors. Save `d1-lane-6.log`. Pass when the two medians differ by 5 percent or less.
- [ ] Lane 7. Read the proxy counts for one call of each kind on a reused idle connection. Save `d1-lane-7.log`. Pass when `Pool.query` shows 2 round trips, `Pool.query_values` shows 3, and each shows 1 `Parse`.
- [ ] Lane 8. Run the ticker with no database load, then during `pg_sleep` at 16 actors on one worker. Save `d1-lane-8.log`. Pass when the idle p99 is under 5 ms and the loaded run reports a higher p99 with at least 1,000 samples.
- [ ] Lane 9. Run the three SQLite modes. Save `d1-lane-9.log`. Pass when `sqlite_rows` returns 10,000 rows and every checksum matches a second run.
- [ ] Lane 10. Stop PostgreSQL in the middle of a trial. Save `d1-lane-10.log`. Pass when `bench.py` exits nonzero and reports the trial as invalid.

**Verify, perf.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked.

- [ ] Metric. The noise floor of each latency metric, and the main baseline for round trips, `Parse` count, p50, p99, calls per second, rows per second, and ticker p99.
- [ ] Probe. Run `bench.py` with the main binary on both sides, five shuffled blocks, on an idle machine. Check the load with `uptime` first.
- [ ] Baseline. Record main first and commit the summary to `tools/db-performance/results/<date>/`.
- [ ] Rule. D1 changes no driver behavior. Reject the benchmark when the two sides differ by more than 5 percent on any latency median, or when any mode has fewer than five valid blocks.

**Review gate.** None. D1 is not review-gated.

**Merge.**

- [ ] Root's clean verdict at the exact head SHA.
- [ ] The root starts the stack with D1 on base `main`. The operator lands it.

## Skip the health check on a recently used connection (D2)

**Depends on.** D1.

**Files.**

- [ ] Edit `compiler/mesh-rt/src/db/pool.rs` and `compiler/mesh-rt/src/db/pg.rs`.
- [ ] Edit `website/docs/docs/databases/index.md` and `docs/postgresql-binary-values.md`.

**Build.**

- [ ] Store the checkin time with each idle handle in `PoolInner.idle`. Set it in `mesh_pool_checkin`.
- [ ] In `checkout`, call `health_check` only when the connection sat idle for 500 ms or longer. Name the constant and mark it with a `// ponytail:` comment, as the fixed caps in `pg.rs` are marked.
- [ ] In `request` in `pg.rs`, when a read fails after the server sent an `ErrorResponse`, return the stored server failure. Today the `?` on `read_wire_message` drops it and the caller sees only an end-of-file error.
- [ ] Rewrite the tests `an_idle_connection_is_checked_without_holding_the_pool` and `an_idle_connection_that_died_is_replaced` so that each states the new rule.
- [ ] In the docs, say that the pool reuses a connection used in the last 500 ms without a check. Say that a query on a connection the server closed returns an error, that the pool discards the connection, and that the pool never retries a statement.

**You see.**

- [ ] Under load, the proxy shows 1 round trip for `Pool.query` and 2 for `Pool.query_values`.

**Verify, unit.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked.

- [ ] Cover three cases with the loopback `fake_server`. A recently used connection sends no `Q` before its query. A connection idle past the limit is checked first. A connection that died inside the limit returns the server's reason, and the next checkout opens a new connection. Run `cargo test -p mesh-rt --lib db::pool`.

**Verify, live.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked. Ten lanes on `claudeAgent/claude-opus-5-5 effort=xhigh` at the PR head, per the boot recipe.

- [ ] Lane 1. Regression lane against main. Run `pg_point` at 16 actors on main and head. Save `d2-lane-1.log`. Pass when the checksums match and round trips per call fall from 2 to 1.
- [ ] Lane 2. Run `pg_typed` at 16 actors on main and head. Save `d2-lane-2.log`. Pass when round trips per call fall from 3 to 2.
- [ ] Lane 3. Leave the pool idle for 2 s, then run one query. Save `d2-lane-3.log`. Pass when the proxy shows `SELECT 1` before the query.
- [ ] Lane 4. Call `pg_terminate_backend` on a connection that has been idle for 2 s, then run one query. Save `d2-lane-4.log`. Pass when the query succeeds on a new connection.
- [ ] Lane 5. Terminate one backend during the 16-actor load. Save `d2-lane-5.log`. Pass when each failed call names the server's reason, later calls succeed, and the pool returns to its maximum.
- [ ] Lane 6. Restart PostgreSQL during the 16-actor load. Save `d2-lane-6.log`. Pass when calls succeed again without a process restart and `pg_stat_activity` never shows more than the pool maximum.
- [ ] Lane 7. Run `bash scripts/verify-postgres-jobs.sh`. Save `d2-lane-7.log`. Pass when the Jobs proof passes.
- [ ] Lane 8. Read the hosted 18.7 and 19.0 compatibility jobs. Save `d2-lane-8.log`. Pass when both pass with TLS and SCRAM.
- [ ] Lane 9. Check in a connection with an open transaction, then borrow it again inside 500 ms. Save `d2-lane-9.log`. Pass when the proxy shows `ROLLBACK` at checkin and the borrower sees no open transaction.
- [ ] Lane 10. Hold the only connection of a one-connection pool past the checkout timeout. Save `d2-lane-10.log`. Pass when the second caller gets the timeout error within 50 ms of `timeout_ms`.

**Verify, perf.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked.

- [ ] Metric. Round trips per pooled call, and p50 of `pg_point` and `pg_typed` at 1, 16, and 64 actors.
- [ ] Probe. `bench.py --baseline <D1 head> --candidate <D2 head>`, five shuffled blocks.
- [ ] Baseline. Record the D1 head first.
- [ ] Rule. Fail when `Pool.query` shows more than 1 round trip under load, or when any p50 is above the base by more than the D1 noise floor.

**Review gate.** None. D2 is not review-gated.

**Merge.**

- [ ] Root's clean verdict at the exact head SHA.
- [ ] The root appends D2 to the stack on the D1 branch. The operator lands it.

## Reuse parsed statements on each connection (D3)

**Depends on.** D2.

**Files.**

- [ ] Edit `compiler/mesh-rt/src/db/pg.rs` and `compiler/mesh-rt/src/db/repo.rs`.
- [ ] Edit `docs/postgresql-binary-values.md` and `website/docs/docs/databases/index.md`.

**Build.**

- [ ] Add a statement cache to `PgConn`. Key it by SQL text. Give each entry a server statement name and, for the typed path, the result formats from its `Describe` reply. Cap it at 256 entries with least-recently-used eviction, and mark the cap with `// ponytail:`.
- [ ] On a miss, send `Parse` with the statement name in `write_parse`. Add the entry only after `request` sees `ParseComplete`. Today `request` ignores that message.
- [ ] On a hit, skip `Parse`. In `prepare`, skip the separate `Parse`, `Describe`, `Sync` exchange, so that `Pg.query_values` costs one round trip.
- [ ] On eviction, send `Close` for the statement at the start of the next request.
- [ ] Drop the entry on SQLSTATE `0A000`. Drop the whole cache on SQLSTATE `26000`, and on a `DISCARD ALL` or `DEALLOCATE` command tag.
- [ ] After either SQLSTATE, retry once with a fresh `Parse` only when `txn_status` was `I` before the call. Inside a transaction, return the server's error.
- [ ] Parse a `statement_cache=off` URL option in `parse_pg_url`. With it, send unnamed statements as today. Transaction-mode poolers need it, and Jobs phase J3 tests one.
- [ ] Leave the Simple Query path unchanged. `BEGIN`, `COMMIT`, the health check, and the Jobs migration use it.
- [ ] In `repo.rs`, bind `LIMIT` and `OFFSET` as parameters. Today the builder writes them into the SQL text, so every page is a different statement.
- [ ] Update `docs/postgresql-binary-values.md`, which promises unnamed statements and a describe step before every typed query.

**You see.**

- [ ] After the first use on a connection, the proxy shows no `Parse` for a repeated statement.
- [ ] A typed query on a warm connection costs 1 round trip.

**Verify, unit.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked.

- [ ] Extend the loopback peers in `pg.rs` to answer the extended protocol. Cover a hit that sends no `P`, an eviction that sends `Close`, a `0A000` retry outside a transaction, a `0A000` error inside one, a `26000` reset, a `DISCARD ALL` reset, and `statement_cache=off`. Run `cargo test -p mesh-rt --lib db::pg`.
- [ ] Assert the SQL that the repository builds for a paged query. Run `cargo test -p mesh-rt --lib db::repo`.

**Verify, live.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked. Ten lanes on `claudeAgent/claude-opus-5-5 effort=xhigh` at the PR head, per the boot recipe.

- [ ] Lane 1. Regression lane against main. Run `pg_point` at 16 actors on main and head. Save `d3-lane-1.log`. Pass when the checksums match and `Parse` per call falls from 1 to 0 after warm-up.
- [ ] Lane 2. Run `pg_typed` at 16 actors on the D2 head and on head. Save `d3-lane-2.log`. Pass when round trips per call fall from 2 to 1 after warm-up.
- [ ] Lane 3. Run `SELECT * FROM t`, add a column from a second session, and run the query again outside a transaction. Save `d3-lane-3.log`. Pass when the second result has the new column and the proxy shows exactly one retry.
- [ ] Lane 4. Repeat lane 3 inside `Pg.transaction`. Save `d3-lane-4.log`. Pass when the call returns the server's error, the transaction rolls back, and the next transaction succeeds.
- [ ] Lane 5. Run `Pool.execute` with `DISCARD ALL`, then a cached query. Save `d3-lane-5.log`. Pass when the query succeeds with no retry.
- [ ] Lane 6. Run 300 distinct statements on one connection. Save `d3-lane-6.log`. Pass when `pg_prepared_statements` for that backend never exceeds 256 rows.
- [ ] Lane 7. Connect with `statement_cache=off` and run the same statement 100 times. Save `d3-lane-7.log`. Pass when every call sends an unnamed `Parse` and `pg_prepared_statements` stays empty.
- [ ] Lane 8. Read 50 pages with `Repo.all` at 50 offsets. Save `d3-lane-8.log`. Pass when the proxy shows 1 `Parse` and the rows match main.
- [ ] Lane 9. Run `bash scripts/verify-postgres-jobs.sh` and read the hosted 18.7 and 19.0 jobs. Save `d3-lane-9.log`. Pass when all three pass.
- [ ] Lane 10. Run `e2e_migrate` with `--ignored`. Save `d3-lane-10.log`. Pass when migrations run up and down and later queries see the new schema.

**Verify, perf.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked.

- [ ] Metric. `Parse` messages per repeated call, round trips per `pg_typed` call, and p50 and p99 of `pg_point` and `pg_typed` at 1, 16, and 64 actors.
- [ ] Probe. `bench.py --baseline <D2 head> --candidate <D3 head>`, five shuffled blocks.
- [ ] Baseline. Record the D2 head first.
- [ ] Rule. Fail when a repeated statement sends a `Parse` after warm-up, when `pg_typed` takes more than 1 round trip, or when any p50 is above the base by more than the D1 noise floor.

**Review gate.** None. D3 is not review-gated.

**Merge.**

- [ ] Root's clean verdict at the exact head SHA.
- [ ] The root appends D3 to the stack on the D2 branch. The operator lands it.

## Park actors that wait for a pool connection (D4)

**Depends on.** D3.

**Files.**

- [ ] Edit `compiler/mesh-rt/src/db/pool.rs`.
- [ ] Edit `website/docs/docs/databases/index.md`.

**Build.**

- [ ] Replace `available: Condvar` in `PgPool` with a first-in, first-out waiter queue. Model each waiter on `Waiter` in `compiler/mesh-rt/src/channel.rs`, an actor or a thread.
- [ ] In `checkout`, when no connection is free, drop the pool lock, set the actor to waiting, and park it with `cooperative_recv_timeout`. A caller with no actor, such as a Jobs runner thread or `main`, blocks as it does today.
- [ ] In `mesh_pool_checkin`, hand the connection to the oldest waiter directly. Put it in `idle` only when nobody waits.
- [ ] On timeout, remove the waiter, then check once more for a connection that arrived in the same instant. Return that connection to the pool.
- [ ] Make one deadline cover the wait, the health check, and `open`. Today the deadline covers only the wait.
- [ ] Change every `mesh_pool_*` function that can park to `extern "C-unwind"`, as the comment above the ABI functions in `compiler/mesh-rt/src/actor/mod.rs` requires.
- [ ] Add a drop guard so that an actor killed while it waits leaves no waiter and loses no connection. The guard must not park.

**You see.**

- [ ] On one worker, an actor that waits 500 ms for a connection no longer stops the ticker.
- [ ] On one worker with a one-connection pool, an actor that yields inside `Repo.transaction` no longer makes a second actor's query time out.

**Verify, unit.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked.

- [ ] With the `in_actor` helper, cover handoff in arrival order, a timeout at the deadline that leaves no waiter, a killed waiter that leaves the pool whole, and a thread caller that still blocks and is served. Run `cargo test -p mesh-rt --lib db::pool`.

**Verify, live.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked. Ten lanes on `claudeAgent/claude-opus-5-5 effort=xhigh` at the PR head, per the boot recipe.

- [ ] Lane 1. Regression lane against main. On one worker with a one-connection pool, actor A yields inside `Repo.transaction` while actor B calls `Pool.query`. Run it on main and head. If main does not time out, record that. Save `d4-lane-1.log`. Pass when both actors finish on head with no timeout.
- [ ] Lane 2. Run `pg_point` from 64 actors with a pool maximum of 4. Save `d4-lane-2.log`. Pass when every call succeeds and the checksum matches main.
- [ ] Lane 3. On one worker, hold the only connection for 500 ms while a second actor waits. Save `d4-lane-3.log`. Pass when the ticker p99 during the wait is under 5 ms.
- [ ] Lane 4. Kill 100 actors while each waits for a connection. Save `d4-lane-4.log`. Pass when the pool then serves its maximum again and `pg_stat_activity` matches.
- [ ] Lane 5. Queue 8 waiters on a one-connection pool. Save `d4-lane-5.log`. Pass when they receive the connection in arrival order.
- [ ] Lane 6. Run `bash scripts/verify-postgres-jobs.sh`. Save `d4-lane-6.log`. Pass when the Jobs proof passes with its runner threads.
- [ ] Lane 7. Call `Pool.query` from `main` while actors hold every connection. Save `d4-lane-7.log`. Pass when the call returns after a checkin.
- [ ] Lane 8. Close the pool while 8 actors wait. Save `d4-lane-8.log`. Pass when each waiter gets the closed-pool error within 50 ms.
- [ ] Lane 9. Race a checkin against a waiter's timeout 10,000 times. Save `d4-lane-9.log`. Pass when the pool still holds its maximum afterward.
- [ ] Lane 10. Read the hosted compatibility matrix for the head. Save `d4-lane-10.log`. Pass when the Windows and musl builds pass.

**Verify, perf.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked.

- [ ] Metric. Ticker p99 on one worker during a 500 ms pool wait, and p50 of `pg_point` at 1 actor and at 64 actors with a pool maximum of 4.
- [ ] Probe. `bench.py --baseline <D3 head> --candidate <D4 head>`, five shuffled blocks.
- [ ] Baseline. Record the D3 head first.
- [ ] Rule. Fail when the ticker p99 during the wait is 5 ms or more, or when the 1-actor p50 is above the base by more than the D1 noise floor.

**Review gate.** None. D4 is not review-gated.

**Merge.**

- [ ] Root's clean verdict at the exact head SHA.
- [ ] The root appends D4 to the stack on the D3 branch. The operator lands it.

## Run SQLite writes off scheduler workers and make handles fail safely (D5)

**Depends on.** D1.

**Files.**

- [ ] Edit `compiler/mesh-rt/src/db/sqlite.rs` and `compiler/mesh-rt/src/actor/mod.rs`.
- [ ] Edit `website/docs/docs/databases/index.md`.

**Build.**

- [ ] Replace the raw `Box<SqliteConn>` pointer handle with an entry in a registry, keyed by an index and a generation. `mesh_sqlite_close` removes the entry. A second close does nothing. A call on a closed handle returns an error. Today both free or read freed memory.
- [ ] Start one thread per connection at `mesh_sqlite_open`. Give it a bounded queue and return an error when the queue is full, as `DurableWriter` in `compiler/mesh-rt/src/dist/continuity_store.rs` does.
- [ ] Prepare each statement on the caller's thread. Step it there only when `sqlite3_stmt_readonly` is true and `sqlite3_get_autocommit` shows no open transaction. A point read then costs what it costs today.
- [ ] Send every other statement to the connection thread. That covers writes, `BEGIN`, `COMMIT`, `ROLLBACK`, and every statement inside an open transaction.
- [ ] Add `cooperative_recv` next to `cooperative_recv_timeout` in `actor/mod.rs`. It parks with no deadline and arms no timer, because the timer thread never cancels an entry.
- [ ] Copy the SQL and the parameters out of the actor heap before the park. Build the Mesh result on the actor's thread after the wake. The collector scans only the actor's own stack.
- [ ] Change every `mesh_sqlite_*` function that can park to `extern "C-unwind"`.
- [ ] Keep the row cap, the byte cap, and their error text.
- [ ] In the docs, say that one connection has one transaction, and that actors that need separate transactions open separate connections.

**You see.**

- [ ] On one worker, a slow SQLite write no longer stops the ticker.
- [ ] A call on a closed handle returns an error.

**Verify, unit.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked.

- [ ] Cover statements from two actors that run in the order sent, a call after close, a double close, a full queue, and both caps. Run `cargo test -p mesh-rt --lib db::sqlite`.
- [ ] Cover `cooperative_recv` for a reply, a dropped sender, and a caller with no actor. Run `cargo test -p mesh-rt --lib actor::`.

**Verify, live.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked. Ten lanes on `claudeAgent/claude-opus-5-5 effort=xhigh` at the PR head, per the boot recipe.

- [ ] Lane 1. Regression lane against main. Run `e2e_sqlite_db_values`, `e2e_sqlite_todo`, and `sqlite_point` on main and head. Save `d5-lane-1.log`. Pass when the tests pass on both and the checksums match.
- [ ] Lane 2. On one worker, run a write that takes 200 ms while the ticker runs. Run it on main and head. Save `d5-lane-2.log`. Pass when the head ticker p99 is under 5 ms.
- [ ] Lane 3. Close a handle in one actor and query it from another. Save `d5-lane-3.log`. Pass when the query returns an error and the process exits with code 0.
- [ ] Lane 4. Close the same handle twice. Save `d5-lane-4.log`. Pass when the process exits with code 0.
- [ ] Lane 5. Insert 10,000 rows between `Sqlite.begin` and `Sqlite.commit`. Save `d5-lane-5.log`. Pass when the table holds 10,000 rows and a read inside the transaction sees them.
- [ ] Lane 6. Write from two actors that share one handle. Save `d5-lane-6.log`. Pass when every row is present and no call returns a misuse error.
- [ ] Lane 7. Kill an actor during a slow write, then use the same handle from another actor. Save `d5-lane-7.log`. Pass when the later statements succeed.
- [ ] Lane 8. Select 100,001 rows. Save `d5-lane-8.log`. Pass when the error text matches main.
- [ ] Lane 9. Open `:memory:`, create a table, and read it back across separate calls. Save `d5-lane-9.log`. Pass when the rows are present.
- [ ] Lane 10. Run `e2e_sqlite_built_package` and the hosted compatibility matrix. Save `d5-lane-10.log`. Pass when the scaffolded project passes and the Windows and musl builds pass.

**Verify, perf.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked.

- [ ] Metric. Ticker p99 on one worker during `sqlite_write`, and p50 of `sqlite_point` and `sqlite_write` at 1 and 16 actors.
- [ ] Probe. `bench.py --baseline <D1 head> --candidate <D5 head>`, five shuffled blocks.
- [ ] Baseline. Record the D1 head first.
- [ ] Rule. Fail when the ticker p99 is 5 ms or more, when `sqlite_point` p50 is above the base by more than the D1 noise floor, or when `sqlite_write` p50 at 1 actor is above 1.25 times the base.

**Review gate.** None. D5 is not review-gated.

**Merge.**

- [ ] Root's clean verdict at the exact head SHA.
- [ ] The root appends D5 to the stack on the D4 branch. The operator lands it.

## Open SQLite files in WAL mode with a busy timeout (D6)

**Depends on.** D5.

**Files.**

- [ ] Edit `compiler/mesh-rt/src/db/sqlite.rs`.
- [ ] Edit `website/docs/docs/databases/index.md`.

**Build.**

- [ ] Confirm the operator's approval of the default before any edit. The approval is a chat message.
- [ ] In `mesh_sqlite_open`, run `PRAGMA journal_mode = WAL` after the open. Treat a failure as a non-error and keep the mode the file has. A read-only file and a network file system both refuse WAL.
- [ ] Run `PRAGMA busy_timeout = 5000`, the value in `compiler/mesh-rt/src/dist/continuity_store.rs`.
- [ ] Record the journal mode on the connection. Step a read on the caller's thread only in `wal` or `memory` mode. In any other mode, send reads to the connection thread, so that a busy wait never sleeps on a worker.
- [ ] In the docs, describe the `-wal` and `-shm` files and state that copying only the main file can lose committed rows. Give the opt-out, `PRAGMA journal_mode = DELETE` after the open.

**You see.**

- [ ] A reader and a writer on two connections to one file both succeed. On main the reader gets `database is locked`.

**Verify, unit.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked.

- [ ] Cover a new file that reports `wal`, a `:memory:` database that reports `memory`, a read-only file that opens and reads, and a reader that succeeds while a second connection holds a write transaction. Run `cargo test -p mesh-rt --lib db::sqlite`.

**Verify, live.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked. Ten lanes on `claudeAgent/claude-opus-5-5 effort=xhigh` at the PR head, per the boot recipe.

- [ ] Lane 1. Regression lane against main. Read from 16 actors while one actor writes, each on its own connection, on main and head. Save `d6-lane-1.log`. Pass when head returns no `database is locked` error and the checksums match.
- [ ] Lane 2. Open a file that main created in rollback mode and that holds rows. Save `d6-lane-2.log`. Pass when every row reads back and `PRAGMA journal_mode` reports `wal`.
- [ ] Lane 3. Open a database file with the write bit removed. Save `d6-lane-3.log`. Pass when the open succeeds and a read returns rows.
- [ ] Lane 4. Open `:memory:` and run the D5 lane 9 scenario. Save `d6-lane-4.log`. Pass when the rows are present.
- [ ] Lane 5. Run two writers on two connections to one file. Save `d6-lane-5.log`. Pass when both finish inside the busy timeout with every row present.
- [ ] Lane 6. Run `PRAGMA journal_mode = DELETE` after the open, then repeat lane 1 on one worker. Save `d6-lane-6.log`. Pass when the ticker p99 stays under 5 ms while a reader waits.
- [ ] Lane 7. Kill the process during a write, then open the file again. Save `d6-lane-7.log`. Pass when every committed row is present and no partial row is.
- [ ] Lane 8. Run `e2e_sqlite_todo` and `e2e_sqlite_built_package`. Save `d6-lane-8.log`. Pass when both pass with the sidecar files present.
- [ ] Lane 9. Close the last connection, then list the directory. Save `d6-lane-9.log`. Pass when the `-wal` file is gone or empty and the main file holds every row.
- [ ] Lane 10. Run `bash scripts/verify-db-drivers.sh` and read the hosted compatibility matrix. Save `d6-lane-10.log`. Pass when the script passes and the Windows and musl builds pass.

**Verify, perf.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked.

- [ ] Metric. Failed calls in a 16-reader, one-writer run, and p50 of `sqlite_point` and `sqlite_write` at 1 and 16 actors.
- [ ] Probe. `bench.py --baseline <D5 head> --candidate <D6 head>`, five shuffled blocks.
- [ ] Baseline. Record the D5 head first, with its count of `database is locked` errors.
- [ ] Rule. Fail when head returns any `database is locked` error in the mixed run, or when either p50 is above the base by more than the D1 noise floor.

**Review gate.** None. D6 is not review-gated.

**Merge.**

- [ ] Root's clean verdict at the exact head SHA.
- [ ] The root appends D6 to the stack on the D5 branch. The operator lands it.

## Cut the work done for each result row (D7)

**Depends on.** D3 and D6.

**Files.**

- [ ] Edit `compiler/mesh-rt/src/db/pg.rs`, `compiler/mesh-rt/src/db/sqlite.rs`, and `compiler/mesh-rt/src/collections/map.rs`.

**Build.**

- [ ] Read PostgreSQL messages through a buffer owned by the connection. Today `read_message` makes three `read_exact` calls for each message, one each for the tag, the length, and the body.
- [ ] Allocate each column-name key once for a result set and share it across the rows. Today `row_map` in `pg.rs` and `read_rows` in `sqlite.rs` allocate every key again for every row.
- [ ] Add a bulk constructor beside `mesh_map_from_string_entries` that sizes the table once. Today a map of 8 entries or fewer is copied on every insert.
- [ ] Keep the `Map<String, String>` shape, the duplicate-column behavior, and `from_row` as they are. `from_row` is public API.

**You see.**

- [ ] `pg_rows` and `sqlite_rows` return the same 10,000 rows in less time.

**Verify, unit.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked.

- [ ] Compare the bulk constructor with insert-by-insert for 0, 1, 8, 9, and 64 entries and for a repeated key. Run `cargo test -p mesh-rt --lib collections::map`.
- [ ] Cover a message split across two reads, a length over the 64 MiB cap, and an end of file in the middle of a body. Run `cargo test -p mesh-rt --lib db::pg` and `cargo test -p mesh-rt --lib db::row`.

**Verify, live.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked. Ten lanes on `claudeAgent/claude-opus-5-5 effort=xhigh` at the PR head, per the boot recipe.

- [ ] Lane 1. Regression lane against main. Dump 1,000 rows with nulls, empty strings, non-ASCII text, and `BYTEA` as JSON on main and head, from both drivers. Save `d7-lane-1.log`. Pass when the dumps are byte-identical.
- [ ] Lane 2. Run `e2e_db_values` with `--ignored`. Save `d7-lane-2.log`. Pass when it passes.
- [ ] Lane 3. Run `e2e_sqlite_db_values`. Save `d7-lane-3.log`. Pass when it passes.
- [ ] Lane 4. Run `cargo test -p meshc --test e2e_stdlib e2e_deriving_row`. Save `d7-lane-4.log`. Pass when an empty string still decodes to `None` for an `Option` field.
- [ ] Lane 5. Run `SELECT 1 AS a, 2 AS a` on main and head. Save `d7-lane-5.log`. Pass when the two maps are equal.
- [ ] Lane 6. Read a result just under the 64 MiB cap and one just over it. Save `d7-lane-6.log`. Pass when each outcome matches main.
- [ ] Lane 7. Add a mode to the proxy that forwards one byte at a time, then run `pg_point`. Save `d7-lane-7.log`. Pass when the checksum matches.
- [ ] Lane 8. Run `postgres_repo_runs_every_repository_operation` with `--ignored`. Save `d7-lane-8.log`. Pass when it passes.
- [ ] Lane 9. Run `bash scripts/verify-postgres-jobs.sh`. Save `d7-lane-9.log`. Pass when the Jobs proof passes with its large contexts.
- [ ] Lane 10. Read the hosted 18.7 and 19.0 jobs. Save `d7-lane-10.log`. Pass when both pass over TLS.

**Verify, perf.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked.

- [ ] Metric. Rows per second and peak resident memory for `pg_rows` and `sqlite_rows`, and p50 of `pg_point`.
- [ ] Probe. `bench.py --baseline <D6 head> --candidate <D7 head>`, five shuffled blocks. Profile one unreported run to name what limits `pg_rows`.
- [ ] Baseline. Record the D6 head first.
- [ ] Rule. Fail when `pg_rows` is below 1.2 times the base, when `sqlite_rows` is below 1.1 times the base, or when peak memory is above the base. The two ratios are estimates. Replace them with the share of time that the D1 profile gives to row building, before D7 starts.

**Review gate.** None. D7 is not review-gated.

**Merge.**

- [ ] Root's clean verdict at the exact head SHA.
- [ ] The root appends D7 to the stack on the D6 branch. The operator lands it.

## Run PostgreSQL socket waits off scheduler workers (D8)

**Depends on.** D4, D5, and D7.

**Files.**

- [ ] Edit `compiler/mesh-rt/src/db/pg.rs`, `compiler/mesh-rt/src/db/pool.rs`, and `compiler/mesh-rt/src/db/jobs_connection.rs`.
- [ ] Edit `website/docs/docs/databases/index.md` and `website/docs/docs/concurrency/index.md`.

**Build.**

- [ ] Confirm that no Jobs J3 PR is open on `pg.rs` or `jobs_connection.rs` before the first edit.
- [ ] Give each `PgConn` one I/O thread. When an actor calls, send the encoded request to that thread and park the actor with `cooperative_recv` from D5.
- [ ] On the I/O thread, run the `request` loop and collect the raw reply messages, inside the existing 64 MiB cap. Build the Mesh rows on the actor's thread after the wake.
- [ ] When the caller has no actor, such as a Jobs runner thread or `main`, call the blocking path directly. That caller pays no thread hop.
- [ ] Route `open`, `health_check`, and the checkin `ROLLBACK` through the same thread when an actor calls them. Each blocks a worker today.
- [ ] Pass the Jobs read and write timeouts with the request. Today `jobs_connection.rs` sets them on the socket and restores them.
- [ ] Extend the D4 guard. When an actor dies during a query, the I/O thread finishes the exchange, rolls back an open transaction, and checks the connection in. The guard itself never parks.
- [ ] Change every `mesh_pg_*` function that can park to `extern "C-unwind"`.
- [ ] Stop the I/O thread when the connection closes.
- [ ] In the docs, state that a pool of N connections runs N I/O threads.

**You see.**

- [ ] On one worker, 16 actors that each run `SELECT pg_sleep(0.05)` finish in about 50 ms. On main the queries run one at a time, which is 800 ms by arithmetic.
- [ ] The ticker keeps its schedule while the queries run.

**Verify, unit.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked.

- [ ] With `in_actor` and the loopback peers, cover two actors on one worker whose queries overlap at the server, a killed actor whose connection returns to the pool, a killed actor inside a transaction whose server sees `ROLLBACK`, and a thread caller that takes the blocking path. Run `cargo test -p mesh-rt --lib db::`.

**Verify, live.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked. Ten lanes on `claudeAgent/claude-opus-5-5 effort=xhigh` at the PR head, per the boot recipe.

- [ ] Lane 1. Regression lane against main. On one worker, run `pg_sleep` from 16 actors with a pool of 16, on main and head. Save `d8-lane-1.log`. Pass when head finishes in 150 ms or less with the same results.
- [ ] Lane 2. On one worker, run `pg_point` from 16 actors with the ticker, on main and head. Save `d8-lane-2.log`. Pass when the head ticker p99 is under 5 ms.
- [ ] Lane 3. Kill 200 actors during their queries. Save `d8-lane-3.log`. Pass when the pool serves its maximum afterward and `pg_stat_activity` shows no backend idle in a transaction.
- [ ] Lane 4. Kill an actor after its insert and before its commit. Save `d8-lane-4.log`. Pass when the row is absent.
- [ ] Lane 5. Read 100,000 rows, then a result over the 64 MiB cap. Save `d8-lane-5.log`. Pass when the checksum and the error text match main.
- [ ] Lane 6. Restart PostgreSQL during the 16-actor load on one worker. Save `d8-lane-6.log`. Pass when the ticker p99 stays under 5 ms and calls succeed again afterward.
- [ ] Lane 7. Read the hosted 18.7 and 19.0 jobs. Save `d8-lane-7.log`. Pass when both pass with TLS and SCRAM.
- [ ] Lane 8. Run `bash scripts/verify-postgres-jobs.sh` on the D7 head and on head. Save `d8-lane-8.log`. Pass when both pass and the head run takes no more than 1.1 times as long.
- [ ] Lane 9. Call `Pg.connect` from an actor to an address that does not answer. Save `d8-lane-9.log`. Pass when the ticker p99 stays under 5 ms until the connect times out.
- [ ] Lane 10. Open a pool of 32, run load, then call `Pool.close`. Save `d8-lane-10.log`. Pass when the process gains at most 32 threads plus a fixed few, and loses them after the close.

**Verify, perf.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked.

- [ ] Metric. p50 and p99 of `pg_point` at 1 actor, calls per second at 64 actors on one worker and on the default workers, and ticker p99.
- [ ] Probe. `bench.py --baseline <D7 head> --candidate <D8 head>`, five shuffled blocks.
- [ ] Baseline. Record the D7 head first.
- [ ] Rule. Fail when the 1-actor p50 is above 1.25 times the base, when 64-actor throughput on one worker is below 4 times the base, or when the ticker p99 is 5 ms or more. A 1-actor failure sends the design to the readiness-wait alternative in Appendix B.

**Review gate.** None. D8 is not review-gated.

**Merge.**

- [ ] Root's clean verdict at the exact head SHA.
- [ ] The root appends D8 to the stack on the D7 branch. The operator lands it.

## Close the program

- [ ] Every box above is checked with its evidence, or its PR is recorded as skipped or stopped in Appendix A with the numbers.
- [ ] Commit the final `comparison.csv` of main against the stack tip to `tools/db-performance/results/<date>/`.
- [ ] Reply to the operator with the report that `playbooks/autopilot-stack.md` names.

## Appendix A. Prototype evidence

No prototype ran for this plan. The evidence is a code reading at main `7c162056` by four read-only explorers. The plan author spot-checked the claims marked checked.

Checked in the code:

- Nothing in `db/` or `jobs/` yields to the scheduler. `pg.rs` uses a blocking `std::net::TcpStream`, and `pool.rs` waits on a `parking_lot::Condvar`.
- `checkout` in `pool.rs` runs `SELECT 1` on every idle connection it lends. A pooled `Pool.query` costs 2 round trips, and a pooled `Pool.query_values` costs 3.
- `write_parse` always targets the unnamed statement. No statement is named, cached, or closed.
- After the handshake, every socket read and write goes through `request`, `send`, and `close` in `pg.rs`.
- `mesh_sqlite_open` passes only `SQLITE_OPEN_READWRITE | SQLITE_OPEN_CREATE` and sets no pragma. `mesh_sqlite_close` frees the handle with `Box::from_raw`, so a second close or a later call touches freed memory.
- `repo.rs` writes `LIMIT` and `OFFSET` into the SQL text.
- The live `db::` unit tests are `#[ignore]`d and no workflow runs them. No workflow runs the SQLite end-to-end tests.
- `http/server.rs` uses `mio` only for its listener. `ws/reactor.rs` is the only non-blocking reactor, and it has no general wait-for-readiness call.
- A coroutine resumes only on the worker that started it, so a blocked worker stalls every actor on it.

Inferred and not yet reproduced:

- A one-connection pool can time out when its holder yields and a second actor on the same worker waits on the `Condvar`. D4 lane 1 reproduces it on main first.
- Two actors can share one `SqliteConn`, because the type lowers to `Int`. D5 lanes 3 and 6 exercise it.
- One park and one wake cost a few microseconds, and one thread hop costs a few tens of microseconds. Nobody measured either.

Unmeasured, with the PR that measures each:

- Every latency, the noise floor, and the ticker delay on main. D1.
- The share of a pooled query that the health check takes. D1, then D2.
- The cost of a thread hop for one PostgreSQL query. D8, under its 1.25 times rule.
- The share of `pg_rows` time that row building takes. D1, before D7 starts.

### D1 benchmark calibration

D1 stopped after the redesigned benchmark missed the 5% same-binary latency
limit. Every later phase depends on D1. The driver branches are preserved for
a future qualification run.

The first Linux run used four scheduler workers. All 432 trials returned the
expected results, and 20 latency comparisons exceeded 5%.
[First Linux run](https://github.com/snowdamiz/mesh-lang/actions/runs/45054595892)

The redesign used one scheduler worker, longer point and write trials, and
balanced which side ran first. Both sides ran the same release binary built
from main at `e7abb534`. All 300 trials returned the expected results. Two
one-actor call p99 comparisons exceeded 5%, using the median of ten blocks per
side. [Redesigned Linux run](https://github.com/snowdamiz/mesh-lang/actions/runs/45057487695)

- PostgreSQL writes measured 780 µs and 890 µs, a 14.14% difference. The
  per-block p99 ranges were 711–2,293 µs and 777–2,339 µs.
- SQLite writes measured 1,635 µs and 1,385 µs, a 15.33% difference. The
  per-block p99 ranges were 956–2,759 µs and 1,096–3,346 µs.

The runner skipped the redesigned row and sleep tests after the write checks
failed. The run did not establish a noise floor for comparing driver changes.

### D8 PostgreSQL socket waits

D8 stopped after the connection-worker implementation and one shared-readiness
prototype missed the fourfold throughput target at 64 actors on one scheduler
worker. Every measured trial returned the expected results.

- With the counting proxy, the connection-worker implementation moved
  one-actor point-query p50 from 156 µs to 219 µs. At 64 actors, throughput rose
  from 7,054 to 11,139 calls/s, a 1.58× gain.
- The shared-readiness prototype measured an 18% one-actor p50 increase through
  the proxy. At 64 actors it delivered 20,811 calls/s against the D7 baseline of
  21,811 calls/s.
- A direct-socket probe removed the counting proxy. At 64 actors the prototype
  reached 106,014 calls/s against 41,464 calls/s on D7, a 2.56× gain. One-actor
  p50 was 28.1 µs against 22.1 µs.

The local same-binary calibration exceeded the 5% tolerance, so the timings are
local observations. The D8 implementation stays on the verification branch.
The release branch preserves D1 through D7.

## Appendix B. Alternatives rejected

- A `mio` reactor for PostgreSQL sockets. The only reactor, `ws/reactor.rs`, owns the WebSocket protocol and drives `rustls` by hand. A move of `PgStream` to non-blocking `rustls` rewrites the I/O in `pg.rs`. D8 uses one thread per connection, which the HTTP client and `DurableWriter` already do. If D8 fails its 1-actor rule, prototype a shared readiness thread that wakes the actor to do its own read.
- An automatic retry on a dead connection. Once bytes reach the server, the statement can have committed. D3 retries only on two errors that the server raises before it runs the statement, and only outside a transaction.
- A background health check on a timer. A limit on idle time is a smaller change and needs no extra thread. HikariCP skips its check for a connection used in the last 500 ms. [HikariCP pool source](https://code.yawk.at/com.zaxxer/HikariCP/3.4.5/com/zaxxer/hikari/pool/HikariPool.java)
- Every SQLite statement on the connection thread. A cached point read costs less than a thread hop, so D5 keeps read-only statements on the caller's thread.
- A typed decoder in place of `from_row`. `from_row` takes a `Map<String, String>` and user code calls it directly. A typed decoder is a second generated function with new query functions, which is a new API. D7 removes the per-row waste inside the current shape.
- New steps in `scripts/verify-postgres-jobs.sh`. Jobs phase J1 extends that script, so this plan has its own.
- A compile-time ownership rule for `SqliteConn`, as `PgConn` has. The rule rejects programs that compile today. D5 makes a bad handle an error at run time with no source change.
- A new database, replicated SQLite, and durable actors. The operator scoped this plan to fixes.

## Appendix C. Risks

- D2. Inside the 500 ms limit, a caller can get an error from a connection that the server closed. Today the pool replaces that connection silently. The caller decides whether to retry.
- D3. A named statement can switch to a generic plan after five runs, and that plan can be slower on skewed data. The server holds up to 256 statements for each connection. `statement_cache=off` is the way out for both, and for transaction-mode poolers.
- D4. Handoff in arrival order changes which waiter gets a connection. A guard that parks during a forced unwind aborts the process, so the owner tests the kill paths first.
- D5. A long read-only scan still blocks its worker, as it does today. Two threads use one SQLite connection, which is safe only in the serialized mode that the bundled build uses.
- D6. WAL is stored in the file and stays after the program exits. A reader can still wait on the busy timeout on its worker during WAL recovery. An older Mesh binary opens a WAL file without trouble, because SQLite reads the mode from the file.
- D7. Shared key strings rely on Mesh strings being immutable.
- D8. Every query from an actor pays a thread hop. The 1-actor rule bounds it. D8 also edits the two files that Jobs J3 edits, so the two must not overlap.
- The HTTP server blocks its worker on socket reads by design, as the header of `http/server.rs` says. A request handler that queries a database gains less than the ticker numbers show. This plan does not change the HTTP server.
- D1. The live `db::` tests were written for the coverage container in `scripts/compiler-coverage.py`, which uses TLS and md5 passwords. The D1 owner finds what each test needs from the server before the script runs them in CI.
- Each push costs about 33 minutes of Authoritative verification. The compatibility matrix does not block a merge, so a Windows failure needs a person to read it. The database end-to-end tests are `#![cfg(unix)]`, so Windows gets a build check only.

Later scope, outside this plan:

- A typed row decoder, if row parsing still limits `pg_rows` after D7.
- A PostgreSQL cancel request when a caller gives up on a query.
- An idle timeout and a maximum lifetime for pooled connections.
- A statement cache for SQLite.
- Non-blocking reads in the HTTP server.

## Appendix D. Links and reading list

- Read first. `compiler/mesh-rt/src/db/pg.rs`, `pool.rs`, `sqlite.rs`, `row.rs`, and `jobs_connection.rs`.
- Parking and waking. `cooperative_channel` and `cooperative_recv_timeout` in `compiler/mesh-rt/src/actor/mod.rs`, `Waiter` in `compiler/mesh-rt/src/channel.rs`, and `DurableWriter` in `compiler/mesh-rt/src/dist/continuity_store.rs`.
- The other plan. [Durable Jobs and Oban parity plan](durable-jobs-oban-parity.md).
- User docs to keep true. [Databases guide](../website/docs/docs/databases/index.md) and [PostgreSQL binary values](postgresql-binary-values.md).
- Benchmark pattern. `tools/performance/quick.py` and `tools/performance/contract.md`.
- Protocol and engine references. [PostgreSQL message flow](https://www.postgresql.org/docs/19/protocol-flow.html) and [SQLite write-ahead logging](https://sqlite.org/wal.html).
- D3, D5, and D8 get `<plugin root>/skills/how/SKILL.md` before the first edit and `<plugin root>/skills/interrogate/SKILL.md` before the PR opens. Their designs carry cache invalidation, thread ownership, and kill safety.
- Keep the decision trail per `<plugin root>/skills/show-me-your-work/SKILL.md`.
