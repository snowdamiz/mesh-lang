# GNU release Jobs parity for sparse and short16

Both release images come from source f8f8ed21 with the existing compiled named-effect fixture and native constructor. All 19 source gates passed. The runtime archive and link inputs match the accepted relink receipt.

- Ordinary GNU SHA-256 32ceecf560ef70c5f1443d0688690a77a206070a897156a58b7ba321cd2f3070.
- Exact layout SHA-256 f14316c56d193cedd74b9d1b2be1940eff2d4844cd5532750dcae773885c17f4.

## Sparse, one slot

5 of five preselected groups satisfied the evidence checks. Rows follow the actual order within each guest.

Median [min, max] over eligible groups.

| Arm | Durable jobs/s | Pickup p99 ms | Claim p99 ms | CPU ms/job | Peak RSS MiB |
| --- | --- | --- | --- | --- | --- |
| Ordinary GNU | 2.798 [2.696, 2.864] | 23.522 [9.769, 33.883] | 13.537 [6.036, 30.032] | 23.234 [15.626, 33.010] | 28.832 [28.676, 30.672] |
| Exact layout | 2.699 [2.035, 2.864] | 33.360 [10.042, 141.452] | 24.987 [4.884, 49.082] | 30.933 [18.430, 84.255] | 26.809 [24.020, 28.621] |
| Stock Oban | 2.016 [1.975, 2.043] | 246.942 [234.195, 312.394] | 244.359 [232.795, 276.145] | 52.800 [39.507, 72.625] | 113.719 [110.957, 114.438] |

| ELF vs Oban | Rate | Pickup p99 | CPU/job | RSS | Series gates |
| --- | --- | --- | --- | --- | --- |
| Ordinary GNU | 1.388× | 0.095× | 0.440× | 0.254× | Pass |
| Exact layout | 1.339× | 0.135× | 0.586× | 0.236× | Pass |

| Group | Arm | Jobs/s | Pickup p99 / max ms | Claim p99 ms | Conn / proc / thrd | RSS MiB | CPU ms/job | Enqueue / app / settle XIDs | Attempts / retries | App + startup resets | Same-boot gate |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| sparse-01-attempt-01 | Ordinary GNU | 2.696 | 33.787 / 33.787 | 26.622 | 5 / 2 / 14 | 30.527 | 21.065 | 24 / 48 / 24 | 24 / 0 | 48 + 1 | Pass |
| sparse-01-attempt-01 | Exact layout | 2.688 | 33.360 / 33.360 | 25.067 | 5 / 2 / 14 | 28.621 | 30.933 | 24 / 48 / 24 | 24 / 0 | 48 + 1 | Pass |
| sparse-01-attempt-01 | Stock Oban | 2.010 | 242.856 / 242.856 | 241.010 | 4 / 2 / 12 | 113.719 | 72.625 | 24 / 48 / 24 | 24 / 0 | — | Reference |
| sparse-02-attempt-01 | Stock Oban | 2.016 | 246.942 / 246.942 | 244.359 | 4 / 2 / 12 | 110.957 | 66.358 | 24 / 48 / 24 | 24 / 0 | — | Reference |
| sparse-02-attempt-01 | Exact layout | 2.864 | 10.042 / 10.042 | 4.884 | 5 / 1 / 10 | 24.020 | 18.430 | 24 / 48 / 24 | 24 / 0 | 48 + 1 | Pass |
| sparse-02-attempt-01 | Ordinary GNU | 2.864 | 18.026 / 18.026 | 9.836 | 5 / 2 / 14 | 30.672 | 15.626 | 24 / 48 / 24 | 24 / 0 | 48 + 1 | Pass |
| sparse-03-attempt-01 | Exact layout | 2.831 | 19.627 / 19.627 | 15.437 | 5 / 2 / 14 | 26.930 | 28.018 | 24 / 48 / 24 | 24 / 0 | 48 + 1 | Pass |
| sparse-03-attempt-01 | Stock Oban | 2.032 | 234.195 / 234.195 | 232.795 | 4 / 2 / 12 | 112.410 | 39.507 | 24 / 48 / 24 | 24 / 0 | — | Reference |
| sparse-03-attempt-01 | Ordinary GNU | 2.798 | 9.769 / 9.769 | 6.036 | 5 / 2 / 14 | 28.676 | 23.234 | 24 / 48 / 24 | 24 / 0 | 48 + 1 | Pass |
| sparse-04-attempt-01 | Ordinary GNU | 2.813 | 23.522 / 23.522 | 13.537 | 5 / 2 / 14 | 28.707 | 33.010 | 24 / 48 / 24 | 24 / 0 | 48 + 1 | Pass |
| sparse-04-attempt-01 | Stock Oban | 2.043 | 249.111 / 249.111 | 248.098 | 4 / 2 / 12 | 114.438 | 43.659 | 24 / 48 / 24 | 24 / 0 | — | Reference |
| sparse-04-attempt-01 | Exact layout | 2.699 | 37.791 / 37.791 | 24.987 | 5 / 2 / 12 | 26.680 | 32.063 | 24 / 48 / 24 | 24 / 0 | 48 + 1 | Pass |
| sparse-05-attempt-01 | Stock Oban | 1.975 | 312.394 / 312.394 | 276.145 | 4 / 2 / 12 | 114.059 | 52.800 | 24 / 48 / 24 | 24 / 0 | — | Reference |
| sparse-05-attempt-01 | Ordinary GNU | 2.787 | 33.883 / 33.883 | 30.032 | 5 / 2 / 14 | 28.832 | 27.387 | 24 / 48 / 24 | 24 / 0 | 48 + 1 | Pass |
| sparse-05-attempt-01 | Exact layout | 2.035 | 141.452 / 141.452 | 49.082 | 5 / 2 / 14 | 26.809 | 84.255 | 24 / 48 / 24 | 24 / 0 | 48 + 1 | budgets.cpu_seconds_per_job |

