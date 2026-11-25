# Released R2 hot completion-hold screen

Two reverse-order Linode blocks released with exit 0 at 00:43:46 UTC
on November 25. The offline replay reconciled all 28 raw archives.
The original builder-overlap failure remains in the
[separate interruption archive](../../evidence/native).

The 1 ms hold reduced terminal transactions to 591 and 557.
The zero-hold control used 601 and 621. In the first block, the
hold shortened drain by 3.890 ms against the fastest control,
below the 57.607 ms threshold. In the second, drain lengthened
by 101.966 ms. The hold failed the submit and middle-write
thresholds in both blocks. The advancement gate stayed closed.

See the [release](controller-release.json) and [replay](summary.json).
Recompute from the repository root.

```bash
python3 -B tools/durable-jobs-bench/results/2026-11-24-dense-hot-completion-hold/load-retry/controller.py replay
```
