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
