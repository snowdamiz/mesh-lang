# Dense terminal transactions across runner processes

All five public B runs contain adjacent terminal transactions from different processes that could fit within the existing eight-row SQL limit. Within a 1 ms commit-spacing window, 60–130 disjoint pairs per rotation pass the ledger checks below. The earlier same-runner audit found zero pairs at 1 ms across the same five runs. Each public run had eight independently admitted runners, 64 seats, 1,024 jobs, and 2,048 independent application writes. [Fifteen raw identities, clocks and counts](cross-runner.json) · [Same-runner audit](README.md)

| Group | Terminal XIDs | Different-PID pairs within 0.25 / 1 / 4 ms | Pairs within 1 ms with four fixed two-runner groups | Mesh minus stock River submit / drain, ms |
|---|---:|---:|---:|---:|
| 1 | 352 | 13 / 62 / 133 | 11 | +93.9 / −5.4 |
| 2 | 552 | 43 / 130 / 223 | 29 | +1,196.6 / +1,096.1 |
| 3 | 375 | 17 / 60 / 146 | 13 | +153.3 / +19.0 |
| 4 | 375 | 17 / 69 / 145 | 18 | +216.6 / +55.6 |
| 5 | 381 | 20 / 71 / 150 | 23 | +256.4 / +124.6 |

The calculation retains each terminal transaction's complete cohort. It orders transactions by commit timestamp, pairs neighbors only, and uses each transaction at most once. Both owners' prior terminal transactions and every effect in the pair must have lower XIDs than the first of the two terminal XIDs. Their recorded commit timestamps plus an illustrative 250 µs readiness delay must precede the earliest recorded commit at or above that first XID. This last timestamp bounds submission from above. The stricter 1 ms readiness delay gives the same counts in the 1 ms spacing column. For the fixed two-runner comparison, the calculation pairs successive PIDs in sorted order, then searches within each pair of runners. [Calculation](cross_runner.py)

The range is sensitive to whom each writer may serve. One millisecond of commit spacing identifies transactions that ended close together. It does not measure a millisecond available to coordinate them. The ledger has no timestamp for completion of actor teardown, client receipt of the previous write, arrival at a common writer, or transmission of either terminal request. The counts neither establish simultaneous exit readiness nor bound all possible groupings under a changed schedule.

If these 1 ms pairs were the only source of improvement, tying stock River on submit would require an average net critical-path gain of **1.51, 9.20, 2.55, 3.14 and 3.61 ms per pair**. On drain the corresponding requirements are **0, 8.43, 0.32, 0.81 and 1.76 ms**. These divisions express the size of the target. Parallel SQL elapsed times cannot be credited as elapsed savings. The receipt also retains Oban's original clocks and each peer's 20% throughput deadline. Rotation 2 remains in every calculation.

A cross-runner implementation needs to establish that both owners can hand off an exit-confirmed success before the existing flush deadline. Each owner must retain its charged seat through a correctly mapped terminal receipt, including partial and lost replies. A shared writer can also delay independent SQL. The useful measurements are actual mixed-owner terminal XIDs, any added handoff delay, first and last durable application writes, and CPU on the original matched deployment.

Run `python3 -B cross_runner.py` here to reproduce the archive arithmetic. It reads the five Mesh, five River and five Oban measured captures and reconciles their independent writes.

## How much of the submit gap can the producer reach?

Subtracting drain from submit gives the observed producer window. Holding Mesh drain fixed, the largest producer window that would tie stock River is River submit minus Mesh drain. A negative budget means Mesh would still finish late with instantaneous enqueue.

| Rotation | Mesh producer, ms | River producer, ms | Mesh producer budget for a tie, ms | Mesh drain minus River drain, ms |
|---|---:|---:|---:|---:|
| 1 | 176.553 | 77.303 | 82.670 | −5.367 |
| 2 | 179.649 | 79.145 | −1,016.952 | +1,096.097 |
| 3 | 214.159 | 79.860 | 60.862 | +18.998 |
| 4 | 237.861 | 76.869 | 21.262 | +55.607 |
| 5 | 204.933 | 73.140 | −51.494 | +124.634 |

Matching River's enqueue time exactly would put Mesh ahead only in rotation 1. Even instantaneous enqueue leaves Mesh short of a 20% submit-throughput lead over River in every rotation. That deadline is River submit divided by 1.2 for the same 1,024 jobs. Against Oban, Mesh already won both clocks in rotations 1, 3, 4 and 5. In rotation 2, Mesh drain alone exceeded Oban's full submit time by 578.288 ms. These budgets retain the original schedules and do not estimate how a changed producer would affect follow-on claims.

