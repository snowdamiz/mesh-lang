# How much idle worker CPU would Direct4 need to recover?

The private 2×32 pilot needs **23.158 CPU-ms forward and 63.776 CPU-ms reverse** to bring Direct4 inside the original runner CPU ceiling. Both rotations already improved submit and drain over the faster two-worker service control. Across the nominal three-second idle period, four extra workers would have to save an average of 1.930 and 5.315 CPU-ms per worker per second. The reverse block sets the larger requirement. The pilot request was `5cb27783` and both blocks used the same ELF `e836b79a` for the direct and service controls.

The replay uses the same pre-submit sample rule as the 8×8 CPU analysis on the **24 archives from the original Direct4 pilot**. Each reading below is the busy share across CPUs 0 and 1. The last two columns use the same-ELF Direct4 minus Direct2 difference, multiplied by two CPUs and three seconds. This extrapolates a host rate outside its observation window. The fraction is the share of that hypothetical difference the extra workers would have to recover to pay the lifetime runner shortfall. [Sampling rule](BEFORE-SUBMIT-CPU.md)

| Block and phase | Direct2 | Direct4 | Service2 A/B | Service4 | Projected host difference | Share required |
|---|---:|---:|---:|---:|---:|---:|
| Forward warmup | 4.87% | 5.73% | 4.25 / 3.83% | 4.26% | 51.4 CPU-ms | 30.3% |
| Forward measured | 3.07% | 4.23% | 4.89 / 4.53% | 5.38% | 69.9 CPU-ms | 33.1% |
| Reverse warmup | 3.82% | 6.79% | 4.52 / 4.91% | 3.88% | 178.5 CPU-ms | 28.5% |
| Reverse measured | 3.50% | 6.39% | 4.20 / 3.11% | 5.04% | 173.3 CPU-ms | 36.8% |

The direction repeated on the direct path. Service4 fell below both Service2 controls in reverse warmup, and the later spin experiment had same-image four-worker duplicate spreads of 2.1 and 1.8 percentage points. The host totals include other processes, PostgreSQL and interrupts. The recorded runner CPU covers the entire lifetime. These archives cannot assign a positive lower bound to the CPU an individual empty worker could save.

A qualification needs worker-thread CPU through verified empty intervals alongside the time from runnable publication to pickup and the first durable writes. A four-worker runner still has to service new work immediately when its extra threads are parked. An idle-only CPU improvement earns no credit on the durable clocks. The original forward incremental middle interval also missed by 2.285 ms, and the two rotations missed bridge calibration. Those conditions require fresh controls.

Run `python3 -B direct4_idle_budget.py` in this directory to rehash both phases of both blocks, read the raw pinned CPU counters and recover the unrounded inputs. The reducer uses the immutable evidence in Git commit `2d96798df87180edf0ea4744b4a16116b1d301f5`.
