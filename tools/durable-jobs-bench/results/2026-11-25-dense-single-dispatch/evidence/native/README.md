# Dense single-dispatch pilot

The controller released the two reversed groups with exit 1 at 02:53:24
UTC. All 16 warmup and measured arms exported raw evidence.

All eight native invocations in the first group exited 0. The scorer
rejected the observed claim body hash `017ad733…` against sealed
`f2d47207…`. The observed lock configuration matched the request.
The second group also contains startup failures for the warmup and
measured `single-two` arms. The adapter rejected their warmed application
TLS roster before timing.

The sealed replay selected neither group. Peer timing remains closed.
[Release](controller-release.json) · [Replay](summary.json)

```bash
python3 -B tools/durable-jobs-bench/results/2026-11-25-dense-single-dispatch/driver.py replay
```
