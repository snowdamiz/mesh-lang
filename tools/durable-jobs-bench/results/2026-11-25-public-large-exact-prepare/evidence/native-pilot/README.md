# Public LARGE exact-prepare pilot

Both reversed groups reconciled all 16 warmup and measured arms.
The controller released the pilot with exit 0 at 02:28:24 UTC.
Last confirmed grant and drain cleared their margins in both groups.
The prepared lane saved 34.113 ms and 45.173 ms through settlement
against the faster same-image stock controls. Those savings missed
the respective 63.318 ms and 150.833 ms submit thresholds.

CPU, RSS, and temporary bytes passed both groups. The sealed gate
requires every clock to pass. Peer timing remains closed.
[SETUP proof](../native-setup) · [Pilot release](controller-release.json) · [Raw replay](summary.json)

```bash
python3 -B tools/durable-jobs-bench/results/2026-11-25-public-large-exact-prepare/controller.py replay pilot
```
