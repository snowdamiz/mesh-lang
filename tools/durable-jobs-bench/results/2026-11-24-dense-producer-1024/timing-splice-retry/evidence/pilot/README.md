# Private producer-width pilot retry

The controller released the build-free retry with exit 1 at 01:56:54 UTC.
The Linux entry preflight passed as `nobody` for both producer widths.
All 12 warmup and measured invocations exported failure archives.
The workload stopped before timing when `warm_producer` raised
`TypeError: '_GeneratorContextManager' object is not an iterator`.
Neither rotation qualified. The peer gate stays closed.

The [release](controller-release.json), [replay](replay.json), and
[entry preflight](entry-preflight) belong to this retry. The
[successful SETUP](../../../evidence/setup) and
[original failed pilot](../../../evidence/pilot) have separate evidence.

```bash
d=tools/durable-jobs-bench/results/2026-11-24-dense-producer-1024/timing-splice-retry
python3 -B "$d/retry.py" replay pilot
```

Replay emits the retained result and exits 1 because both groups are missing.
