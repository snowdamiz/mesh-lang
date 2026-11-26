# Host CPU budget at peer completion times

Each row compares the hot grouped-settlement Mesh and peer arms from six dense Linode pairs. The lower bound counts only busy CPU time proven inside the Mesh window. The peer budget is twice its elapsed time on the same two CPUs, granting it 100% utilization. A positive difference is the minimum observed CPU service that would have to disappear from that window for Mesh to match the peer. Scheduling changes can move work across window boundaries.

| Peer | Window | Pairs with positive excess | Lower-bound excess CPU-ms, min / median / max |
| --- | --- | ---: | ---: |
| Oban | submit | 0/6 | 0.0 / 0.0 / 0.0 |
| Oban | drain | 0/6 | 0.0 / 0.0 / 0.0 |
| Oban | middle | 0/6 | 0.0 / 0.0 / 0.0 |
| River | submit | 0/6 | 0.0 / 0.0 / 0.0 |
| River | drain | 0/6 | 0.0 / 0.0 / 0.0 |
| River | middle | 6/6 | 65.3 / 90.6 / 146.3 |
| Apalis | submit | 4/6 | 0.0 / 122.5 / 226.0 |
| Apalis | drain | 0/6 | 0.0 / 0.0 / 0.0 |
| Apalis | middle | 0/6 | 0.0 / 0.0 / 0.0 |

Submit and drain end at the last durable settlement. Middle runs from the 103rd through the 922nd durable second write. Each interval uses its own clock on the same pinned boot. The archived host estimator excludes boundary samples, allows 2 ms of clock uncertainty and one accounting tick per CPU and busy counter. Busy includes user, nice, system, IRQ and softirq service for everything on the pinned CPUs. Some of that service may be unrelated to Jobs. Zero excess leaves the CPU question open.

Run `python3 -B host_capacity.py` to verify the 24 archive hashes and regenerate the table. [Frozen host estimator](../2026-11-23-ordinary-native-cpu/replay/offline.py) · [Matched four-engine scores](NATIVE-RESULTS.md)
