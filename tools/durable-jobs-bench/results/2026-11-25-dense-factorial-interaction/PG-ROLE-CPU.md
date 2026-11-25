# Where the dense factorial saved PostgreSQL CPU

The eight PostgreSQL queue backends account for the largest measured CPU reduction in both fixed8+d388 pilots. All ten pilot arms used the same PostgreSQL 17.11 build, reconciled 1,024 jobs, and retained all 41 client backend identities across the CPU samples. Their PostgreSQL clock ticks give the following totals. Each tick represents 10 CPU-ms.

| Rotation | Configuration | Queue backends (8) | Application backends (32) | Producer/observer backend (1) |
|---|---|---:|---:|---:|
| 1 | Stock A / B | 109 / 109 | 53 / 52 | 15 / 13 |
| 1 | Logical key only | 75 | 50 | 17 |
| 1 | Fixed8 only | 84 | 51 | 13 |
| 1 | Both | 49 | 51 | 13 |
| 2 | Stock A / B | 112 / 112 | 53 / 50 | 13 / 14 |
| 2 | Logical key only | 79 | 51 | 13 |
| 2 | Fixed8 only | 88 | 53 | 16 |
| 2 | Both | 48 | 51 | 14 |

**The direction survives a conservative clock bound.** The combined arm saved at least 62 CPU-ms in rotation 1 and 178 CPU-ms in rotation 2 inside the actual submit-to-terminal interval against either stock duplicate, on those eight queue backends alone. The raw bracket differences were 600 and 640 CPU-ms. This comparison concerns aggregate queue processes. It cannot assign the savings to individual SQL statements or turn CPU time into elapsed time.

In the separate six warm-River rotations, Mesh's application backends accumulated 48–54 ticks and its queue backends 52–71. Exactly four of the 32 application backends accumulated any whole tick on each rotation. The other 28 could still have done sub-tick work. Testing synchronous worker supply and direct pool access measures whether the two runners can use more of the warm pool without raising total cost. River did not export comparable endpoint process counters in this series.

## Reproduce the bounds

Run `python3 -B tls_role_ticks.py | diff tls-role-ticks.json -` in this directory. The replay hashes the 21 measured Mesh archives across both series, joins the real PostgreSQL TLS login roster to both postmaster process frames by PID, and requires identical parent and process start ticks. The kernel counters sum user and system time. Against the factorial scorer, the stable process sum reproduced all 15 published PostgreSQL CPU totals.

Each process sample falls inside its frame. The lower bound subtracts two ticks from each positive per-process delta (floored at zero), then charges the entire bracket outside submit-to-terminal at both pinned CPUs. The upper bound adds two ticks per process, including zero-tick processes. This allows for separate user/system tick truncation at each endpoint. Each comparison subtracts the combined upper bound from the lower bound of each stock arm and takes the smaller saving. The bounds refer to kernel-accounted process time.

The two stock-peer blocks numbered 3 and 5 had early TLS censuses capturing only four and six queue sessions. Their 29 and 16 remaining ticks are reported as other stable processes. The comparison table and six warm-River rotations all had complete rosters. The raw archive hashes, per-backend tick distributions, missing PID checks, and both frame margins are preserved in `tls-role-ticks.json`.
