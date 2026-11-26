# Where the two-runner dense drain gained time

Two native 1,024-job rotations compared one, two, and eight trusted Mesh V1 runners on the same two Linode CPUs. All deployments kept 64 slots and 32 warm application connections. Each of the eight arms recorded two application transactions and one separate settlement transaction per job. The replay verifies all eight archive digests, durable XIDs and occupancy integrals.

| Rotation | Faster eight-runner control | Two-runner drain gain (ms) | Earlier last claim (ms) | Change in last-claim-to-finish tail (ms saved) | Control jobs still unclaimed when two runners finished |
|---:|---|---:|---:|---:|---:|
| 1 | stock-b | 950.0 | 950.7 | -0.7 | 445–445 |
| 2 | stock-a | 983.0 | 1005.4 | -22.4 | 447–448 |

The last-claim and tail differences sum to the total gain. This locates the elapsed difference. The time of a claim also depends on the release of earlier slots, scheduling and database service. Each cutoff is aligned to that arm’s own enqueue commit. The unclaimed interval allows for the millisecond resolution of the claim stamp.

| Layout | Mean claimed to entry (ms/job) | Entry to first commit | First to second commit | Second commit to settlement | Mean occupied slots |
|---|---:|---:|---:|---:|---:|
| Eight runners (both controls) | 9.15 | 11.41 | 2.85 | 8.40 | 14.11 |
| Two runners | 28.73 | 12.42 | 1.92 | 7.49 | 38.62 |
| One runner | 39.93 | 16.99 | 1.27 | 5.78 | 42.62 |

Each stage is the across-arm median of its mean per-job residence time. Two runners sustained more work in flight and moved the final claim forward despite a longer wait from claim to handler entry per job. Process count, scheduler supply and the maximum grant size differed together in this native diagnostic. [Original topology report](../2026-11-23-ordinary-runner-topology/README.md) · [Replay](topology.py)
