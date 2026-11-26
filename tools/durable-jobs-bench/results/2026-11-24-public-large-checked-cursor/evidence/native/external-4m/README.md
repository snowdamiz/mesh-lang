# Checked-cursor diagnostic with EXTERNAL storage

The Linode controller released both reverse-order groups with exit 0 at
2026-11-24 23:02:44 UTC. Offline replay reconciled all 16 warmup and
measured arm archives under EXTERNAL storage and the sealed 4 MB setting.

Against the faster stock control, the early fallback marker shortened
first commit to final grant by 169.429 and 54.425 ms. The clock required
129.250 ms in both groups. It shortened first commit to last settlement
by 150.428 and 35.696 ms against a floor of 178.636 ms. Submit-to-settlement
gains were 105.542 and 15.522 ms. The advancement gate stayed closed.

Replay the raw evidence and gate from the worktree root.

```bash
python3 -B tools/durable-jobs-bench/results/2026-11-24-public-large-checked-cursor/controller.py replay large-external
```

See the [replay summary](summary.json) and [release receipt](controller-release.json).
