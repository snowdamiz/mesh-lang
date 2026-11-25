# SHORT16 application progress and final queue commits

Across six released rotations, warm River had 111–128 jobs whose first application write had committed while their final queue row was still uncommitted. Mesh R2 D2 peaked at 15–16. Both ran with 16 worker slots. River's final rows shared one or two transaction IDs per rotation. Mesh used 113–121. The timestamps come from PostgreSQL commit records for the first write, second write, and final queue row. They do not measure handler exit, free workers, or the client's receipt of COMMIT.

| Rotation | Mesh / warm River peak pending final commit | Warm River first writes with 16 earlier jobs still pending | Mesh / River second writes before first final commit | Mesh adjacent transactions with all next effects committed early |
|---|---:|---:|---:|---:|
| 1 | 16 / 128 | 112 | 12 / 128 | 104 of 120 |
| 2 | 16 / 112 | 112 | 8 / 32 | 95 of 115 |
| 3 | 15 / 128 | 112 | 8 / 128 | 100 of 119 |
| 4 | 15 / 126 | 112 | 11 / 126 | 93 of 112 |
| 5 | 16 / 111 | 97 | 2 / 18 | 97 of 118 |
| 6 | 16 / 128 | 112 | 2 / 128 | 89 of 114 |

Mesh recorded zero first writes with 16 strictly earlier jobs still pending final commit in every rotation. Its policy holds the seat through confirmation. Achieving River's recorded order with that policy would require earlier Mesh settlements. The rightmost column counts adjacent Mesh terminal transactions in commit order for which every job in the later transaction had committed its second write before the earlier transaction committed. This gives an upper bound on the pairs eligible for concurrent settlement at that point. The ledger lacks exit confirmations, send and acknowledgement timestamps. A second settlement connection's service cost and effect on refill remain unmeasured.

River v0.51.2 explains how the observed counts can exceed 16. The benchmark sets `MaxWorkers` to 16. On success the executor submits the job to the batch completer, then calls `JobDone`. The completer can return after queuing the update in memory. The producer releases worker capacity when it processes the completed executor, while the batch completer writes the queued states to PostgreSQL separately. This allows later handlers to start before the earlier jobs’ final queue rows commit. [Client configuration](https://github.com/riverqueue/river/blob/v0.51.2/client.go#L956) · [Executor](https://github.com/riverqueue/river/blob/v0.51.2/internal/jobexecutor/job_executor.go#L172-L174) · [Batch completer](https://github.com/riverqueue/river/blob/v0.51.2/internal/jobcompleter/job_completer.go#L632-L676) · [Worker capacity](https://github.com/riverqueue/river/blob/v0.51.2/producer.go#L814-L824)

## Middle 112 durable effects

The window begins at the 16th second write and ends at the 128th. Each handler contributes the part of each interval inside that window. Dividing job-microseconds by 16 converts occupancy differences to milliseconds on a 16-seat scale. The last column is the window capacity outside entry through second write. It includes work before recorded entry and after the second write. CPU time and recoverable wall time require separate evidence.

| Rotation | Mesh minus warm River middle wall, ms | Entry through first write | First 1 ms of hold | Rest through second write | Outside entry through second write |
|---|---:|---:|---:|---:|---:|
| 1 | 69.0 | 14.9 | 0.0 | 11.0 | 43.1 |
| 2 | 64.1 | 15.0 | 0.0 | 5.0 | 44.1 |
| 3 | 73.1 | 17.2 | 0.1 | 14.5 | 41.3 |
| 4 | 74.0 | 18.9 | 0.1 | 10.6 | 44.4 |
| 5 | 58.0 | 17.5 | −0.1 | 11.9 | 28.7 |
| 6 | 24.8 | 16.2 | 0.0 | 5.9 | 2.7 |

The terms sum to the measured middle-wall difference before rounding. The hold column uses the required first 1,000 µs after the first durable write. SQL, wakeup and scheduling can all contribute to the rest of the interval.

## Replay

Run `python3 -B settlement_capacity.py` in this directory and compare stdout to `settlement-capacity.json`. The script verifies the original summary digest and all 42 measured archive digests, checks the matched payloads, reconciles 256 independent application transactions per arm, and reproduces the original submit and drain clocks. The raw captures and released decisions belong to the existing River readiness series.
