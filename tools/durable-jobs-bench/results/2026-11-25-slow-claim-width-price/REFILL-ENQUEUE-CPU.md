# CPU availability during the public SLOW enqueue

The full-refill arms had about one idle CPU across a middle portion of the producer transaction. Forward recorded 19 idle ticks out of 43 total (44%) on CPUs 0 and 1. Reverse recorded 21 of 45 (47%). At the configured 100 ticks/s these idle counts correspond to 190 and 210 CPU-ms. They cover part of the approximately 568 ms submit-to-commit interval in each rotation.

| Full-refill rotation | First read after submit | Last read after submit | Time between reads | Idle ticks |
|---|---:|---:|---:|---:|
| Forward | 120–142 ms | 339–362 ms | 198–241 ms | 19 |
| Reverse | 110–131 ms | 328–360 ms | 198–249 ms | 21 |

The sealed worker reads CPU counters before it stamps a frame. Every intermediate frame follows a timed-out 200 ms wait for the same live adapter process. The replay pins those exact worker bytes, excludes the final post-exit frame, and brackets each read between the previous stamp plus the wait and its own stamp. It allows 1 ms either way for clock alignment and checks consecutive realtime and monotonic frame deltas. The bounds assume no unobserved realtime step and reversal within one interval. Quantization limits the CPU counts. The table rounds the time bounds outward to whole milliseconds.

All ten measured arms had two or three reads inside enqueue under these bounds. A coarser bracket that uses the preceding stamp without the known wait leaves only one read in each combined arm. The worker's timeout is what permits measuring these two differences.

The frozen inline adapter reads and parses the batch file and re-encodes the request after submission starts. The broker reads the full line, parses the JSON, opens the transaction, constructs its first batch, then issues the first admission. Preparing batch two during batch one's writer cannot hide the input transfer or the first batch's construction. The host counters establish spare CPU during enqueue. They do not attribute time to those stages or show which PostgreSQL statement was pending at a read.

From this directory, run `python3 -B refill_enqueue_cpu.py "$timing_evidence" | cmp - refill-enqueue-cpu.json` with the timing evidence from `a3070ff56`. The replay checks the exact controller release and all 20 original captures. [Worker source](../../worker.py) · [Per-arm receipts](refill-enqueue-cpu.json)
