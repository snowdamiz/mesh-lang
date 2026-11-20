# Durable Jobs and Oban parity plan

Bring Mesh Jobs up to Oban 2.24.2 Core for PostgreSQL-backed applications. Preserve transactional enqueue, at-least-once execution, renewable claim fencing, isolated worker termination, and database-wide admission limits.

PRs #37 and #38 are the starting implementation on main. J1 through J7 are future implementation phases. Compare their behavior against both Mesh main and the pinned Oban Basic engine. Track other database engines and Oban Pro features as separate targets.

Commands, implemented behavior, and recorded qualification results are in the [PostgreSQL Jobs evidence guide](durable-jobs-parity-evidence.md).
The [compiled lane receipts](durable-jobs-parity-evidence.md#compiled-lane-receipts)
give the tested platform, PostgreSQL version, and assertion for the checked macOS lanes.

## How to read this

One box is one unit of work. Each box names the evidence that checks it. Check a box only when its evidence exists. Use `playbooks/orchestrate.md` if the program is assigned across several PRs, or execute the graph one PR at a time. Future implementation PRs follow the merge instructions authorized for that run.

Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked.

### First release scope

Qualify PostgreSQL 17, 18, and 19 on Linux and macOS. Record the tested patch versions. Add Windows to the matrix only after the compiled timeout, parent-death, and recovery suite passes there. Publish functional parity only after J7. Publish throughput claims per measured workload.

| Oban PostgreSQL workflow | Mesh contract and expected difference | Phase and required proof |
| --- | --- | --- |
| Transactional single and bulk enqueue | Keep `Jobs.enqueue_in` on the caller's connection and preserve request-key receipts. | J1 and J7. Commit and rollback a batch with a business write. Verify replay with the same request identity. |
| Uniqueness and replacement | Preserve field, key, state, window, and replacement policy. An active attempt keeps its claimed context, token, worker, queue, and schedule. Allowed field edits affect stored state and later attempts, including settlement decisions that consult the row. | J6 and J7. Race a duplicate insert and an allowed replace against an active attempt. |
| Scheduled and recurring work | Preserve due times, cron cursor commits, and missed-occurrence policies. | J3 and J6. Prove pickup after a missed wakeup and takeover after the cron owner dies. |
| Snooze, retry, and exhaustion | Automatic retry keeps the effect key. Snooze preserves the attempt budget. An operator retry of terminal work starts a new run with a new effect key. | J5 through J7. Assert due times, observed attempts, terminal states, and the old and new effect keys. |
| Cancellation and crash recovery | Committed cancellation takes precedence over settlement. Renewable tokens fence old attempts and scheduled recovery preserves the retry budget. | J2 and J7. Lose a worker or an acknowledgement, race cancellation, and inspect both durable state and effects. |
| Queue controls and concurrency | Keep namespace-wide limits on active claims and add per-runner controls. | J2 and J6. Race multiple workers and verify both scopes during queue shutdown and node loss. |
| Testing and bounded drain | Provide manual assertions, test-only direct invocation, and real PostgreSQL drain with an explicit budget and cutoff. | J5. Prove transactional visibility, namespace isolation, cutoff, exhaustion, and error policy. |
| Telemetry and maintenance | Report confirmed lifecycle outcomes and bounded maintenance work without exposing job payloads by default. | J4 and J6. Verify commit ordering, subscriber backpressure, receipt replay after pruning, and index maintenance under traffic. |

The workflow matrix follows the pinned Oban API and uniqueness guide. Execution models and effect-idempotency rules remain specific to each engine. [Oban 2.24.2 API](https://oban.hexdocs.pm/Oban.html) · [Oban uniqueness](https://oban.hexdocs.pm/2.24.2/unique_jobs.html)

## Program checklist

### Arm the program

- [ ] Read the plan, the documented Jobs API, and the current comparison links with `cat <plugin root>/skills/poteto-mode/playbooks/orchestrate.md` available for a coordinated run.
- [ ] Record the current main SHA, the owner for each active PR, and the actual PostgreSQL server versions. If the operator requests an automated audit, use an hourly schedule_task and post a status message only for a changed verdict, check, or blocker.

### Spawn owners

- [ ] Start J1 first. J2, J4, and J5 may follow independently. Start J3 and J6 after J2. Start J7 once J2 through J6 pass.
- [ ] Keep changes to shared Jobs dispatch, store, and runtime files in separate commits. Coordinate integration if independent PRs touch the same paths.

### PR mechanics

- [ ] Base each implementation PR on current main or its explicit dependency. Verify the diff before asking for review.
- [ ] Run the smallest behavior test while iterating. Before presenting the PR, run the applicable workspace and proof gates from `AGENTS.md`.
- [ ] Record each benchmark configuration before measuring and attach the raw samples to the PR evidence.

### Verdict and merge

- [ ] Obtain an independent verdict for the exact reviewed head. Exercise the compiled API and PostgreSQL transaction boundaries where the PR changes runtime behavior.
- [ ] Merge in dependency order after the current checks and reviews pass. Re-run verification where a rebase or a change of base alters behavior.

### Boot recipe

- [ ] Check out the pinned head in its own worktree. Run `bash scripts/verify-postgres-jobs.sh` for the standard proof. Use `--compatibility 18.7` or `--compatibility 19.0` for the versioned matrix.
- [ ] For load and fault scenarios, have the workload driver start its own disposable database and run both compiled fixtures. Require a freshly created database and a driver-owned nonce before kill, disconnect, or failover injection. Use a second connection for independent observation.
- [ ] Save each lane's command output and database evidence under `target/jobs-parity/<head>/<lane>/`. Shut down PostgreSQL and remove its temporary state.

Run the current proof with the matching PostgreSQL binaries on `PATH` or in `PG_BIN_DIR`. Replace the example installation paths with local paths. These commands clear connection overrides so the script creates a disposable cluster. The local fixture's bootstrap role can create that cluster. The role verification runs workers under separately scoped grants. [Authoritative verification workflow](../.github/workflows/authoritative-verification.yml)

```sh
env -u MESH_TEST_DATABASE_URL -u MESH_JOBS_TEST_PG_URL \
  PG_BIN_DIR=/path/to/postgresql-18.7/bin \
  bash scripts/verify-postgres-jobs.sh --compatibility 18.7
env -u MESH_TEST_DATABASE_URL -u MESH_JOBS_TEST_PG_URL \
  PG_BIN_DIR=/path/to/postgresql-19.0/bin \
  bash scripts/verify-postgres-jobs.sh --compatibility 19.0
```

## Measure the current queue against Oban (J1)

**Depends on.** The hard-timeout and runner-reliability stack in PRs #37 and #38.

**Files.**

- [x] Extend `scripts/verify-postgres-jobs.sh` and add a separate load and fault driver under `scripts/`.
- [x] Reuse `compiler/meshc/tests/e2e_durable_jobs.rs` and `tests/e2e/durable_jobs/main.mpl` for compiled scenarios. Keep the Oban comparison fixture pinned to `2.24.2` with its lockfile.

**Build.**

- [ ] Record an exact Mesh SHA, Oban and Erlang versions, PostgreSQL server version, host and runner settings, and workload seed with every receipt. Use `db606548` as the pre-reliability reference and `59f6a7b3` as the post-reliability starting point. Capture both engines on the same machine and database configuration using separate job namespaces or databases.
- [ ] Equalize effective concurrency. Oban Basic limits individual producers. Mesh also limits queue admission across the database. Keep job bodies, transaction and payload sizes, schedules, retry policies, and effect idempotency equivalent. Account for Mesh's per-attempt process startup as part of user-visible latency.
- [ ] Run warm-up and five alternating trials per engine at one, four, and eight runners. Use concurrency profiles of 1, 16, and 64 total slots where they fit, a warm short job and a 100 ms job, 1 KiB and 16 KiB arguments, sparse queues, and a separate large-context stress run. Mark production-shaped workloads separately from a no-op stress case.
- [ ] Freeze measured profiles, slot allocation, pool sizes, notifier and maintenance settings, and absolute sparse-latency budgets before optimizing. Define pickup as first handler entry minus the enqueue transaction's durable commit time. Capture commit time on the disposable database with `track_commit_timestamp` and align clocks on the same host. Record commit-to-claim and commit-to-settlement separately. Reject timing samples outside the declared clock tolerance.
- [ ] Freeze resource budgets per profile before optimizing. Include Mesh child processes and the Oban VM in totals. Proposed relative limits for Mesh are 1.25 times Oban CPU seconds per completed job and 1.5 times its peak RSS. Use the same absolute connection budget for both engines, initially total slots plus four connections per runner. Measure OS process and thread peaks against the same caps, initially two processes and eight threads per slot plus twelve per runner and 64 per host. Record peak and steady-state samples separately.
- [ ] Keep raw executions separate from deduplicated effects. Capture fault placement and reconcile each engine under its configured recovery rules. Install a barrier to kill or disconnect on an observed claim, write, or acknowledgement. Run the compiled commit, rollback, clean exit, and crash-recovery baseline on Linux and macOS before optimizing. Add native-call timeout on both and executable replacement on macOS.

**You see.**

- [x] A repeatable command produces a manifest and machine-readable evidence for both engines, with test failure distinguished from a queue defect.

**Verify, unit.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked.

- [ ] Test fixture data equality, monotonic latency accounting, percentiles, and rejection of missing or mismatched samples. Run the driver self-check and `bash scripts/verify-postgres-jobs.sh`.

**Verify, live.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked. Ten lanes on `gpt-6.2-sol high` at the PR head using the compiled fixtures and the PostgreSQL boot recipe.

- [ ] Lane 1. Regression lane against main. Run the existing transaction and recovery proof on main and head. Save `j1-lane-1.log`. Pass when the added driver reports the same persisted effects.
- [ ] Lane 2. Run the compiled Jobs workload on PostgreSQL 17 on Linux and macOS. Save `j1-lane-2.log`. Pass when the recorded patch version matches the server and expected claims, persisted settlements, and recovery pass on both hosts.
- [ ] Lane 3. Run PostgreSQL 18.7 with SCRAM over TLS on Linux and macOS. Save `j1-lane-3.log`. Pass when authentication and the compiled transaction, workload, settlement, and recovery checks pass.
- [ ] Lane 4. Run PostgreSQL 19.0 with SCRAM over TLS on Linux and macOS. Save `j1-lane-4.log`. Pass when authentication and the compiled transaction, workload, settlement, and recovery checks pass.
- [x] Lane 5. Run the Oban Basic fixture twice with the same seed. Save `j1-lane-5.log`. Pass when the derived inputs and total job counts match.
- [ ] Lane 6. Run the matched sparse workload. Save `j1-lane-6.log`. Pass when every eligible committed job is accounted for.
- [x] Lane 7. Run the matched dense workload. Save `j1-lane-7.log`. Pass when the run records occupancy and all completed jobs.
- [ ] Lane 8. Kill a worker after its claim and before settlement. Save `j1-lane-8.log`. Pass when the same job identity is recovered and the old token cannot settle it.
- [ ] Lane 9. Lose an acknowledgement on both sides of commit. Save `j1-lane-9.log`. Pass when the final persisted state accounts for each attempt.
- [ ] Lane 10. Run a rolled-back enqueue and a duplicate effect. Save `j1-lane-10.log`. Pass when the rollback produces zero effects and the effect key deduplicates the retry.

**Verify, perf.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked.

- [ ] Metric. Jobs per second, pickup p99, SQL calls per job, and idle queries per minute.
- [ ] Probe. Run five paired main and head trials with the same workers and publish the raw artifacts for both.
- [ ] Baseline. Record main first and measure Oban separately under the same conditions.
- [ ] Rule. Baseline-only change. Reject a driver that samples fewer than five valid pairs or omits any configured metric. Run measurements with the driver out of the worker hot path.

**Review gate.** None. This phase has no graphical interaction. Independent review uses the CLI receipts and database proof.

**Merge.**

- [ ] Land J1 after its dependencies and exact-head checks pass. Record the SHA and evidence in the program receipt.

## Batch claims and protect lease progress (J2)

**Depends on.** J1.

**Files.**

- [x] Edit `compiler/mesh-rt/src/jobs/schema.sql`, `compiler/mesh-rt/src/jobs/store.rs`, and `compiler/mesh-rt/src/jobs/runner.rs`.
- [x] Extend `compiler/mesh-rt/src/jobs/store_tests.rs` and `compiler/mesh-rt/src/jobs/runner_tests.rs` with batch and failure tests.

**Build.**

- [x] Add a versioned `claim_batch` entry point. Under one `READ COMMITTED` transaction, take the queue admission lock once, measure global headroom, and select the ordered `SKIP LOCKED` candidates by priority, schedule, and position. Limit the result by local reserved slots, global headroom, and an initial quantum of 16, then rotate queues. Document that continuous higher-priority arrivals can delay lower-priority work in the same queue.
- [x] Return one PostgreSQL row per attempt. Enforce per-context and total reply byte budgets before creating claims, because JSONB can expand a small enqueue into a large context. Start with the existing 16 MiB per-column ceiling and a 32 MiB total reply budget under the driver's 64 MiB result cap. Expose an oversized-context error that does not block valid work behind it.
- [ ] Dispatch only after confirmed commit. Recover ambiguous claims by lease and refund only confirmed unstarted claims. Preserve attempt accounting, effect keys, cancellation priority, and stale-token fencing. Keep the original SQL entry points, bodies, and role grants available during upgrades.
- [x] Give renewal and settlement bounded database access independent of claim traffic. Reserve capacity or use bounded independent I/O so that a slow claim or a locked row does not stop another queue's renewals. Retain the per-attempt process until measurements justify a different lifetime design.

**You see.**

- [x] A ready backlog fills available slots in bounded batches. A sparse eligible queue gets service on its next turn when it has available capacity.

**Verify, unit.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked.

- [ ] Cover capacity, SQL result size, ordered prefixes, queue rotation, unstarted refunds, and same-token retries. Run `bash scripts/verify-postgres-jobs.sh`.

**Verify, live.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked. Ten lanes on `gpt-6.2-sol high` at the PR head using the compiled fixtures and the PostgreSQL boot recipe.

- [ ] Lane 1. Regression lane against main. Run the mixed short-job workload on main and head. Save `j2-lane-1.log`. Pass when head preserves all job IDs and results.
- [x] Lane 2. Race four compiled runners on a queue at capacity. Save `j2-lane-2.log`. Pass when claims never exceed the configured global cap.
- [ ] Lane 3. Pause and scale during a batch. Save `j2-lane-3.log`. Pass when no new claims bypass the committed queue control.
- [x] Lane 4. Hold one row through renewals and acknowledgements. Save `j2-lane-4.log`. Pass when an unrelated attempt completes on its first lease.
- [x] Lane 5. Inject a blocked claim transport with separate renewal capacity. Save `j2-lane-5.log`. Pass when the active peer keeps its lease.
- [ ] Lane 6. Inject lost claim responses before and after commit. Save `j2-lane-6.log`. Pass when the runner launches only confirmed claims and recovery handles ambiguous rows.
- [ ] Lane 7. Fail a child launch partway through a batch, then kill the runner during a separate startup whose result is uncertain. Save `j2-lane-7.log`. Pass when only proven unstarted rows get refunds, held slots return, uncertain rows recover under a new token, and the runner never launches twice or settles stale with the old token.
- [ ] Lane 8. Mix large and small contexts in a backlog. Save `j2-lane-8.log`. Pass when every successful reply stays within the byte budget and oversized input does not block the queue.
- [ ] Lane 9. Run old and new binaries with scoped function grants. Save `j2-lane-9.log`. Pass when both versions process their eligible jobs and the old migration cannot change new entry points.
- [ ] Lane 10. Burden a busy queue while a sparse queue becomes due. Save `j2-lane-10.log`. Pass when the sparse queue claims on its next available rotation.

Run lane 7 on macOS after checking out the proof commit. Set `PG_BIN_DIR`
to the chosen PostgreSQL installation if it is outside the shell's path.

```bash
python3 scripts/jobs-parity.py --j2-lane-7 --mesh-batch --mesh-ref "$(git rev-parse HEAD)"
```

The driver prints its evidence directory and writes `j2-lane-7.log` after
both cases pass. The lane build enables a fixture-only gate at the second
OS spawn call and a gate at the existing native worker-start marker.

**Verify, perf.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked.

- [ ] Metric. Claim round trips per job, steady-state throughput, pickup p99, and peak queued context bytes.
- [ ] Probe. Interleave J1 dense and sparse workloads with batching enabled and disabled.
- [ ] Baseline. Record main with one-row claim and identical concurrency before changing defaults.
- [ ] Rule. Require at least 30% fewer claim round trips per job on the dense reference workload. Require sparse pickup p99 no worse than 1.10 times main and no reply above 32 MiB.

**Review gate.** None. This phase has no graphical interaction. Independent review uses the CLI receipts and database proof.

**Merge.**

- [ ] Land J2 after its dependencies and exact-head checks pass. Record the SHA and evidence in the program receipt.

## Wake workers after database commits (J3)

**Depends on.** J1 and J2.

**Files.**

- [x] Edit `compiler/mesh-rt/src/db/pg.rs`, `compiler/mesh-rt/src/db/jobs_connection.rs` and `compiler/mesh-rt/src/jobs/runner.rs`.
- [x] Extend `compiler/mesh-rt/src/jobs/schema.sql` and the compiled PostgreSQL fixtures.

**Build.**

- [x] Decode PostgreSQL asynchronous notification frames on a dedicated, interruptible session. Reuse the existing TLS and SCRAM transport. Keep partial reads framed across wakeups and prevent the listener from occupying a lease or settlement pool slot.
- [x] Commit `LISTEN` before the initial catch-up scan. Repeat that order on reconnect and after failover. Coalesce namespace and queue hints without including payloads or claim tokens. Treat hints as a reason to read authoritative rows.
- [x] Wake on committed enqueue and queue resume, and use due-time timers for scheduled or retryable jobs. Keep jittered fallback polling for outages, missed hints, and transaction poolers. Make transactional publishing opt-in until commit coupling and resource use pass the gate. Document that PostgreSQL can reject a commit if its notification queue is full.
- [x] Make the subscription and shutdown path release its session within the runner's shutdown budget. Define a poll-only setting and document connection cost.

**You see.**

- [x] A remote runner picks up newly committed work without waiting for the configured fallback poll. With notifications disabled, the same job still completes through polling.

**Verify, unit.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked.

- [ ] Cover partial and coalesced wire frames, reconnect order, subscription fallback, and bounded wake queues. Run the PostgreSQL driver tests and `bash scripts/verify-postgres-jobs.sh`.

**Verify, live.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked. Ten lanes on `gpt-6.2-sol high` at the PR head using the compiled fixtures and the PostgreSQL boot recipe.

- [x] Lane 1. Regression lane against main. Run the sparse queue workload on main and head. Save `j3-lane-1.log`. Pass when both run the committed job and head also records its wake source.
- [x] Lane 2. Enqueue in a transaction and delay commit. Save `j3-lane-2.log`. Pass when the worker sees no job until commit.
- [x] Lane 3. Roll back a transaction with enqueue. Save `j3-lane-3.log`. Pass when there is no execution or committed notification.
- [x] Lane 4. Resume a paused queue from a second process. Save `j3-lane-4.log`. Pass when the eligible work wakes the configured worker.
- [x] Lane 5. Schedule work with no fresh enqueue. Save `j3-lane-5.log`. Pass when the due-time path claims it on schedule.
- [x] Lane 6. Interrupt the listener before an enqueue. Save `j3-lane-6.log`. Pass when fallback polling picks up the job.
- [x] Lane 7. Commit during listener reconnect. Save `j3-lane-7.log`. Pass when the resubscribe scan sees the job.
- [x] Lane 8. Split and interleave notification frames over the test transport. Save `j3-lane-8.log`. Pass when the listener sees valid hints and handles invalid frames without corrupting query state.
- [x] Lane 9. Use a transaction pooler or poll-only mode. Save `j3-lane-9.log`. Pass when jobs process without a held listener session.
- [x] Lane 10. Stop an idle runner with an active listener. Save `j3-lane-10.log`. Pass when its session is released and the process exits within its shutdown budget.

**Verify, perf.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked.

- [ ] Metric. Commit-to-claim p99 and idle SQL queries per minute.
- [ ] Probe. Run the J1 sparse profile with a 1 s fallback poll and repeat with the listener disconnected.
- [ ] Baseline. Measure polling first at both the current default cadence and the same 1 s interval.
- [ ] Rule. Target p99 no greater than 100 ms on a healthy local database with free slots, fallback pickup within two polls, and at least 50% fewer idle polling queries than the current default.

**Review gate.** None. This phase has no graphical interaction. Independent review uses the CLI receipts and database proof.

**Merge.**

- [ ] Land J3 after its dependencies and exact-head checks pass. Record the SHA and evidence in the program receipt.

## Publish job lifecycle events (J4)

**Depends on.** J1. This can run in parallel with dispatch work.

**Files.**

- [x] Edit `compiler/mesh-rt/src/jobs/runner.rs`, `compiler/mesh-rt/src/jobs/store.rs`, and `compiler/mesh-rt/src/jobs/mod.rs`.
- [x] Add the event reader or subscriber API through the Jobs compiler and REPL bindings. Update `website/docs/docs/concurrency/index.md`.

**Build.**

- [x] Add a versioned event envelope for runner start and stop, attempt start, committed settlement, lost lease, stale or uncertain acknowledgement, storage failures, and maintenance. Include job and attempt identities in events. Keep them out of metric labels.
- [x] Report queue wait, handler duration, and bounded failure kinds. Reuse the database-confirmed settlement enum so a retry at exhaustion appears as discarded and a concurrent operator cancellation wins. Keep existing `outcomes` and `stale_acks` counts.
- [x] Isolate event delivery with a bounded buffer and dropped-event counter. A slow, failing, or absent consumer must leave leases and shutdown responsive. Provide an opt-in JSON logger and a programmatic reader or subscriber. Omit args, metadata, credentials, and lease tokens by default.
- [x] Document which events reflect an observed commit. An ambiguous network response produces an uncertainty event and cannot fabricate a committed result.

**You see.**

- [x] An application can trace why a job retried or stopped from structured events and can detect a saturated event consumer.

**Verify, unit.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked.

- [ ] Test event schemas, privacy, same-token retries, and subscriber overflow. Run Jobs tests plus compiler, REPL, and compiled API coverage for the public binding.

**Verify, live.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked. Ten lanes on `gpt-6.2-sol high` at the PR head using the compiled fixtures and the PostgreSQL boot recipe.

- [x] Lane 1. Regression lane against main. Compare default behavior on main and head under the same job stream. Save `j4-lane-1.log`. Pass when with telemetry off, both leave the same durable results.
- [x] Lane 2. Complete a healthy attempt. Save `j4-lane-2.log`. Pass when the terminal event follows the durable completion.
- [x] Lane 3. Request a retry on the last attempt. Save `j4-lane-3.log`. Pass when the event reports discard.
- [x] Lane 4. Race operator cancellation and handler success. Save `j4-lane-4.log`. Pass when the terminal event reports cancellation.
- [ ] Lane 5. Inject a stale token. Save `j4-lane-5.log`. Pass when only the stale acknowledgement indicator changes.
- [ ] Lane 6. Drop a commit reply. Save `j4-lane-6.log`. Pass when an uncertainty event appears without a duplicate terminal disposition.
- [ ] Lane 7. Run a cron tick and expired claim recovery. Save `j4-lane-7.log`. Pass when both maintenance events identify the action and their counts.
- [ ] Lane 8. Use an intentionally blocked subscriber. Save `j4-lane-8.log`. Pass when the drop count advances while lease renewal and other queues continue.
- [x] Lane 9. Enqueue synthetic sensitive arguments and metadata. Save `j4-lane-9.log`. Pass when default output contains none of those values.
- [x] Lane 10. Stop a runner with subscribers attached. Save `j4-lane-10.log`. Pass when subscribers terminate cleanly and shutdown progress is observable.

**Verify, perf.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked.

- [x] Metric. The dense telemetry trials record throughput, pickup p99, dropped events, sampled peak bytes, and the fixed bound. The unread reader reached the bound and reported drops in every measured trial. [Telemetry measurement](durable-jobs-parity-evidence.md#measure-runner-telemetry)
- [ ] Probe. Interleave J1 workloads with events disabled, consumed normally, and deliberately backpressured.
- [ ] Baseline. Record the same workloads on main without an event stream.
- [ ] Rule. Require disabled throughput and p99 within 5% of main. Require enabled throughput within 10% on the dense profile and enforce the configured fixed buffer bound.

**Review gate.** None. This phase has no graphical interaction. Independent review uses the CLI receipts and database proof.

**Merge.**

- [ ] Land J4 after its dependencies and exact-head checks pass. Record the SHA and evidence in the program receipt.

## Make queue behavior easy to test from Mesh (J5)

**Depends on.** J1. Build on the existing compiled drain and settlement counts.

**Files.**

- [x] Edit `compiler/mesh-test-rt/src/lib.rs` and the compiler's Jobs type, codegen, and REPL bindings where test APIs need them.
- [x] Add compiled examples and tests under `tests/e2e/durable_jobs/` and `compiler/meshc/tests/`. Update the concurrency and test docs.

**Build.**

- [x] Provide scoped manual inspection and assert or refute helpers with namespace, queue, worker, version, argument subset, state, and bounded waits. Let assertions inside a transaction read through the caller's borrowed connection. Keep namespace ownership explicit so parallel tests cannot see one another's rows.
- [x] Provide test-only job construction and direct handler execution with the production validation and outcome decoder. Inline mode uses a fresh test scope and system clock or injected test clock as appropriate. It must not acknowledge a production row or trigger unexpected work on a rollback. Keep production execution on the existing isolated path.
- [x] Add a synchronous drain helper that uses the normal claim and settlement path with a maximum job count, an optional scheduled cutoff, explicit recursion and error policy. Return committed outcome counts and indicate remaining work when a budget is hit.
- [x] Show testing patterns for enqueue-only assertions, handler outcomes without a database, and physical multi-process recovery backed by PostgreSQL.

**You see.**

- [x] A service test can assert its jobs on the same transaction, run a worker directly, or drain a bounded namespace and assert its persisted results.

**Verify, unit.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked.

- [ ] Test the payload matcher, scopes, outcome decoding, drain cutoff and budget, and rejection of test-only hooks in ordinary builds. Run the compiler and meshc test suites.

**Verify, live.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked. Ten lanes on `gpt-6.2-sol high` at the PR head using the compiled fixtures and the PostgreSQL boot recipe.

- [ ] Lane 1. Regression lane against main. Run an enqueue-and-inspect fixture on main and head. Save `j5-lane-1.log`. Pass when both persist the same job and the head assertion verifies it.
- [x] Lane 2. Assert a job inside a borrowed transaction. Save `j5-lane-2.log`. Pass when the borrowed connection sees the row before commit.
- [x] Lane 3. Rollback an asserted enqueue. Save `j5-lane-3.log`. Pass when a second connection never sees the job.
- [x] Lane 4. Run parallel manual tests in separate namespaces. Save `j5-lane-4.log`. Pass when each test sees only its rows.
- [x] Lane 5. Call a test helper from a production build. Save `j5-lane-5.log`. Pass when the compiler rejects the test-only capability.
- [x] Lane 6. Perform an inline job with invalid input. Save `j5-lane-6.log`. Pass when validation and outcome match the documented worker API.
- [x] Lane 7. Drain a known future cutoff. Save `j5-lane-7.log`. Pass when jobs on the allowed side execute and later jobs remain scheduled.
- [x] Lane 8. Enqueue recursively during a bounded drain. Save `j5-lane-8.log`. Pass when the documented budget stops further claims.
- [x] Lane 9. Drain with one failing and one healthy job. Save `j5-lane-9.log`. Pass when the error policy and both dispositions match durable state.
- [x] Lane 10. Kill a compiled test runner in the middle of a real drain. Save `j5-lane-10.log`. Pass when the next runner recovers eligible work without a stale write.

**Verify, perf.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked.

- [ ] Metric. Direct-handler and committed drain duration per job, and default runtime throughput.
- [ ] Probe. Interleave the existing due-only drain with the new drain at identical cutoff and concurrency. Time direct execution separately.
- [ ] Baseline. Record main's due-only drain and direct worker fixture before adding helpers.
- [ ] Rule. Require the comparable drain within 10% of main. Test-only assertions must add no query to production enqueue or claim.

**Review gate.** None. This phase has no graphical interaction. Independent review uses the CLI receipts and database proof.

**Merge.**

- [ ] Land J5 after its dependencies and exact-head checks pass. Record the SHA and evidence in the program receipt.

## Close queue control and maintenance gaps (J6)

**Depends on.** J1 and J2. Keep notification and event integrations optional until J3 and J4 land.

**Files.**

- [x] Edit `compiler/mesh-rt/src/jobs/store.rs`, `compiler/mesh-rt/src/jobs/schema.sql`, `compiler/mesh-rt/src/jobs/runner.rs`, and `compiler/mesh-rt/src/jobs/cron.rs`.
- [x] Extend store and cron tests and the compiled runner proof. Add an operations runbook next to the Jobs docs.

**Build.**

- [x] Add stable runner and queue inspection, targeted local start, pause, resume and scale, and queue-only graceful stop. Retain namespace-wide defaults and database-wide admission. Make the behavior during restart and node loss explicit. Add optional namespace-wide pause and resume.
- [x] Add bounded job search, filtered bulk delete, validated edits, and an explicit run-now action for scheduled or retryable work. Preserve global limits, the current run's effect key, and the attempt budget. Reject edits to an actively claimed attempt unless the action specifically supports cancellation.
- [x] Expose cadence and page sizes for recovery, cron, and pruning. Preserve transactional cron cursors, current missed-occurrence policies, and receipt replay after pruning or delete. Put opt-in concurrent index maintenance on a separate, bounded administrative path with an index allowlist and one owner at a time.
- [x] Document missed wakeups, database outage triage, bounded maintenance, and the operator view of a job after failover. Derive the role grants from invoker SQL and test the matrix in Appendix C on a fresh and an upgraded database with default `PUBLIC EXECUTE` revoked.

**You see.**

- [x] Operators can inspect and control one runner or the namespace, safely advance due work, and maintain a busy database while jobs continue to settle.

**Verify, unit.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked.

- [ ] Cover command scope, edits racing claims, terminal-only deletes, receipt replay, and cron takeover. Run `bash scripts/verify-postgres-jobs.sh` plus store and cron tests.

**Verify, live.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked. Ten lanes on `gpt-6.2-sol high` at the PR head using the compiled fixtures and the PostgreSQL boot recipe.

- [ ] Lane 1. Regression lane against main. Run namespace pause and resume on main and head. Save `j6-lane-1.log`. Pass when both retain the documented persisted effect.
- [x] Lane 2. Pause one of two runners. Save `j6-lane-2.log`. Pass when the other process continues to claim within the namespace cap.
- [x] Lane 3. Scale both local and global limits. Save `j6-lane-3.log`. Pass when the lower available headroom determines new admissions.
- [x] Lane 4. Stop one queue with an active handler. Save `j6-lane-4.log`. Pass when stopping that queue leaves another queue able to make progress.
- [x] Lane 5. Run-now a retryable job. Save `j6-lane-5.log`. Pass when the scheduled time moves and the existing effect key survives.
- [ ] Lane 6. Race bulk edit or delete with claim, then separately enqueue a duplicate with allowed replacement fields while a compiled handler is active. Save `j6-lane-6.log`. Pass when bulk actions change only eligible rows, the replacement persists, the active context and token stay unchanged, and settlement and the next attempt use the documented stored values.
- [x] Lane 7. Kill the cron owner across cursor advancement. Save `j6-lane-7.log`. Pass when a new owner neither loses nor duplicates the committed occurrence.
- [x] Lane 8. Prune terminal jobs and replay an existing request key. Save `j6-lane-8.log`. Pass when the receipt replay still resolves to the same identity.
- [x] Lane 9. Reindex under concurrent enqueue and dispatch. Save `j6-lane-9.log`. Pass when normal application work keeps moving and a second reindex owner is refused.
- [x] Lane 10. Restart each role with default function execution revoked. Save `j6-lane-10.log`. Pass when supported commands succeed and workers cannot change roles, apply schema DDL, or reindex as administrators.

**Verify, perf.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked.

- [ ] Metric. Dispatch p99 under maintenance, backlog catch-up time, and administration transaction time.
- [ ] Probe. Interleave J1 traffic with equal prune and recovery pages. Measure reindex against native PostgreSQL concurrent reindex under the same traffic.
- [ ] Baseline. Record current staging and pruning costs and the database-only reindex control.
- [ ] Rule. Require dispatch p99 no worse than 1.15 times main during comparable maintenance. Keep work within configured page bounds and fail a reindex that stalls ordinary claims for more than two fallback polls.

**Review gate.** None. This phase has no graphical interaction. Independent review uses the CLI receipts and database proof.

**Merge.**

- [ ] Land J6 after its dependencies and exact-head checks pass. Record the SHA and evidence in the program receipt.

## Qualify the PostgreSQL queue and publish the results (J7)

**Depends on.** J2, J3, J4, J5, and J6.

**Files.**

- [x] Extend `.github/workflows/authoritative-verification.yml`, `scripts/verify-postgres-jobs.sh`, and `compiler/meshc/tests/e2e_durable_jobs.rs`.
- [x] Publish the shared API, guarantees, upgrade and support matrix, workload manifest, and comparison results in the Jobs documentation.

**Build.**

- [ ] Run database-backed correctness on PostgreSQL 17, 18.7, and 19.0. Record the exact 17 patch version and keep the minimum-version policy separate from the tested list. Run real compiled timeout, child-death, join, and recovery tests on each platform advertised for durable Jobs. Record platform boundaries explicitly.
- [ ] Pressure-test delivery under worker and coordinator death, row locks, network partitions, primary failover, and exhausted pools. Preserve the at-least-once contract, token fences and idempotency keys. Document that remote effects can commit after worker termination and that whole-host or worker suspension has a different boundary from blocking native I/O.
- [x] Replay the upgrade with preexisting scheduled and retryable jobs, old and new worker binaries, scoped grants, and default function execution revoked. Demonstrate rollback to the old runner without dropping rows or removing additive SQL.
- [ ] Publish the feature-by-feature matrix and exact measurements. Only claim PostgreSQL core parity when the behavioral matrix and measured limits pass. Record workload-specific exclusions and deployment assumptions.

**You see.**

- [ ] A release receipt shows where Mesh meets Oban Core's PostgreSQL workflows, what was measured, and which platforms and boundaries the results cover.

**Verify, unit.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked.

- [ ] Run the authoritative workspace build, format, runtime tests, proof gates, and the versioned PostgreSQL matrix. Keep the full compiler-to-worker entry point in the compiled proof.

**Verify, live.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked. Ten lanes on `gpt-6.2-sol high` at the PR head using the compiled fixtures and the PostgreSQL boot recipe.

- [ ] Lane 1. Regression lane against main. Run the full J1 matrix on main and the release candidate. Save `j7-lane-1.log`. Pass when the same input manifests reconcile without lost jobs or stale writes.
- [ ] Lane 2. Run the Linux compiled timeout and parent-death proof. Save `j7-lane-2.log`. Pass when the handler process terminates and the peer can recover the job.
- [x] Lane 3. Run the macOS executable replacement and shutdown proof. Save `j7-lane-3.log`. Pass when an unstarted claim keeps its attempt budget.
- [ ] Lane 4. Run real PostgreSQL on any advertised Windows configuration. Save `j7-lane-4.log`. Pass when timeout, parent death, and recovery are proved by a compiled executable.
- [ ] Lane 5. Upgrade a live namespace with old scheduled and retryable work. Save `j7-lane-5.log`. Pass when persisted attempts and schedules survive.
- [ ] Lane 6. Roll back to the previous runtime against the updated schema. Save `j7-lane-6.log`. Pass when the previous runner can settle its existing claim with its original grants.
- [ ] Lane 7. Fail over the primary with a committed queue and an ambiguous acknowledgement. Save `j7-lane-7.log`. Pass when the old primary is fenced, recovered job identities reconcile against the declared replication policy, and eligible work resumes within a lease plus two recovery intervals after the writable primary is ready.
- [x] Lane 8. On macOS arm64 with PostgreSQL 17.11, 18.7, and 19.0, the compiled proof exhausted the pool and interrupted notifications and maintenance. Stop and join met their bounds. The peer proved fallback polling, expired-lease recovery, and cron cursor advancement. [Receipt](../proof/jobs-parity/j7-lane-8.log)
- [x] Lane 9. The guides reproduced enqueue, retry, drain, and lease recovery from a fresh application on macOS arm64 with PostgreSQL 17.11. [Receipt](../proof/jobs-parity/j7-lane-9.log) · [Rerun the guide proof](durable-jobs-parity-evidence.md#replay-the-fresh-application-guides)
- [ ] Lane 10. Run the matched Oban comparison and inspect the raw results. Save `j7-lane-10.log`. Pass when each supported performance claim maps to a reproducible receipt.

**Verify, perf.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked.

- [ ] Metric. Median jobs per second and p99 pickup per declared workload, plus observer resource budgets.
- [ ] Probe. Alternate five trials per engine using J1's pinned configuration. Report raw samples and note any measurement-noise boundary.
- [ ] Baseline. Record main and Oban Basic before the candidate on the same host and database settings.
- [ ] Rule. Proposed parity gate. Require Mesh median throughput at least 90% of Oban and pickup p99 no greater than 1.25 times Oban on the declared PostgreSQL profiles, within J1's frozen absolute latency and resource budgets. Freeze or revise these budgets in J1 before optimizing. A failed profile limits the parity claim until resolved.

**Review gate.** None. This phase has no graphical interaction. Independent review uses the CLI receipts and database proof.

**Merge.**

- [ ] Land J7 after its dependencies and exact-head checks pass. Record the SHA and evidence in the program receipt.

## Close the program

- [ ] Publish the supported PostgreSQL and OS matrix, remaining scope boundaries, and benchmark results by workload.
- [ ] Confirm that all guarantees advertised in the reference docs have a reproducible positive case and an exercised failure case.
- [ ] Keep scope beyond PostgreSQL Core visible in a separate backlog.

## Appendix A. Prototype evidence

- [Hard timeouts](https://github.com/snowdamiz/mesh-lang/pull/37) merged at `db606548`. [Runner reliability](https://github.com/snowdamiz/mesh-lang/pull/38) merged at `59f6a7b3` with the same tree as its verified head `afd48895`. PostgreSQL 18.7 and 19.0 each passed 38 hosted Linux checks. The local macOS run on that reliability head also reported 38 passes per version. The compiled cases include application commit and rollback, worker death, native timeout, stale acknowledgements, retry and exhaustion, row contention, and shutdown. [Hosted verification](https://github.com/snowdamiz/mesh-lang/actions/runs/44667929654)
- The reliability head demonstrated that a large PostgreSQL JSONB context can exceed 8 MiB despite a much smaller input. J2 must preserve the one-row-per-context representation and the actual size boundary when claiming multiple jobs.
- Dedicated PostgreSQL listener transport, the steady-state cost of one OS process per attempt, and throughput relative to Oban are unmeasured. J1 records a baseline and J2 and J3 retain a switchable fallback before their new defaults are considered.

## Appendix B. Alternatives rejected

- Keep PostgreSQL as the first parity target. The distributed runtime and a separate embedded file cannot replace enqueue on the same connection as a PostgreSQL business transaction.
- Keep notification messages advisory. A listener can disconnect or miss a hint while rows remain durable. A committed row, queue control, and claim token remain the source of authority.
- Keep one process per attempt through the first comparison. A reusable worker adds state leakage and recycling concerns. Evaluate it only if J1 and J2 isolate process startup or resource usage as the reason a measured target fails.
- Reuse lease expiry and fencing for recovery. Oban Core's optional Lifeline uses an age threshold and documents possible duplicate execution. Match the recoverability workflow without adopting the weaker ownership test. [Oban Lifeline](https://oban.hexdocs.pm/Oban.Lifeline.html)

## Appendix C. Risks and later scope

Derive and verify exact grants before releasing the operational changes. The matrix names intended capabilities. PostgreSQL functions run with invoker privileges, so their grants must cover the tables and any sequences used by the SQL. Use namespaces within a trusted database. Use separate databases for applications that require isolation from one another.

| Database role | Required authority | Verification |
| --- | --- | --- |
| Migrator | Own the Jobs schema, tables, sequences, and functions. Apply DDL and record role grants. | Fresh install and rolling upgrade with default execution revoked. |
| Producer | Execute enqueue functions and use only the business and queue objects needed for enqueue. Check `admissions`, `queues`, `jobs`, `receipts`, and any required sequences. | Commit a business write with enqueue, roll it back, and reject worker and administrative operations. |
| Worker | Read schema metadata and execute claim, renew, settlement, and recovery. Grant only the underlying queue and job operations plus any maintenance explicitly enabled for that worker. | Claim and settle, then prove the role cannot apply DDL, change roles, or reindex. |
| Operator | Execute the selected state changes and inspect their underlying records. Put index maintenance behind a separately granted administrative capability. | Exercise pause, retry, and drain with scoped grants. Require distinct authority to reindex. |
| Inspector | Read approved summary data with no mutation or raw payload granted by default. | Read queue statistics and reject enqueue, settlement, and job edits. |

- J2 can increase response size, resource peaks, or later-queue latency. Cap total bytes and per-queue dispatch quantum, hold reserved slots only through confirmed claims, and test the same fault cases at small and large payloads.
- J3 consumes a session and can couple successful commits to PostgreSQL notification availability. Keep polling, expose its own resource budget, and qualify a session pool or explicit poll-only configuration.
- J4 event subscribers can accumulate work or disclose job data. Bound buffers, label outcomes with fixed categories, and omit payloads and credentials from default output.
- J5 inline execution can mask physical durability or start real application effects. Keep it in a test-only scope and preserve the PostgreSQL drain and process proofs.
- J6 maintenance can block shared work or destroy replay evidence. Keep bounded terminal-row pruning, separate receipts, and a scoped administrative reindex path.
- J7 verifies at-least-once behavior under declared database and platform assumptions. A forced worker suspension or a service that has already accepted a request can still lead to repeated effects. Use the stable effect key and state the storage and primary fencing assumptions.
- For backend-wide Core parity, follow J7 with one separate plan each for SQLite and MySQL. Each must prove same-transaction enqueue, competing claims, migration continuity, crash recovery and durable schedules, and publish its supported platform and concurrency limits.
- Pro parity is a distinct goal. Evaluate partitioned concurrency, rate limits, batch settlement, richer uniqueness and lifecycle hooks, dynamic queue policy, workflow dependencies and archival after PostgreSQL Core is qualified. Mesh already has database-wide queue limits and durable editable schedules. [Oban Pro Engine](https://oban.pro/docs/pro/Oban.Pro.Engine.html) · [Oban Pro composition](https://oban.pro/docs/pro/composition.html)

## Appendix D. Links and reading list

- [Oban 2.24.2 API](https://oban.hexdocs.pm/Oban.html) · [Basic engine source](https://hex.pm/packages/oban/2.24.2/files/lib/oban/engines/basic.ex) · [Database notifications and scaling](https://oban.hexdocs.pm/scaling.html)
- [Oban testing](https://oban.hexdocs.pm/Oban.Testing.html) · [Oban telemetry](https://oban.hexdocs.pm/2.24.2/Oban.Telemetry.html) · [Oban maintenance](https://oban.hexdocs.pm/operational_maintenance.html)
- [PostgreSQL LISTEN](https://www.postgresql.org/docs/19/sql-listen.html) · [PostgreSQL NOTIFY](https://www.postgresql.org/docs/19/sql-notify.html) · [Asynchronous messages](https://www.postgresql.org/docs/19/protocol-flow.html#PROTOCOL-ASYNC-OPERATIONS)
- [Mesh Jobs APIs](../website/docs/docs/concurrency/index.md#durable-jobs) · [Runtime state and schema](../compiler/mesh-rt/src/jobs/schema.sql) · [Compiled proof](../compiler/meshc/tests/e2e_durable_jobs.rs) · [PostgreSQL test entry point](../scripts/verify-postgres-jobs.sh)
