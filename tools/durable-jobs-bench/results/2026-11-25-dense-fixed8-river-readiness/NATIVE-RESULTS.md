# Dense fixed8 + d388 versus warm River

**Mesh had a deficit on both predeclared workload clocks.** Warm River finished ahead in all six matched blocks, exceeding each block's 5% floor and stock-to-cold bridge spread. All arms used PostgreSQL 17.11, cap 96, CPUs 0 and 1, two 32-slot runners, and the frozen batch of 1,024 jobs. Each measured arm reconciled 2,048 independent application commits and 1,024 durable settlements. [Final replay](evidence/native/replay-rest.json) · [Independent custody audit](evidence/release-audit.json)

| Paired measurement across six blocks | First submit → final settlement | Enqueue commit → final settlement |
| --- | ---: | ---: |
| Warm River time saved vs. Mesh, median (range) | 272.638 ms (187.937–720.176) | 158.896 ms (102.349–635.662) |
| Mesh throughput deficit vs. warm River, paired median | 26.1% | 18.5% |
| Warm River time saved vs. same-image cold, median (range) | 241.358 ms (217.835–276.158) | 243.539 ms (204.746–277.431) |
| Frozen stock elapsed minus same-image cold, median (range) | +9.333 ms (−31.587 to +31.488) | +12.260 ms (−22.756 to +24.957) |
| Mesh time saved vs. frozen stock bridge, median (range) | −27.104 ms (−475.605 to +80.565) | +91.287 ms (−380.987 to +155.403) |

In this sensitivity's frozen stock bridge, Mesh led from submit in 2/6 blocks and on drain in 5/6. With warm River those counts fell to 0/6 on both clocks. The independently released five-block stock series is still the historical comparison. These results price readiness on the dense workload. [Original stock-peer series](../2026-11-25-dense-fixed8-logical-factorial/PEER-RESULTS.md)

| Measured lane | Submit jobs/s, median (range) | Drain jobs/s, median (range) | Last idle → timed peak, total sessions |
| --- | ---: | ---: | ---: |
| Mesh fixed8 + d388 | 980 (683–1,053) | 1,195 (770–1,256) | 41 → 41 |
| Frozen River stock | 999 (972–1,016) | 1,081 (1,055–1,095) | 7 → 67–69 |
| River adapter cold | 1,003 (969–1,019) | 1,079 (1,054–1,113) | 7 → 67–69 |
| River adapter warm | 1,314 (1,301–1,329) | 1,467 (1,432–1,496) | 67 → 67–69 |

## Readiness cost

Both River modes use the same ELF. Warm held two 33-session TLS cohorts, each with one detached LISTEN connection, plus the producer/observer. The identities held through the three-second idle period and up to submit. Mesh held 32 application TLS sessions within its 41 total sessions.

| River startup measure | Cold median | Warm median |
| --- | ---: | ---: |
| Runner launch → readiness | 56.400 ms | 314.664 ms |
| Producer connection and health checks | 15.242 ms | 14.976 ms |
| Two runners' total CPU at readiness | 10 CPU-ms | 160 CPU-ms |
| Two runners' summed RSS at readiness | 23.6 MiB | 32.1 MiB |
| PostgreSQL client-backend summed RSS before submit | 147.9 MiB | 1,004.0 MiB |

Warming added a paired median **267.002 ms** to readiness (244.012–270.047), with **279.603 ms** median fill time per runner across 12 fills. The fills can overlap. RSS sums include shared pages. Client-backend CPU was measured at 10 ms resolution per process and rounded to zero at the median in both modes.

On the River-only clock from runner launch through final settlement, warming added a paired median **29.466 ms** (−4.750 to +35.493). This clock includes the common three-second idle period and starts after producer setup and schema provisioning. Mesh has no matching launch origin in these archives.

## Write and settlement progress

| Median across six blocks | Mesh | River stock | River cold | River warm |
| --- | ---: | ---: | ---: | ---: |
| Commit → 64th second write (ms) | 87.0 | 302.2 | 296.8 | 74.3 |
| Second writes, rank 256 → 768 (ms) | 382.8 | 213.6 | 213.3 | 217.4 |
| Commit → final second write (ms) | 850.7 | 739.9 | 748.1 | 505.7 |
| Last second write → final settlement (ms) | 5.6 | 207.5 | 202.0 | 191.0 |
| Terminal transaction count | 164 | 12.5 | 12 | 10 |

Warming moved River's early writes forward while leaving the middle write interval close to cold. Warm River also traversed the middle interval faster than Mesh. Mesh's shorter settlement tail recovered part of that gap. Group 5 remains in the six-block result. Its eight raw arms reconciled, and Mesh's 11.71% host I/O wait exceeded its 0.88–1.77% in the other five measured blocks. It recorded zero CPU steal and completed every job. In that block warm River led by 720.176 ms from submit and 635.662 ms on drain. The other five blocks also passed the deficit threshold on both clocks. These counters describe group 5 and do not isolate a single cause for its longer Mesh service time. [Group 5 reconciliation](evidence/native/blocks/05-noop.1795580423273889000/reconciled.json)

## Release and offline verification

SETUP released exit 0 at 04:15:34 UTC. The reversed pilot released exit 0 at 04:17:59 UTC. Continuation, including group 5, released exit 0 at 04:22:36 UTC on November 25. Commit `e17ea56cb` retains the exact releases, order, staged inventories, logs, three controller replays and 52 raw archives. The independent audit checks every archive and the matching ledger hashes and reconstructs all three replays byte for byte. [Release receipts](evidence/native/controller) · [Audit manifest](evidence/release-audit.json)

Run this command from the checkout or the root of the Git export. The verification reads committed evidence without building or launching a workload.

```sh
python3 -B tools/durable-jobs-bench/results/2026-11-25-dense-fixed8-river-readiness/publication/verify_release.py
```
