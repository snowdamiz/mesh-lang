# Bulk producer on ordinary V2 short

The inventory pins the same ordinary V2 runner ELF for both Mesh lanes.
Both use set-based admission and the `success_cohort` terminal policy. The
scalar and bulk lanes differ in the producer switch. The suite retains the
original River, Oban, and Apalis executables, 128 short jobs per measured
arm, two independently committed app writes per job, and CPUs 0 and 1
shared by the runners and PostgreSQL 17.11. [Exact inventory](inventory.json)

| Native identity | SHA-256 |
| --- | --- |
| Ordinary V2 ELF | `55d7000d562a6ad395f5e64e33e0e973fac0084ed05ad6c30a33db2a3fc6303a` |
| Ordinary V2 manifest, both lanes | `7f228c192630bb6d95cbf9e06c5e0132931f429c1cfa07391bb28cb6fdec8911` |
| River ELF | `5db25a75ccfbf992cb72479a86470baa3a1ac054b5d780e0854e41caaaadf4d0` |
| Oban BEAM VM | `921aeaddb61ce696db24621eaece52220b16993d3fe591920a271bf3dd720206` |
| Apalis ELF | `b3c2fb2b472d7c8b2706e13c38281a46958b669f9d4cf22ca252636d083e7e66` |

The five canonical crossover rotations reverse the order in groups 2
and 4. The timing wrapper changes only that plan. The native V2 driver
reconciles both writers, their terminal XIDs, and the durability and
affinity gates. The paired reducer uses measured host counters bracketing
first submit and last settlement for idle and steal. It keeps submit and
drain rates, their p99 windows, and CPU costs separate. [Frozen orders](planned-orders.json)

The shared source archive contains the exact three runtime variants and
the compiled workload. [Source archive](../2026-11-23-set-based-enqueue/native/bulk-enqueue-ce5e551d079e.tar.gz) · [Native qualification](../2026-11-23-set-based-enqueue/evidence/native-setup-3b09dc3d0e23/README.md)

## Native results

Five reconciled blocks completed on November 23, 2026. The first group-2
attempt recorded 1.06% CPU steal and failed the 1% gate. It remains in the
packet. A separate attempt in the same order passed. Observed CPU steal in
the five selected blocks was zero. [Replay selection and retained failure](evidence/bulk-enqueue-v2-short/summary.json)

| Lane | Jobs/s from submit, median [range] | Jobs/s from first commit | Submit-to-settle p99 (ms) | Runner CPU (ms/job) | Pinned CPU (ms/job) | Pinned idle (%) |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| V2 bulk | 596.54 [589.21–603.75] | 714.01 | 213.89 | 1.26 | 3.28 | 48.86 |
| V2 scalar | 498.16 [449.21–528.19] | 700.18 | 256.94 | 1.28 | 3.75 | 50.56 |
| Oban | 308.13 [263.69–325.53] | 353.14 | 415.13 | 10.87 | 4.77 | 54.07 |
| River | 697.01 [557.04–734.09] | 754.41 | 183.64 | 0.86 | 3.44 | 58.89 |
| Apalis | 742.12 [609.44–879.94] | 790.22 | 172.48 | 0.64 | 3.28 | 52.81 |

Bulk finished ahead of same-image scalar and Oban in every full-submit
window, and ahead of River in two. Apalis led bulk in all five. Relative
to scalar, the median paired time to first committed enqueue fell by
34.07 ms. The paired median p99 fell by 45.56 ms from submit and 6.84 ms
from first commit. Measured drain was faster in four scalar pairs and two
River pairs. The bulk lane used a median of 121 terminal XIDs [112–125]
and scalar used 118 [115–122]. The largest observed terminal cohort was
six for bulk and five for scalar. Both stayed within the eight-job limit
and retained two separate app commits per job. [Paired proof, CPU and latency data](evidence/bulk-enqueue-v2-short/paired.json)

```sh
suite=tools/durable-jobs-bench
packet="$suite/results/2026-11-23-bulk-enqueue-v2-short"
PYTHONPATH="$suite" python3 -B "$packet/replay/rotated.py" summarize bulk-enqueue-v2-short
python3 -B "$packet/replay/paired.py" "$packet/evidence/bulk-enqueue-v2-short"
```
