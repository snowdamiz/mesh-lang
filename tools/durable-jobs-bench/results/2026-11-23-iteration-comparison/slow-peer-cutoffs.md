# Slow Mesh jobs outstanding at peer completion

Verified 5 of five Linode rotations of the 512-job storage comparison. Each job held for 100 ms between two independent application writes. Each row compares two arms from the same rotation and storage policy on CPUs 0 and 1.

Gap is Mesh finish minus peer finish in milliseconds, relative to each arm’s own submit or enqueue commit. Positive values mean Mesh finished later. The four counts partition its outstanding jobs at the peer’s finish. Waiting for the second write includes the required hold. These counts describe progress and do not assign CPU cost.

| Group | Storage | Peer | Clock | Gap (ms) | Awaiting entry | Awaiting first write | Awaiting second write | Awaiting settlement |
|---:|---|---|---|---:|---:|---:|---:|---:|
| 1 | default | oban | submit | -87.292 | 0 | 0 | 0 | 0 |
| 1 | default | oban | commit | -122.702 | 0 | 0 | 0 | 0 |
| 1 | default | river | submit | +177.512 | 55 | 0 | 2 | 0 |
| 1 | default | river | commit | +131.563 | 5 | 3 | 48 | 0 |
| 1 | default | apalis | submit | +462.330 | 122 | 0 | 64 | 0 |
| 1 | default | apalis | commit | +253.586 | 57 | 0 | 64 | 0 |
| 1 | external | oban | submit | -61.228 | 0 | 0 | 0 | 0 |
| 1 | external | oban | commit | -135.469 | 0 | 0 | 0 | 0 |
| 1 | external | river | submit | +2.566 | 0 | 0 | 0 | 1 |
| 1 | external | river | commit | -54.632 | 0 | 0 | 0 | 0 |
| 1 | external | apalis | submit | +606.437 | 219 | 2 | 27 | 1 |
| 1 | external | apalis | commit | +381.095 | 108 | 0 | 60 | 1 |
| 2 | default | oban | submit | -198.946 | 0 | 0 | 0 | 0 |
| 2 | default | oban | commit | -282.574 | 0 | 0 | 0 | 0 |
| 2 | default | river | submit | +198.790 | 49 | 0 | 25 | 16 |
| 2 | default | river | commit | +103.277 | 0 | 0 | 49 | 0 |
| 2 | default | apalis | submit | +485.675 | 155 | 2 | 22 | 0 |
| 2 | default | apalis | commit | +267.178 | 50 | 0 | 64 | 0 |
| 2 | external | oban | submit | -55.288 | 0 | 0 | 0 | 0 |
| 2 | external | oban | commit | -100.957 | 0 | 0 | 0 | 0 |
| 2 | external | river | submit | -35.874 | 0 | 0 | 0 | 0 |
| 2 | external | river | commit | -89.972 | 0 | 0 | 0 | 0 |
| 2 | external | apalis | submit | +429.733 | 127 | 0 | 64 | 0 |
| 2 | external | apalis | commit | +254.160 | 63 | 0 | 64 | 0 |
| 3 | default | oban | submit | -162.712 | 0 | 0 | 0 | 0 |
| 3 | default | oban | commit | -215.501 | 0 | 0 | 0 | 0 |
| 3 | default | river | submit | -82.293 | 0 | 0 | 0 | 0 |
| 3 | default | river | commit | -123.794 | 0 | 0 | 0 | 0 |
| 3 | default | apalis | submit | +189.828 | 17 | 0 | 60 | 3 |
| 3 | default | apalis | commit | +16.225 | 0 | 0 | 9 | 2 |
| 3 | external | oban | submit | -148.096 | 0 | 0 | 0 | 0 |
| 3 | external | oban | commit | -178.801 | 0 | 0 | 0 | 0 |
| 3 | external | river | submit | -51.410 | 0 | 0 | 0 | 0 |
| 3 | external | river | commit | -88.104 | 0 | 0 | 0 | 0 |
| 3 | external | apalis | submit | +497.199 | 166 | 0 | 25 | 15 |
| 3 | external | apalis | commit | +286.030 | 72 | 13 | 17 | 0 |
| 4 | default | oban | submit | -124.764 | 0 | 0 | 0 | 0 |
| 4 | default | oban | commit | -164.019 | 0 | 0 | 0 | 0 |
| 4 | default | river | submit | -37.107 | 0 | 0 | 0 | 0 |
| 4 | default | river | commit | -70.128 | 0 | 0 | 0 | 0 |
| 4 | default | apalis | submit | +501.768 | 180 | 0 | 8 | 8 |
| 4 | default | apalis | commit | +306.565 | 77 | 6 | 33 | 0 |
| 4 | external | oban | submit | -39.687 | 0 | 0 | 0 | 0 |
| 4 | external | oban | commit | -91.094 | 0 | 0 | 0 | 0 |
| 4 | external | river | submit | -2.278 | 0 | 0 | 0 | 0 |
| 4 | external | river | commit | -35.032 | 0 | 0 | 0 | 0 |
| 4 | external | apalis | submit | +518.508 | 158 | 0 | 35 | 10 |
| 4 | external | apalis | commit | +310.703 | 89 | 3 | 2 | 1 |
| 5 | default | oban | submit | -225.610 | 0 | 0 | 0 | 0 |
| 5 | default | oban | commit | -237.147 | 0 | 0 | 0 | 0 |
| 5 | default | river | submit | -72.184 | 0 | 0 | 0 | 0 |
| 5 | default | river | commit | -95.278 | 0 | 0 | 0 | 0 |
| 5 | default | apalis | submit | +436.148 | 122 | 0 | 64 | 0 |
| 5 | default | apalis | commit | +287.915 | 56 | 0 | 64 | 0 |
| 5 | external | oban | submit | -19.971 | 0 | 0 | 0 | 0 |
| 5 | external | oban | commit | -102.254 | 0 | 0 | 0 | 0 |
| 5 | external | river | submit | +27.478 | 0 | 0 | 23 | 0 |
| 5 | external | river | commit | -66.918 | 0 | 0 | 0 | 0 |
| 5 | external | apalis | submit | +450.425 | 144 | 3 | 39 | 0 |
| 5 | external | apalis | commit | +222.639 | 40 | 0 | 64 | 0 |

## Where the elapsed time differs

Each column gives the median paired Mesh minus peer difference in milliseconds. The middle interval covers the next 448 jobs reaching their first durable write. The four intervals sum to the submit-to-settlement difference in each individual pair. Column medians can come from different pairs.

| Storage | Peer | Submit to enqueue commit | Enqueue commit to 64th first write | 64th to 512th first write | Last first write to final settlement | Mesh faster during middle interval |
|---|---|---:|---:|---:|---:|---:|
| default | oban | +39.3 | -224.4 | +43.5 | +8.9 | 2/5 |
| default | river | +41.5 | -237.4 | +361.3 | -166.5 | 0/5 |
| default | apalis | +195.2 | -16.5 | +279.7 | +4.0 | 0/5 |
| external | oban | +51.4 | -172.2 | +25.2 | +0.2 | 2/5 |
| external | river | +54.1 | -271.2 | +385.0 | -143.8 | 0/5 |
| external | apalis | +211.2 | -35.9 | +298.3 | +6.5 | 0/5 |

The ranks use committed application writes from each engine’s own ledger. They leave CPU attribution and individual claim timing open. [Native storage receipts](../2026-11-24-slow-external-compat-scorer-input/evidence/native) · [Ledger replay](slow-peer-cutoffs.py)
