# Same-page WAL retention pilot

The controller released both reversed groups with exit 0 at 02:00:18 UTC.
All 12 arms reconciled. Fillfactor 50 recorded 2,032 and 2,003 same-page
prefix-reusing updates. The two defaults recorded 454 and 459 in group 1
and 399 and 595 in group 2.

Against the faster default, fillfactor 50 saved 28.305 ms through
settlement and 41.867 ms through drain in group 1. Neither saving cleared
the duplicate spread. In group 2, it took 66.032 ms longer through
settlement and 33.592 ms longer through drain. Both groups passed the
resource and footprint bounds. Peer pricing remains closed.
[Release](controller-release.json) · [Replay](summary.json)

The [original decoder failure](../../../evidence/native/failure.json) is
archived separately. The [decoder preflight](pg-waldump-preflight.txt)
records the pinned PostgreSQL 17.11 build.

```bash
d=tools/durable-jobs-bench/results/2026-11-25-dense-same-page/wal-retention-retry
python3 -B "$d/controller.py" replay
```
