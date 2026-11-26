# Public LARGE file-session result

The adapter-path retry released with exit 0 at 00:38:19 UTC on
November 25. Offline replay reconciled five interleaved blocks across
all seven lanes and verified 70 warmup and measured archives. The
original stock runtime and broker binding used the [recovered SETUP](../setup-export-recovered).

Against the faster inline duplicate on each clock, file transport
saved a median of 290.167 ms to enqueue commit across five blocks
(range 188.116–354.488 ms) and 288.092 ms to final settlement
(range 177.620–389.201 ms). Both clocks improved in each block.
Post-commit drain took a median 10.496 ms longer against the faster
duplicate. Each arm used 64 jobs on the native default-storage workload.

After release the three-lock cleanup removed the disposable Cargo,
temporary, and rebuild paths. It verified their absence and retained
stage, runtime, broker, and adapter hashes.

See the [controller release](../timing-adapter-path-retry-release.json),
[full scores](../timing-adapter-path-retry-scores.json), and
[cleanup receipt](../timing-adapter-path-retry-cleanup.txt).
Replay from the repository root.

```bash
python3 -B tools/durable-jobs-bench/results/2026-11-24-public-large-file-session/resume/adapter-path-retry/retry.py scores
```
