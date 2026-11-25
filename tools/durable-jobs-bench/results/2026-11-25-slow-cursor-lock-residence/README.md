# Time inside the SLOW batch cursor on two runners

In the two released pilot orders, at least **102–107 ms per Mesh arm** elapsed between the first and last row stamps of productive grants while PostgreSQL held the queue row lock. At least **63–71 ms** of those spans fell between the 64th and 448th first application commits. These bounds describe the frozen V1 cursor on 512 jobs with two 32-slot runners.

| Order | Codec | Contention hints | Cursor SELECTs | Minimum between stamps, ms | Minimum in middle, ms | Middle overlap with 8 or more unstamped seats on other runner, ms |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| Forward | PGLZ | 20 | 125 | 103.055 | 68.270 | 0–48.840 |
| Forward | LZ4 | 23 | 123 | 102.068 | 71.058 | 0.022–104.861 |
| Reverse | PGLZ | 19 | 127 | 107.068 | 63.060 | 0–26.162 |
| Reverse | LZ4 | 21 | 128 | 104.060 | 64.051 | 0.019–151.300 |

In the last column, a seat is outside the recorded interval from its SQL claim stamp to durable terminal commit. The other runner can still have a pending claim, an unfinished settlement receipt, or no request ready. The LZ4 lower bounds establish less than one millisecond of overlap with eight such seats in either order. These captures leave the return from changing the queue lock or its retry delay unresolved. The first stamp occurs after some grant work has already run, and the final stamp precedes the last UPDATE and COMMIT.

## Derivation

At source `ce5e551d`, `claim_batch_v1` locks the queue row before the cursor loop, then assigns a fresh millisecond clock value before each job UPDATE. Each runner issues at most one grant at a time. Each grant contains at most 16 jobs. A different runner's strictly interior timestamp rules out a single grant spanning the two stamps. Every job's first durable application write follows its grant's confirmed COMMIT.

The SQL wrapper can return a contention hint on an empty claim. Each measured arm has one top-level cursor statement whose row delta is exactly 512 plus the two runners' reported hints. All 512 jobs finished. Calls minus hints bounds the number of productive grants from above because other empty calls can return zero rows.

The [archive reader](residence.py) considers every per-runner partition consistent with these constraints. It combines the two runners under the shared call-count bound. For a multirow grant whose extreme recorded stamps are `a` and `b`, the interval from `a + 999 µs` to `b` is guaranteed when nonempty. The interval from `a` to `b + 999 µs` covers every possible span on PostgreSQL's microsecond clock. The upper endpoint is also bounded by the earliest first write in the group. A one-row grant contributes zero. For each time window the reader minimizes or maximizes the clipped spans independently. Grant membership and the individual waiters are absent from the archive.

The [pilot receipt](pilot.json) binds the four original raw hashes to the exit-zero controller release and score. The script accepts one or more released phase evidence directories from the [matched comparison](../2026-11-25-slow-lz4-unfused-2x32/amendment/README.md).
