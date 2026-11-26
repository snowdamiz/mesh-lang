# Public dense queue with hot PostgreSQL checkout

Six Linode rotations used two CPUs shared with PostgreSQL 17.11. Every engine used two 32-slot runners. The three Mesh lanes used the same public trusted executable and grouped successful completions after actor teardown. Every measured arm and warmup completed 1,024 jobs and reconciled 2,048 distinct application transactions.

| Lane | Submit to settlement, jobs/s | Commit to settlement, jobs/s | Hot wins, submit / drain |
|---|---:|---:|---:|
| Mesh hot | 757.42 | 859.78 | — |
| Mesh checked A | 693.10 | 780.50 | 6/6 · 6/6 |
| Mesh checked B | 691.05 | 776.14 | 6/6 · 6/6 |
| Oban | 762.02 | 854.02 | 2/6 · 3/6 |
| River | 1,001.10 | 1,083.72 | 0/6 · 0/6 |
| Apalis | 1,360.67 | 1,437.05 | 0/6 · 0/6 |

The hot pool passed the predeclared four-of-six advancement rule. Qualifying groups were 1, 2, 4, 5, 6. Hot checkout made 4 probes per block. The checked lanes made 2048 probes per block.

## Work remaining when a peer finishes

Each comparison aligns its own enqueue commit to zero. Counts show how many Mesh jobs still had to enter their handler or durably commit their second write when the paired peer had settled its last job. Values are medians of six within-rotation differences or counts.

| Peer | Extra Mesh drain, ms | Jobs still to enter | Second writes still to commit | Extra Mesh time for second writes 103–922, ms |
|---|---:|---:|---:|---:|
| Oban | +5.6 | 0 | 3.5 | +80.0 |
| River | +246.3 | 187.5 | 197 | +579.1 |
| Apalis | +471.5 | 393 | 404.5 | +428.9 |

Mesh's last application commit preceded final settlement by 0.7–3.7 ms. Earlier settlement can also change when slots refill. The ledgers measure the overlap and leave the CPU and lock costs of each stage open.

Run `python3 -B native_report.py` to reproduce this report from the archived ledgers. The original controller decision and all measured and warmup archive hashes are in [final-replay.json](evidence/native-closure-retry/final-replay.json).
