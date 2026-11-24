# Application writes and durable settlement on the Linode

## Dense jobs with two 32-slot runners per engine

The first three columns count jobs between timestamps. For each job, the replay clips the interval to the 103rd through 922nd second-write commits, sums the overlapping durations, and divides by elapsed time. The table shows the median across five rotations. Each arm committed 2,048 application writes for 1,024 jobs.

| Engine | Entered, awaiting first commit | Between application commits | Second commit to terminal | Last entry (ms) | Last write (ms) | Final settlement after last write (ms) |
|---|---:|---:|---:|---:|---:|---:|
| Mesh V1 | 10.27 | 1.53 | 5.56 | 1,326.66 | 1,334.41 | 1.40 |
| River | 2.56 | 10.69 | 171.54 | 743.20 | 746.07 | 191.81 |
| Oban | 6.64 | 10.85 | 8.44 | 1,144.64 | 1,153.73 | 3.76 |
| Apalis | 4.84 | 7.81 | 25.50 | 702.79 | 710.82 | 7.66 |

Last entry and last write are measured from each arm’s durable enqueue commit. The intervals include scheduling, application code, database work and waits. A peer can continue taking jobs while earlier terminal acknowledgements are outstanding.

Mesh V1 records entry before decoding the context and calling the application service for its pool handle. River and Apalis receive a pool with their worker state. The ledgers have no timestamp for the Mesh service reply. A native comparison that changes only how the handler receives its pool can price that step. [Mesh fixture](../2026-11-23-ordinary-trusted-v1/ordinary-trusted-v1.tar.gz) · [River worker](../../go/worker.go) · [Apalis worker](../../rust/src/main.rs)

## Short jobs with the same bulk Mesh and River binaries

Each row compares 128-job runs. A positive gap means Mesh reached that boundary later. Last-write gaps use enqueue commit as the origin. The terminal tail measures elapsed time between the final application commit and the final durable settlement across the arm.

| Series | Rotation | Mesh enqueue (ms) | River enqueue (ms) | Last-write gap (ms) | Mesh terminal tail (ms) | River terminal tail (ms) | Submit-to-settlement gap (ms) |
|---|---:|---:|---:|---:|---:|---:|---:|
| Three lanes | 1 | 34.15 | 13.50 | +19.30 | 1.34 | 23.16 | +18.13 |
| Three lanes | 2 | 34.43 | 14.96 | +5.16 | 3.63 | 101.89 | -73.62 |
| Three lanes | 3 | 37.21 | 13.13 | +18.33 | 1.99 | 14.58 | +29.81 |
| Three lanes | 4 | 35.38 | 13.96 | +4.52 | 3.99 | 10.85 | +19.09 |
| Three lanes | 5 | 32.74 | 13.15 | +4.31 | 3.11 | 95.99 | -68.98 |
| Six lanes | 1 | 34.00 | 16.47 | +6.34 | 2.64 | 95.09 | -68.58 |
| Six lanes | 2 | 34.78 | 13.97 | +11.31 | 2.09 | 100.14 | -65.93 |
| Six lanes | 3 | 49.22 | 17.28 | -9.72 | 0.86 | 27.75 | -4.67 |
| Six lanes | 4 | 36.23 | 16.86 | -8.38 | 5.78 | 94.17 | -77.40 |
| Six lanes | 5 | 38.44 | 15.89 | +10.91 | 2.81 | 90.42 | -54.15 |

The two series ran at different times and in different orders. The observed tails alone cannot identify the cause of their variation. [Native rates and paired wins](four-engine-scorecard.md) · [Score inputs and raw archive digests](scores.json)

## PostgreSQL execution time for the dense Mesh handler

Each 2×32 Mesh run recorded 2,048 pool checkout probes, 1,024 attempt inserts and 1,024 effect inserts for its dedicated application role. Every call returned or inserted one row. The first three times come from PostgreSQL’s cumulative `pg_stat_statements.total_exec_time` for those calls.

