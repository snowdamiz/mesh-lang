# How early Direct4 needs more grants

The two private Direct4 arms finished in 909.286 and 929.240 ms from submit. Six separately released warm River arms finished in 770.658–787.280 ms. At the current Direct4 enqueue times, **at least 137 forward and 153 reverse jobs had yet to receive their database claim stamp** by the time Mesh would have had to finish. Giving every already-entered handler an immediate success at those deadlines would leave those jobs outstanding. A change that saves only idle CPU can help the pilot's resource gate. A change aimed at matching warm River must also advance productive claims or the earlier work that frees their seats.

Each row compares one original Direct4 arm to all six historical warm River arms. The cutoff is the latest time after the Mesh enqueue commit that would tie or beat the peer on both submit and drain. In the matched-enqueue sensitivity, Mesh gets the peer's actual enqueue duration and keeps its own subsequent schedule. The four right columns count jobs that have yet to reach each event. Columns overlap.

| Direct4 arm | Enqueue assumption | Cutoff from commit (ms) | Drain advance required (ms) | Certainly unstamped | Awaiting handler entry | Awaiting second write | Awaiting settlement |
|---|---|---:|---:|---:|---:|---:|---:|
| Forward | Actual 166.596 ms | 604.062–620.684 | 122.006–138.628 | 137–171 | 155–186 | 161–190 | 177–193 |
| Reverse | Actual 173.063 ms | 597.595–614.217 | 141.960–158.582 | 153–165 | 176–196 | 182–202 | 204 |
| Forward | Matched to River | 684.688–715.131 | 27.559–58.002 | 6–35 | 20–65 | 25–70 | 51–88 |
| Reverse | Matched to River | 684.688–715.131 | 41.046–71.489 | 12–48 | 32–67 | 35–71 | 45–95 |

The stock peer comparison with River and Oban used a different readiness policy. Warm River had 67 ready sessions and Direct4 had 41. The deadlines compare workloads from separate releases and preserve each release's measured session supply. A peer qualification requires fresh matched arms. [Warm River release](../2026-11-25-dense-fixed8-river-readiness/NATIVE-RESULTS.md)

## Capacity at the original claim stamp

The original Direct4 runs had 107 and 102 productive grants. The PostgreSQL row counts independently reconcile the number of grant groups, and all four runner logs have zero contention hints. Only 23 and 24 grants were full at 16 rows. For each partial grant the replay counts same-owner terminal commits in the four milliseconds before its stamp through the end of its one-millisecond stamp bin. It caps any potential increase by remaining local and global seats, preserves the other owner's rows at the same stamp, and counts only future rows still available in the recorded workload.

| Direct4 arm | Partial grants | Partials with room to grow | Partials with room to reach 16 |
|---|---:|---:|---:|
| Forward | 84 | 35 | 17 |
| Reverse | 78 | 48 | 23 |

The earlier public two-runner reference had 12 and 8 grants with room to grow and zero that could reach 16. Direct4's faster handlers changed the opportunity. Checking remaining workload supply removes one forward growth opportunity from the initial 36-grant estimate. A durable commit can precede the original reservation or follow the claimant's SELECT. Even a timely commit needs a checked receipt applied to the right reservation before it can release a seat. These counts cannot be totaled into transactions saved or elapsed time.

Direct4 already waits for a checked BEGIN before sending its ordinary bounded SELECT. That boundary gives a candidate one opportunity to consume a confirmed release without adding a queue prelock or another exchange. The decisive measurement is how often the effective limit actually changes at that point and how many additional rows reach the workers. Runner CPU, later durable ranks and the full-submit clock determine whether that pays.

## Where the durable effects fall behind

Direct4 reached its 64th durable effect earlier than all six warm River arms. By the 256th effect both Direct4 arms were behind every River arm. Times are milliseconds, measured from each arm's own enqueue commit for the first two rows.

| Durable milestone or interval | Direct4 forward | Direct4 reverse | Warm River range |
|---|---:|---:|---:|
| Effect 64 | 60.785 | 62.757 | 65.037–84.634 |
| Effect 256 | 185.982 | 183.812 | 152.593–175.441 |
| Effects 256 to 768 | 337.887 | 345.255 | 205.990–227.280 |
| Effects 768 to 1,024 | 209.995 | 216.654 | 121.942–144.773 |
| Final effect to final settlement | 8.826 | 10.456 | 178.013–209.890 |

Replacing only Direct4's middle 512 interval with each River arm's measured interval would still leave 0.664–22.211 ms forward and 13.250–34.797 ms reverse to recover to match both submit and drain. This calculation keeps each Direct4 arm's enqueue time, early progress, last 256 effects and final settlement tail fixed.

The first handler entry in a grant witnesses that the runner had received the checked grant for all its members by that time. During the middle 512 effects, both runners simultaneously held at least one such job whose handler had yet to enter for 191.678 ms forward and 184.656 ms reverse. Each held at least four unentered jobs for 67.376 and 72.846 ms. During the last 256 effects, both held an unentered job for 101.450 and 140.301 ms. These durations are lower bounds because the receipt can precede the first handler entry. Publication to the scheduler and the availability of an idle worker are unobserved. A refill change must demonstrate that additional granted jobs reach their durable writes sooner even while earlier grants are still being dispatched.

Run `python3 -B direct4_peer_deadlines.py` in this directory for all 24 deadline comparisons and the individual arm hashes. It checks 1,024 jobs and 2,048 independent application transactions in each of the eight source archives. Jobs with a claim stamp in the cutoff's ambiguous millisecond are reported separately. The [Direct4 idle CPU analysis](DIRECT4-IDLE-BUDGET.md) covers the distinct runner cost question.

## Would a 32-row claim keep four workers supplied?

A larger ceiling can change a full grant even when the owner has no new capacity to add after BEGIN. The two Direct4 runs reached the 16-row ceiling 23 and 24 times. For each full grant the replay keeps all preceding grants, credits every durable settlement through the end of the stamp's millisecond and respects both seat limits, same-stamp peer grants and remaining jobs. This gives an optimistic bound on room at the decision. The archives omit the client's checked credit at that instant.

| On the original schedule | Forward | Reverse |
|---|---:|---:|
| Full grants with room for any extra jobs | 16 | 14 |
| Full grants with room for the next entire same-owner batch | 13 | 10 |
| Such pairs with no intervening or same-stamp peer grant | 4 | 3 |
| Such uninterrupted pairs starting in the middle 512 | 2 | 0 |

For the four and three uninterrupted pairs, the first entry of the earlier batch proves the owner already had its checked receipt. Up to the first entry of the later batch, the owner always had at least one previously received job whose second write was outstanding. It had fewer than four such jobs for at most **8.551 ms out of 69.902 summed owner-ms** forward and **3.333 ms out of 49.555** reverse. The count uses only grants already proved received at the start. A job awaiting its second write may be running, waiting or awaiting dispatch. The other pairs would require reasoning about how an earlier, wider claim redistributes the ordered prefix between the two owners.

Widening might reduce fixed transaction overhead or change the later schedule. It would also put more row work before the first checked receipt. The original ledgers supply little evidence that contiguous pairs delayed a worker for lack of received jobs. The [earlier width-32 experiment](../2026-11-23-wide-trusted-grant/README.md) used two-worker runners and scalar settlement. Its noisy local rotations supply no measured Direct4 CPU saving. Keep the 16-row ceiling for the current native comparison. Run `python3 -B direct4_wide_claim.py` here to reproduce the per-grant bounds from the two original native archives.
