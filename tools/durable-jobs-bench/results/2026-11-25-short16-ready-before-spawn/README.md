# Earlier handler entry on the released SHORT16 runner

The exact R2 D2 scheduler drains its current ready list before taking a fresh actor from the spawn queue. A resumed job can issue its synchronous second application write on that worker. The compiled fixture records handler entry before decoding the context and calling the application-pool service. The source capsule contains both landmarks in `source/compiler/mesh-rt/src/actor/scheduler.rs` and `source/fixture/main.mpl`. [Released source capsule](../2026-11-24-exit-confirmed-bulk-d2/combined-source.tar.gz)

The six native R2 D2 and six matched prewarmed River archives let us price two changes to entry time. The replay groups jobs by their recorded millisecond claim stamp. The first scenario lets every job enter as soon as the first job with the same stamp entered. The second lets every job enter at the SQL claim stamp, even before a validated COMMIT could return. A stamp bucket can include more than one actual transaction. Both scenarios retain each job’s recorded duration from entry to each write and terminal settlement. When earlier settlements make room for the next group, the simulation preserves the observed delay after both the preceding grant and sufficient durable releases. This allows savings to carry through successive groups at the original 16-seat cap. [Replay](price.py) · [Paired inputs and clocks](price.json)

| Rotation | Submit saved by simultaneous sibling entry, ms | Submit saved by entry at claim stamp, ms | Remaining middle-112 deficit to warm River with entry at claim stamp, ms |
|---|---:|---:|---:|
| 1 | 14.7 | 44.1–52.9 | 25.1–32.9 |
| 2 | 10.6 | 46.3–58.0 | 14.5–25.2 |
| 3 | 13.5–13.9 | 51.0–60.8 | 26.0–34.8 |
| 4 | 23.1 | 63.0–72.0 | 16.8–24.8 |
| 5 | 3.7–3.9 | 43.1–53.3 | 9.7–18.8 |
| 6 | 4.3 | 32.8–42.4 | −11.7 to −3.0 |

Ranges use the earliest and latest feasible placements of each millisecond stamp. Negative deficits favor Mesh. Both placements reconstruct the original three clocks exactly when entry is unchanged. Earlier sibling entry still loses five of six full submits. Even entry at the SQL stamp leaves Mesh behind on middle writes in five of six rotations. These are fixed-service scenarios. The archives lack worker queue depths, exact start-of-runnable times, and SQL socket wait intervals. A real scheduling change could alter subsequent service or cohort membership, or delay second writes. The evidence supplies no repeatable path through the SHORT16 peer comparison from reprioritizing starts alone.

Regenerate the receipt from the repository root using `python3 -B tools/durable-jobs-bench/results/2026-11-25-short16-ready-before-spawn/price.py`. The reader checks all 12 input hashes against the [released timing summary](../2026-11-24-short16-river-readiness/evidence/summary.json). The calculation uses archived native timestamps and performs no product execution.
