# Larger success cohorts at the existing dense flush times

In six released 2×32 rotations, letting an eight-job writer take additional already-completed jobs at the same instant could absorb an entire later terminal transaction in at most two instances across 1,047 actual terminal transactions. This prices a wider batch with the existing flush trigger on the recorded schedule. Changes to future grant timing, actor execution or deliberate collection time need a separate model.

| Rotation | Terminal transactions | Batches of eight | Other jobs possibly ready at those commits | Later transactions wholly covered | Mesh effects / settlements at River’s 768th effect | Warm River settlements |
|---|---:|---:|---:|---:|---:|---:|
| 1 | 159 | 54 | 87 | 1 | 448 / 438 | 584 |
| 2 | 166 | 52 | 69 | 0 | 463 / 451 | 538 |
| 3 | 162 | 56 | 65 | 1 | 478 / 470 | 609 |
| 4 | 167 | 46 | 47 | 0 | 452 / 441 | 523 |
| 5 | 233 | 8 | 14 | 0 | 261 / 257 | 535 |
| 6 | 160 | 48 | 49 | 0 | 499 / 493 | 600 |

A job can contribute to the fourth column only if its second write had committed by the earlier batch’s commit record, had an earlier XID, belonged to the same runner and settled later. The writer actually fixes membership before it receives a terminal XID, and actor exit follows the second application write. The counts grant every possibly eligible job credit without waiting for either confirmation. No individual eight-job batch had more than seven such jobs. The fifth column credits a later transaction even when its members could only have been absorbed by different earlier batches. The replay leaves all group membership and timings intact.

The last two columns align the engines at durable enqueue. With 64 seats charged through terminal confirmation, an engine needs at least 704 earlier settlements to record 768 effects. Mesh would have needed another 211–447 settlements to reach that necessary count at River’s elapsed time in these rotations. Warm River itself had 159–245 effect-committed jobs awaiting terminal commit at that point. Mesh had 4–12. The original comparison includes those engine-specific settlement policies. Its paired median deficit was 159 ms from commit and 273 ms from submit. [Original paired results](../2026-11-25-dense-fixed8-river-readiness/NATIVE-RESULTS.md)

Run `python3 -B ready_success_capacity.py | cmp - ready-success-capacity.json` in this directory. The program checks both timing releases, 12 measured archives and ledgers, the 2×32 topology, both clocks, cross-engine input identity, and 2,048 application XIDs per arm. [Per-rotation replay](ready-success-capacity.json)
