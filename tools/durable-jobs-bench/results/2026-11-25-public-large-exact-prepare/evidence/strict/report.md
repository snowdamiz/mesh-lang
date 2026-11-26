# Large Mesh Jobs payload preparation

The isolated candidate is commit `955b356b51f0dbef36a2958bc27ed45c15af3582`,
based on `3828f20d`. Its opt-in protocol is `locked_prepare_pg17_v1`.
[Single patch](candidate.patch) · [Native source and adapter pins](bundle-955b356b/identity.json)

## Decision

Keep this protocol opt-in. Local verification establishes correctness of the
prepared claim path and a shorter claim component on macOS. Native performance
qualification requires the exact PG17 build and the matched strict large arm.

## What changed

Each runner uses its existing claim connection and pending slots. It reads the
priority-ordered candidates and locks each exact heap tuple it can prepare.
PostgreSQL 17's table access method skips a busy tuple and returns stale for an
updated tuple. The lock call disables update-chain following. Preparation
returns the complete original PostgreSQL JSON context and leaves the job ready.

The Rust receiver checks UTF-8, framing, bounds, order, and byte counts before
sending activation. The client also checks the SELECT row count. Activation checks the same transaction and tuple, live
namespace and queue controls, eligibility, and the current capacity under the
stock queue lock. It writes only the admitted prefix. A skinny count record
checks the shape of the confirmation reply. Dispatch waits for the
server's COMMIT tag and well-formed idle acknowledgement. The server bounds the preparation
transaction at 10 seconds and activation's queue-lock wait at 250 ms,
respecting any stricter existing configuration. The new path uses the existing
per-runner slot reservation and application connection topology.

Reservations are job-row locks. Disconnect and transaction expiry release them.
Cancellation and edits serialized ahead of a reservation are visible to
selection. Updates behind a reservation wait for its transaction. An eligible
stale tuple stops that preparation at the proven prefix. A fresh version is
eligible on the next probe. A fresh check for a previously committed oversized
marker prevents spending the marker allowance on a blocked tuple. Each
reservation selects in priority, scheduled time, and insertion order under
SKIP LOCKED. Independently reserved batches can finish delivery out of order.
The ordinary effect fence, two transactions, handler, and legacy entry points
use the stock code.

## Verification and iterations

- The first AMD64 build `0dfcb95a` reached the original handler, then failed the
  ledger on job `job_000051`. PostgreSQL reported a NOWAIT conflict in
  `compiled_demo.first`, and one job retried. The smoke's launcher PATH error
  and corrected native execution are independently retained. [Native failure archive](../../tools/durable-jobs-bench/results/2026-11-23-large-v3-setup/smoke-retry/diagnostic-mesh/raw.tar.gz)
- The SQL repair `1bdbebfd` released a discarded row lock. Its AMD64 diagnostic
  reconciled 64 first attempts and both writes with a 65-connection peak.
  Submit to enqueue commit was 1,910.0 ms. Commit to first claim was
  251.5 ms. Claims spanned 572.0 ms. Commit to last settlement was 989.5 ms.
  The narrow replay then demonstrated that the SQL lock could still follow an
  update and briefly block a valid effect before the savepoint rolled back.
  [Native diagnostic](../../tools/durable-jobs-bench/results/2026-11-23-large-v3-setup-1bdbebfd/reconciled.json) · [Narrow race](narrow-01/receipt.json)
- The exact-lock witness held the preparer between the visibility check and
  the exact lock while a legacy claimant committed. The executing legacy
  attempt renewed and its original effect fence admitted the valid token
  while the exact lock was still paused. The same fence denied a stale token.
  A concurrent eligible edit stopped the stale prefix, and the next scan read
  the updated metadata. [Legacy race](fence-final/receipt.json) · [Edit race](exact-edit-01/receipt.json)
- The role proof covers separate install, scoped grants, mixed claimants,
  capacity, pause, cancellation, timeout recovery, ordinal and byte shape,
  oversized markers, and old-schema reinstall. The C helper rejects invalid
  tuple coordinates before heap access. The compiled macOS upgrade fixture
  reconciled 64 first attempts and two independent durable writes with eight
  runners and a sampled peak of 32 connections.
  [SQL contract](contract-06/receipt.jsonl) · [Compiled TLS cohort](compiled-cohort/receipt.json)
- The compiled wire proof covers truncated data, wrong late length, invalid UTF-8,
  missing preparation and activation rows, duplicate IDs, lost activation reply, and three
  incomplete COMMIT acknowledgement variants. All precommit failures left both final attempts
  unused. A committed claim with missing acknowledgement launched zero
  handlers. [Wire receipts](faults-05/receipts.jsonl)
- The local PostgreSQL 17.11 TLS claim-only screen used the frozen 64 × 512 KiB
  job inputs and eight independent sessions. The exact route reached last
  confirmed claim in 91.0 ms median (88.0–116.6 ms) across three rotations.
  Stock reached 180.1 ms median (165.2–957.1 ms). The macOS arms use libpq
  and a one-transaction component producer. Setup and producer clocks are in
  each receipt. [Rotated claim component](screen-02/summary.json)

## Installation scope

The stock `Jobs.migrate` runs first. A database administrator compiles
`pg17_lock_exact.c` against the installed PostgreSQL 17 headers, supplies the
absolute library path and worker role to `prepare_claim_install.sql`, and
keeps the existing scoped Jobs grants. Only the exact module and the two
opt-in entry points receive new grants. The helper requires the heap-table
Jobs schema, a read-committed transaction, SELECT authority and Jobs update
authority. It declines a role with active row security. Enable batch claims
in the runner config and set `MESH_JOBS_BATCH_PROTOCOL=locked_prepare_pg17_v1`
for the runner process. Revert a runner to stock batch claims by removing the
environment setting. Existing lease recovery governs any committed attempts
during a worker restart.

The installer checks effective object ACL entries for unexpected named grants.
Use `prepare_claim_remove.sql` after stopping opted-in runners and before
a PostgreSQL major upgrade. [Installation and removal witness](installer-final/receipt.json)

These invoker-rights functions use the existing scoped worker DML privileges.
The compiled runner binds the manifest to the prepared reply and its held transaction.
Priority is relative to each reservation snapshot and its available row locks.
