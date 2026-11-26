# Released symmetric PostgreSQL 100 µs group-commit screen

The Linode timing grant released with exit 0 at 00:54:43 UTC on
November 25. Both reversed settings rotations reconciled, with
48 raw warmup and measured arms. Each of the four engines received
the same PostgreSQL treatment.

Mesh recorded 1,151 WAL syncs with the delay in the first rotation
and 1,233 in the second. Both counts fell below either duplicate
default in their rotation. The delay lengthened submit-to-settlement
by 69.522 and 57.333 ms and commit-to-settlement by 48.748 and
36.214 ms against the faster default. Middle-write timings also
missed the margin in both rotations. The follow-up gate stayed closed.

See the [controller release](controller-release.json) and
[replay](release-replay.json). Recompute the six sub-blocks from the
repository root.

```bash
python3 -B tools/durable-jobs-bench/results/2026-11-24-dense-group-commit/controller.py replay
```
