# When the dense public runner launches a confirmed claim

Across six matched two-CPU Linode rotations, **at least 49.8–51.0% of aggregate Mesh time from the SQL claim stamp to handler entry elapsed after the claim transaction was already durable**. At least 20.7–25.2% elapsed after the dispatcher had spawned the actor. These bounds apply to the hot pool on public source `1e70f18d`, using two 32-slot runners, synchronous PostgreSQL and an individual terminal transaction per job. They price delays along the same job timelines. Summing overlapping jobs does not produce a wall-time saving. [Six-rotation report](../2026-11-24-dense-individual-hot-pool/logger-off-retry/NATIVE-RESULTS.md)

| Group | Productive grant batches | After durable grant, minimum share | After dispatch starts, minimum share | After spawn, minimum share | Jobs demonstrably waiting after spawn | Distinct same-runner settlements between known durability and first entry |
|---:|---:|---:|---:|---:|---:|---:|
| 1 | 78 | 49.8% | 25.4% | 20.7% | 608 | 347 |
| 2 | 83 | 51.0% | 29.4% | 23.1% | 650 | 358 |
| 3 | 84 | 50.7% | 26.6% | 21.8% | 687 | 409 |
| 4 | 82 | 50.8% | 26.3% | 21.6% | 656 | 385 |
| 5 | 80 | 50.6% | 26.0% | 20.9% | 609 | 391 |
| 6 | 88 | 51.0% | 30.2% | 25.2% | 652 | 372 |

The three shares have the same denominator and overlap. The raw reductions include both checked controls in all six groups. All 18 measured Mesh arms reconciled to their sealed hashes and grant-call counts. [Unrounded evidence](native.json)

## Reconstructing grants

Each successful set-based statement computes one millisecond `started_at` value in a materialized aggregate and stamps all granted jobs with it. It returns one row per grant. An unproductive call can return zero rows or one marker. Every scored job was claimed once, both runners finished without errors, and the native SQL audit excluded cursor fallback. Subtracting 1,024 from the returned row count gives the number of single-row markers. Subtracting those markers from the call count gives an upper bound on productive calls. The number of distinct (runner PID, claim millisecond) pairs is a lower bound. They match exactly in every arm. This equality rules out both ambiguous same-runner timestamp buckets and unaccounted zero-row calls in these captures. The replay refuses an ambiguous count. [Set-based grant SQL](../../../../compiler/mesh-rt/src/jobs/schema.sql) · [Native SQL checks](../2026-11-24-dense-individual-hot-pool/score.py)

The workload uses one queue and one host clock. For each batch the replay establishes three latest possible times.

1. **Durable grant.** All granting statements for the queue lock the same queue row until transaction end. A strictly later successful grant stamp, from either runner, proves the earlier durable transaction released that lock. Add a full millisecond to the later stamp to account for floor rounding. Entry of any member of the batch also proves durability. Use the earlier of these witnesses.
2. **Dispatcher began consuming the grant.** The first handler entry proves consumption started. A strictly later claim on the same runner also proves this, since that runner can reserve its next batch only after launching the previous one.
3. **All actors in the batch were spawned.** The next same-runner grant again witnesses the entire loop. So does a successful same-runner terminal transaction recorded strictly after the first member entered. The dispatcher spawns the entire batch before it can issue another synchronous individual settlement. Any batch member's own settlement also witnesses the entire spawn loop. Take the earliest available witness and count only positive time from it to the job's entry. [Dispatch and individual settlement](../../../../compiler/mesh-rt/src/jobs/runner.rs) · [Trusted actor start](../../../../compiler/mesh-rt/src/jobs/trusted.rs)

The median batch had already been durable for **at least 5.5–8.4 ms** before its first handler entered, depending on rotation. The last column identifies hundreds of settlements during these windows. The ledgers lack the client's grant-COMMIT receipt and scheduler enqueue timestamps. A settlement before first entry could have overlapped grant delivery to the dispatcher or execution of an actor already spawned. The count establishes an opportunity to test overlap, not causation. An experiment can hold a terminal acknowledgment and observe whether another confirmed job starts while the original slot remains charged. A matched native run is needed to measure any gain.

## Reproduce from the checkout

```sh
d=tools/durable-jobs-bench/results/2026-11-25-dense-claim-to-entry
python3 -B "$d/test_bounds.py"
python3 -B "$d/replay.py" | cmp - "$d/native.json"
```
