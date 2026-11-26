# Slow storage crossover on Linode

Five frozen blocks reconciled on November 24, 2026 UTC. Each block includes
eight warmups and eight measurements of the exact 512-job slow fixture.
PostgreSQL 17.11 and workers shared CPUs 0 and 1 on the original AMD64
Linode. The default and EXTERNAL lanes for each engine used the same
executables and workload. [Inventory](frozen/native-config.json) ·
[Controller release](evidence/native/controller-timing-release.json)

| Engine | Median paired submit-to-settlement saving (ms) | Range (ms) |
| --- | ---: | ---: |
| Mesh bulk | 179.39 | 147.32 to 208.72 |
| Oban | 293.80 | 184.38 to 352.96 |
| River | 200.64 | −34.89 to 246.98 |
| Apalis | 225.46 | 143.83 to 477.13 |

Savings are default elapsed time minus EXTERNAL elapsed time within each
block. Mesh bulk's median commit-to-settlement saving was 18.68 ms, with a
range of −25.63 to 49.67 ms. Median stored column bytes were equal between
the two settings for each engine on this fixture. [Full replay and paired distributions](evidence/native/summary.json)

An interrupted group 2 attempt retained one successful River warmup. The
controller verified the same boot, affinity, disk and PostgreSQL executable,
checked all three locks, and recovered that arm's raw export. The scorer
selected the original complete group 1 and complete replacement blocks
for groups 2–5. [Interrupted attempt and recovery receipt](evidence/native/timing-d54d1fc3fec9497dafea90933a8a1255/) ·
[Archive hashes](evidence/native/SHA256SUMS)

Replay the preserved archives without starting a workload.

```sh
python3 -B tools/durable-jobs-bench/results/2026-11-24-slow-external-compat-scorer-input/replay/native.py replay
```
