# Fixed-time settlement on the grouped dense runner

The fixed 8 ms collection policy completed 1,024 jobs in 183 terminal transactions in each of the two Linode rotations. The lower duplicate-stock counts were 869 and 858. Every arm used two independent application transactions per job and waited for actor exit. The table compares the fixed policy with the faster stock duplicate on each clock.

| Rotation | Submit saving | Submit stock spread | Drain saving | Drain stock spread | Last application effect earlier |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1 | 75.796 ms | 69.935 ms | 118.190 ms | 40.521 ms | 120.256 ms |
| 2 | 99.295 ms | 92.454 ms | 122.298 ms | 45.951 ms | 123.151 ms |

Fixed-time collection was a secondary comparison. The preselected policy also flushed when the dispatcher requested a refill. That policy won the first block and lost the second. In the second block its middle 820 application effects took 1,169 ms, versus 810 ms with fixed-time collection. During the refill arm’s drain the host device reported 614–740 ms busy, versus 231–261 ms in the fixed-time arm and at most 269 ms for either duplicate. The bounds come from the first and last device samples strictly inside the window and their adjacent outside samples on the PostgreSQL ext4 device. Device counters cover every process using that device. These observations leave the cause of the slow arm unresolved. Both original measured blocks count.

The fixed policy and the [bounded grant](../2026-11-25-dense-logical-grant-key-native/NATIVE-RESULTS.md) change different parts of the same queue. Their individual medians cannot establish how the combination behaves. A fresh matched comparison can isolate each effect and the interaction.

Run `python3 -B replay.py | cmp - review.json` from this directory. Replay verifies all 20 warmup and measured archive hashes against the [released pilot](../2026-11-25-dense-grouped-refill/evidence/native-pilot/README.md), checks the 2,048 distinct application XIDs per arm and reads the original clocks and host-device counters.
