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
