# How much of the FILE writer exchange can be PostgreSQL Bind?

The client-visible wait from sending a scalar write to reading BindComplete includes nearly the entire execution of the SQL writer. In each of the 12 recorded arms of LARGE FILE diagnostic `7e949089`, at least **71.2–77.9%** of the interval from send completion to reading BindComplete was already occupied by scalar executor elapsed. The total wall time outside the measured executor across **all 64 complete writer exchanges** was at most **71.942–77.491 ms** per arm (median **74.992 ms**).

A change that saves time only before execution, and only inside these exchanges, would have to remove at least **77–84% of that entire residual** to save 60 ms in these runs. The residual includes sending the frames, Parse, Bind, planning, waiting and receiving the rest of the response. The split-text component also changes how the SQL writer obtains its args. The split-JSON control helps distinguish gains inside the writer from gains on wire input. The component needs measured end-to-end savings after paying to stage the parameters and preserve the original fingerprint.

## How the bound works

The same producer backend serviced 64 serial top-level executions of the exact scalar SELECT. The recorder brackets each client send, send completion, BindComplete read and ReadyForQuery read with monotonic timestamps. The PostgreSQL execution for a call fits inside its full send-to-ready interval. Subtracting the upper bounds for both end intervals (initial send and BindComplete-to-ready) from the backend’s aggregate executor elapsed gives a lower bound on execution inside the middle interval. Dividing by the upper wall bound for the middle interval gives a conservative fraction. This calculation needs neither synchronized server/client wall clocks nor per-event backend CPU readings. It cannot allocate that executor time by rank or identify the CPU portion of execution.

| Across 12 observed arms, median per arm | Milliseconds per 64 calls |
|---|---:|
| Full writer exchanges, upper wall bound | 330.663 |
| Scalar executor elapsed | 254.973 |
| Wall outside the executor, upper bound | 74.992 |
| Send complete to observed BindComplete, upper wall bound | 301.789 |
| Executor elapsed within that middle interval, lower bound | 226.835 |

The medians summarize each paired quantity separately. Read each arm’s own bounds before subtracting or forming ratios.

## Where the split-bind component saved time

In screen `03e15310`, the split-JSON writer saved **101.283–140.706 ms** per 64 admissions against the faster stock duplicate in each of six blocks. The writer's measured executor elapsed accounted for **40.572–58.438 ms** of those savings. Split text saved **106.869–146.818 ms** overall, including **44.287–59.555 ms** inside the writer executor. Both estimates use the same stock arm for every term in the subtraction. All 24 arms committed the independent product write and 64 jobs. The preceding unscored release `eef3ab3e` verified physical args, ordered fingerprints and the normalized savepoint and release keys.

| Block | Faster stock by enqueue | Split JSON executor saving (ms) | Split JSON remaining wall saving (ms) | Split text executor saving (ms) | Split text remaining wall saving (ms) |
|---|---|---:|---:|---:|---:|
| 1 | B | 58.438 | 82.268 | 59.555 | 87.263 |
| 2 | B | 49.711 | 51.572 | 46.615 | 60.254 |
| 3 | B | 56.981 | 71.982 | 53.039 | 86.795 |
| 4 | B | 40.572 | 72.573 | 51.935 | 85.642 |
| 5 | B | 49.823 | 83.054 | 57.066 | 82.095 |
| 6 | A | 51.909 | 85.878 | 44.287 | 93.600 |

The component starts after constructing the binds and stops after receiving COMMIT. Its remaining wall includes the rest of the writer exchange, all 64 namespace fence statements, savepoints and releases, the independent insert and the outer transaction. In five split-text blocks and three split-JSON blocks this saving alone exceeded **77.491 ms**, the largest upper bound for all non-executor time inside the 64 stock writer exchanges in the separate, instrumented FILE series. Those measurements use different clients and runs. Charging the entire residual saving to the FILE writer's Bind phase would exceed the available clock in those cases.

If the observed executor gains transferred in full, split JSON would need another **6.524–25.079 ms** of net savings per block to clear the component's elapsed floors. Split text would need **5.407–22.430 ms**. Any added broker preparation cost increases the required saving elsewhere. The product comparison must measure enqueue through durable commit and submit through last settlement with the private writer exercised on the eligible path. Its drain clock starts at commit.

