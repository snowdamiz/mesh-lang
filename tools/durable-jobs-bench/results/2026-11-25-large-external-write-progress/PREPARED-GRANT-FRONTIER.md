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

## What ran before the first activation?

All eight runners in each prepared arm recorded one notification-driven queue scan. The released runner counts scans by wake source. Its report omits the time of the notification, the start of the scan, and the identity of the call that produced the first grant. A lost notification therefore has no direct witness here, and the 250 ms fallback polling interval cannot serve as an estimate of pickup delay.

The PostgreSQL observer collected process counters and read memory accounting for 71 processes in each sample below. Each capture began after enqueue commit and ended after the first activation, including its 1 ms timestamp uncertainty. None of the three arms had a complete PostgreSQL sample between commit and first activation.

| Arm | Capture start after commit | First activation stamp | Capture end after commit |
|---|---:|---:|---:|
| 4 MiB A | 54.597 ms | 257.076 ms | 396.020 ms |
| 16 MiB | 48.395 ms | 209.481 ms | 294.652 ms |
| 4 MiB B | 76.189 ms | 132.398 ms | 264.505 ms |

These are elapsed capture windows. The observer does not record its own CPU consumption or the individual counter read times within each window. The archived samples cannot split the pre-activation delay between PostgreSQL execution, notification delivery, and runner scheduling. [Raw-bound reduction](prepared-grant-frontier.json)

## When did other prepared activations reach PostgreSQL?

In five of seven arms, at least one later owner had already started its activation statement before the preceding owner stamped its last job. This matters for a change that would shorten the serialized part of activation. Those callers had already received and validated their contexts and sent their manifests back to PostgreSQL.

| LARGE prepared arm | Aggregate activation executor time | Minimum aggregate time before own first stamp | Minimum aggregate lead over preceding owner's last stamp | Minimum lead for at least one caller |
|---|---:|---:|---:|---:|
| 4 MiB A diagnostic | 834.930 ms | 591.599 ms | 259.523 ms | 37.074 ms |
| 16 MiB diagnostic | 887.013 ms | 604.118 ms | 319.637 ms | 45.662 ms |
| 4 MiB B diagnostic | 480.479 ms | 239.303 ms | 0 | 0 |
| Earlier factorial forward, consumer only | 677.763 ms | 463.009 ms | 139.660 ms | 19.951 ms |
| Earlier factorial forward, combined | 550.094 ms | 335.078 ms | 33.188 ms | 4.741 ms |
| Earlier factorial reverse, consumer only | 416.687 ms | 207.407 ms | 0 | 0 |
| Earlier factorial reverse, combined | 647.937 ms | 391.120 ms | 117.884 ms | 14.735 ms |

The PostgreSQL statement counter covers exactly the productive activations. Each returns one count row and one row per job. The Rust client sends this statement after decoding the checked batch; it confirms the grant only after the subsequent COMMIT. For each turn, its executor time therefore fits between the enqueue commit and the client's grant receipt. One earlier turn in the reverse combined arm lacks its own receipt timestamp, so its first handler entry supplies a later upper bound. [Earlier factorial receipts](confirmed-handler.json)

For the third numeric column, start the first call at enqueue commit and each later call at the preceding owner's final row stamp. End each at its own receipt upper bound. These spans give the calls every opportunity to accumulate executor time without starting early. In the 16 MiB arm they sum to 567.376 ms. Subtracting that total from the 887.013 ms measured by PostgreSQL leaves at least 319.637 ms of early starts summed across seven later callers. At least one had a lead of 45.662 ms. The computation rounds down and uses the database's floored row stamps. Zero means that this bound is inconclusive. The aggregate includes time that concurrent statements spent waiting, so it cannot be counted as a reduction in drain. [Calculation](activation_arrival.py)

