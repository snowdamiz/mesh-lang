# Slow-job refill on two 32-slot runners

The eight Linode blocks each ran 512 jobs with a 100 ms hold and two independently committed application writes per job. Each rank uses a different job’s first committed write in each engine’s execution order. Times start at the engine’s own enqueue commit. Positive differences mean Mesh reached that rank later.

## Mesh minus Apalis at each first-write rank (ms)

| Block | 1 | 64 | 128 | 192 | 256 | 320 | 384 | 448 | 512 |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 1 | -5.6 | +16.1 | +56.0 | +94.8 | +124.4 | +148.6 | +178.7 | +255.0 | +281.8 |
| 2 | -39.8 | -18.6 | +4.7 | +68.9 | +127.6 | +145.1 | +163.3 | +190.5 | +219.5 |
| 3 | -39.8 | -1.8 | +49.4 | +130.5 | +115.7 | +120.9 | +125.3 | +118.2 | +92.7 |
| 4 | -6.0 | -10.8 | +50.8 | +88.1 | +125.8 | +138.9 | +173.6 | +181.8 | +183.1 |
| 5 | -7.5 | -5.6 | -3.9 | +42.7 | +88.4 | +119.1 | +138.9 | +162.3 | +166.1 |
| 6 | -38.6 | -33.3 | -8.7 | +21.2 | +41.4 | +71.8 | +73.4 | +90.2 | +116.9 |
| 7 | -34.8 | -20.2 | -8.7 | +36.3 | +55.5 | +76.3 | +99.1 | +97.6 | +96.2 |
| 8 | -33.8 | -22.2 | +9.6 | +49.3 | +77.4 | +86.1 | +90.0 | +103.7 | +109.8 |

## Slot occupancy during refill

The window begins with the 64th first write and ends with the 512th. Each job contributes only its time inside that window. Summed durations are divided by 64, so the four occupancy columns add up to the window width in each arm. The minimum hold column counts exactly 100 ms after each first write, clipped at either window edge. The last column counts slot time outside the recorded interval from handler entry through the second write. It includes work before entry and after the second write, as well as time without a reservation.

| Paired median, Mesh minus peer (ms) | Window width | Entry to first write | Minimum hold | End of minimum hold to second write | Outside entry to second write | Mesh had longer window |
|---|---:|---:|---:|---:|---:|---:|
| Apalis | +161.0 | +5.6 | +39.3 | -10.2 | +138.1 | 8/8 |
| River | +186.8 | +18.7 | +25.2 | +5.5 | +129.1 | 8/8 |
| Oban | +16.5 | -0.1 | +12.4 | -27.1 | +33.6 | 4/8 |

Paired medians can come from different blocks. Engines can have different numbers of holds crossing the window edges. The occupancy differences measure wall time and cannot isolate SQL, dispatcher CPU, or the time a runner took to release a slot.

[Matched results and raw ledgers](../2026-11-24-ordinary-slow-equal-topology-cap71/NATIVE-RESULTS.md) · [Replay](slow-equal-refill.py)
