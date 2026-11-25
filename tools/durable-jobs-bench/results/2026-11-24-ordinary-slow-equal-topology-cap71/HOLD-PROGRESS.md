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

River reported seven physical connections in every idle sample and peaked at 68–69 during the runs. These stock runs leave the contribution of connection opening to the first-wave gap unresolved. The paired River settlement tail also varied from 16 to 242 ms. Retaining the middle first-write interval alongside the full clock allows a new topology or codec comparison to distinguish sustained job progress from either boundary.

The [archive reader](evidence/native-analysis/hold_progress.py) reads only the two measured arms in each released block. It verifies each input against the raw digest in the original scorer output. All calculations use the embedded durable commit timestamps.