As a sensitivity check, put all eight turns on two lanes, ready at the observed first stamp, preserving launch order and every owner's observed receipt-to-last-settlement interval. Charging only the seven interior row gaps of each eight-row activation, rounded down by 1 ms per turn, gives modeled final settlements of 451.144, 420.967 and 344.565 ms in 4 MiB A, 16 MiB and 4 MiB B. Charging the full first-stamp-to-receipt duration gives 473.564, 454.469 and 359.383 ms. These examples assume ample capacity and no extra CPU or lock cost. Even under that readiness assumption the 16 MiB result falls on opposite sides of the 446.855 ms historical Oban cutoff depending on the turn cost. The useful native measurements are actual arrival at the queue lock, time under it and elapsed time until grant confirmation. [Three diagnostic reductions](prepared-grant-frontier.json)

### Why the measured overlap does not authorize two concurrent grants

Consider a cap-64 queue with two eligible jobs and two open transactions. A prepares the head job. B prepares the next job while A holds the head tuple lock. A then activates and receives a positive SQL reply but leaves its transaction open. Stock still holds the queue row lock here. A second productive activation lane would let B commit and start its handler before A commits or rolls back. An independent observer could see B executing while the head remains available, even after A received its positive activation reply. Stock can reach the same final states if A rolls back first. It forbids B's progress during that open queue turn. This is a source-level counterexample on the prepared path.

The 16-attempt limit also applies per call. One top-level transaction can present fresh manifests more than once while another transaction is open. A fixed reservation of 16 seats for the other writer would omit its earlier uncommitted calls. Moving the actual job updates before a final capacity check would introduce another difference, because their recorded timestamps start the leases. Keep the serialized activation through transaction resolution and price a reduction in the cost of each turn separately. [Exact activation and lease contract](../2026-11-25-public-large-exact-prepare/README.md)

## Which work was outstanding when Oban finished?

The five original EXTERNAL FILE Oban drain clocks were 491.690, 485.620, 511.754, 508.086 and 536.226 ms from enqueue commit. The table uses the slowest of those five as a historical reference. Each Mesh arm ran separately. The three rightmost columns count distinct unfinished jobs at 536.226 ms. The two writes are the independently committed attempt and effect.

| Prepared Mesh arm | Drain (ms) | Awaiting first write | First committed, awaiting effect | Effect committed, awaiting settlement |
|---|---:|---:|---:|---:|
| Factorial forward, consumer only | 534.929 | 0 | 0 | 0 |
| Factorial forward, combined | 542.803 | 0 | 0 | 6 |
| Factorial reverse, consumer only | 562.844 | 2 | 5 | 4 |
| Factorial reverse, combined | 532.397 | 0 | 0 | 0 |
| 4 MiB A diagnostic | 608.689 | 8 | 0 | 16 |
| 16 MiB diagnostic | 584.196 | 6 | 2 | 5 |
| 4 MiB B diagnostic | 598.082 | 12 | 3 | 7 |

In the 16 MiB arm the last owner's checked grant landed at 510.550 ms. At that point, 45 jobs had settled, six had committed their effect and 13 still needed at least one write. Two earlier owners still held unfinished jobs. If the final owner could finish everything at its grant receipt, with earlier owners' histories fixed, final settlement would still be at 551.676 ms. That is 15.450 ms behind the slowest Oban reference. If every owner could settle at its last effect with the writes fixed, the arm would finish at 565.125 ms.

The grant timestamp also limits what a faster reply alone can achieve. For each productive turn, give every job the entire interval from its last row's activation stamp through the checked COMMIT receipt, moving its subsequent milestones earlier by that amount. Hold row stamps, other grants and per-job service durations fixed. This credits even final update and commit work and the millisecond rounding of the row stamp. For the single turn with a censored receipt, credit through the first handler entry. The maximum advances in final settlement are 4.035, 19.232, 9.065 and 5.751 ms in the four factorial arms above, then 6.437, 11.069 and 8.376 ms in the three diagnostic arms. Crediting that entire interval in the 16 MiB arm would leave 36.901 ms to the slowest historical Oban finish. Any larger gain from changing grant service would have to advance other events too, such as the next owner's actual activation stamps.

[Per-arm job counts at every historical Oban finish and fixed-schedule bounds](post-grant.json) · [Seven-archive replay](post_grant.py)
