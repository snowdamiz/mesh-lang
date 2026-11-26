# LARGE prepared claims at the PostgreSQL memory boundary

All eight prepared captures at 4 MiB show the same relationship. The number of runners whose peak queued context bytes crossed 4 MiB equals the number of temporary files recorded by the database. A full runner peaked at 4,196,960 context bytes, 2,656 bytes above 4 MiB even before PostgreSQL tuple overhead. Across warmup and measurement the count ranged from four to eight. The two measurements with the ordinary FILE producer generated eight files apiece. Those with producer lookahead generated eight and six. All had eight runners, 64 completed jobs and 192 distinct attempt, effect and settlement XIDs.

| Measured group | Prepared producer configuration | Database temp files | Database temp bytes | Effect 8 after enqueue | Effect 64 after enqueue | Last settlement after enqueue |
|---|---|---:|---:|---:|---:|---:|
| Forward | Ordinary FILE | 8 | 33,583,726 | 338.258 ms | 527.481 ms | 534.929 ms |
| Forward | FILE with lookahead | 8 | 33,583,726 | 294.087 ms | 531.631 ms | 542.803 ms |
| Reverse | Ordinary FILE | 8 | 33,583,726 | 258.346 ms | 549.044 ms | 562.844 ms |
| Reverse | FILE with lookahead | 6 | 25,187,793 | 170.897 ms | 523.107 ms | 532.397 ms |

These are independent controls on the same public prepared runner. The switch between producers does not isolate spill cost. PostgreSQL recorded temp use database-wide. The public captures lack temp-block counts for individual statements. The reverse group also failed its original stock comparison. The existing peer deadlines price the combined arm at 47–57 ms of drain savings to tie the fastest historical Oban block, and 128–138 ms for a 20% throughput lead in that block. [Peer deadlines](README.md)

## What the older 16 MiB intervention measured

In the earlier ordinary V1 cursor series, 16 MiB removed claim temp I/O in every measured rotation. Each row compares it to the better of the two 4 MiB stock controls for that metric. Positive values are gains at 16 MiB. Pinned CPU includes PostgreSQL and the runners.

| Group | Submit gain (ms) | Drain gain (ms) | Pinned CPU saved (ms) | 4 MiB drain spread (ms) |
|---|---:|---:|---:|---:|
| 1 | −17.173 | 46.196 | 40 | 38.055 |
| 2 | −50.582 | −14.839 | −240 | 39.848 |
| 3 | 37.503 | 22.016 | 200 | 51.245 |
| 4 | 7.979 | −5.653 | 20 | 23.735 |
| 5 | 86.024 | 80.186 | 260 | 104.830 |

Median gains were 7.979 ms on submit and 22.016 ms on drain. Only group 1 improved drain beyond its own duplicate spread. With symmetric 16 MiB settings, that historical Mesh build beat River on drain in all five rotations and Oban in one. A useful exact-image intervention must establish how much of the public prepared statement's temporary work disappears and when the independent application effects move. Reuse the public prepared producer separately from lookahead, retain duplicate 4 MiB controls, and account for PostgreSQL memory and CPU. A comparison to peers requires the same memory setting for them.

The [reducer](prepared_spill.py) hashes all 24 public raw archives against the scored record at Git revision `39f7fae80` and all 25 historical measured Mesh, River and Oban raw archives against the pinned V1 summary. The [receipt](prepared-spill.json) records the measured and warmup public landmarks, the actual runner peak byte counts, and the V1 claim temp-block and memory results. Replay with `python3 -B prepared_spill.py | cmp - prepared-spill.json` in this directory.
