# Dense WAL write method pilot

The Linode pilot released successfully at 01:45:30 UTC on November 25, 2026. All six warmups and six measurements reconciled. The `open_datasync` drain took 16.639 and 16.644 ms longer than the faster stock control in the two rotations. It missed all six advancement gates. Four-engine pricing stayed closed. [Controller release](evidence/native/pilot/controller-release.json) · [Controller reduction](evidence/native/pilot/final-replay.json)

## Measured arms

Each measured arm used the frozen hot R2 Mesh runner, two 32-slot runners, and 1,024 ordinary jobs. Both phases in every sub-block completed all 1,024 jobs with 2,048 distinct application write XIDs. The readbacks confirm PostgreSQL 17.11, the selected method, synchronous commits, fsync, full-page writes, commit timestamps, TLS, CPUs 0 and 1, and ext4 on `/dev/sda` at `8:0`. [Independent raw reduction](evidence/native/pilot/independent-replay.json)

| Rotation | Arm in execution order | Submit to settlement (ms) | Drain (ms) | Middle writes (ms) | PG WAL writes | PG explicit WAL syncs | PG write + sync (ms) |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | Stock A | 1,328.635 | 1,164.759 | 891.245 | 1,670 | 1,669 | 842.581 |
| 1 | `open_datasync` | 1,341.700 | 1,181.398 | 903.718 | 1,612 | 0 | 856.293 |
| 1 | Stock B | 1,760.808 | 1,581.735 | 1,209.667 | 1,448 | 1,447 | 1,313.192 |
| 2 | Stock B | 1,354.548 | 1,192.024 | 915.753 | 1,732 | 1,731 | 855.389 |
| 2 | `open_datasync` | 1,402.792 | 1,208.668 | 903.266 | 1,672 | 0 | 890.679 |
| 2 | Stock A | 1,383.162 | 1,206.631 | 916.505 | 1,684 | 1,683 | 871.204 |

The candidate's synchronized writes recorded 856.293 and 890.679 ms entirely in `wal_write_time`. Combined PostgreSQL WAL time grew by 13.712 and 35.290 ms against the faster stock control in each rotation. These are cumulative process times that include off-clock publication and shutdown. Device counters independently bracket the whole disk from enqueue commit through final settlement. The candidate still recorded 991–1,581 and 1,018–1,639 completed device flushes inside those brackets. [PostgreSQL source for write and sync accounting](https://github.com/postgres/postgres/blob/REL_17_11/src/backend/access/transam/xlog.c) · [Raw counter reduction](evidence/native/pilot/independent-replay.json)

### The divergent stock control

Rotation 1 Stock B took 416.976 ms longer to drain than Stock A. It recorded 1,287.346 ms in explicit PostgreSQL WAL sync time versus 811.741 ms in Stock A, with 222 fewer explicit syncs. Whole-device flush time was 314–410 ms for Stock B and 71–114 ms for Stock A. Whole-device write time was 636–854 ms and 174–310 ms, respectively. These brackets show a slower storage window during Stock B. The captures leave its cause unresolved. Both controls remain in the decision. The gate uses the faster control as the reference and both controls to set the spread. [Measured-arm reductions](evidence/native/pilot/independent-replay.json)

## Advancement decision

Positive gain means the candidate beat the faster control. Each gain had to strictly exceed the largest of 5% of that control, the live duplicate spread, and the historical duplicate spread. Rotation 1 used the live spread on all three clocks. Rotation 2 used 5% on submit and drain, and the 47.351 ms historical spread on middle writes. Its 12.487 ms middle-write improvement was about 1.36%. [Published gates](evidence/native/pilot/final-replay.json)

| Rotation | Clock | Gain (ms) | Live control spread (ms) | Required gain (ms) | Pass |
| --- | --- | ---: | ---: | ---: | --- |
| 1 | Submit | −13.065 | 432.173 | 432.173 | No |
| 1 | Drain | −16.639 | 416.976 | 416.976 | No |
| 1 | Middle writes | −12.473 | 318.422 | 318.422 | No |
| 2 | Submit | −48.244 | 28.614 | 67.727 | No |
| 2 | Drain | −16.644 | 14.607 | 59.601 | No |
| 2 | Middle writes | +12.487 | 0.752 | 47.351 | No |

## Replay the archive

The independent reducer reads the raw ledgers, WAL snapshots, and disk samples for both phases. It checks all 12 raw hashes, their sequence and distinct cluster IDs, and recomputes the six rows and gates. The frozen controller replay also runs the original workload and staging reconciler. Both commands run offline from the checkout root.

```sh
d=tools/durable-jobs-bench/results/2026-11-25-dense-wal-write-method
python3 -B "$d/replay_pilot.py" | cmp - "$d/evidence/native/pilot/independent-replay.json"
python3 -B "$d/controller.py" replay pilot | cmp - "$d/evidence/native/pilot/final-replay.json"
```
