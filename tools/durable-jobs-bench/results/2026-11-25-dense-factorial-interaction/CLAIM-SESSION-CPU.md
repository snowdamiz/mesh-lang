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

For the actual first-submit-to-last-settlement window, a conservative bound on the eight identified backends is **132–610 CPU-ms**. Each process’s separately truncated user and system counters contribute up to two ticks of uncertainty at either end of the difference. The outermost frame margins total 78.917 ms, across two CPUs. Neither the bracket nor the bound isolates the time spent on lock acquisition, SQL execution, recovery, or a particular function. The application runner is a separate process from its PostgreSQL backends.

Run `python3 -B claim_session_ticks.py | cmp - claim-session-ticks.json` in this directory. [Verified PIDs, ticks and bounds](claim-session-ticks.json)
