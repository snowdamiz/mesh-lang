# Empty recovery pilot staging failure

The eight-minute pilot released with exit 1. The first shakedown
stopped at import. Its temporary stage was shallower than the
repository layout assumed by `contract.py`, so evaluating
`HERE.parents[3]` raised `IndexError`. The shakedown never reached
the workload, the stock/candidate pair stayed pending, and groups
2–3 remained gated.

See the [release](2026-11-25-empty-recovery-native-groups-1-1.released.json),
[entry traceback](entry-failure.txt), and [pilot replay](pilot-summary.json).

```bash
python3 -B tools/durable-jobs-bench/results/2026-11-25-empty-recovery-native/controller.py replay
```
