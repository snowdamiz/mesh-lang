# Trusted Jobs product composition

This isolated product candidate uses the ordinary `Jobs.start` handler and
public scheduler exit callback from the shared product. It keeps the
product's checked claim transaction, set-based producer, and PostgreSQL Bind
encoder. The benchmark fixture also uses the product executable for
`Jobs.migrate` and its producer session. The handler and the two application
write statements match the frozen r2 fixture. The fixture disables cron
because these workloads have no cron schedules and the worker login is
scoped. Separate roles own migration, queue work, app writes, and producer
writes. The pinned V2 harness supplies the existing workloads and scorer.

## Enable the composition

Migrate and grant the opted-in worker
`mesh_jobs.claim_batch_set_v1(text,text,jsonb,bigint,integer,bigint)`
and, for grouping, `mesh_jobs.complete_batch_v1(text,uuid[],uuid[])`.
Keep the existing cursor and scalar grants for compatibility. Use PostgreSQL
17 with the supported bulk producer grants. The producer sets
`MESH_JOBS_SET_BASED_ENQUEUE=1` and calls `Jobs.enqueue_in` within its own
transaction. Use the released producer fallback for ineligible batches.

Pass the runner settings below in addition to the ordinary namespace,
queue slot limits and worker registry. The app pool opens independently.

```json
{
  "execution": "trusted_shared_process",
  "claim_batch": true,
  "claim_batch_strategy": "set_based",
  "group_success": true,
  "listen": true
}
```

Use `mesh_pool_hot_checkout=1` in the URL passed to `Pool.open` for the
handler's application pool. Only that pool skips the probe on checkout when
the session was idle for less than 500 ms. A recent silent socket can leave the
first application statement waiting with ordinary statement read semantics.
A later checkout can replace a broken connection. The pool sends each
application statement once. Ordinary strict defaults, handler provenance,
and process and actor exit behavior remain the documented public contract.

On each dispatch pass, two to eight fully exited, first-submission successes
can share a terminal transaction. PostgreSQL checks the exact worker token,
executing state, cancellation and expiry under the row lock. A locked or
ineligible row uses scalar settlement. Errors, retries, snoozes and releases
also use scalar settlement. An ambiguous grouped receipt produces uncertain
events and no retransmission. The database retains authority over outstanding
leases and their eventual recovery. Each application write still commits in
its own transaction.

## Evidence and differences from the experiments

- V2 grouped success could run before actor teardown. The product waits for
  the public callback. The r2 writer-off arm also groups successes. That arm
  disables its separate asynchronous completion writer. The candidate uses
  the small synchronous grouping pass and keeps the public runtime's teardown.
- The candidate uses checked product claims with BEGIN, validated SELECT and
  confirmed COMMIT. The experimental admission function and its grants stay
  in the experiment. The product schema keeps a shared capacity fence with
  legacy claimers and checks receipt shape before attempting to start work.
- The candidate retains r2's 500 ms D2 threshold but sets it per pool. The
  process-wide experimental switch does not control this candidate. The app
  pool handoff, prepared SQL and producer-spec experiments are separate.
- The local unmeasured proof links the real fixture, proves the lease, cancel
  and terminal cuts in disposable TLS PostgreSQL, and drops the app write,
  verified claim COMMIT and grouped terminal receipts. A separate proof
  executable uses the same runtime and adds an actor termination gate.
  Its force-kill and normal-release checks observe both application XIDs
  while the job is still executing.

## Native SETUP

The sole controller first finishes the separately sealed
`2026-11-24-exit-confirmed-bulk-d2` intake. Keep any r2 failures and compare
the product only in fresh matched blocks. A valid r2 improvement and its
semantic receipts admit the full short comparison. Otherwise use the
predeclared smaller short comparison after reviewing the failure. The
smaller modes retain supported bulk enqueue, checked claims and the strict
checkout probe, with and without grouping.

Under its exclusive product SETUP grant the controller passes the sealed
`source.tar.gz` to `replay/native-setup.sh` on the pinned Linode. The exact
arguments are in the intake. It uses Rust 1.99.0 and LLVM 21.1.8, checks the
retained Linux meshc SHA, builds the product runtime from source, links the
fixture at opt level 2, and records the ELF, runtime, compiler and
toolchain hashes. It uses the retained compiler because tracked compiler
build inputs match the retained revision outside the product runtime.
The native proof runs the actual ELF and the separately hashed teardown
witness with PostgreSQL 17.11. Keep the setup log, ELF header and build ID,
semantic results, both executable seals and the exact release receipt.

## Native comparison

Bind each plan to the same native `mesh.json` from SETUP using
`replay/intake.py bind`. Begin with the short groups 1 and 2 in opposite
orders. Keep diagnostics and all partials. Continue to five matched groups
under TIMING after both groups reconcile. The stock peer pins are River,
Oban and Apalis from the short V2 all-peer repeat. Three same-ELF Mesh
lanes isolate grouping and D2 with the same producer path and profile.
The smaller short plan omits D2. Record startup and auth failures,
unreconciled ledgers and every raw archive alongside any passed results.

Carry forward the selected short configuration and exact build into the
dense, sparse, slow and large comparisons with all three peers. Use a
separate grant and series for each topology or storage setting. The
primary dense stock series uses eight runners and 64 slots. Dense mixed
uses two Mesh runners against eight-runner peers. Dense equal uses two
32-slot runners for every engine, with the released equal-topology
peer wrappers and binary pins. Keep their evidence separate. Default
storage and symmetric EXTERNAL storage are separate large series. Capture
effective work_mem, actual column storage, storage device, durability and
cap for every compared engine. A live setting mismatch invalidates the
matching claim. Do not use the earlier mixed tuning or mixed executable
series to fill in missing cells.

Each measured block reports jobs/s from first submit to last durable
settlement and from first enqueue commit to last durable settlement.
Use the matched paired directions, p99 on both clocks, job errors,
capacity, terminal cohort sizes, checkout calls and the rebound raw
ledger. Each job must retain two distinct independent application XIDs.
Record r2 and V2 numbers as historical comparisons with their own
executable and contract identities. Product performance requires a
successful Linode run of this product ELF. No Linux outcome is claimed
by the local package.
