# CPU budget for the public dense individual-terminal comparison

Across six matched 1,024-job Linode rotations, hot Mesh used at least **2.020–2.210 CPU-seconds** on CPUs 0 and 1 between first submit and last durable settlement. At River’s elapsed time those CPUs could supply **2.019–2.050 CPU-seconds**. Mesh’s recorded work exceeded that capacity in every pair. The smallest excess was 1 ms across the entire batch. Against a target of 20% higher throughput than River, the shortfall was **337–512 CPU-ms per batch**. An optimization that only hides waits would also have to reduce or move some of the recorded work outside the window to reach that target.

| Rotation | Mesh submit busy floor, CPU-ms | Two CPUs at River submit time, CPU-ms | Excess for parity, CPU-ms | Excess for 20% higher submit throughput, CPU-ms |
|---|---:|---:|---:|---:|
| 1 | 2,122 | 2,034 | 89 | 428 |
| 2 | 2,210 | 2,037 | 172 | 512 |
| 3 | 2,052 | 2,041 | 11 | 351 |
| 4 | 2,020 | 2,019 | 1 | 337 |
| 5 | 2,118 | 2,050 | 69 | 410 |
| 6 | 2,189 | 2,022 | 167 | 504 |

Columns are rounded independently. Taking first enqueue commit as the start, the hot Mesh floor exceeded River's drain capacity in three pairs. The other three leave drain parity by better overlap unresolved. Beating River's drain throughput by 20% would require removing or moving at least **239–409 CPU-ms** in all six pairs under the same host-load assumption.

## Interpretation

The floor includes all busy work on the two assigned logical CPUs. It includes the runner, PostgreSQL, kernel and any incidental host activity on those CPUs. It cannot allocate the excess to SQL, the scheduler or an individual process. Moving CPU outside a measured clock can change the score without reducing whole-workload cost. The 20% target here quantifies the engineering scale and does not change any candidate's admission rule.

## Replay

Run `python3 -B replay.py` here to reproduce [bounds.json](bounds.json) and `python3 -B test_bounds.py` for the synthetic edge and quantization checks. The replay verifies the raw archive hash, host, completed job count and transaction timestamps for each of the 36 measured arms from the [six-rotation release](../2026-11-24-dense-individual-hot-pool/logger-off-retry/NATIVE-RESULTS.md). Warmups contribute zero samples.

The pinned worker reads per-CPU counters before recording each timestamp. Between ordinary samples it finishes the previous sample and waits at least 200 ms for the still-running adapter. The reducer skips the initial sample and final early-return sample and allows 2 ms of clock uncertainty at each edge. For each pair of remaining frames it subtracts one 100 Hz tick per increasing busy counter and subtracts the full two-core capacity of all time possibly outside the ledger window. It uses the largest such lower bound. This assumes ordinary kernel tick accounting for the pinned host. No interpolation or extrapolation of sampled utilization is used.

## How much of the submit gap can individual statements explain?

The six hot arms each report all 1,024 first application INSERTs, 1,024 second INSERTs, and 1,024 individual settlement calls. There were zero statement-counter evictions. The two serial INSERT executors together used **70.4–78.9 ms of total elapsed executor time** per batch, summed over all application backends. Even giving a hypothetical optimization CPU credit for that entire elapsed sum leaves **260–433 CPU-ms** of the submit target elsewhere in each rotation. The two writes still have to commit separately. Planning, WAL flush on transaction completion, client protocol work and subsequent scheduling are outside that executor-only estimate.

The 1,024 scalar settlement SELECTs accumulated **408–447 ms** of executor time per batch, including nested function work and waits. A change confined to the settlement executor alone would need to remove the equivalent of **82–124%** of that recorded elapsed sum as CPU. In rotations 2 and 6, even adding both INSERT executors to the settlement executor leaves a shortfall. This is an upper bound on direct CPU savings from serial executor work. Shortening a lock hold could also change work in other sessions. The archive does not price that indirect effect.

The alternative of finding the whole gain on the runner side requires the equivalent of **62–93% of both Mesh runners' combined lifetime CPU**, including startup and idle. These bounds favor an experiment that measures which PostgreSQL work actually consumes CPU while the existing dispatcher and concurrency changes are tested.

Run `python3 -B sql_budget.py | cmp - sql-budget.json` in this directory for all six matched comparisons. It also reruns the original 36-arm host budget and verifies the clock, statement counts and archive hashes. [Per-group SQL and runner budgets](sql-budget.json)
