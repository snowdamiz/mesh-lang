# Canonical public dense 8×8 transfer

The sole native controller owns this build-free qualification. It follows the
terminal release of the separately reserved public 2×32 logger-off recovery
pilot and every earlier reservation. Each new native phase has its own receipt.

The workload is the frozen 1,024-job dense fixture, one durable producer commit
through eight public set-enqueue calls, and two committed application writes per
job. All Mesh arms use the retained qualified 4924b588 ELF, eight 8-slot
runners, two scheduler workers each, CPUs 0 and 1, stock storage, PostgreSQL
17.11 with durability and TLS, the original cap of 96, and 32 warmed app
sessions. No relink or alternate product overlay. Each runner uses the original
four application pool lanes. The ready roster checks 32 app connections, 32
queue connections and the producer. The scorer joins eight direct runner
identities to their ledgers and clean logger-off reports.

Activation exercises checked-individual hot A, bounded-grouped cold C and
bounded-grouped hot B. C changes only the application checkout option on the
same public bounded-plus-grouped path. It prices 2,048 checked application
probes and preserves the 32 warmed sessions. A1 and A2 use the same config and
pin. Two pilot trials run A1/C/B/A2 and A2/B/C/A1, each with fresh warmups and
measured runs. The migrated SQL and role grants identify the checked and d388
routes. Each terminal XID belongs to one runner with a single commit timestamp.
The exact scalar and cohort calls and returned rows must reconcile to all
widths, including width one. Each application XID is unique and disjoint from
the producer and terminal XIDs. The export retains per-runner XID membership,
per-job ordered writes, the effect-to-terminal seat-time bound and occupancy.
The time after the second effect commit includes actor exit and settlement.
It bounds writer-held seat time from above.

The pilot admits peers only if both rotations improve both full clocks and
terminal XID count by more than 5% of the faster control and the observed
duplicate spread. The last durable effect must beat both duplicates. Runner,
PostgreSQL and host CPU must stay within the higher duplicate. Runner RSS is reported.
Every raw arm also checks host identity, no build activity, steal, readback,
capacity, connections and durability. The two reversed peer trials use the
byte-identical standard stock River, Oban and Apalis inventories with fresh
paired Mesh duplicates and B. A peer result requires both full clocks in the
same direction in both rotations, each beyond 5% of the peer clock and
duplicate spread, with Mesh resource bounds intact. Resource figures accompany
each peer result. Peer enrollment requires a successful released pilot replay.

From the checkout root, run `python3 -B <this-directory>/controller.py check`
and `python3 -B <this-directory>/test_witness.py`. The sole controller registers
the exact packet as `<id>-activation.json` and later `<id>-pilot.json`. It
dispatches with
`python3 -B <this-directory>/controller.py dispatch PHASE SHA --controller-root CONTROLLER_CHECKOUT`.
Put its exit receipt at `evidence/PHASE/controller-release.json` and replay
with `python3 -B <this-directory>/controller.py replay PHASE`. Only a positive
pilot replay permits registering `<id>-peers.json`. Raw exports are retained
per arm and preserved on failure.

## Headroom from the released canonical arms

The faster checked stock control in each rotation and both bounded-claim
individual arms came from the same released 8×8 continuation. Times are from
the native ledgers and CPU brackets.

| Rotation | Submit stock → d388 | Drain stock → d388 | d388 last effect from submit | d388 final effect to final settlement | Runner / PG / host CPU, d388 |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1 | 2,187 → 1,715 ms | 1,975 → 1,543 ms | 1,708 ms | 7.4 ms | 1,446 / 1,990 / 3,360 ms |
| 2 | 2,128 → 1,739 ms | 1,944 → 1,557 ms | 1,733 ms | 6.1 ms | 1,472 / 1,950 / 3,270 ms |

Both arms still ran 2,048 checked application probes and 1,024 individual
terminal commits. Rotation 2 saved 33 ms runner CPU against 56 ms duplicate
spread, so the d388-only decision stands at one of two and its peers stay
closed. In the faster schedule there are 177 and 155 guaranteed disjoint
same-runner pairs within 8 ms, versus 248–305 in the stock arms. These are
exit-bracket observations from individual settlement. The grouped policy can
change when slots become available, so the pilot must measure actual XID
widths, seat hold bounds, effect progression and both clocks.
[Continuation replay](../2026-11-25-public-dense-stock-logical-key/continuation/evidence/native/replay.json)

The five historical canonical stock medians for submit / drain were
1,616 / 1,473 ms for Oban, 1,247 / 1,144 ms for River and 920 / 882 ms
for Apalis. Against the d388 arm range these price approximately
99–123 / 70–84 ms for Oban, 468–492 / 399–413 ms for River, and
796–820 / 661–675 ms for Apalis. Those are planning barriers. Fresh
matched trials determine the comparison. Even deleting the entire 6–8 ms
after the final effect leaves the Oban and River barriers almost intact.
Both CPUs had 3.27–3.36 core-seconds of measured host work across the
bracketing windows. At two busy CPUs, 100 ms is roughly 0.2 core-seconds
and 500 ms roughly one core-second. The pilot can test whether cutting
checked probes and terminal transactions advances effects enough for the
smaller barrier. The larger barrier needs a broader throughput change
across the active interval. Group hold can offset some of the savings.
[Historical stock replay](../2026-11-23-dense-matched-topology/evidence/stock/replay.json)
