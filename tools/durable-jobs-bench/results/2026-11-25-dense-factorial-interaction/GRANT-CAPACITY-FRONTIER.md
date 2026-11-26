# Time between confirmed settlements and the next dense grant

On the public eight-runner deployment, allowing an outstanding request to grow after a confirmed settlement produced 74–112 full eight-job refills per rotation. The same-image off controls produced zero to four. For the full refills in the enabled arm, the median interval from the previous jobs’ last durable terminal commit on that runner to the next database grant stamp was 19–26 ms. This interval includes terminal acknowledgement, scheduling, queue-lock acquisition, and part of the next checked SELECT. The archives cannot divide the time among them.

| Rotation | Full grants / productive grants | Full refills after first grant | Median last terminal → full grant | Partial grants with an older job still unsettled at first entry¹ | Runners between batches during middle 512 writes², B / public reference |
|---|---:|---:|---:|---:|---:|
| 1 | 120 / 134 | 112 | 21.3 ms | 5 / 13 | 2.40 / 2.46 |
| 2 | 82 / 159 | 74 | 25.8 ms | 44 / 76 | 1.34 / 2.30 |
| 3 | 113 / 136 | 105 | 19.1 ms | 11 / 22 | 2.14 / 2.50 |
| 4 | 110 / 137 | 102 | 23.9 ms | 9 / 26 | 1.98 / 2.35 |
| 5 | 112 / 137 | 104 | 20.9 ms | 6 / 25 | 2.12 / 1.94 |

¹ The denominator counts partial grants for which the current group plus jobs granted strictly later across all runners contained at least eight jobs. Work granted later may have been locked at the earlier instant. The numerator counts groups where even the first handler entry preceded a previous same-runner job’s terminal commit. Moving the cutoff five milliseconds later gives 5, 38, 9, 8 and 6. In these cases that seat could not have been credited from a confirmed prior settlement at grant time.

² Sum the intervals from the last terminal commit of all earlier same-runner jobs to the next productive grant, then divide by the observed middle-write duration. Clip each interval to effects 256–768 and subtract one millisecond from its right endpoint for the grant clock’s resolution. Startup and the interval after each runner’s final grant are excluded. A runner between batches can be executing claim or lifecycle code. The slowest B arm, rotation 2, had the fewest runners between batches on this measure. Reducing this count alone would be a poor advancement criterion. [Original phase measurements](LATE-CAPACITY-PEER-PHASES.md)

**Correctness constraint.** Asking PostgreSQL for even one extra provisional job and then rolling back changed what a concurrent recovery worker could promote with `SKIP LOCKED`. A trusted retry committed the same two jobs yet left the next job scheduled. Commit `de8eeeae7` preserves the PostgreSQL 17.11 witness. A candidate can use a genuine terminal receipt already processed by the runner to increase its reservation before taking a job lock. The frequency of such receipts at the instant a partial request enters the claim lane remains unknown.

From this directory, run `python3 -B grant_capacity_frontier.py | cmp - grant-capacity-frontier.json`. Replay verifies the release seal, controller scores and archive hashes for 20 measured Mesh arms. Each arm reconciles 1,024 jobs and 2,048 distinct application XIDs. The number of runner-and-stamp groups equals checked calls minus excess marker rows in every arm. That equality rules out two productive grants on one runner sharing a stamp in these captures. [Per-arm measurements](grant-capacity-frontier.json) · [Released peer comparison](../2026-11-25-public-dense-late-capacity/peer-comparison/RESULTS.md)

## Two eight-seat owners at the same claim

Of 249 full refills whose whole gap fell inside the middle 512 effects, 229 saw another runner receive a productive grant between the same owner's last terminal commit and its next grant stamp. In every rotation the median was two intervening turns. The receipts do not timestamp when either client asked for its next grant.

The released SQL and Rust reader accept up to 16 rows. A single owner on this deployment can reserve only eight. The earlier coordination screen at `b344fdb09` limited each combined request to eight total. Here one checked transaction can serve two adjacent owners, each retaining its eight-seat limit. Count a pair only when the later owner's observed width fits its modeled available seats at the earlier stamp. The counts are maximum disjoint pairs on the original grant order. Every stamp was unique across owners.

| Rotation | Positive turns | Whole-run pairs at width 8, 4 ms receipt lag | Whole-run pairs at width 16, 0 / 4 / 8 ms lag | Middle pairs at width 16, 8 ms lag |
|---|---:|---:|---:|---:|
| 1 | 134 | 0 | 64 / 60 / 51 | 26 |
| 2 | 159 | 8 | 72 / 68 / 62 | 32 |
| 3 | 136 | 0 | 61 / 57 / 50 | 23 |
| 4 | 137 | 1 | 65 / 61 / 54 | 23 |
| 5 | 137 | 1 | 64 / 60 / 57 | 28 |

The model credits the earlier owner with its observed grant and moves the later owner's seat check to the earlier stamp. That owner may still have been receiving an acknowledgement or deciding to claim. Even if both requests were ready, the combined transaction would still have to select, validate and update all the rows, then deliver the confirmed grant to both owners.

At 8 ms assumed lag, matching River on submit would require 1.8–4.5 ms of net advance per pair outside rotation 2 and 19.3 ms in rotation 2. Matching River's middle-write pace through this mechanism alone would require 7.4–11.2 ms per middle pair outside rotation 2 and 29.6 ms in rotation 2. The actual saving is unmeasured. The first question for an implementation is whether two compatible reservations are ready when the first claim can run, without delaying a lone request.

The [raw hashes, seat sensitivities and intervening grant counts](two-owner-prefix.json) cover the five B captures. Reproduce them with `python3 -B two_owner_prefix.py`.

## How many granted rows actually used the late capacity?

The stopped runners record how many seats they added between reservation and the bounded SELECT. A grant can return fewer rows than it requests. Even after allowing the maximum possible unused capacity, **at least 210–384 of the 1,024 jobs in each measured B arm used a seat added after the original reservation**. This puts a cost on freezing the leader's width early to combine two owners. In the four ordinary rotations, at least 30–36 full grants in the middle 512 effects used a late credit. The middle-specific lower bound is zero in G2.

| Rotation | Seats added | Lower bound on added seats filled | Middle full grants using added seats, lower bound / full grants |
|---|---:|---:|---:|
| 1 | 593 | 384 | 36 / 60 |
| 2 | 633 | 210 | 0 / 36 |
| 3 | 605 | 359 | 32 / 57 |
| 4 | 621 | 374 | 30 / 53 |
| 5 | 609 | 348 | 31 / 55 |

The calculation gives every post-idle empty SELECT and every failed SELECT after a successful prelock seven unused credits, the maximum increase from one seat to eight. On positive grants it allows every unfilled seat to consume a credit. Each runner's first positive grant starts with its full eight-seat allowance. The terminal claim counters equal the final observer's successful SELECT count in every arm. The runner samples also equal the successful prelock rows, and G2's two failures inside the bounded SELECT are included. For the middle bound, assign as many topups as possible to empty, failed, partial, or outside-window turns. Each remaining topup on a full grant necessarily admitted at least one additional job.

A two-owner diagnostic should record both original reservations, the credits available to each owner immediately before the combined SELECT, and the actual ordered row assignment. A prospective saving from dropping a database turn needs to account for rows displaced onto later turns. These counts describe the actual schedule. A displaced row's delay and the effect on subsequent grants require native measurement. [Archive calculation](realized_topups.py) · [Five-arm results](realized-topups.json)
