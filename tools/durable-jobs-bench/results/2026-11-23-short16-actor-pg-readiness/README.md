# Actor-owned PostgreSQL readiness on V2 short jobs

**Hold. This result does not earn a native slot.** Readiness used all eight
application backends, but drained faster than both identical off controls in
zero of six rotations. The candidate's 12.2 ms median gain against each
rotation's control midpoint was smaller than the 35.8 ms median difference
between the controls. The second half of handler entries improved by 1.3 ms
against the midpoint, below the 8.8 ms duplicate spread. [Reconciled scores](evidence/summary.json) · [Timing guard](replay/guard.json)

The earlier readiness archive covers a standalone actor fixture and a dense
multi-runner V1 queue. The short16 ticket search found no prior run with
readiness, V2 grouped successes, and the genuine bulk producer together.

## Same release binary, six local rotations

Each arm ran the frozen 128 × 1 KiB input on one runner with 16 slots, two fixed
actor workers, and eight warmed application connections. Each handler wrote
an attempt, slept for 1 ms, and wrote its effect in a separate autocommit.
PostgreSQL 17.11 enforced a 20-connection cap and verified TLS. The trials
warmed eight application, four queue, and one shared producer and observer
session before submitting work. Each arm ran one true bulk enqueue and zero
scalar enqueues. The V2 `success_cohort` policy used set-based admission in
every arm and grouped successes when several were ready. One control
settled each success separately. The local screen ran on a shared macOS
host with unrestricted CPU affinity. The original `raw.json` files retain
the generic dense label `one process per batch`. The pinned adapter used
the warm broker, as recorded in `experiment.json` and the broker logs.

| Metric | Off A median [range] | Off B median [range] | Readiness median [range] |
| --- | ---: | ---: | ---: |
| Submit to last settlement, ms | 184.3 [105.1, 759.8] | 127.5 [93.3, 138.1] | 169.3 [114.4, 229.7] |
| Enqueue commit to last settlement, ms | 123.0 [79.4, 650.0] | 99.3 [78.1, 111.5] | 141.5 [87.2, 188.1] |
| Entry rank 1 from commit, ms | 6.1 [2.8, 13.7] | 3.2 [2.2, 4.4] | 3.8 [2.4, 6.6] |
| Entry rank 64 from commit, ms | 74.9 [41.0, 262.0] | 49.4 [35.7, 60.3] | 55.3 [35.1, 90.5] |
| Entry rank 128 from commit, ms | 119.1 [73.6, 613.1] | 94.8 [72.7, 107.9] | 131.2 [79.7, 174.5] |
| Entry rank 64 to rank 128, ms | 45.2 [31.5, 369.0] | 37.7 [33.9, 61.5] | 64.9 [38.4, 96.9] |
| Last effect commit to last settlement, ms | 1.8 [1.0, 7.2] | 1.0 [0.5, 2.1] | 4.8 [1.5, 9.4] |
| Runner lifetime CPU, ms | 105.6 [88.6, 115.4] | 97.6 [90.4, 100.7] | 107.7 [94.4, 112.0] |

The single release executable had the same SHA-256 in every arm. Both off
labels used identical workload and runtime options. Readiness added
`mesh_cooperative_pool=1` to the application URL. Queue and producer sessions
used the frozen V2 path. The archived runtime patch applied after removing
its overlapping `ReadyForQuery` check. The V2 parser already validates the
response unconditionally. The compiled handler and producer source came
unchanged from the accepted V2 bulk source archive. The manifest carries
the V2 fixture's base version. The input and executable hashes identify
the local overlay precisely. [Release hashes](evidence/release-sha256.txt) · [Arm order and pins](evidence/scored/plan.json)

## More backends held active writes

Every scored run recorded 128 successful first INSERTs, 128 successful second
INSERTs, 256 checkout probes, and zero application reset or rollback statements.
Median queue SQL calls fell to 156.5 from 212.0 and 198.0. The median
producer session recorded 16 statements in each arm, including observer
queries. Median summed application statement execution rose to 32.9 ms from 11.4 ms and 6.2 ms. This counter sums
the execution intervals across backends and can grow when the intervals
overlap. Runner CPU rose 4.7% against the paired control midpoint.

A separate diagnostic pass sampled up to five simultaneous active writes
with readiness and up to two with the off control. There were 10 such samples
with readiness out of 83, and two with the control out of 42. Each warm
runner had two live scheduler worker stacks. Scored runs peaked at ten
process threads with readiness and nine with either control. The traced run parked
for 511 socket reads. Scored runs kept the standard observer cadence and
omitted polling from the extra diagnostic connection. The extra observation
shows overlapping backend writes, while the timing guard governs the
transfer decision. [Raw activity and stacks](portable-results.tar.gz) · [Diagnostic reducer output](evidence/summary.json)