| Rotation | Checkout (ms) | Attempt INSERT (ms) | Effect INSERT (ms) | Total (ms) | Residual vs River (ms) | Residual vs Oban (ms) | Residual vs Apalis (ms) |
|---:|---:|---:|---:|---:|---:|---:|---:|
| 1 | 7.71 | 43.54 | 30.64 | 81.89 | +303.58 | +94.82 | +537.36 |
| 2 | 6.57 | 37.21 | 35.32 | 79.10 | +439.24 | +80.42 | +550.16 |
| 3 | 7.28 | 37.73 | 28.86 | 73.88 | +295.93 | -8.58 | +494.22 |
| 4 | 8.40 | 44.13 | 31.52 | 84.06 | +303.62 | +95.03 | +447.44 |
| 5 | 5.73 | 39.28 | 30.56 | 75.57 | +368.12 | +128.74 | +595.79 |

The residual subtracts the entire PostgreSQL execution total from Mesh’s observed enqueue-commit-to-final-settlement time and compares it with the peer in the same rotation. Positive means Mesh would still finish later. This credits every millisecond to the critical path even when backends overlapped and holds all other work fixed. The counters leave client scheduling, protocol exchanges, planning and transaction commit time unpriced. They cannot predict the effect of changing concurrency. [Per-arm archive digests](scores.json) · [Raw equal-topology series](../2026-11-23-dense-matched-topology/evidence/equal/)

### Dense jobs already claimed by each Mesh runner

The ledger links each job to one of the two runner PIDs. Over the 103rd to 922nd second writes, the table counts jobs with a database claim stamp that have yet to enter their handler. The percentages use elapsed time across both runners. Each runner has two scheduler workers.

| Rotation | Mean stamped jobs awaiting entry per runner | Backlog with 0–1 handlers between entry and second write | Backlog with 2+ handlers between entry and second write |
|---:|---:|---:|---:|
| 1 | 10.85–11.24 | 7.4–7.7% | 78.7–79.2% |
| 2 | 9.88–10.26 | 8.5–8.8% | 75.5–76.3% |
| 3 | 10.79–11.19 | 5.6–6.0% | 79.1–79.7% |
| 4 | 10.92–11.32 | 6.6–6.8% | 81.6–82.4% |
| 5 | 10.98–11.37 | 6.6–6.9% | 81.9–82.5% |

The ranges allow for millisecond claim stamps. A stamp precedes the client’s COMMIT acknowledgement. The handler interval includes decoding, scheduling and database waits. It measures overlap, while actual scheduler occupancy and the number of in-flight SQL requests require separate observations.

## Exit-confirmed short jobs against stock peers

The Mesh runner uses the bulk producer, skips checkout probes for recently used connections and waits for actor teardown before settlement. Each signed difference is Mesh elapsed time minus the paired peer time. The three intervals partition the full submit-to-settlement clock. Positive values mean Mesh took longer.

| Rotation | Peer | Submit to enqueue commit (ms) | Enqueue commit to last application write (ms) | Last write to last settlement (ms) | Full submit difference (ms) |
|---:|---|---:|---:|---:|---:|
| 1 | River | +25.37 | +34.35 | -9.35 | +50.38 |
| 1 | Oban | -11.81 | -215.13 | +0.56 | -226.39 |
| 1 | Apalis | +28.63 | +25.97 | -2.09 | +52.51 |
| 2 | River | +19.18 | -1.63 | -39.34 | -21.79 |
| 2 | Oban | -24.31 | -191.93 | -0.80 | -217.04 |
| 2 | Apalis | +25.76 | +35.61 | -2.68 | +58.70 |
| 3 | River | +23.10 | -3.90 | -90.46 | -71.27 |
| 3 | Oban | -12.46 | -204.29 | +2.08 | -214.67 |
| 3 | Apalis | +23.71 | +14.92 | +0.41 | +39.04 |
| 4 | River | +22.05 | +1.14 | -48.90 | -25.71 |
| 4 | Oban | -13.58 | -228.70 | -3.21 | -245.48 |
| 4 | Apalis | +25.22 | +23.37 | -3.98 | +44.60 |
| 5 | River | +33.44 | +60.17 | -8.79 | +84.82 |
| 5 | Oban | -11.86 | -230.92 | -1.82 | -244.60 |
| 5 | Apalis | +36.36 | +72.24 | -2.13 | +106.47 |

All four engines committed two independent application transactions per job. A queue can group terminal acknowledgements and can have an earlier job still waiting to settle when the last application write commits. [Raw matched short runs](../2026-11-24-exit-confirmed-bulk-d2/evidence/native-timing/2026-11-24-exit-confirmed-bulk-d2/)

