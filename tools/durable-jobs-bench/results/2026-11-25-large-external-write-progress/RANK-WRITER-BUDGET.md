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

## Release and reproducibility

Canonical release `04e0e316` is exit 1. All 30 arms ran and exported; strict replay rejected all 12 recorded arms at the per-segment CPU upper bound. The largest enqueue and submit spreads between identical OFF arms were 121.672 and 222.888 ms against the 15 ms limits. The wall-time calculation describes the observed release and supplies no stock-path performance qualification.

From the repository root, run the [raw reducer](rank_writer_budget.py) with the directory containing the controller release, native status and raw archive. It verifies all three bindings, the 12 trace/backend identities, all 768 scalar calls and the complete ordered marker sequence for each writer.

```sh
d=tools/durable-jobs-bench/results/2026-11-25-large-external-write-progress
python3 -B "$d/rank_writer_budget.py" "$evidence"
```
