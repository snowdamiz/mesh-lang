# Hot R2 completion-hold interruption

The first Linode timing grant released with exit 1 at 23:17:48 UTC.
Six warmup arms returned 0 and were exported. The Oban warmup was
rejected because two host samples included the same `apt-get`
process. The build exclusion stopped the screen before any measured
arm, so this attempt produced no comparison group or gate verdict.

The [controller release](controller-release.json), [raw block](blocks),
and [offline replay](summary.json) preserve the interruption. Replay
from the repository root exits 1 with both groups missing.

```bash
python3 -B tools/durable-jobs-bench/results/2026-11-24-dense-hot-completion-hold/controller.py replay
```