### Short-job database connections and write progress

The connection columns span all five rotations. Each count covers the queue stack's database connections, including listeners and the producer. The last two columns split each 128-job drain at the 16th committed second application write and show the paired runs’ medians.

| Engine | Last idle sample (connections) | Highest measured sample (connections) | Enqueue commit to 16th second write (ms) | 16th to 128th second write (ms) |
|---|---:|---:|---:|---:|
| Trusted Mesh · hot checkout | 13 | 13 | 20.62 | 145.53 |
| River | 4 | 18 | 76.11 | 79.47 |
| Oban | 19 | 19 | 225.37 | 168.98 |
| Apalis | 19 | 19 | 36.31 | 105.73 |

These snapshots show when connections were present. A separate matched run with River’s connections open before submission can measure how much pool startup changes its early writes and final settlement.

### Where the 16 slots spent time after the first 16 writes

Each row subtracts a peer’s time from trusted Mesh with hot checkout in the interval from its own 16th to 128th second write. The columns sum to the elapsed gap before rounding. They integrate the number of slots in each state over time and divide by 16. The last state includes admission, settlement and idle slots.

| Rotation | Peer | Handler entry to first write (ms) | Between writes (ms) | Before handler entry or after second write (ms) | Total difference (ms) |
|---:|---|---:|---:|---:|---:|
| 1 | River | +24.99 | +17.53 | +47.58 | +90.11 |
| 1 | Apalis | +18.00 | +7.31 | +23.97 | +49.28 |
| 2 | River | +13.85 | +6.00 | +46.21 | +66.06 |
| 2 | Apalis | +7.55 | +2.02 | +33.85 | +43.42 |
| 3 | River | +12.72 | +9.05 | +30.04 | +51.81 |
| 3 | Apalis | +7.70 | +0.23 | +18.97 | +26.91 |
| 4 | River | +11.54 | +7.60 | +46.21 | +65.35 |
| 4 | Apalis | +8.14 | +2.29 | +33.55 | +43.97 |
| 5 | River | +26.04 | +22.67 | +63.87 | +112.59 |
| 5 | Apalis | +21.24 | +17.19 | +48.65 | +87.08 |

The next table counts empty Mesh slots only while an unclaimed job is available to fill each one. It excludes empty slots after the last claim. Bounds place each millisecond claim stamp at the start or end of its bin, capped by handler entry. A claim stamp is written inside the transaction, before its COMMIT acknowledgement. The second column for each mode counts slots occupied after the second write through terminal commit. Both measures use the same 16-slot normalization. They include required work and cannot establish the speedup from combining settlement and refill.

| Rotation | Hot checkout, free with backlog (ms) | Hot, awaiting settlement (ms) | Checked checkout, free with backlog (ms) | Checked, awaiting settlement (ms) |
|---:|---:|---:|---:|---:|
| 1 | 29.99–36.99 | 27.70 | 25.34–32.33 | 17.18 |
| 2 | 30.03–37.03 | 21.92 | 28.17–35.16 | 22.40 |
| 3 | 27.26–34.25 | 22.49 | 26.11–33.10 | 20.31 |
| 4 | 31.93–38.92 | 25.52 | 29.21–36.20 | 19.24 |
| 5 | 37.57–44.56 | 29.67 | 32.01–39.00 | 21.74 |

## Dense jobs after hot checkout

Each value is the median of six native 1,024-job arms. The middle interval begins at the 103rd committed second application write and ends at the 922nd. The last three columns give time-weighted counts of jobs in each stage during that interval. A job can finish its writes and free a handler before the queue records its terminal commit.

