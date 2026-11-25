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