The 15-core host's one-minute load at arm start ranged from 23.2 to 128.3.

## Short16

5 of five preselected groups satisfied the evidence checks. Rows follow the actual order within each guest.

Median [min, max] over eligible groups.

| Arm | Durable jobs/s | Pickup p99 ms | Claim p99 ms | CPU ms/job | Peak RSS MiB |
| --- | --- | --- | --- | --- | --- |
| Ordinary GNU | 416.975 [143.906, 501.613] | 299.220 [241.990, 883.380] | 295.407 [236.166, 879.059] | 2.970 [2.078, 5.662] | 54.992 [42.266, 80.691] |
| Exact layout | 350.457 [166.696, 912.877] | 353.902 [137.588, 751.844] | 344.533 [134.397, 739.278] | 3.297 [1.342, 5.201] | 47.867 [44.879, 98.559] |
| Stock Oban | 572.961 [284.894, 977.965] | 218.356 [126.011, 438.401] | 215.325 [123.773, 432.809] | 8.670 [4.678, 24.108] | 120.188 [115.574, 123.047] |

| ELF vs Oban | Rate | Pickup p99 | CPU/job | RSS | Series gates |
| --- | --- | --- | --- | --- | --- |
| Ordinary GNU | 0.728× | 1.370× | 0.343× | 0.458× | comparison.jobs_per_second, comparison.pickup_p99_ms |
| Exact layout | 0.612× | 1.621× | 0.380× | 0.398× | comparison.jobs_per_second, comparison.pickup_p99_ms |

| Group | Arm | Jobs/s | Pickup p99 / max ms | Claim p99 ms | Conn / proc / thrd | RSS MiB | CPU ms/job | Enqueue / app / settle XIDs | Attempts / retries | App + startup resets | Same-boot gate |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| short-01-attempt-01 | Exact layout | 166.696 | 751.844 / 751.889 | 739.278 | 13 / 11 / 76 | 98.559 | 5.201 | 1 / 256 / 128 | 128 / 0 | 256 + 8 | comparison.jobs_per_second, comparison.pickup_p99_ms |
| short-01-attempt-01 | Ordinary GNU | 358.995 | 347.462 / 352.964 | 337.261 | 13 / 9 / 64 | 73.996 | 2.970 | 1 / 256 / 128 | 128 / 0 | 256 + 8 | comparison.jobs_per_second, comparison.pickup_p99_ms |
| short-01-attempt-01 | Stock Oban | 586.293 | 211.657 / 211.801 | 207.975 | 19 / 2 / 12 | 117.484 | 11.872 | 1 / 256 / 128 | 128 / 0 | — | Reference |
| short-02-attempt-01 | Stock Oban | 977.965 | 126.011 / 126.107 | 123.773 | 19 / 2 / 12 | 123.047 | 4.678 | 1 / 256 / 128 | 128 / 0 | — | Reference |
| short-02-attempt-01 | Ordinary GNU | 416.975 | 299.220 / 302.276 | 295.407 | 13 / 2 / 18 | 42.266 | 3.015 | 1 / 256 / 128 | 128 / 0 | 256 + 8 | comparison.jobs_per_second, comparison.pickup_p99_ms |
| short-02-attempt-01 | Exact layout | 740.390 | 158.889 / 159.305 | 156.024 | 13 / 4 / 32 | 44.984 | 1.668 | 1 / 256 / 128 | 128 / 0 | 256 + 8 | comparison.jobs_per_second, comparison.pickup_p99_ms |
| short-03-attempt-01 | Ordinary GNU | 467.175 | 259.964 / 264.966 | 250.559 | 13 / 5 / 40 | 54.992 | 2.379 | 1 / 256 / 128 | 128 / 0 | 256 + 8 | Pass |
| short-03-attempt-01 | Stock Oban | 334.446 | 375.797 / 375.824 | 372.791 | 19 / 2 / 12 | 120.188 | 8.670 | 1 / 256 / 128 | 128 / 0 | — | Reference |
| short-03-attempt-01 | Exact layout | 350.457 | 353.902 / 354.246 | 344.533 | 13 / 5 / 35 | 47.867 | 3.297 | 1 / 256 / 128 | 128 / 0 | 256 + 8 | Pass |
| short-04-attempt-01 | Exact layout | 227.167 | 530.970 / 559.388 | 517.806 | 13 / 5 / 36 | 47.918 | 3.501 | 1 / 256 / 128 | 128 / 0 | 256 + 8 | comparison.jobs_per_second |
| short-04-attempt-01 | Stock Oban | 284.894 | 438.401 / 438.438 | 432.809 | 19 / 2 / 12 | 120.914 | 24.108 | 1 / 256 / 128 | 128 / 0 | — | Reference |
| short-04-attempt-01 | Ordinary GNU | 143.906 | 883.380 / 883.446 | 879.059 | 13 / 5 / 41 | 53.855 | 5.662 | 1 / 256 / 128 | 128 / 0 | 256 + 8 | comparison.jobs_per_second, comparison.pickup_p99_ms |
| short-05-attempt-01 | Stock Oban | 572.961 | 218.356 / 218.378 | 215.325 | 19 / 2 / 12 | 115.574 | 8.077 | 1 / 256 / 128 | 128 / 0 | — | Reference |
| short-05-attempt-01 | Exact layout | 912.877 | 137.588 / 137.603 | 134.397 | 13 / 4 / 27 | 44.879 | 1.342 | 1 / 256 / 128 | 128 / 0 | 256 + 8 | Pass |
| short-05-attempt-01 | Ordinary GNU | 501.613 | 241.990 / 242.046 | 236.166 | 13 / 11 / 71 | 80.691 | 2.078 | 1 / 256 / 128 | 128 / 0 | 256 + 8 | comparison.jobs_per_second |

