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
