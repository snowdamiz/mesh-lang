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
