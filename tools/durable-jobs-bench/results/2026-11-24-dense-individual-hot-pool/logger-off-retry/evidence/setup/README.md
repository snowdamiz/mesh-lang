# Qualified logger-off public individual-terminal relink

The Linode SETUP released with exit 0 on November 25. It rebuilt
the exact 1e70f18d runtime archive and qualified both relink manifests.
The canonical fixture disables the per-job logger. The raw proof
retains all eight original semantic cases and both dense checkout
diagnostics on the new executable with independent logger-off checks.
The earlier Jobs proof and failed adapter SETUP remain archived.

See the [controller release](controller-release.json) and
[qualification replay](summary.json). From the repository root,
replay the raw package and compare the recorded qualification.

```bash
d=tools/durable-jobs-bench/results/2026-11-24-dense-individual-hot-pool/logger-off-retry
PYTHONPATH="$d" python3 -B - <<'PY'
import json
from pathlib import Path
import retry
retry.configure()
actual = retry.base.setup_raw(retry.relink_binding())
assert actual == json.loads((Path(retry.OUT) / "setup/verified.json").read_text())
PY
```
