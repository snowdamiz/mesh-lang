# Public large-file claim and settlement clocks

The comparison aligns each engine to its own durable enqueue commit. The Mesh arm reads the batch from a file and keeps individual terminal transactions. Every group checks 64 jobs and 128 distinct application transactions per measured arm.

In 4 of 5 rotations, 48–56 of Mesh’s 64 jobs still lacked a claim stamp when Apalis had settled its last job.

In rotation 4, Apalis had committed all 128 application writes by 175.4 ms. It settled 61 jobs by 181.2 ms and the last 3 in one transaction at 25.282 s. All completed on their first attempt. The archives record when the commits occurred. The cause of the delay is unresolved.

Mesh had 9–13 overlapping claim-stamp-to-settlement intervals at each rotation’s peak, across eight runners with 64 configured slots. Slot reservations and client acknowledgements may extend beyond these database timestamps. Times in the table are milliseconds.

| Group | Peer | Mesh drain minus peer drain | Mesh jobs stamped after peer settled | Last Mesh claim stamp minus peer settlement | Mesh last stamp to last settlement |
| ---: | --- | ---: | ---: | ---: | ---: |
| 1 | oban | +145.5 | 16 | +100.0 | 45.4 |
| 1 | river | -236.9 | 0 | -282.3 | 45.4 |
| 1 | apalis | +483.8 | 53 | +438.4 | 45.4 |
| 2 | oban | +158.4 | 16 | +123.1 | 35.3 |
| 2 | river | -258.1 | 0 | -293.4 | 35.3 |
| 2 | apalis | +511.4 | 56 | +476.1 | 35.3 |
| 3 | oban | +166.1 | 16 | +124.0 | 42.0 |
| 3 | river | -185.8 | 0 | -227.8 | 42.0 |
| 3 | apalis | +448.0 | 48 | +406.0 | 42.0 |
| 4 | oban | +153.2 | 16 | +118.3 | 35.0 |
| 4 | river | -183.9 | 0 | -218.9 | 35.0 |
| 4 | apalis | -24630.8 | 0 | -24665.8 | 35.0 |
| 5 | oban | +114.1 | 8 | +94.1 | 20.0 |
| 5 | river | -112.7 | 0 | -132.7 | 20.0 |
| 5 | apalis | +518.2 | 48 | +498.2 | 20.0 |

The database floors each Mesh claim stamp to a millisecond before COMMIT. The count gives lower and upper bounds when that millisecond straddles the cutoff. A positive last-stamp difference means that, with the stamp schedule fixed, even instant settlement at every recorded stamp would still leave Mesh behind the peer. Earlier processing can change later grants. The ledger does not time lock ownership or client acknowledgement.

The original series also scores enqueue and full submit, plus both same-broker inline controls. [Controller and native receipts](../2026-11-24-public-large-file-session/resume/adapter-path-retry/) · [Earlier checked-cursor experiment](../2026-11-24-public-large-checked-cursor/README.md)

Run `python3 -B replay.py` to reproduce the table from all five reconciled blocks.
