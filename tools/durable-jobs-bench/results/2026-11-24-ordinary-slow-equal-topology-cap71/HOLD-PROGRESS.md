# How the two-runner SLOW workload fills 64 seats

Mesh reached the 64th first durable application write earlier than stock River in all eight released blocks. River then reached the 448th first write in 112–254 ms less time in every block. Mesh went on to win six of eight full-submit clocks and seven drain clocks. [Original results](NATIVE-RESULTS.md)

The table gives River elapsed minus Mesh elapsed in milliseconds. Each phase starts from the preceding landmark within the same arm. The first and last application writes for a job belong to independent transactions. The four phases sum to drain before rounding.

| Block | Enqueue commit to 64th first write | 64th to 448th first write | 448th first write to final effect | Final effect to final settlement |
| --- | ---: | ---: | ---: | ---: |
| 1 | +263.2 | −254.4 | −28.7 | +208.0 |
| 2 | +238.2 | −241.1 | −32.4 | +15.9 |
| 3 | +231.8 | −136.7 | −6.9 | +121.0 |
| 4 | +211.2 | −166.2 | +6.9 | +241.6 |
| 5 | +232.7 | −175.9 | +10.1 | +181.1 |
| 6 | +231.5 | −112.1 | −13.8 | +231.7 |
| 7 | +253.9 | −219.5 | −22.2 | +38.0 |
| 8 | +229.3 | −195.1 | −13.7 | +96.0 |

For each job, intersect the interval between its committed first write and its committed effect with the 64th-to-448th first-write window. Summing those durations and dividing by the window duration gives the mean number of jobs concurrently between commits. Mesh averaged **42.8–48.6** across blocks and River **50.1–56.2**. Both engines reached a peak of 64. Time outside a committed hold includes acquisition, dispatch, SQL and confirmation. Only a matched intervention can establish how much of it a change recovers. [Raw-derived microseconds and archive hashes](evidence/native-analysis/hold-progress.json)

River reported seven physical connections in every idle sample and peaked at 68–69. PostgreSQL had authorized six runner sessions before enqueue commit in every block. Matching connection receipts to TLS authorization lines by backend PID identifies 61–62 additional runner sessions that arrived after commit. By the 64th durable first write, 59–60 of those sessions were authorized in seven blocks. In block seven the millisecond timestamp leaves the count at 58–59. The first new authorization occurred 56–191 ms after commit across blocks, including timestamp uncertainty. These records establish when River opened its pool. A matched prewarmed run would be needed to measure the effect on the clock.

Within the middle interval, the Mesh ledgers account for each runner's 32 seats separately. On average across time the claim stamp to recorded application entry occupied 3.26–4.42 seats across both runners, entry to first commit occupied 1.20–1.60, and effect commit to terminal commit occupied 3.09–4.71. Another **7.07–10.93 seats** fell outside every stamped claim-to-settlement interval. In each rotation **73–89%** of that last quantity accrued while an individual runner had at least eight such seats. The exact share in block two was 89.43%. This distribution directs attention to periods with space for several replacements. The runner can already have a claim outstanding or still be waiting for settlement confirmation during those periods.

These counts use the database's millisecond claim stamp. Moving every stamp to the latest possible time in its millisecond adds at most 6.26 ms of slot-time divided by 64 to the outside interval per rotation. On the recorded stamps this interval totals 97.91–171.19 ms on that normalized scale. Neither the outstanding claim start nor the arrival of the client receipt was recorded, so these are occupancy measurements and supply no wall-time saving for a proposed change.

The paired River settlement tail also varied from 16 to 242 ms. Retaining the middle first-write interval alongside the full clock allows a new topology or codec comparison to distinguish sustained job progress from either boundary.

The [archive reader](evidence/native-analysis/hold_progress.py) reads the two measured arms in each released block. It verifies each input against the raw digest in the original scorer output. Phase and occupancy calculations use the embedded durable commit timestamps. Pool opening uses the PostgreSQL connection log on the database clock and allows 999 µs above each logged millisecond.

## Where the middle-window difference accumulates

Mesh took a paired median **185.5 ms longer than River** between first-write ranks 64 and 448. Most of that difference accumulated while slots were outside the recorded handler-entry to effect-commit interval. Across eight blocks, Mesh spent **93.4–202.1 ms more** there on a 64-slot-equivalent basis. The corresponding difference between the end of the required 100 ms and the effect commit ranged from **5.7 ms less to 11.1 ms more**. Timer wakeup, application-pool checkout and the second write can all contribute to that latter interval. Their individual costs remain unresolved.

| Mesh minus peer, paired median in ms | River | Oban | Apalis |
| --- | ---: | ---: | ---: |
| Elapsed from 64th to 448th first commit | +185.5 | −4.9 | +146.9 |
| Recorded entry to first commit, slot-time ÷ 64 | +16.4 | −1.0 | +5.4 |
| Required first 100 ms after commit, slot-time ÷ 64 | +30.2 | +7.1 | +38.4 |
| End of required hold to effect commit, slot-time ÷ 64 | +3.4 | −24.5 | −10.4 |
| Outside recorded entry to effect commit, slot-time ÷ 64 | +121.0 | +22.1 | +124.4 |

The calculation clips every interval at the two rank boundaries. Both products can complete the same work and have different amounts of required hold inside that window because different jobs cross its boundaries. The four terms sum exactly to elapsed within each arm. The table takes a separate paired median for each row. The outside interval includes time spent claiming jobs, awaiting handler entry, and completing earlier jobs. It gives neither a CPU measurement nor the time a faster operation would save.

The [ledger arithmetic](evidence/native-analysis/middle_service.py) verifies all 32 measured archive hashes, the 100 ms minimum for every job, and the 64-slot limit. The [per-arm results](evidence/native-analysis/middle-service.json) retain integer microseconds and each input digest.
