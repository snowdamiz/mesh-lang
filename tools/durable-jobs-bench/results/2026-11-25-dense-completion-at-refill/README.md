# Where River catches Mesh in dense work

In all six native 2×32 rotations, public individual-terminal hot Mesh reached its 256th second application commit earlier than River when measured from enqueue commit. Mesh then took **849–887 ms** to complete the remaining 768 second writes. River took **325–354 ms**. The difference survives charging River for its final settlement tail. From the 256th second write through the last durable settlement Mesh took **855–888 ms**, while River took **530–571 ms**. At that boundary Mesh had already settled 244–253 jobs and River had settled 5–119. These are ranks within each full 1,024-job arm. A fresh 256-job run can have different timing.

River's producer also finished earlier. Measured from first submit, the sign of the lead at the 256th write varies. If Mesh retains its observed submit-through-256th-write time and its final settlement tail, exceeding matched River submit throughput by 20% requires **2,032–2,585 second writes/s** for the remaining 768 writes. Mesh actually delivered **866–905/s** during that stretch.

## Could the public grouped mode perform fewer settlements?

The already measured public grouped-hot runner used **869–906 terminal transactions** for 1,024 dense jobs. The six additional raw archives come from the [grouped product series](../2026-11-24-dense-product-d2-transfer/evidence/native-closure-retry). The separate [individual-terminal comparison](../2026-11-24-dense-individual-hot-pool/logger-off-retry/NATIVE-RESULTS.md) supplies the stage clocks above and the two checked controls per rotation.

For each runner, the replay makes an outcome available at its actual recorded terminal commit timestamp. Actor teardown had already occurred. It holds at most eight available successes and considers an 8 ms deadline from the oldest receipt. With the fixed deadline alone, **17–26 original grant timestamps per grouped-hot arm** would require more than 32 occupied slots on that runner. An idealized version also flushes all pending successes on every original grant stamp. It models **244–257 terminal transactions**, and every recorded grant fits the slot cap under instantaneous confirmation.

| Group | Actual grouped terminal XIDs | Model with an 8 ms hold | Original grant stamps over cap | Model also flushing at claim |
|---|---:|---:|---:|---:|
| 1 | 900 | 232 | 17 | 254 |
| 2 | 869 | 224 | 21 | 251 |
| 3 | 906 | 234 | 26 | 257 |
| 4 | 874 | 225 | 19 | 244 |
| 5 | 872 | 233 | 17 | 253 |
| 6 | 888 | 231 | 20 | 254 |

This calculation uses observed claim stamps as a proxy for refill demand and counts occupied jobs on each runner. The real dispatcher reserves capacity before the database stamps a grant. A grouped completion still requires SQL execution, durable commit and acknowledgement. Those costs, deadlines, lease renewals and time spent waiting for a slot can all change the future grant and write schedule. The zero over-cap count follows from the ideal flush and demonstrates feasibility only under those assumptions. The model offers **no wall-time prediction**.

The implementation experiment targets the opt-in grouped mode. It can flush when actual refill demand arrives, preserve two application writes, wait for actor teardown, and keep slots charged until acknowledgement. The individual-terminal runner remains its own comparison. The earlier zero-hold writer reduced terminal transactions without a repeatable dense win. A new native comparison must beat matched grouped controls on both durable clocks and advance the last effect before peer expansion.

## Reproduce

Run `python3 -B test_replay.py` here, then `python3 -B replay.py | cmp - native.json`. The replay checks 42 native archive hashes across two released series. It records each measured lane's stage clocks and the model counts separately for each Mesh arm. The tests cover slot pressure, simultaneous outcomes, and deadline expiry. [Unrounded output](native.json)
