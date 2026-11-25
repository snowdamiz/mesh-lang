# Qualified logger-off actor-readiness fixture

The native SETUP released with exit 0 at 01:17:33 UTC on
November 25. All five untimed correctness cases replayed from the
raw export. The same ELF completed the cold readiness bridge with
1,024 jobs, 2,048 application writes and 2,048 checkout probes.
The bridge verified TLS waits in both runners and the canonical
logger-off output. The manifest binds the same ELF for the pilot.

See the [controller release](controller-release.json) and
[complete replay](summary.json).

```bash
python3 -B tools/durable-jobs-bench/results/2026-11-24-dense-hot-readiness/controller.py replay setup
```
