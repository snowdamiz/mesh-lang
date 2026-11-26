# Device flushes during the released cap68 SLOW refill pilot

The forward duplicate spread of 542.110 ms coincided with slower flushes on the PostgreSQL volume. Inside the drain, the two OFF controls recorded **0.068 and 0.334 ms per completed device flush**. In reverse, both recorded **0.067 ms**. These are host block-device counters. They cannot identify the backend, distinguish WAL from other writes, or measure how much of a particular job's latency came from the device.

| Measured arm | Device flush ms / count, forward | Device flush ms / count, reverse |
|---|---:|---:|
| Original stock | 69 / 1,033 | 68 / 1,032 |
| Same-image serial A | 71 / 1,044 | 68 / 1,030 |
| Same-image serial B | 366 / 1,096 | 69 / 1,034 |
| Adoption only | 437 / 1,270 | 66 / 913 |
| Full refill | 76 / 1,132 | 68 / 982 |

Host-wide full I/O pressure accumulated 272.828 ms inside the forward B observation and 30.109 ms inside A. The B observation spanned at most 1,790 ms and the A observation spanned at least 686 ms. Even using those conservative bounds, full pressure covered at least 15.2% of the B interval and at most 4.4% of A. The similar per-flush counts and differing pressure support treating the large duplicate spread as operationally meaningful. Neither metric provides a correction to subtract from the score.

The worker reads diskstats and PSI after stamping each frame. The replay uses each frame's timestamp and its successor to bound each read. Both endpoint reads must lie between durable enqueue and final settlement. It selects the device by the recorded PostgreSQL mount major and minor, keeps the warmups, and verifies all 20 archives against the exact controller release. Each counter interval covers part of the drain.

The original gate qualified one rotation out of two. Reproduce the diagnostic using the timing evidence from commit `a3070ff56` and Python 3.10 or later.

```sh
python3 -B refill_io.py "$timing_evidence" | cmp - refill-io.json
```
