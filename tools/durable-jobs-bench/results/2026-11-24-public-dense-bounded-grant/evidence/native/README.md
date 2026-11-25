# Bounded-grant group 0 stopped at the native plan gate

The amended controller released the original grant with exit 1 at
00:44:38 UTC on November 25. The released receipt serializes the
complete requested group list. The sole executed qualification arm
exported the real PG17.11 grant plans and correctness observations.

The candidate missed the bound, UPDATE-side visits, and shared-buffer
checks. For example, the empty auto-plan candidate visited 1,024
UPDATE-side rows while stock visited zero. The group 0 failure stopped
the six workload groups. The peer gate stayed closed.

See the [release](2026-11-24-public-dense-bounded-grant-groups-0-6.released.json),
[plan gate](plan-gate.json), and [replay](primary-summary.json). The
original `adapter/grant-plans.json` is in the raw arm archive.

```bash
python3 -B tools/durable-jobs-bench/results/2026-11-24-public-dense-bounded-grant/controller.py replay
```
