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

Run `python3 -B direct4_peer_deadlines.py` in this directory for all 24 deadline comparisons and the individual arm hashes. It checks 1,024 jobs and 2,048 independent application transactions in each of the eight source archives. Jobs with a claim stamp in the cutoff's ambiguous millisecond are reported separately. The [Direct4 idle CPU analysis](DIRECT4-IDLE-BUDGET.md) covers the distinct runner cost question.
