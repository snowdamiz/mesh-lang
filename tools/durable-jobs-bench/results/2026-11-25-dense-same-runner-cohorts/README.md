# Same-runner settlement opportunities in released dense 8×8 runs

The five measured public B arms used 352–552 terminal transactions for 1,024
jobs. The released writer takes up to eight exit-confirmed successes from
one runner and keeps their seats charged until the receipt. Its 8 ms collection
window starts at the first observed eligible exit. Commit spacing is a
different clock.

| Rotation | Terminal XIDs | Scalar | Adjacent pairs within 4 ms | Within 8 ms | Any spacing |
|---|---:|---:|---:|---:|---:|
| 1 | 352 | 62 | 2 | 12 | 60 |
| 2 | 552 | 249 | 2 | 13 | 125 |
| 3 | 375 | 87 | 2 | 24 | 74 |
| 4 | 375 | 88 | 4 | 18 | 82 |
| 5 | 381 | 80 | 5 | 23 | 80 |

Each pair consists of two **intact, consecutive** transactions from the same
runner, with eight or fewer jobs combined. The model moves the later group's
members into the earlier submission. Both groups' effects must already have
XIDs below the earlier terminal XID. Their latest effect commit must precede
or coincide with the earliest observed commit of any transaction with that
terminal XID or a higher XID. The earlier submission precedes its own XID
assignment, which precedes that commit. This gives a necessary condition on
the measured history even when the group's SQL spent time in PostgreSQL
after submission. The script checks that these fixtures contain a short XID
range, and that successive transactions on each runner are in commit order.

The table gives the maximum number of disjoint **pairs** satisfying these
conditions at zero delay after the effect commit. None qualified within 1 ms
in any rotation. Crediting a 1 ms effect-to-readiness delay gives 11, 13, 24,
18 and 23 pairs within 8 ms. The unrestricted counts quantify the sensitivity
to the spacing filter. Actor exit, the actual handoff time and its candidate
list are missing from the ledgers. These counts cannot establish that the
members were ready, how much work a changed schedule would retire, or a
wall-time gain. The replay retains rotation 2 with all 249 scalar transactions.

Replay from this directory with `python3 -B replay.py | diff - result.json`.
It checks the five archive hashes against the released series, all 2,048
independent application XIDs in each arm, the single owner of each terminal
XID, and the SQL call and row counts of the batch path. The original series
and gates are in the [peer comparison](../2026-11-25-public-dense-late-capacity/peer-comparison/RESULTS.md).
