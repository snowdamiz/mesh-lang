# Dense PostgreSQL concurrency limits on Linode

Keep synchronous PostgreSQL access for the checked-pool R2 runner. Neither limit met the predeclared wall, CPU, and memory criteria in either order. Both limited modes consumed more runner CPU per job than either synchronous control in both rotations. Each runner had 32 job slots and 16 warmed application connections. PostgreSQL 17.11 and the two runners shared CPUs 0 and 1.

| Mode | Submit to settlement, jobs/s (forward / reverse) | Enqueue commit to settlement, jobs/s (forward / reverse) | Runner CPU-ms/job (forward / reverse) |
| --- | ---: | ---: | ---: |
| Synchronous A | 697 / 602 | 786 / 674 | 0.609 / 0.630 |
| Synchronous B | 680 / 678 | 778 / 766 | 0.615 / 0.629 |
| Readiness, 2 permits per runner | 649 / 596 | 723 / 672 | 0.829 / 0.843 |
| Readiness, 4 permits per runner | 673 / 628 | 756 / 714 | 0.744 / 0.789 |
| Uncapped readiness | 699 / 698 | 788 / 788 | 0.663 / 0.655 |

The uncapped arm was a reference. The planned peer pilot required a bounded candidate to qualify. Five diagnostic arms, ten warmups and ten measured arms each reconciled 1,024 jobs and 2,048 distinct application write transactions. The standalone replay verifies the raw digests, job counts, XIDs, and decision for each rotation.

```sh
python3 -B tools/durable-jobs-bench/results/2026-11-24-dense-postgres-permits/replay.py
```

The controller released both measured rotations successfully. Its first post-release replay used the base matrix adapter and rejected the amended artifact identity. Installing the same stage and replay functions used during timing reproduced the two frozen decisions from their original exports. Both orders and all raw arm archives are retained here.
