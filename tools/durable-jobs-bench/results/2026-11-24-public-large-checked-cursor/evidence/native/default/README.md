# Checked-cursor diagnostic with default storage

The Linode controller released both reverse-order groups with exit 0 at
2026-11-24 22:59:28 UTC. Offline replay reconciled all 16 warmup and
measured arm archives. Each measured group compared the early fallback
marker with two byte-identical stock controls and a separate trace arm.

The early marker shortened first commit to final grant by 72.914 and
41.388 ms against the faster stock control. It shortened first commit
to last settlement by 65.392 and 37.670 ms. Both clocks required at
least 129.281 ms in each group. Submit-to-settlement gains were 46.860
and 20.046 ms. The advancement gate stayed closed for default storage.

From the worktree root, replay the block digests, raw receipts, trace
chains, and gate with:

```bash
python3 -B tools/durable-jobs-bench/results/2026-11-24-public-large-checked-cursor/controller.py replay large
```

The recorded output is [summary.json](summary.json). The controller
ledger receipt is [controller-release.json](controller-release.json).
