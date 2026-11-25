# How much dense enqueue preparation could overlap the preceding write?

Preparing one batch ahead has seven opportunities in an eight-batch transaction. Saving 30 ms would require a net 4.286 ms per opportunity. The stock 128-job preparation component occupied 0.994–1.063 ms of wall time per batch on Linode. Transferring that range to seven batches gives **6.96–7.44 ms** even with perfect overlap. That component begins with an already formed frame and ends with disposal of the prepared admissions. It prices only one part of the real producer.

The compiled SHORT broker also built the specification strings before calling the runtime. The trace recorded a second interval while the client formed the bulk command after acquiring the namespace admission fence. The table sums the two elapsed intervals in each 128-job arm and multiplies by seven, assuming each later chunk looks like that arm.

| Historical trace | Rotation 1 | 2 | 3 | 4 | 5 |
|---|---:|---:|---:|---:|---:|
| Client recorder, ms | 38.80 | 37.11 | 39.68 | 39.10 | 26.01 |
| Client and PostgreSQL recorders, ms | 24.81 | 26.26 | 40.56 | 37.60 | 24.97 |

Both SHORT observers missed their perturbation gates. These are elapsed windows between real wire marks, which can include waits and scheduling. The amount transferable to another CPU worker is unknown. The component is already contained in the broader preparation path and must not be added to these figures. The first chunk, eight database writes, receipt validation and outer commit still occur in order.

The five exact public dense B arms took 176.553–237.861 ms to enqueue. Their host logs contain one CPU sample inside the producer interval in four rotations and none in the second. No rotation contains a pair from which to measure how busy the two CPUs were wholly inside enqueue. The separate one-call width experiment saved 9.5–12.3 ms to durable enqueue in its two rotations. It changed the database call count and does not measure client overlap. [Width pilot](../2026-11-24-dense-producer-1024/canonical-broker-retry/evidence/pilot/README.md)

The open implementation question is whether the compiled caller can prepare the next chunk, including its outbound argument, during the current write on the same scoped PostgreSQL connection. A held-write proof can establish ordering and actual concurrency. Only a matched native run can establish the saving after scheduling, ownership transfer and contention on the two benchmark CPUs.

The replay checks ten historical raw SHA-256s against the released scorer, recomputes the intervals from the original monotonic marks, verifies the stock component summary against its receipt, and checks the five public dense archive hashes and transaction sizes.

```sh
python3 -B producer_lookahead.py | cmp - producer-lookahead.json
```

## LARGE FILE has more preparation to overlap

The same native component took **125.9–134.5 ms** of stock wall time for the 64 separately bounded 512 KiB frames together. If all frames cost the same and the preceding database call hid every subsequent preparation, the 63 opportunities would be worth **124.0–132.4 ms** before handoff costs. The earliest frame still precedes the first write. The component includes disposal and excludes the outer FILE producer. It used one CPU. [Released component](../2026-11-24-enqueue-spec-preparation/README.md)

In the five measured default-storage FILE arms, 5% of actual submit-to-settlement elapsed was **105.7–112.3 ms**. Under symmetric EXTERNAL storage it was **66.1–82.0 ms**. These figures indicate the scale of a useful improvement. Differences between fresh duplicate controls would also matter in a prospective trial.

| Inside submit through durable enqueue | Rotation 1 | 2 | 3 | 4 | 5 |
|---|---:|---:|---:|---:|---:|
| Default, idle ticks summed over CPUs 0 and 1 | 74 | 62 | 72 | 59 | 59 |
| EXTERNAL, idle ticks summed over CPUs 0 and 1 | 16 | 16 | insufficient samples | 15 | 13 |

The stock sampler reads CPU counters before stamping its sample. Using the second interior sample through the last interior sample keeps both counter reads inside the producer interval. The counts exclude I/O wait and show idle ticks recorded on the benchmark CPUs during part of submission. The samples cannot locate idle time within any one of the 64 calls or establish that a preparation worker could use it. The default-storage arms provide the clearest reason to test actual concurrency between the compiled producer and its PostgreSQL connection.

```sh
python3 -B large_producer_headroom.py | cmp - large-producer-headroom.json
```