## Would a shared terminal receipt arrive too late for the next grant?

For each disjoint pair from the 1 ms search, retain its terminal seats at both owners until the later terminal commit plus a hypothetical delivery delay. Keep the other commitments and grants at their observed times. Check each owner independently against its actual eight seats. The count below identifies pairs for which at least one later grant would require a change in width or timing. Each pair is assessed alone. Unaffected terminal commits get instant local credit, and the calculation uses the latest possible grant timestamp within its millisecond. These choices can hide a conflict. The ledger cannot establish that actors had exited in time to form a real pair.

| Rotation | Pairs | Conflicts at 1 ms | At 4 ms | At 8 ms | At 16 ms |
|---|---:|---:|---:|---:|---:|
| 1 | 62 | 0 | 0 | 4 | 16 |
| 2 | 130 | 0 | 1 | 9 | 17 |
| 3 | 60 | 0 | 1 | 4 | 21 |
| 4 | 69 | 0 | 0 | 1 | 15 |
| 5 | 71 | 0 | 1 | 5 | 19 |

The lost supply is sensitive to receipt latency. One millisecond produced zero forced grant changes under these assumptions. At 8 ms, 23 of 392 pairs crossed an observed seat limit. The public writer's existing 8 ms collection deadline already includes time spent waiting for earlier exits. The calculation gives no basis for granting another 8 ms after handoff.

The same replay integrates ledger states during effects 256–768 in B, both same-image controls and the reference. Slow B rotation 2 spent **1,238 ms** in that window. A time average of **15.05 jobs** had entered their handler and had yet to commit the first write, versus **5.93 and 5.02** in its same-image controls (592 and 628 ms). Only **0.74 of eight runners** on average had exclusively post-effect jobs awaiting settlement, versus **1.75 and 1.79** in the controls. Moving each claim stamp to the latest possible point before entry changes slow B's latter value to 0.75. These are overlapping jobs and runner states. Settlement can affect future admission, but a terminal-only backlog cannot by itself explain the observed slowdown. The SQL and host readbacks independently record queue-lock timeouts and higher device activity in this rotation. [Native phase analysis](../2026-11-25-dense-factorial-interaction/LATE-CAPACITY-PEER-PHASES.md)

The [archive calculation](middle_supply.py) reads 20 hash-bound measured arms. The [per-arm output](middle-supply.json) includes both claim-clock endpoints, the four job stages, and the delay sensitivity.

## How often do the prototype's two collection windows line up?

The implementation in `fe8901c4d` lets a writer take another runner's group when its own normal flush is due and the other group is still accepting an election. The two events must occur in that order. Two transactions committing close together need not satisfy that condition.

In the illustration below, each job becomes eligible 250 µs after its effect commit and each owner's previous receipt is applied 250 µs after its terminal commit. A group is available once all its members are eligible and the prior receipt has arrived. Its ordinary deadline is eight milliseconds after its earliest member became eligible. For a full intact pair, the leader must pass its deadline, while the other group's deadline must still be ahead. Both groups must also meet their original XID-assignment upper bounds. The calculation allows either owner to lead and examines each pair independently.

| Rotation | Original pairs | Eligible with 250 µs election delay | Net advance per eligible pair to tie stock River on submit |
|---|---:|---:|---:|
| 1 | 62 | 30 | 3.13 ms |
| 2 | 130 | 61 | 19.62 ms |
| 3 | 60 | 26 | 5.90 ms |
| 4 | 69 | 27 | 8.02 ms |
| 5 | 71 | 27 | 9.50 ms |

The illustration retains **171 of 392** pairs. With the same assumed exit and receipt latency, immediate election retains 172 and a 1 ms delay retains 142. In an individual pair, a later election can miss an open offer or give the other runner enough time to publish one. A qualifying pair has to displace enough elapsed work after accounting for the two-exchange shared commit, local handoff, and any delayed refill. This is scheduling sensitivity for the recorded cohorts. Actual teardown, polling, publication, renewal, and receipt latency can change the groups and their ordering. Native evidence needs the real mixed-owner XIDs and durable write clocks. [Window calculation](offer_windows.py) · [Five archive hashes and all lag assumptions](offer-windows.json)
