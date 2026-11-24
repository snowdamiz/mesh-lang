# Native slow 2×32 at PostgreSQL cap 71

Mesh V1 bulk was faster than Oban on both clocks in all eight paired blocks. It finished ahead of River in six submit comparisons and seven drain comparisons. Apalis led both clocks in every block. The median paired Mesh savings were 124 ms from submit against Oban and 102 ms against River. Apalis finished a median 338 ms earlier than Mesh from submit. [Paired scorecard](evidence/native-analysis/paired-scorecard.json)

## Timing

Each arm ran 512 canonical 16 KiB jobs with a 100 ms hold, two 32-slot processes, shared CPUs 0 and 1, and PostgreSQL 17.11 at cap 71. The eight blocks used balanced positions, with one unscored warmup per engine in each block. Submit measures from producer submission to the last durable terminal commit. Drain measures the same endpoint from the enqueue commit. The cap-68 series retains its own deployment, data, and scores. [Frozen cap and topology](README.md) · [Full timing replay](evidence/native-analysis/timing-replay.json)

| Engine | Submit jobs/s | Drain jobs/s | Runner CPU ms/job |
| --- | ---: | ---: | ---: |
| Mesh V1 bulk | 284.0 [261.7, 293.1] | 408.0 [376.4, 430.1] | 1.022 [0.994, 1.119] |
| River | 255.6 [252.3, 289.3] | 337.7 [327.8, 396.8] | 1.061 [1.017, 1.148] |
| Oban | 267.5 [245.8, 283.3] | 362.1 [332.6, 384.9] | 5.911 [5.760, 6.077] |
| Apalis | 353.3 [326.6, 366.9] | 461.6 [419.4, 481.3] | 0.552 [0.532, 0.635] |

Values are medians with full ranges. Runner CPU includes startup, idle, and shutdown. The scorecard separately records CPU sampled on the pinned pair, including PostgreSQL. Host steal was zero in all measured Mesh, River, and Apalis arms. The highest observed Oban steal was 0.242%. [CPU and clock receipts](evidence/native-analysis/paired-scorecard.json)

| Paired peer | Mesh submit savings, ms | Submit wins | Mesh drain savings, ms | Drain wins |
| --- | ---: | ---: | ---: | ---: |
| River | +102.2 [−107.3, +267.8] | 6/8 | +198.6 [−19.3, +337.3] | 7/8 |
| Oban | +124.3 [+43.8, +245.3] | 8/8 | +169.4 [+50.9, +297.5] | 8/8 |
| Apalis | −337.9 [−548.4, −281.9] | 0/8 | −136.3 [−277.9, −84.2] | 0/8 |

The signed differences are peer time minus Mesh time within each block. River committed enqueue a paired median 80 ms earlier, which reduced Mesh's advantage on the submit clock. River's interval from the last effect commit to terminal was a median 151 ms longer. Oban's last effect commit arrived a paired median 175 ms later from enqueue commit. Apalis committed enqueue a paired median 218 ms earlier and completed its last application effect a median 138 ms earlier from enqueue commit. These phase differences quantify when the gaps accumulated. [Per-block phase decomposition](evidence/native-analysis/paired-scorecard.json)

## Work, overlap, and limits

The offline replay matched the controller's scores from 68 raw exports. That covers four SETUP arms, 32 measured arms, and 32 warmups. Every arm reached 64 concurrent committed holds globally and 32 per runner. The shortest hold in the 64 timing exports was 100.425 ms. The timed arms completed 16,384 jobs with one attempt and one effect per job. Each arm had one enqueue XID and 1,024 independent application XIDs, disjoint from its terminal XIDs. [SETUP replay](evidence/native-analysis/setup-replay.json) · [Per-arm timing and ownership](evidence/native-analysis/timing-replay.json)

| Engine | Observed physical peak, full measured range | Terminal XIDs per batch | Median claim SQL calls | Median settlement SQL calls |
| --- | ---: | ---: | ---: | ---: |
| Mesh V1 bulk | 17 | 512 | 133 | 512 |
| River | 68–69 | 10–12 | 147.5 | 10.5 |
| Oban | 69 | 512 | 74.5 | 512 |
| Apalis | 69 | 36–42 | 48 | 40 |

Every arm stayed within cap 71 and the local and global slot limits. No administrator client was present during the scored interval. Mesh held eight warm application sessions and used four on every measured arm. Its producer retained eight true-bulk calls for 504 inserts and eight scalar inserts. The distinct terminal XID counts match the settlement SQL calls and cover 512 terminal rows per arm. The replay also records claim rows, SQL execution time, and lease or maintenance counters per arm. River's two detached listeners remain part of its 71-connection maximum allocation even though the replay observed peaks of 68 to 69 in this series. [Connection budget](evidence/cap-proof.json) · [SQL and physical-session receipts](evidence/native-analysis/paired-scorecard.json)

All eight planned timing blocks qualified with no exclusions. Both timing grants released with exit status 0. The original Mesh SETUP failure remains archived separately. It stopped before submit with SQLSTATE `42703` and contributed no score. [Controller timing releases and raw exports](evidence/native/sql-retry-timing) · [Original failure and repair](SQL-AUDIT-RETRY.md)

## Portable replay for the viewer

Use `replay_native(raw_series_path)` from [native-report.py](native-report.py). The series path names the directory that contains `series.json` and `blocks/`. The function accepts the controller's timing archive below or a renamed copy. It verifies the block seals and recomputes the result from raw exports. The default requires the complete eight-block series. The result's `qualified` and `peer_minus_mesh_ms` keys contain the per-arm scores and the paired clock comparisons. `scorecard(replay)` returns the aggregates.

```python
from pathlib import Path
from runpy import run_path

packet = Path("tools/durable-jobs-bench/results/2026-11-24-ordinary-slow-equal-topology-cap71")
api = run_path(str(packet / "native-report.py"))
raw_series = packet / "evidence/native/sql-retry-timing"
replay = api["replay_native"](raw_series)
scores = api["scorecard"](replay)
```

The callable needs the sealed source files in the checkout and the raw series directory. It starts no workload, requires no staged binary, and works outside the original raw archive path.
