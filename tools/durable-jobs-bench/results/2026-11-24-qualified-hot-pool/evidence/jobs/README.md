# Linux AMD64 Jobs proof for 1e70f18d

The fenced Linode SETUP released with exit 0 at 23:22:33 UTC on
2026-11-24. All eight cases replayed from one raw export. They cover
protected short and mixed-size grants, uncertain claim and application
write receipts, released and killed teardown, and hot-checkout grants
and uncertain application writes. The short checked control recorded
four application health probes. The short hot case recorded zero.

The product and teardown witness used the same runtime archive and
distinct linked ELFs. The retained product ELF is
`/var/lib/durable-jobs-bench/native/source-trusted-1e70f18d-jobs-78468180f0c0/artifacts/jobs/mesh`.
Its identity and the raw archive digest are in the [summary](summary.json).
The [release](controller-release.json) records the native slot.

From the repository root, replay the sealed export.

```bash
python3 -B tools/durable-jobs-bench/results/2026-11-24-qualified-hot-pool/controller.py jobs replay
```
