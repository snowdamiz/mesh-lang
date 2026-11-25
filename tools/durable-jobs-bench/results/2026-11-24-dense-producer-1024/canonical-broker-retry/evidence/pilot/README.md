# Canonical-broker producer-width pilot

The released correctness smoke qualified both widths under the original
scorer. The controller opened a separate timing grant and released it
with exit 0 at 02:56:53 UTC. All 12 arms reconciled across the two reversed
groups.
[Correctness release](../smoke/controller-release.json) ·
[Pilot release](controller-release.json) · [Raw replay](replay.json)

The 1024-wide lane saved 12.264 ms and 9.519 ms through enqueue against
the faster 128-wide control in groups 1 and 2. Group 1 lost 32.801 ms
through drain against a 24.139 ms cap. Group 2 missed the 10 ms enqueue
floor. Neither group qualified and peer timing remains closed.

```bash
d=tools/durable-jobs-bench/results/2026-11-24-dense-producer-1024/canonical-broker-retry
python3 -B "$d/controller.py" replay pilot
```