For the separate historical EXTERNAL FILE series, the fourth Oban rotation required **254.8 ms** of submit savings to reach 1.20× throughput at fixed drain. Even transferring the largest split-text saving in full leaves about **108 ms** to find there. Across all five rotations the independent drain requirement remains **208.7–439.1 ms**. [Matched peer deadlines](README.md#which-clock-still-needs-to-move-on-public-file)

At product source `81629be35`, each eligible scalar call binds the fifth parameter as JSONB, checks availability in a separate SQL exchange, and checks the writer's routine definitions before and after admission. The split-JSON component's remaining wall margin per job after its original floor was **1.183, 0.570, 0.990, 0.742, 1.014 and 1.110 ms** in order. In the tightest block, an extra **36.487 ms per 64 admissions** would consume the margin if all other costs transferred unchanged. The native product run must also account for any broker CPU saved constructing the split spec and for its own measured duplicate spread.

Run `rank_writer_budget.py RANK_EVIDENCE --split-bind SCREEN_EVIDENCE` to reproduce the table and the per-job margins. It rehashes both raw archives and controller releases, joins each component snapshot to its writer and arm, and checks the 12 FILE traces.

## Where the longest SQL calls occurred

One backend executed the 64 writes serially. Its recorded maximum executor time can belong only to a call whose client send-to-ready upper bound is at least that large. In 11 of 12 arms exactly one call fits. Ten of those maxima fall at ranks **16–19 or 42–49**; the other is rank 1. In the remaining arm ranks 1 and 45 both fit. **No arm’s longest SQL execution falls in ranks 2–11.** The single worst call therefore cannot explain the repeated difference between the first two groups of five writer exchanges. Storage, WAL and scheduling causes remain unassigned.

Even removing the worst SQL execution entirely would save at most **12.608–42.135 ms** per observed arm. To price multiple stalls, the reducer also asks what the aggregate executor saving would be if every SQL call above a threshold finished at that threshold instead, with everything else held fixed. Thresholds are sensitivity assumptions. The existing 60 ms enqueue gate stays fixed.

| Assumed cap on individual SQL execution | Arms where even the upper bound falls below 60 ms (of 12) |
|---|---:|
| 15 ms | 12 |
| 10 ms | 11 |
| 5 ms | 4 |

These bounds use the backend’s count, sum, minimum, maximum and population standard deviation, plus each client call’s wall upper bound. For a threshold T above the measured minimum, subtract T only from the portion of each execution above it. The maximum possible saving is bounded by summing the positive excess of the individual wall bounds, capped by the largest SQL time. The variance supplies a lower bound because for an execution x, (x − min)(x − T) is at most (max − min) times the positive excess above T. The reducer rounds bounds outwards. Even the upper bound assumes one-for-one transfer of saved executor elapsed to enqueue, with no extra work. Run with `--spikes` for the per-arm rank sets and bounds. The clusters’ initial WAL offsets and individual segment allocations are unavailable from these traces.

## Broker CPU before the first write and between later calls

The first FILE command through the start of Rust admission preparation used **120.060–139.337 broker-thread CPU-ms**. The 63 intervals from receipt of the previous savepoint release to the start of the next preparation used **120.047–125.108 CPU-ms** in total. These windows are disjoint. The first includes reading and parsing the 33,561,352-byte file, request validation, transaction setup and the first collection. The later windows include processing the prior receipts, collecting and encoding further specs, and decoding the next batch before the preparation marker. Run the reducer with `--client` to inspect the same-thread clock differences from all 12 arms.

The first 15 of those later intervals used 49.915–54.370 CPU-ms per submission. Even eliminating all that CPU would fall short of 60 ms on a one-for-one transfer to enqueue. The remaining 48 intervals used 69.064–72.311 CPU-ms. Attributing a benefit to a specific encoder or file reader requires measuring its share of the corresponding window on the matched Linux images. The archived observer failed its perturbation gates.

## How much encoder CPU is in overflow peeks?

In native FILE encoder SETUP `12faf76a`, the collector encoded each of 64 accepted specs and also encoded the next spec on each of the first 63 calls to check the batch limit. The next call encoded that same spec again. The table gives same-thread CPU in milliseconds from one instrumented submission per image.

| Encoder | 64 accepted encodes | 63 overflow peeks | 63 accepted re-encodes after a peek |
|---|---:|---:|---:|
| Stock | 38.668 | 28.124 | 37.316 |
| Copy control | 27.095 | 14.767 | 25.851 |
| Fast | 12.324 | 13.433 | 11.685 |

A collector that carries the already-encoded overflow spec would save work from the last column. A collector that can establish overflow before encoding would save work from the middle column. These are alternative ways to eliminate one encode per overflow. The first column already includes the last column. Both alternatives would need to preserve the supplier's observable behavior. With the fast encoder, removing all 63 accepted re-encodes from its measured 25.757 CPU-ms gives a combined encoder saving of **52.720 CPU-ms** against the stock total of 66.792. The product enqueue gate requires 60 ms of wall time. The quiet product arms measure that clock.

Run [file_encode_visits.py](file_encode_visits.py) with the released evidence directory and canonical release JSON to recover each instrumented arm's CPU and wall sums. The reduction checks the raw and replay digests, all 127 ordered visits, the size of each encoded spec, and the original per-arm totals.

## Release and reproducibility

Canonical release `04e0e316` is exit 1. All 30 arms ran and exported; strict replay rejected all 12 recorded arms at the per-segment CPU upper bound. The largest enqueue and submit spreads between identical OFF arms were 121.672 and 222.888 ms against the 15 ms limits. The wall-time calculation describes the observed release and supplies no stock-path performance qualification.

From the repository root, run the [raw reducer](rank_writer_budget.py) with the directory containing the controller release, native status and raw archive. It verifies all three bindings, the 12 trace/backend identities, all 768 scalar calls and the four ordered boundary markers used for each writer.

```sh
d=tools/durable-jobs-bench/results/2026-11-25-large-external-write-progress
python3 -B "$d/rank_writer_budget.py" "$evidence"
```
