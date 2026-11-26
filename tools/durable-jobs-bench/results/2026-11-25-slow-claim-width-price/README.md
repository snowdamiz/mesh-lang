# What a wider grant could change on the one-runner SLOW queue

In all five canonical 1×64 Mesh runs, the first 64 jobs were claimed in four groups of 16. The fourth claim stamp followed the first by **57–75 ms**. For each of the first three groups, the first handler entered **14.5–24.6 ms** after its claim stamp and **1.6–7.6 ms** before the next stamp. That ordering matters for a 32-job grant. A larger reply could allow the later 16 jobs to start sooner but delay the first 16. The native comparison needs first handler entry as well as last effect and last settlement.

| Stock block | Claim stamps | Full 16-job stamps | Full stamps with spare capacity and a following group | Disjoint pairs that fit entirely in 32 | Normalized seat-time in the disjoint-pair model |
|---|---:|---:|---:|---:|---:|
| 1 | 67 | 18 | 15 | 6 | 34.3 ms |
| 2 | 55 | 21 | 19 | 9 | 53.7 ms |
| 3 | 56 | 21 | 17 | 10 | 57.6 ms |
| 4 | 61 | 20 | 19 | 7 | 48.0 ms |
| 5 | 55 | 22 | 17 | 8 | 44.0 ms |

Four later inline controls at the same topology produced 7–12 disjoint pairs and 39.2–50.8 normalized ms. Each of the nine archives reconciled 512 jobs, two independent application XIDs per job, a hold of at least 100 ms between committed writes, and 512 individual terminal XIDs. Each run used one runner, 64 slots and the cap of 68 connections.

## Interpreting the model

Group jobs on the one runner by their recorded grant millisecond. For an original group of 16, count every job granted earlier whose terminal commit was at or after that stamp as occupying a seat. Credit up to 16 jobs from the next grant, limited by its actual size and spare capacity at the original stamp. The first pair measure counts how many complete following groups could fit. The second weights the eligible seats by the gap between stamps less 1 ms for timestamp resolution, then divides by 64. Each measure independently selects disjoint adjacent pairs, so a cohort cannot help twice.

This is headroom in the observed database ledger. The runner still needs terminal acknowledgements before reusing seats. Stamps are written inside SQL before grant confirmation. Wider batches change execution, payload transfer, lock duration and handler scheduling. Normalized seat-time cannot predict elapsed savings. The three stock submit losses to River were 95, 183 and 97 ms. The useful native question is whether earlier admission advances the subsequent waves enough to overcome the larger initial reply.

From this directory, run `python3 -B replay.py | cmp - claims.json`. The replay verifies the measured raw digests from the earlier [same-image price audit](../2026-11-25-public-slow-hot-checkout-price/evidence/prices.json). All observations use existing Linode archives.
