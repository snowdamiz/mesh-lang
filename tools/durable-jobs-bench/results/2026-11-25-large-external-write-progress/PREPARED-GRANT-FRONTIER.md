# Checked grants at the historical LARGE FILE drain deadline

At 446.855 ms after enqueue commit, two eight-job grants in the 16 MiB diagnostic still awaited a checked COMMIT reply. At that point 20 jobs still needed a second durable write. Sixteen belonged to those two grants. The last grant was confirmed at 510.550 ms, 63.695 ms after the deadline. Its final effect followed 54.575 ms later and its final settlement followed the effect by 19.071 ms. This runner also owned the last settlement in the entire arm.

| State at the historical deadline, jobs | 4 MiB A | 16 MiB | 4 MiB B |
|---|---:|---:|---:|
| Awaiting checked grant | 24 | 16 | 24 |
| Grant confirmed, effect outstanding | 10 | 4 | 8 |
| Effect committed, settlement outstanding | 8 | 13 | 3 |
| Settled | 22 | 31 | 29 |

The cutoff is the loosest historical Oban deadline for a 20% drain-throughput lead. Each column comes from one diagnostic in the same failed SETUP. No peer ran alongside these captures. The reducer reconciles eight activation calls and 72 returned rows with eight distinct owners who each claimed and finished eight jobs. That makes each owner's final positive receipt the receipt for its entire grant. It checks each receipt against the durable claim stamp and handler entry for all eight jobs before counting jobs in each state. [Per-owner receipt and write clocks](prepared-grant-frontier.json)

With this grant schedule fixed, even instantaneous handler execution and settlement miss the deadline in all three arms. Advancing confirmation can change later scheduling and the time each runner spends doing application work. These captures do not isolate the removable time in context rendering, response transfer, or activation. The failed PostgreSQL CPU bracket and original pilot dependency still apply.

From the repository root, pass the recovery SETUP evidence directory to `prepared_grant_frontier.py`. It reads the pinned custody commit and verifies the three raw archive hashes. Its JSON output reproduces the per-owner receipt.

## How early must preparation advance the first activation?

In the 16 MiB arm, the first job's activation stamp arrived 209.481 ms after enqueue commit. Even if every remaining grant were ready at that point, retaining the observed order, the measured interiors of the eight serialized activations and each owner's receipt-to-final-settlement delay puts the last settlement at or after 490.127 ms. That misses the historical 446.855 ms target by 43.272 ms. An earlier first activation or shorter activation and application intervals would be needed under those assumptions.

| Diagnostic lane | First activation stamp | Sum of interior durations, rounded down | Earliest modeled final settlement at observed start | Latest possible first stamp for the target |
|---|---:|---:|---:|---:|
| 4 MiB A | 257.076 ms | 175.000 ms | 519.252 ms | 184.679 ms |
| 16 MiB | 209.481 ms | 207.000 ms | 490.127 ms | 166.209 ms |
| 4 MiB B | 132.398 ms | 163.000 ms | 375.168 ms | 204.085 ms |

The queue row lock serializes successful activation turns. The budget charges each turn only the time between its first and last job stamp, less a full millisecond for the server clock's rounding. It assigns zero time to all gaps, each turn's leading and trailing SQL, its network reply and COMMIT. It then adds the observed time from that owner's confirmed receipt to that owner's last settlement, allowing all owners' application work to overlap subsequent turns. The maximum of these eight finish times is the modeled settlement. [Activation source](../2026-11-25-public-large-exact-prepare/README.md) · [Per-owner timestamps and arithmetic](prepared-grant-frontier.json)

The two 4 MiB controls put the first stamp 124.678 ms apart. The model therefore sets a target for a fresh matched comparison, without assigning the entire pre-stamp interval to rendering. A cheaper renderer could also reduce competition for the two cores while other owners activate or run handlers. To distinguish those effects, retain the first activation, each interior span, checked receipts, last effect and settlement alongside the separate backend render CPU measurement.