After the runner and shared broker exited, the verifier reconciled all
application rows and timestamps with the frozen payload and producer
receipts. Each arm had 256 distinct application XIDs and a separate enqueue
XID. Terminal XIDs were distinct from those 257 and shared by at most
eight successes. All 128 jobs completed on their first attempt in each
arm. Handler entry through durable settlement stayed within 16 active
jobs. The durable ledgers and raw trial receipts are in the scored and
diagnostic directories. [Original scored ledgers](evidence/scored) · [Original diagnostic ledgers](evidence/diagnostic)

## Cancellation can precede the server's outcome

The fault build used the same release runtime and the existing V2 fault
fixture variant. Both modes passed its queue, retry, recovery, and grant
checks. With readiness, the job reached `cancelled` while its second INSERT
was still blocked in PostgreSQL. After releasing the lock, PostgreSQL
committed that INSERT once, 156.5 ms after the terminal commit. The off
control stayed `executing` while blocked and committed its effect 2.3 ms
before the cancellation settled. Actor cancellation releases local ownership
of the wait. A submitted SQL statement can outlive that wait.

The separately linked actor-pool fixture passed all nine transport and
lifecycle checks. They include cancellation before submission, cancellation
after submission and after a lost commit answer, replacement, and a timeout
with SQL still active. The V2 fault fixture uses individual fault jobs and
documents scalar fault setup. The 128-job timed arms prove the bulk producer.
[V2 readiness faults](evidence/faults/queue-readiness/cases.json) · [V2 off faults](evidence/faults/queue-off/cases.json) · [Actor-pool faults](evidence/faults/actor/cases.jsonl)

## Native reference

The earlier five selected rotations from commit `22578fba` used the same
Linux boot, PostgreSQL 17.11, and CPUs 0 and 1. V2 bulk entered the first 64
jobs earlier than River in every rotation. Its remaining 64 took 81.5 to
85.2 ms, compared with 36.6 to 49.1 ms for River.

| Native lane | Median submit, ms | Median drain, ms | Median rank 64 to 128, ms |
| --- | ---: | ---: | ---: |
| Mesh V2 bulk | 214.6 | 179.3 | 82.0 |
| River | 183.6 | 169.7 | 45.3 |
| Oban | 415.4 | 362.5 | 76.5 |
| Apalis | 172.5 | 162.0 | 57.8 |

The native reference contains the selected boot, block identities, and
the exact raw hashes. The macOS result is the local transfer decision. The
dense V1 hold also stays in place. [Matched reference](evidence/native-reference.json) · [Matched Linux raw data](../2026-11-23-bulk-enqueue-v2-short/evidence/bulk-enqueue-v2-short)

## Source proof

The source proof uses the patched V2 tree. The first pass stopped at the
runtime test build because the benchmark archive omits the test vectors.
The first retry restored those files from the pinned fixture revision
`3828f20d`. The Jobs store suite then exposed a test adapter omission.
The benchmark bundle includes the new enqueue schema but omits
`ce5e551d`'s matching test adapter change. The final proof applies that
exact test-only change in the private proof tree. The release workspace
build and the PostgreSQL proof passed. The proof includes 71 Jobs runtime
tests and three compiled Jobs integration tests. All 12 pool cases and 43
scheduler cases passed. The archive retains
each attempt and the test patch. The changed Rust files pass formatting.
Whole-workspace formatting reports the same four inherited diffs on the
frozen source and the patched tree.
[Proof status](evidence/proof-status.json) · [Formatting exits](evidence/gates/fmt-status.txt)

## Offline replay

The portable archive contains the original raw receipts, the exact accepted
V2 source bundle, the canonical batch bytes, and the unmodified prior
readiness archives. The workspace
build and PostgreSQL proof receipts are in the archive. [Portable archive](portable-results.tar.gz) · [Archive checksum](portable-results.sha256)

```sh
packet="$PWD/tools/durable-jobs-bench/results/2026-11-23-short16-actor-pg-readiness"
lane="$PWD/target/short16-actor-pg-readiness"
mkdir -p "$lane/replay"
(cd "$packet" && shasum -a 256 -c portable-results.sha256)
python3 -B "$packet/replay/archive.py" "$packet/portable-results.tar.gz"
tar xzf "$packet/portable-results.tar.gz" -C "$lane/replay"
tar xzf "$lane/replay/sources/bulk-enqueue-ce5e551d079e.tar.gz" -C "$lane" ordinary-v2
python3 -B "$lane/replay/replay/reconcile.py" "$lane/replay/evidence" \
  "$lane/replay/replay/frozen-verifier.py" "$lane/ordinary-v2" > "$lane/summary.json"
cmp "$lane/summary.json" "$lane/replay/evidence/summary.json"
```

Use `replay/rebuild.sh` from the extracted archive to rebuild the release
and fault executables in a fresh lane with the pinned compiler. The local
timing driver exits if the shared timing lock is held.
