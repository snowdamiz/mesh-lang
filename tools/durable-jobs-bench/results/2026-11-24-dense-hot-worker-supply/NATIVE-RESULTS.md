# Hot dense queue with four scheduler workers per runner

Two opposite-order Linode blocks each ran six warmups and six measured arms. Every arm completed 1,024 jobs, two independent application writes per job, and all terminal settlements. Doubling workers per runner from two to four missed the advancement threshold in both orders.

| Group | Workers/runner | Distinct writing backends | Runner CPU, ms/job | Submit, ms | Drain, ms | Middle 820 writes, ms |
|---|---|---:|---:|---:|---:|---:|
| 1 | 2 (A) | 4 | 0.530 | 1315.9 | 1160.9 | 871.5 |
| 1 | 4 | 8 | 0.637 | 1305.3 | 1148.0 | 883.0 |
| 1 | 2 (B) | 4 | 0.529 | 1352.7 | 1188.5 | 904.8 |
| 2 | 2 (A) | 4 | 0.546 | 1342.0 | 1174.2 | 894.0 |
| 2 | 4 | 8 | 0.665 | 1323.1 | 1135.2 | 856.0 |
| 2 | 2 (B) | 4 | 0.534 | 1326.8 | 1170.2 | 881.4 |

The four-worker arm beat the faster duplicate on full drain by 12.9 ms and 35.0 ms. Both fell short of the 58 ms thresholds, and runner CPU per job rose in each order. The last write arrived earlier in both groups. River and Apalis beat every Mesh arm from submission and enqueue commit in both groups. The original gate closed the four-rotation extension.

Run `python3 -B controller.py replay` from this directory to reproduce the [native scores and thresholds](evidence/native/release-replay.json).
