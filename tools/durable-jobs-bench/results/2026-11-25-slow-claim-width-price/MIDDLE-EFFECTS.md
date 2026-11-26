# River overtakes Mesh during sustained SLOW refill

On the canonical 1×64 Linode workload, Mesh reached the 64th committed effect 192–291 ms ahead of stock River in every rotation. River then completed the next 384 effects 260–300 ms faster. In four blocks River reached the final effect earlier. The fourth block still ended with Mesh ahead on drain because River spent another 207 ms settling after its final effect.

| Block | First 64 effects, Mesh / River (ms) | Next 384 effects, Mesh / River (ms) | Final 64 effects, Mesh / River (ms) | Last effect to terminal, Mesh / River (ms) |
|---|---:|---:|---:|---:|
| 1 | 212 / 417 | 962 / 702 | 134 / 104 | 6 / 54 |
| 2 | 211 / 413 | 1,039 / 739 | 174 / 116 | 4 / 10 |
| 3 | 239 / 530 | 1,051 / 789 | 143 / 122 | 3 / 60 |
| 4 | 233 / 439 | 1,047 / 772 | 114 / 116 | 1 / 207 |
| 5 | 228 / 420 | 1,010 / 719 | 127 / 114 | 4 / 56 |

The first interval begins at the durable enqueue commit. Each boundary counts actual second application commits in time order. Jobs can overlap across boundaries. Adding the four intervals reproduces enqueue commit through final settlement in every arm.

During those middle 384 effects, Mesh averaged **39.6–41.5 of 64 seats in the required 100 ms hold**. River averaged **49.7–56.1**. To compute each average, clip every job’s first 100 ms after its first durable application commit to that arm’s middle interval, sum the occupied microseconds, and divide by the interval’s duration. The calculation includes jobs already holding at the beginning of the interval. Every job in both engines took at least 100 ms between commits.

For Mesh, an average of 7.6–9.0 seats lay between the SQL claim stamp and handler entry, 1.4–1.8 between entry and first commit, 1.2–1.5 between the end of the required hold and the second commit, and 2.1–2.9 between the second commit and durable settlement. Another 8.9–11.4 seats lay outside the recorded claim-to-settlement intervals. Claim stamps have millisecond precision and precede the runner’s receipt. The last category can therefore include waiting for a terminal receipt before reusing a seat. The ledger does not time that receipt.

For the ongoing width and point-lookup experiments, the middle interval shows whether earlier admission sustains more simultaneous holds after the first 64 effects. A change can improve initial pickup or the final settlement tail while leaving that difference intact.

Run `python3 -B slow_middle.py | cmp - slow-middle.json` in this directory. The replay pins the canonical scorecard and ten measured raw archives, checks 512 distinct jobs and 1,024 distinct application XIDs per arm, and reconstructs both durable clocks. [Raw sums and hashes](slow-middle.json) · [Whole-run slot accounting](HOLD-BUDGET.md)
