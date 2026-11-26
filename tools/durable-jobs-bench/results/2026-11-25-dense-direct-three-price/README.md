# What a third worker would have to recover on dense 2×32

The private fixed8+d388 runner can bind application connections directly. Its two-worker and four-worker deployments ran in the same two native blocks. A three-worker deployment would have to deliver most of the four-worker submit gain on a small share of the extra runner CPU in the reverse order. Keep the native slot for the LARGE factorial.

The values below price the original two service2 duplicates’ clock floors and runner CPU ceiling. Each wall-time percentage divides the advance needed from direct2 by the measured direct2-to-direct4 advance in that block. The CPU percentage divides the headroom above direct2 by the measured direct4 CPU increase. Each percentage has its own denominator.

| Quantity per 1,024 jobs | Forward | Reverse |
|---|---:|---:|
| CPU headroom above direct2 | 55.684 ms | 23.939 ms |
| Fraction of direct4 extra CPU affordable | 70.6% | 27.3% |
| Submit advance required from direct2 | 10.800 ms (10.2%) | 56.465 ms (78.9%) |
| Drain advance required from direct2 | 34.590 ms (30.5%) | 37.474 ms (55.4%) |
| Middle 512 durable writes advance required | 10.062 ms (26.8%) | 17.058 ms (65.5%) |

The clock gates require savings strictly above their thresholds. Three workers have no measured result. The two endpoints do not establish how the marginal CPU or throughput scales. The original bridge stability checks missed in both orders, and forward direct2 had 39 of the expected 41 clients ready. These figures justify keeping this configuration off the queue on current evidence.

The replay reads Git objects at `f9002c6ce`, verifies the release and all 12 measured archive hashes, and checks runner lifetime CPU and all three clocks directly against the 1,024-job ledgers. The original pilot independently verified its 12 warmups as well.

```sh
d=tools/durable-jobs-bench/results/2026-11-25-dense-direct-three-price
python3 -B "$d/replay.py" | cmp - "$d/price.json"
```
