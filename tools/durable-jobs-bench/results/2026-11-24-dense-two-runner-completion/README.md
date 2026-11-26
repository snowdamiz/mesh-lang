# Dense application writes and confirmed completions at two runners

In the five native comparisons, trusted Mesh V1 finished its last application write **373–489 ms after River**. Its last terminal commit followed just **0.8–4.0 ms** later. Those are drain times measured from each arm’s durable enqueue commit. Each engine used the same two Linode CPUs for 1,024 jobs and committed two ordered application writes per job. Mesh used two 32-slot runners and the peers used eight 8-slot runners. [Native stage chart](native-stages.html) · [Original crossover and durable ledgers](../2026-11-23-dense-matched-topology/README.md)

| Native rotation | Mesh last write | Mesh final settlement | River last write | River final settlement | Apalis last write | Apalis final settlement |
|---|---:|---:|---:|---:|---:|---:|
| 1 | 1,373 ms | 1,373 ms | 930 ms | 1,163 ms | 814 ms | 824 ms |
| 2 | 1,363 ms | 1,365 ms | 926 ms | 1,144 ms | 874 ms | 882 ms |
| 3 | 1,304 ms | 1,308 ms | 867 ms | 1,118 ms | 876 ms | 884 ms |
| 4 | 1,360 ms | 1,362 ms | 870 ms | 1,097 ms | 889 ms | 896 ms |
| 5 | 1,342 ms | 1,346 ms | 969 ms | 1,167 ms | 850 ms | 855 ms |

During the interval between the 103rd and 922nd Mesh claim stamps, at most 48 of 64 slots had a recorded claim and an outstanding terminal commit for **82–91%** of the time. This count uses the stored millisecond claim stamp, including work waiting to enter a handler. A runner can still hold local capacity while waiting for its grant or settlement acknowledgement. The ledgers identify when application writes fell behind. They do not assign that time to PostgreSQL, dispatch or CPU scheduling. [Unrounded results including Oban and the middle drain](results.json)

## Local completion writer

The compiled r2 runner waits for actor teardown and can group up to eight successful settlements. The zero-hold policy lets the dispatcher serve already-confirmed work while a separate writer awaits terminal acknowledgement. The off control settles inline. Both used the same Mac ARM64 runner, same compiled handler, same batch producer and the same PostgreSQL 17.11 configuration. The producer made eight bulk calls within one durable transaction per arm. Two runners shared 64 slots with two scheduler workers each. We observed 41 encrypted physical sessions, including 32 warm application sessions, under a cap of 96. The earlier V1 native measurements used an individual terminal transaction per job.

**Keep the writer off for the dense follow-up.** Zero hold reduced terminal transactions beyond the control spread in all six rotations. It beat both same-binary controls on drain in two. The drain saving exceeded the difference between controls in zero rotations. No rotation met the predeclared rule for advancing, which required four. The admission query count increased at the median.

| Local outcome | Off A median | Off B median | Zero hold median | Rotations beating both controls |
|---|---:|---:|---:|---:|
| Submit through last settlement | 1,126 ms | 1,202 ms | 1,121 ms | 2/6 |
| Enqueue commit through last settlement | 967 ms | 978 ms | 943 ms | 2/6 |
| Enqueue commit through last application write | 962 ms | 976 ms | 943 ms | 2/6 |
| Terminal transactions per 1,024 jobs | 887.5 | 895.5 | 698 | 6/6 |

The median within-rotation absolute difference between the two off controls was 385 ms from submit and 412 ms from commit. One-minute host load at arm start ranged from 39.3 to 56.4 on the shared 15-core Mac. Use the paired comparisons when interpreting the medians. This local series supplies no performance ranking against the native peers.

All 18 scored arms reconciled **18,432 jobs and 36,864 distinct application write transactions across their respective databases**. Each row matched its frozen payload and queue ID. The two runners finished every attempt, all 41 sessions were accounted for before submission, and no terminal XID overlapped an application write or enqueue XID. The executable, PostgreSQL version and database settings are in the raw receipts. Jobs and XIDs are checked separately for each arm.

## Replay

Run these commands from the checkout root. The reducer verifies the per-file checksums of the local archive and reuses the frozen native matrix verifier and linked raw native series. The native chart selector shows either one rotation or the median of each displayed time.

```sh
p=tools/durable-jobs-bench/results/2026-11-24-dense-two-runner-completion
python3 -B "$p/replay.py" | cmp - "$p/results.json"
python3 -B "$p/chart.py" | cmp - "$p/native-stages.html"
(cd "$p" && shasum -a 256 -c SHA256SUMS)
```

The [local archive](local-evidence.tar.gz) contains 808 checksummed files, covering the initial smoke, eight completed arms of the first series, its failed pre-submit roster check, all 18 restarted arms, and the two build manifests. The first collector attempted to save a second snapshot into a file that already existed. Both runners in that arm finished with zero claims. The repaired collector waits for the roster before saving its single snapshot. The complete series restarted from rotation one with identical workload, order and advancement rule. The incomplete series has its own record and contributes zero scored rotations.

To rerun on the original Mac with the retained bulk broker and r2 runner, invoke `python3 -B "$p/run.py" campaign new-run-name`. The dispatcher acquires the local timing lock and writes fresh receipts under `target/dense-r2-write/`. The original builder and runner archives are linked from the [combined exit-confirmed runner proof](../2026-11-23-ordinary-completion-writer/combined/README.md).