The 15-core host's one-minute load at arm start ranged from 25.7 to 122.1.

## Measurement and admission

Each group recreated the database for every arm inside one Linux ARM64 guest with two vCPUs, 2 GiB RAM and GNU PostgreSQL 17.11. Both sides used the scoped TLS producer/observer session, durable commits and zero reserved connections. The physical and per-database caps were five for sparse and 20 for short16. Stock Oban Basic 2.24.2 used Erlang 29.1.1, Elixir 1.20.4, the file-only resolver from 8526a5bd, and the original flags +S 2:2 +SDcpu 1:1 +SDio 1 +A 2. Each Mesh image was paired independently with that guest's Oban result.

Sparse ran 24 one-job batches at one slot and waited 250 ms after each settlement before the next enqueue. Short16 ran a 128-job batch at 16 slots. Both used 1 KiB payloads, the canonical 1 ms hold and three seconds of runner idle. Mesh retained the listener, batch claims, fixed 2/2 runner scheduler and worker-only THP diagnostic.

Durable rate spans the first enqueue commit through the last settlement commit. Pickup ends at handler entry, and claim latency ends at the recorded claim. Resource peaks use the original idle and active samples. RSS sums runner-tree RSS, including shared pages. CPU covers runner startup, idle, and reaped workers. PostgreSQL and producer processes are outside runner CPU and RSS. Setup and measurement-start host loads are in the receipt.

The scorer takes series medians for relative rate, pickup, CPU and RSS, and worst-group values for connection, process and thread caps. Sparse retains the 100 ms worst-group Mesh claim p99 and 750 ms pickup p99 limits. A timed gate miss counts in the series.

The job ledgers reconcile payloads, a single attempt and effect per job, and distinct enqueue, application and settlement XIDs. Both application transactions commit in order. Mesh also verifies distinct protected worker PIDs and matching named-effect XIDs. Its PostgreSQL statement counters reconcile two full post-transaction DISCARD ALL calls per job plus the separately counted startup resets. Oban performs its two application writes through its own database path.

The dense supervisor stopped before the first launch (observation deadline reached). The series took and released the shared blocking lock once per guest.

All ten groups completed on their first guest attempt.

## Exact worker lifecycle

- Exact layout has a separate exact-image held witness with 64 protected worker execs, zero-status exits and consuming waits. The observer checked the live parent and worker executable paths and mappings. The native constructor emitted one marker in the isolated check.
- Ordinary GNU has a separate exact-image held witness with 64 protected worker execs, zero-status exits and consuming waits. The observer checked the live parent and worker executable paths and mappings. The native constructor emitted one marker in the isolated check.

The timed groups attest the guest-verified ELF digests, untraced runner command paths and distinct committed worker PIDs. The held witnesses use a separate 64-slot cohort.

## Replay

Run from the repository root.

```sh
p=proof/jobs-parity/linux-committed-scheduler-sparse-short
(cd "$p" && shasum -a 256 -c SHA256SUMS)
python3 -B "$p/replay.py" | cmp "$p/score.json" -
```

The compressed ledger retains job timestamps, XIDs, attempt records, resource samples and reset counters. Original guest serial, packages, receipts, host load and retries remain under target/committed-scheduler-sparse-short. The earlier 9eda8768 evidence remains under target/integrated-runtime-sparse-large.

[Receipt](receipt.json) · [Frozen plan](plan.json) · [Scores](score.json) · [Source verification](../linux-integrated-uncut/source-verification.json) · [Accepted relink](../linux-integrated-uncut/relink-result.json)
