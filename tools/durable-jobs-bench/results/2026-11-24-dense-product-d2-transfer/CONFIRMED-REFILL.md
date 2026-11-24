# When work was already granted but had yet to enter the handler

During the middle of the dense drain, the two hot Mesh runners had at least one confirmed job waiting to enter its handler for at least **41.3–48.6% of combined runner time** across six rotations. At least two confirmed jobs were waiting for **37.9–44.9%**. The denominator is twice the elapsed time between the 103rd and 922nd second application commits. Time on the two runners is counted separately.

| Hot rotation | At least one waiting, lower–upper % | At least two waiting, lower–upper % | Groups with 16 jobs at one stamp | Runner/claim-time groups |
|---|---:|---:|---:|---:|
| 1 | 48.6–95.9 | 44.9–95.3 | 34 | 83 |
| 2 | 46.6–95.3 | 42.4–94.4 | 37 | 79 |
| 3 | 41.3–96.0 | 37.9–95.2 | 35 | 85 |
| 4 | 47.6–97.6 | 44.8–96.9 | 29 | 85 |
| 5 | 43.3–95.0 | 40.4–94.6 | 36 | 81 |
| 6 | 46.2–95.5 | 41.0–94.9 | 38 | 80 |

The broad upper bounds count time from the millisecond claim stamp. A lower bound needs evidence that the client received the COMMIT. The public runner handles one claim request at a time and consumes its reply before requesting the next. A later grant on the same PID therefore witnesses the previous grant's receipt. The replay waits until the end of the later stamp’s millisecond before taking credit. All stamps and handler entries use the same Linode realtime clock.

The first handler entry also witnesses receipt for every job in that grant. Two successful claims on one runner could share a millisecond stamp. Each such split requires an extra SQL call beyond the number of distinct runner/stamp groups. The replay charges the maximum possible number of splits against the groups where first-entry evidence would matter most, then takes the stronger of this conservative bound and the later-grant witness alone. The two PID sets match the captured runner processes. All 18 measured Mesh archives match the frozen digests and reconcile 2,048 distinct application transactions per arm.

Even under these assumptions, part of the late-entry interval could respond to dispatch, actor scheduling, or competition for CPU with SQL. The bound does not estimate a removable duration on the critical path. Doubling synchronous scheduler workers in a separate native pilot brought the final effect forward twice and increased runner CPU in both runs. The preceding cooperative PostgreSQL pilot performed 2,048 checkout probes per arm. The production hot pool changes that cost, giving a distinct native test of how quickly already-confirmed handlers can make progress under the original transaction contract.

Run `python3 -B refill_evidence.py` to reconstruct [all bounds](evidence/refill-bounds.json). [Native four-worker pilot](../2026-11-24-dense-hot-worker-supply/NATIVE-RESULTS.md) · [Native checked-pool concurrency pilot](../2026-11-24-dense-postgres-permits/README.md)
