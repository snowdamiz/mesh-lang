# Released logger-off individual-settlement rotations

Both timing windows released with exit 0, and all six rotations
replayed from 72 warmup and measured arm archives. Mesh hot and both
checked duplicates used the logger-off relink with SHA-256
`c9f029a001e2f8d058a00c0af8a376bc283304bff7ed9156baf299455c622cb9`.
The [SETUP proof](../setup) binds that ELF to the exact runtime archive
and the canonical logger-off fixture.

Rotations 1, 3, 4, and 6 met the preregistered gate. Hot beat each
checked duplicate in all six rotations on submit and drain and used
four checkout probes per arm compared with 2,048 for each control.
The [result report](../../NATIVE-RESULTS.md) gives peer contrasts and
the [final replay](../final-replay.json) binds each raw archive.

Reproduce the six blocks from the repository root.

```bash
python3 -B tools/durable-jobs-bench/results/2026-11-24-dense-individual-hot-pool/logger-off-retry/retry.py replay
```
