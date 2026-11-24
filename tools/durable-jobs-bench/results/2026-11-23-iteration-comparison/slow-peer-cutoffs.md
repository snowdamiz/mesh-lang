# Slow Mesh jobs outstanding at peer completion

Verified 1 of five Linode rotations of the 512-job storage comparison. Each job held for 100 ms between two independent application writes. Each row compares two arms from the same rotation and storage policy on CPUs 0 and 1.

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

In the first EXTERNAL rotation, Mesh’s last job settled 2.566 ms after River’s on the submit clock. Every Mesh handler had already committed its second write at River’s finish. At Apalis’s finish on the drain clock, 108 Mesh jobs still awaited entry and 60 more were between their first and second writes. The default-storage arm also had 57 jobs awaiting entry at Apalis’s drain finish. These counts establish a remaining handler-entry and refill target in this rotation. The remaining rotations determine whether it repeats.

[Native storage receipts](../2026-11-24-slow-external-compat-scorer-input/evidence/native) · [Ledger replay](slow-peer-cutoffs.py)
