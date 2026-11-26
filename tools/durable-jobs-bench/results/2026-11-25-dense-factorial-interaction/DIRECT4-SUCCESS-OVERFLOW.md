# Additional successes at an eight-job Direct4 flush

The four-worker Direct4 arms settled 1,024 jobs in 152 and 153 terminal transactions. Of those transactions, 89 and 88 already contained eight successes from the same runner. The two-worker service controls reached eight 38–51 times per run. The earlier six warm-River comparisons yielded at most two wholly covered later transactions across all six Mesh arms under the original, more permissive model. The four-worker result warrants checking how many additional **confirmed actor exits** the writer can actually see at its existing flush decision. [Earlier model](READY-SUCCESS-CAPACITY.md)

For each full eight-job transaction, the replay examines the next terminal transaction on the same runner. A pair qualifies for the model only when every job in the second transaction has a durable second application write by the cutoff. The leader and follower must fit in 16 rows. Each transaction can belong to only one pair. The replay selects the earliest available adjacent pair on each runner, which maximizes the number of disjoint pairs. The table counts pairs in each complete run. The job totals and middle counts refer to that selection.

| Upper bound or sensitivity | Forward | Reverse |
|---|---:|---:|
| All follower writes committed by the leader's XID assignment upper bound | 16 pairs, 86 jobs | 19 pairs, 87 jobs |
| Of those, leader committed between effect ranks 256 and 768 | 10 pairs | 8 pairs |
| Allow 1 ms from each follower's durable write to eligibility | 16 pairs | 17 pairs |
| Allow 2 ms | 12 pairs | 11 pairs |
| Allow 4 ms | 6 pairs | 4 pairs |
| All follower writes already committed by the preceding same-owner terminal COMMIT | 2 pairs, 14 jobs | 3 pairs, 11 jobs |

The client chooses the roster before PostgreSQL assigns its writer XID. The earliest recorded commit among transactions with that XID or a higher XID gives an upper bound on selection time. The preceding same-owner COMMIT is a separate, earlier cutoff. A durable application effect at either cutoff cannot establish that the handler has exited, that its lease is still eligible or that the dispatcher has applied the exit. A fixed lag illustrates sensitivity to that missing evidence. Actual expansion can also move later flushes, grants and writes.

The original SQL function accepts at most eight rows. An opt-in test needs separate support for 16 while retaining the existing eight-job trigger, lease and stop deadlines, checked COMMIT, per-row receipts, and the stock function's behavior for all callers. Its exposure measurement must count actual extra exits taken at dispatch, extra seats made reusable upon acknowledgement, and progress of later grants and application writes. The 16 and 19 modeled pairs provide no direct runner CPU or elapsed-time credit.

Run `python3 -B direct4_success_overflow.py` in this directory to reproduce the controls and per-pair transaction identities. The replay rehashes ten measured same-ELF archives from release `2d96798df`, checks 1,024 jobs and 2,048 distinct application transactions per arm and reconciles each terminal count to the released scorer.
