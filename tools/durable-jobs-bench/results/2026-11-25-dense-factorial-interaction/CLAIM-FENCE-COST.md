# Cost of checking the namespace pause during dense claims

Moving the per-job pause lookup out of the row trigger has little measured headroom. Four native diagnostic arms executed that lookup exactly 1,024 times each. The sum of executor time for those reads was 6.45–13.46 ms. Even treating the entire sum as serial elapsed savings falls well below the 191–947 ms middle-write losses to River on the later 8×8 build. The comparison sets the scale of the opportunity across studies. The archives contain no matched trigger change on the later build.

| Diagnostic runner | Rotation | All 1,024 trigger calls, elapsed ms | Nested pause lookup, executor ms | Separate batch pause lookup, executor ms |
|---|---:|---:|---:|---:|
| Equal | 1 | 56.594 | 9.474 | 0.761 |
| Equal | 2 | 47.426 | 6.452 | 0.791 |
| Stock | 1 | 76.652 | 13.457 | 3.016 |
| Stock | 2 | 64.561 | 13.398 | 3.096 |

The trigger total includes the nested read, the isolation check, and an attempt to acquire a shared namespace lock. Giving a candidate the whole trigger total would assume it could remove all of those operations. The released schema also supports legacy claimants that acquire the queue row before testing the namespace lock. They need the transition fence to keep a paused namespace from admitting work. Session settings alone cannot establish that a particular invocation already holds the necessary lock.

Each trace enabled nested statement tracking, PL function tracking and statement logging. Relative to the midpoint of the two ordinary drain clocks in each rotation, tracing moved the equal clocks by +53.9 and −27.9 ms, and the stock clocks by +103.5 and +627.1 ms. These executor sums include observer costs and cannot measure native CPU savings. Keep the existing admission fence.

[Four trace results and eight control hashes](claim-fence-cost.json) · [Archive reducer](claim_fence_cost.py) · [First-write and peer deadlines](GRANTS-AT-PEER-DEADLINES.md)

## Counting executing jobs while holding the queue lock

The four public PG17 plan captures place little time in the capacity count. In DENSE, calls whose logged intervals could touch the middle 512 effects summed to 6.209 and 10.842 ms across all claimants. The corresponding effect windows lasted 1,169.255 and 973.438 ms. Maintaining a transactionally consistent occupancy counter would add work to transitions into and out of execution. The measured scan costs do not support such a change for these workloads.

| Capture | Count calls | Whole-arm executor (ms) | Largest count | Calls touching the middle half | Sum of logged durations for those calls (ms) |
|---|---:|---:|---:|---:|---:|
| DENSE stock | 392 | 14.888 | 30 | 132 | 6.209 |
| DENSE logical-key grant | 388 | 21.284 | 48 | 131 | 10.842 |
| SLOW stock | 69 | 1.725 | 63 | 26 | 0.840 |
| SLOW logical-key grant | 59 | 1.892 | 63 | 19 | 0.788 |

All 908 count plans scanned the existing partial index `mesh_jobs_expiring`. Across the four arms the queue filter rejected zero visible rows. A queue-qualified index condition therefore has no observed wrong-queue rows to eliminate on these single-queue fixtures. Heap visibility checks still cost work. The largest observed count was below 64 in every arm, so stopping the scan at 64 rows would have saved zero counted rows.

The replay matches each plan census to the role's closed statement counter and joins millisecond plan-log timestamps to the two central durable effect ranks. It credits the whole logged duration to any call whose possible interval overlaps the window. Concurrent calls can overlap, and logging changes execution cost. The figures price the scan on these instrumented public images. Direct4's distinct two-owner path and warm River's clocks require their own measurements. Run `python3 -B grant_occupancy_cost.py | cmp - grant-occupancy-cost.json` from this directory. [Archive digests and counters](grant-occupancy-cost.json) · [Replay](grant_occupancy_cost.py)
