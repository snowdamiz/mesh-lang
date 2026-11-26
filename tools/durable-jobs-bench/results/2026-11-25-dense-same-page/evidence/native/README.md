# Dense fillfactor native pilot observation failure

The pinned Linux PG17.11 `pg_waldump` passed the reduced-UID
preflight. The first default warmup reached the post-run WAL
observer, where the decoder invocation failed. The adapter never
emitted a completed WAL reduction. The fail-fast pilot released with
exit 1 and neither rotation reconciled. Peer pricing stayed closed.

See the [release](controller-release.json), [raw failure](failure.json),
[preflight](pg-waldump-preflight.txt), and [replay](summary.json).
The raw archive includes the before and after WAL frames and the
workload receipts available at the failure.

```bash
python3 -B tools/durable-jobs-bench/results/2026-11-25-dense-same-page/controller.py replay
```
