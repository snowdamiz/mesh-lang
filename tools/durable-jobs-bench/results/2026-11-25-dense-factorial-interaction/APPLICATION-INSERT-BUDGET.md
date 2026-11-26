# Dense application INSERTs and the warm River deadline

In all six matched 2×32 blocks, Mesh would still have more work than fits in warm River’s middle-write interval even if it removed every microsecond PostgreSQL recorded executing its two application INSERTs. The excess after that credit ranges from 14 to 237 CPU-ms. This bounds changes confined to those executors. A treatment that also brings work forward, changes scheduling, or reduces other CPU can affect the result differently.

| Block | Two-core work to remove or move (CPU-ms) | Whole-run INSERT executor elapsed (ms) | Excess after credit (CPU-ms) |
|---|---:|---:|---:|
| 1 | 115.2 | 78.2 | 37.0 |
| 2 | 142.5 | 75.3 | 67.2 |
| 3 | 114.6 | 80.2 | 34.4 |
| 4 | 153.7 | 79.5 | 74.2 |
| 5 | 314.6 | 77.9 | 236.8 |
| 6 | 81.5 | 67.5 | 14.0 |

The middle interval runs from the 256th to the 768th durable second write. The replay bounds busy CPU on the two pinned cores using the existing bracketed host counter reader. It subtracts two CPUs times River’s elapsed interval. Its credit treats all elapsed time in Mesh’s 1,024 first INSERTs and 1,024 second INSERTs as removable CPU inside that interval. These ordinary, single-backend statements can also wait, and the elapsed sums cover the entire run. SQL planning, client work, and transaction completion outside the executor have separate costs. The source SQL for Mesh includes an attempt identity and a conflict check. This calculation even credits the ordinary insert work common to both engines.

In each Mesh arm, 32 application backends appeared in the TLS census and four accumulated whole CPU ticks. Their sums were 480–540 CPU-ms across the process brackets. The counters cover more than the middle interval and do not locate the remaining CPU in a specific PostgreSQL function. The River deadline comes from its own ledger and requires no assumption about how its backend CPU was distributed.

The receipt binds all 12 measured archives to the released score. From this directory run `python3 -B application_insert_budget.py | cmp - application-insert-budget.json`. The [original workload and releases](../2026-11-25-dense-fixed8-river-readiness/NATIVE-RESULTS.md) describe the session supply and startup costs.
