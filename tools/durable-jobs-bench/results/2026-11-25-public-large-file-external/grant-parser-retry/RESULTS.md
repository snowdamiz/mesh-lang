# Public LARGE FILE under symmetric EXTERNAL storage

Mesh FILE finished ahead of River and Oban from submit to durable settlement in all five matched Linode blocks. The median paired throughput ratio was 1.75× against River and 1.20× against Oban. Apalis finished ahead of Mesh FILE in all five blocks. [Replay](evidence/replay.json)

| Engine | Submit to settlement, median and range | Median jobs/s |
| --- | ---: | ---: |
| Mesh FILE | 1,363 ms, 1,321–1,641 ms | 47.0 |
| River | 2,322 ms, 2,285–2,475 ms | 27.6 |
| Oban | 1,663 ms, 1,640–1,750 ms | 38.5 |
| Apalis | 866 ms, 823–896 ms | 73.9 |

Each block ran the same 64 jobs with 512 KiB payloads on CPUs 0 and 1. Each arm used eight runners with eight slots each. The controller ran five rotations across seven lanes and exported 35 warmup and 35 measured arms. The replay verified all 70 raw digests and reconciled 64 successful jobs per arm. The host, runner, broker, storage policies, and clocks match the sealed plan. The controller released exit 0 at 02:48:36 UTC. [Configuration](evidence/native/series.json) · [Release](evidence/native/controller-release.json)

Against the faster same-broker inline control in each block, FILE saved a median 293 ms to durable enqueue (210–346 ms) and won all five pairs. The median submit-to-settlement saving was 320 ms, with four wins. In block 4, FILE saved 210 ms to enqueue and lost 13 ms through settlement after a longer post-commit drain. The smallest FILE lead over Oban was 22 ms. Oban had a faster post-commit drain in all five blocks. [Paired scores and raw receipt digests](evidence/replay.json)

The original 02:28:27 UTC attempt released exit 1 before staging or workload execution. Its original packet, traceback, and release remain in the parent directory. The retry used the original scoring and launch inventories under a separate grant. [Original release](../evidence/native/controller-release.json) · [Retry packet](controller-packet.json)
