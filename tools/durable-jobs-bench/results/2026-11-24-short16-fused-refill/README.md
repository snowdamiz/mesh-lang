# Short16 completion/refill

This private experiment builds on the exit-confirmed trusted Jobs product at
`b3557273`. It adds one opt-in runner setting, `complete_and_refill`. The
setting changes how the single dispatcher handles confirmed successful exits
while the queue is open for claims.

## The gap it targets

The pinned product settles a group of confirmed successes in one transaction.
It claims replacement work in a separate transaction later. A replay of the
checked r2-off control (25 arms) found 25.34–39.00 normalized slot-ms free
while an unclaimed job existed. It also found 17.18–22.40 ms between the
second application write and the terminal commit. Both figures cover the
interval from the 16th to the 128th effect commit. They measure occupancy and
do not estimate speedup.

## What changes

The new setting applies only to a non-drain `trusted_shared_process` runner.
That runner must have one set-based queue and `group_success`. When it has at
least one confirmed exit and its local queue state allows a claim, it sends one
checked transaction:

```
BEGIN; SELECT … FROM mesh_jobs.complete_and_refill_v1(…); COMMIT
```

`complete_and_refill_v1` calls the existing `complete_batch_v1`. It returns a
receipt bitmap of the completions that took effect. Only if at least one
completed does it call the existing `claim_batch_set_v1`. The claim sizes
itself to the free slots plus the releases. Database admission, caps, pause
controls, the scoped grants and the wakeups are unchanged.

The runner does not wait to build a cohort. It does not fuse in these cases:

- A claim reply is outstanding.
- An attempt blocks claims.
- The queue is backing off or paused.
- Shutdown has started.

In those cases it uses the stock group or scalar path.

## Failure handling

- **Rejected before COMMIT.** This covers a malformed or invalid receipt, a
  lock-timeout contention marker, or a server error. Nothing commits. The same
  exits then go through the ordinary settle path. Contention is not reported as
  a storage failure.
- **ROLLBACK receipt for COMMIT.** The runner treats this as a confirmed
  rejection and uses the same fallback.
- **Lost or ambiguous COMMIT.** The runner publishes `AckUncertain` for each
  exit and never launches the provisional grants. It drops the socket and
  leaves the rows to lease recovery.
- **Confirmed COMMIT.** The runner records completions from the bitmap. Stale
  exits stay on the ordinary path. Grants go through the stock
  `consume_batch_reply`, which checks local controls again before each launch.

Commit-before-launch, both independent application writes, callbacks and
teardown, confirmed leases and legacy or mixed administration all stay on the
existing code.

## Comparison

One release ELF serves both the candidate and its control. The control runs the
same ELF with `complete_and_refill` off: the checked pool with group success
and D2 off. The plan also runs the exact public `b3557273` product-group ELF
from the trusted-composition SETUP. All three use the matched stock River,
Oban and Apalis arms from `2026-11-24-bulk-d2-short-peers`. Every arm runs on
two CPUs with a fresh PostgreSQL 17.11 cluster.

| Lane | ELF | Mode |
| --- | --- | --- |
| `public-group` | public `b3557273` | `product-group` |
| `group-a`, `group-b` | candidate | `product-group` (same-image A/A) |
| `fused-a`, `fused-b` | candidate | `product-fused` (same-image A/A) |
| `river`, `oban`, `apalis` | stock pins | stock |

Groups 1 and 2 are opposite orders and form the pilot. Groups 3–5 run only after
both pilot groups reconcile, and only if the pilot improves both execution and
durable performance. `replay/intake.py replay` reports both clocks, the
16th-to-128th second-write interval, the vendored slot occupancy
(`replay/slot_times.py`) and the paired and A/A deltas. It also reports the
`completion_refill` counters and the PostgreSQL statement ledger.

## Commands

- **SETUP (controller only):** `bash native-setup.sh <attempt> <revision>
  <source-sha256> < source.tar.gz`. This builds the release runtime and links
  the product and witness ELFs. It then runs the untimed semantic suite and the
  refill fault proof on the host.
- **Bind:** `python3 -B replay/intake.py bind <attempt> <mesh.json>
  <public-attempt> <public-mesh.json>`
- **TIMING (separate grant):** `python3 -B replay/controller-timing.py
  <requests> <bound.json> <output> 1 2`. After both pilot groups reconcile, run
  it with `3 4 5`.

The local evidence is untimed macOS proof only. It is listed in
`local-evidence.json`.
