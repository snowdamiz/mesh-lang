# CPU use before the DENSE producer starts

Across five matched native 8×8 rotations, the two pinned CPUs spent 11.3–13.4% of their reported ticks doing work shortly before Mesh B submitted its first job. The corresponding ranges were 1.2–2.7% for River and 5.6–8.9% for Oban. The two same-image Mesh controls and the earlier public runner show a similar level to B.

| Rotation | Mesh B | OFF A1 | OFF A2 | Earlier Mesh | River | Oban |
|---|---:|---:|---:|---:|---:|---:|
| 1 | 13.4% | 10.9% | 11.2% | 13.2% | 1.9% | 6.7% |
| 2 | 11.7% | 11.6% | 11.7% | 12.0% | 2.3% | 6.7% |
| 3 | 11.3% | 12.4% | 12.9% | 11.5% | 1.2% | 5.6% |
| 4 | 13.3% | 11.5% | 13.0% | 12.4% | 2.7% | 6.3% |
| 5 | 13.0% | 12.5% | 12.2% | 12.7% | 1.2% | 8.9% |

The reducer reads each arm's submit clock from the producer ledger and uses the host samples from 2 seconds to 250 ms before submit. The previous frame's timestamp bounds the earliest physical CPU read. The current frame bounds the latest. All physical counter deltas fall inside this pre-submit interval. Busy ticks include user, nice, system and interrupt time. The denominator also includes idle, I/O wait and steal. The separate idle observations in each archive verify a roughly three-second idle check with no executing or terminal jobs.

These counters include PostgreSQL, observers and other host processes. During its full idle check Mesh B reported 536–544 SQL calls, and Oban reported 380–384. The SQL counts cover a longer interval than the CPU measurements. Neither count partitions the CPU. In particular the exact Mesh scheduler on `456749c4c` uses a 1.1 ms timed park after exhausting its spin budget. Its 16 ms cold park requires a single worker, whereas these eight runners each use two workers. The record has no per-worker count of expired parks. The earlier Direct4 intervention changed the preceding spin budget and left the 1.1 ms park in place. A change to that park needs to distinguish scheduler work while processing jobs from the startup and idle costs included in total runner CPU.

The [30 raw-derived observations](before-submit-cpu.json) retain the two-CPU counts, read bounds, process thread counts and archive hashes. Reproduce them from the released archives with `python3 -B before_submit_cpu.py`. The [original peer score](../2026-11-25-public-dense-late-capacity/peer-comparison/RESULTS.md) covers the durable clocks.

## Two-runner controls

The same pre-submit calculation on the earlier 2×32 Direct4 pilot finds 3.1–6.5% busy across 16 measured Mesh arms. Changing each runner from two to four scheduler workers on the direct path raised the two-host-CPU busy share from 4.9% to 5.4% forward and from 4.6% to 5.8% reverse. The two nominally identical four-worker controls themselves differed by 2.1 and 1.8 percentage points. On the rebuilt four-worker image, cutting the spin budget to 16 gave 6.4% and 5.0%, compared with 4.3–6.3% and 4.7–6.5% for its 100-spin duplicates.

Those arms recorded 132–136 SQL calls in the full idle check. The independent 8×8 series changed runner count, login roster and claim traffic along with scheduler supply. The observed CPU difference across deployments cannot be assigned to the worker timer. The candidate needs to count how often an empty worker actually reaches its timed park during enqueue and drain, and measure whether fewer expirations advance durable work. [Two-runner capture hashes and counters](before-submit-two-runner.json)

Run `python3 -B before_submit_cpu.py --two-runner` with Git object `cb4f825c1` available to reproduce the comparison.
