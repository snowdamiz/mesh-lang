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
