# Empty-recovery group 1 retry

The shakedown, stock, and candidate arms all completed and exported raw
archives. The grader rejected the block with `KeyError: 'hot'` before it
produced a qualifying comparison. The controller released group 1 with
exit 1 at 02:03:41 UTC. Groups 2 and 3 remain closed.

The [release](controller-release.json), [replay](summary.json), and
[controller log](controller.log) belong to this retry. The
[original shallow-stage failure](../../../2026-11-25-empty-recovery-native/evidence/native/entry-failure.txt)
is archived separately.

```bash
python3 -B tools/durable-jobs-bench/results/2026-11-25-empty-recovery-native-retry/controller.py replay
```
