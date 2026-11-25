# Protected LARGE handler progress on Linode

In all five 64 × 512 KiB rotations the prepared claim reached the eighth handler entry later than stock, then completed the last application write and terminal settlement earlier. Both paths used fresh workers. This series measured protected candidate `955b356b` and stock `3828f20d`.

| Milestone from first durable enqueue commit | Paired median change in elapsed time, prepared minus stock |
| --- | ---: |
| Eighth handler entry | +101 ms |
| Eighth second-write commit | +144 ms |
| 32nd second-write commit | +78 ms |
| 56th second-write commit | −28 ms |
| Last second-write commit | −76 ms |
| Last terminal settlement | −69 ms |

Each row had the same sign in all five pairs. The ledger orders each milestone independently across all jobs. These timestamps locate the change within the drain. They do not isolate the cost of taking locks, transferring payloads, or starting a worker.

Against stock, paired median throughput rose 8.96% after enqueue commit (5/5 wins) and 0.35% from submit (3/5). Prepared Mesh beat River on full submit in 5/5 and on drain in 4/5. Oban beat prepared Mesh in 4/5 full submits and all five drains. Apalis was outside this historical series.

Run `python3 -B replay.py` here to verify all 40 raw archive hashes and reproduce [result.json](result.json) from the [controller release](../2026-11-23-large-v3-timing-955b356b/controller-release.json). The separate public FILE consumer uses a different runner and has its own [native pilot](../2026-11-25-public-large-exact-prepare/README.md).
