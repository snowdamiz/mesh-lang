# Released native logical-key grant pilot

The sole controller released groups 0–2 at 03:08:26 UTC with exit 0. All eight arms completed 1,024 jobs with 2,048 distinct application-write XIDs per arm, one independent durable enqueue transaction and the grouped terminal contract. All arms used the pinned public grouped-hot ELF and broker on PostgreSQL 17.11, two 32-slot runners, CPUs 0–1 and 32 warm application TLS sessions. Packet SHA-256 `cc5eff4abce062383d0217a836ae36b0fddfba0947573d5874dcc35b2240b18f`. Candidate SQL SHA-256 `d388baa3b67c07df1a2498cd1fdc60bf7d0ce11f4b42d950f7b0a64a551c0f45`. [Release](evidence/native/controller/controller-timing-523a54a77bb8.json) · [Offline replay](evidence/native/final-replay.json)

## Live plans

Stock sampled 36 positive grants. The UPDATE-side index scan visited 1,024 rows in each sample, a total of 36,864 target visits and approximately 470,580 join-filter rejections. The candidate sampled 42 positive grants with 427 target visits in total, 10.2 per positive sample, and zero observed join-filter rejections. All candidate positive samples met the bounded-access gate and its eight empty samples made zero target visits. Both runner backends were represented in each arm. The plans came from the fresh pre-submit idle and committed drain of compiled diagnostic arms. The diagnostic wall clocks are excluded from the pilot. [Plan qualification](evidence/native/blocks/00-noop.1795576019904070000/reconciled.json)

## Uninstrumented rotations

- **A/key/B** finished in 1,142 ms from submit and 976 ms from commit. The candidate beat the faster stock by 155 ms (11.9%) on submit and 163 ms (14.3%) on drain. Duplicate spread was 55 ms and 37 ms respectively. [Group 1](evidence/native/blocks/01-noop.1795576047273411000/reconciled.json)
- **B/key/A** finished in 1,143 ms from submit and 977 ms from commit. The candidate beat the faster stock by 161 ms (12.3%) on submit and 162 ms (14.2%) on drain. Duplicate spread was 37 ms and 35 ms respectively. [Group 2](evidence/native/blocks/02-noop.1795576076750507000/reconciled.json)

PostgreSQL CPU in the matched process bracket fell by at least 270 and 310 CPU-ms, respectively, with 47 stable PIDs and zero unmatched PIDs in all six timing arms. Host CPU fell by at least 280 and 350 CPU-ms. Peak connections were 41 in all arms. Runner lifetime CPU rose above the higher duplicate by 18.580 and 2.878 CPU-ms. The separately reported PostgreSQL bracket includes endpoint slack and the procfs ticks are quantized. These two rotations demonstrate a wall-clock gain and lower aggregate CPU for this deployment. They leave the prespecified runner CPU constraint unmet in both groups.

**Disposition.** Live plans and both wall-clock gates passed. The resource gate held River, Oban and Apalis. The fresh-snapshot fallback also remains held. All raw archives, dispatch receipts, inventory pins, failure checks and reductions are retained. There were no failed arms in this reservation. [Evidence hashes](evidence/native/SHA256SUMS)