| Engine | Enqueue (ms) | Middle writes (ms) | Commit to last write (ms) | Terminal tail (ms) | Entered, awaiting first write | Between writes | Awaiting settlement |
|---|---:|---:|---:|---:|---:|---:|---:|
| Mesh · hot checkout | 159.23 | 920.86 | 1211.39 | 1.43 | 8.65 | 1.48 | 5.49 |
| Mesh · checked checkout A | 194.93 | 1011.89 | 1354.67 | 1.77 | 9.22 | 1.60 | 4.50 |
| Mesh · checked checkout B | 167.67 | 1019.50 | 1348.14 | 1.10 | 10.81 | 1.66 | 4.85 |
| Mesh · zero-hold writer | 174.94 | 1046.54 | 1363.99 | 1.46 | 9.93 | 1.64 | 3.81 |
| Mesh · writer off A | 161.44 | 1001.02 | 1318.93 | 1.10 | 10.22 | 1.64 | 4.70 |
| Mesh · writer off B | 164.34 | 1013.79 | 1322.52 | 2.31 | 10.90 | 1.61 | 4.67 |
| Mesh V1 | 530.99 | 1085.22 | 1378.22 | 2.60 | 10.38 | 1.52 | 5.81 |
| Oban | 143.13 | 865.29 | 1235.41 | 1.53 | 6.24 | 9.21 | 7.87 |
| River | 79.40 | 376.35 | 791.00 | 166.58 | 2.75 | 10.10 | 168.22 |
| Apalis | 38.02 | 505.33 | 700.38 | 7.11 | 4.94 | 8.64 | 24.25 |

The next rows compare hot Mesh with River in each matched rotation. The producer, last-write and terminal-tail differences sum to the full submit difference. The middle-write column sits inside the last-write interval. Positive values mean Mesh took longer.

| Rotation | Producer (ms) | Commit to last write (ms) | Terminal tail (ms) | Full submit (ms) | Middle writes (ms) |
|---:|---:|---:|---:|---:|---:|
| 1 | +74.06 | +401.90 | -43.20 | +432.77 | +498.19 |
| 2 | +78.86 | +416.04 | -157.40 | +337.49 | +529.06 |
| 3 | +60.48 | +380.95 | -115.72 | +325.71 | +529.26 |
| 4 | +85.19 | +463.89 | -195.86 | +353.22 | +566.02 |
| 5 | +81.24 | +418.95 | -187.64 | +312.54 | +534.60 |
| 6 | +86.33 | +466.98 | -170.82 | +382.49 | +576.37 |

### Device flushes during the dense drain

The recorder reads the PostgreSQL device counters after timestamping each sample. The next sample bounds when that read finished. Inner differences use two reads fully inside enqueue commit through final settlement. Outer differences span that whole interval. Both bounds include every user of the same ext4 device. [Recorder](../../worker.py) · [Linux 6.8 disk counters](https://www.kernel.org/doc/html/v6.8/admin-guide/iostats.html)

| Rotation | Mesh flushes | River flushes | Mesh flush time (ms) | River flush time (ms) |
|---:|---:|---:|---:|---:|
| 1 | 1001–1641 | 110–492 | 71–117 | 9–50 |
| 2 | 1349–1660 | 326–554 | 93–115 | 23–40 |
| 3 | 1025–1622 | 308–490 | 67–109 | 24–39 |
| 4 | 988–1709 | 330–497 | 71–120 | 29–41 |
| 5 | 989–1620 | 401–562 | 71–117 | 30–44 |
| 6 | 1324–1678 | 271–456 | 95–121 | 22–38 |

Linux counts completed device flushes and accumulates their duration. Several independent commits can share a flush. Attribution to a specific backend or WAL record requires a PostgreSQL measurement.

### CPU allowance at River’s middle-write pace

The 103rd through 922nd second-write timestamps enclose 819 advances in rank. Mesh’s busy-CPU lower bound uses readings wholly inside its window with the archived clock and tick allowances. River’s elapsed time multiplied by two CPUs gives the maximum CPU time available at its observed pace. Both columns use CPU-ms per advance. The readings cover all activity on the two pinned CPUs. [CPU bounds replay](../2026-11-23-ordinary-native-cpu/replay/offline.py)

| Rotation | Mesh busy CPU, lower bound | River total CPU allowance | Lower bound minus allowance |
|---:|---:|---:|---:|
| 1 | 0.965 | 0.980 | -0.016 |
| 2 | 0.977 | 0.939 | +0.038 |
| 3 | 0.977 | 0.859 | +0.118 |
| 4 | 0.965 | 0.899 | +0.066 |
| 5 | 0.977 | 0.961 | +0.016 |
| 6 | 0.977 | 0.880 | +0.097 |

In 5 of 6 rotations, the observed minimum CPU charge would exceed the time available at River’s middle-write pace even with both CPUs fully busy. A change could reduce work, move it outside this window, or do both. The samples leave its process and SQL attribution open.
