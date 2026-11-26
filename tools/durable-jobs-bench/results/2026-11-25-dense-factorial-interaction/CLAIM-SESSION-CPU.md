# PostgreSQL claim-session CPU in the slow 8×8 rotation

In the 2.249-second drain, all eight dedicated claim connections logged a queue-lock timeout. Their PostgreSQL backends used **450 CPU-ms** across the submit-to-observer bracket. The other 24 queue backends used 680 CPU-ms, the 32 application backends 580 CPU-ms, and the producer/observer 150 CPU-ms. These add to the frozen scorer’s 1,860 PostgreSQL CPU-ms. The 8.860 seconds of accumulated successful prelock statement elapsed time include overlapping waits.

| Released B rotation | Queue CPU-ms, all 32 | Application CPU-ms | Producer/observer CPU-ms | Claim connections identifiable from timeouts |
|---|---:|---:|---:|---:|
| 1 | 790 | 510 | 130 | 0 |
| 2 | 1,130 | 580 | 150 | 8 |
| 3 | 820 | 530 | 170 | 0 |
| 4 | 840 | 510 | 180 | 0 |
| 5 | 860 | 490 | 160 | 0 |

The server logged 20 failures on the initial queue lock and two inside the bounded claim. All 22 occurred between durable enqueue and final settlement. Each message identifies its PostgreSQL PID and statement. The eight PIDs each accumulated four to six whole CPU ticks, including work in successful transactions. No other rotation recorded such a failure. The other rotations therefore supply no matched claim-session CPU baseline. [Timeouts and durable progress](LATE-CAPACITY-PEER-PHASES.md)

The replay joins the full readiness TLS census to both process frames by PID, postmaster PID and process start tick. Rotation 4’s earlier census had only 63 connections. All 65 were present at readiness and in the process frames. Every rotation’s CPU sum reproduces its original scorer.

## Comparison with the same-binary OFF controls

Across the 20 measured Mesh arms, most of the difference in observed PostgreSQL CPU between B and its controls fell on queue connections. Both OFF controls used the same executable as B. The table shows CPU-ms accumulated between the two PostgreSQL reads, rounded to the kernel's 10 ms tick.

| Rotation | Queue B | Queue OFF A1 / A2 | Application B | Application OFF A1 / A2 |
|---|---:|---:|---:|---:|
| 1 | 790 | 940 / 1,040 | 510 | 510 / 530 |
| 2 | 1,130 | 1,000 / 1,020 | 580 | 520 / 520 |
| 3 | 820 | 1,030 / 930 | 530 | 520 / 530 |
| 4 | 840 | 940 / 1,070 | 510 | 560 / 520 |
| 5 | 860 | 990 / 1,200 | 490 | 520 / 570 |

In rotations 1, 3, 4 and 5, B used 100–150 fewer queue CPU-ms than the lower queue control. Application CPU differed from the lower application control by −30 to +10 ms. In rotation 2 both categories increased. The 450 CPU-ms on its identified claim connections includes successful work and gives no estimate of the cost of its 22 timeouts alone. Comparing the application-backend totals also leaves the Mesh runner's pool, actor and TLS costs outside that accounting.

Frame margins and per-process rounding limit attribution inside the workload clock. These contrasts describe recorded counter differences and cannot be added to elapsed gains. Run `python3 -B claim_session_ticks.py --controls` for the [matched replay](claim-session-controls.json), including the grouped reference and all 20 raw hashes.

For the actual first-submit-to-last-settlement window, a conservative bound on the eight identified backends is **132–610 CPU-ms**. Each process’s separately truncated user and system counters contribute up to two ticks of uncertainty at either end of the difference. The outermost frame margins total 78.917 ms, across two CPUs. Neither the bracket nor the bound isolates the time spent on lock acquisition, SQL execution, recovery, or a particular function. The application runner is a separate process from its PostgreSQL backends.

Run `python3 -B claim_session_ticks.py | cmp - claim-session-ticks.json` in this directory. [Verified PIDs, ticks and bounds](claim-session-ticks.json)

## Which PostgreSQL sessions consumed CPU wholly inside the diagnostic middle?

The November 25 cross-runner diagnostic captured per-backend Linux scheduler counters during durable progress on exact B, corrected OFF, observe-only and corrected ON. In each arm, eight of the 32 Jobs connections issued terminal SQL. The table charges each backend's scheduled CPU to its authenticated connection. That includes every operation on that connection.

| Diagnostic lane | Durable effect counts at the two frame ends | Time between frame ends (ms) | 32 application backends (CPU-ms) | 8 terminal-bearing Jobs backends (CPU-ms) | Other 24 Jobs backends (CPU-ms) |
|---|---:|---:|---:|---:|---:|
| Exact B | 400–673 | 691.3 | 191.0 | 159.1 | 98.6 |
| Corrected OFF | 429–673 | 630.9 | 177.4 | 150.5 | 98.3 |
| Observe only | 409–662 | 637.0 | 168.9 | 144.2 | 89.7 |
| Corrected ON | 406–675 | 701.8 | 185.6 | 143.8 | 98.9 |

The producer backend recorded zero scheduled CPU in each interval. The 32 application backends consumed the most CPU of the three individual sets in every arm. The two Jobs sets together consumed more. Both the application path and queue SQL warrant accounting when evaluating a runner optimization. The intervals cover different numbers of jobs. Direct subtraction would confound the change with the amount of work captured.

The reads requested at effects 384 and 640 fell entirely between durable effects 256 and 768. For either read, the previous sequential frame had already ended after the lower boundary, and the frame's own end timestamp preceded the upper boundary. The replay joins the TLS registration to terminal executions by launch PID and connection number, matches backend PIDs and roles to the full roster, and verifies process start ticks at the endpoints. The counters are scheduled time for PostgreSQL backend tasks. They exclude the client, TLS relay and sampler processes. They do not identify SQL planning CPU, waits, or the critical path. This fixture uses 1,024 active diagnostic entries with empty supplied argument objects and an instrumented relay. The canonical 1 KiB-payload peer timings remain the comparison for performance. The diagnostic SETUP retained its exit-1 release.

[Raw-bound CPU and frame counts](terminal-backend-cpu.json) · [Archive reducer](terminal_backend_cpu.py)
