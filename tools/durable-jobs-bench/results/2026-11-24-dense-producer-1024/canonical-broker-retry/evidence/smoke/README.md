# Canonical-broker correctness smoke

The controller released the build-free correctness smoke with exit 0
at 02:54:10 UTC. Both cases reconciled 1,024 jobs and 1,024 durable
producer receipts, with the exact broker and two pinned runners.
The 128-wide control used eight producer calls. The 1024-wide case
used one. Each returned all 1,024 rows across its calls.

The original scorer verified ordered producer, application, and terminal
transactions and the executable identities for both arms. The two raw
archives and the [replay](replay.json) substantiate the separate pilot's
correctness prerequisite.
[SETUP release](controller-release.json)

```bash
d=tools/durable-jobs-bench/results/2026-11-24-dense-producer-1024/canonical-broker-retry
python3 -B "$d/controller.py" replay smoke
```
