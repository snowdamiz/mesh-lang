# Combining bounded grants with an 8 ms settlement hold

Both reversed Linode rotations used the same grouped Mesh runner and producer, PostgreSQL 17.11, two 32-slot runners, and 1,024 dense jobs. Each job committed two application writes. The combined treatment made its last application write 352 and 375 ms earlier than the earliest stock control. The interval from the 103rd through the 922nd second write shrank by 314 and 331 ms. Its final settlement tail was 5 and 9 ms longer than the shorter control tail.

| Per 1,024 jobs | First rotation | Reverse rotation |
|---|---:|---:|
| Combined submit through settlement | 976.6 ms | 991.2 ms |
| Combined enqueue commit through settlement | 816.3 ms | 830.1 ms |
| Combined settlement transactions | 158 | 158 |
| Additional claim calls vs lower-count stock control | 73 | 72 |
| Runner CPU saved vs lower-CPU stock control | 25.5 ms | 14.8 ms |
| PostgreSQL CPU saved vs lower-CPU stock control | 610 ms | 630 ms |

Both single changes contributed. To compare their individual drain savings with the combination, subtract combined and stock time from the sum of the two single-change times. The first rotation yields −20 or +5 ms depending on the stock duplicate. The second yields +33 or +36 ms. The direction of any benefit beyond addition remains uncertain in two rotations. PostgreSQL CPU is measured over the original process brackets, and runner CPU covers process lifetime.

Run `python3 -B replay.py | cmp - interaction.json` here. Replay checks the successful release, packet digest, both rotation orders, and all 20 warmup and measured archives against their reconciled hashes. It computes write and settlement intervals from job ledgers. The original campaign also includes two diagnostic arms. [Pilot packet and prospective gates](../2026-11-25-dense-fixed8-logical-factorial/README.md)

## Five fresh comparisons against stock peers

Mesh completed a median 1,021 jobs/s from submit and 1,217 from enqueue commit. Its paired median throughput advantage over Oban was 32.5% and 41.2%, with five wins on each clock. Against stock River it was 2.1% from submit (four wins) and 12.2% from commit (five wins). Apalis finished first on both clocks in every rotation.

River reached its 64th second application commit a paired median 229 ms later than Mesh from enqueue commit. River then wrote faster through the middle of the workload and spent a median 195 ms longer between its final application commit and final settlement. The peak number of jobs with their second write committed and their terminal commit still pending was 307–372 for River and 24–29 for Mesh. These counts describe durable acknowledgements. They do not measure simultaneous handler execution.

Run `python3 -B peer_writes.py | cmp - peer-writes.json` for the five blocks. The reducer checks the successful peer release and 20 measured archive digests, then reconstructs seven adjacent time intervals from each arm's 1,024 job ledgers. The per-block intervals sum to full submit time. Paired medians across blocks need not sum.
