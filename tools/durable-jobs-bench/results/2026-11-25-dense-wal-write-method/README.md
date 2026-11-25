# Dense PostgreSQL WAL write method

Run two reversed Mesh-only pilot blocks on the sole Linode controller. Each sub-block runs a full warmup and one measurement. Each arm starts a fresh isolated PostgreSQL 17.11 cluster. Block 1 runs stock A, candidate, stock B. Block 2 runs stock B, candidate, stock A. Both stock copies explicitly set `wal_sync_method=fdatasync`. The candidate sets `open_datasync`. The packet pins the previously measured hot R2 D2 runner, the original 1,024 dense ordinary jobs, the broker, PostgreSQL, CPUs 0 and 1, and Linux device `8:0`. The frozen scorer checks TLS, the two 32-slot runners, input and phase, all settlements, and 2,048 independent application write XIDs. Each arm keeps `synchronous_commit=on`, `fsync=on`, `full_page_writes=on`, commit timestamps, zero commit delay, and five commit siblings. [Frozen WAL packet](../2026-11-24-dense-wal-cost/controller-packet.json)

## PostgreSQL and Linux setup

PostgreSQL 17.11 offers `open_datasync` when built with `O_DSYNC`. With fsync enabled, `XLogFileOpen` uses the flags from `get_sync_bit`. For `open_datasync` this sets `O_DSYNC`, and `XLogWrite` times the WAL `pwrite` with the synchronization included. The open-method path skips the explicit WAL sync, while the `fdatasync` path instruments `pg_fdatasync` separately. The synchronous commit path still calls `XLogFlush`. Both arms keep `debug_io_direct` empty. [WAL open and flush code](https://github.com/postgres/postgres/blob/REL_17_11/src/backend/access/transam/xlog.c) · [Commit path](https://github.com/postgres/postgres/blob/REL_17_11/src/backend/access/transam/xact.c#L1451-L1492)

Linux `O_DSYNC` makes each write complete after data and the metadata required for data retrieval are synchronized. Linux 6.8 ext4 buffered writes use `generic_write_sync`, which calls the filesystem's range fsync operation for synchronous I/O. Before each isolated postmaster starts, the adapter resolves its actual `pg_wal` mount, checks its device and mount options, and exercises an 8 KiB write with both methods. It verifies the file descriptor flags and reads back the bytes. Both probes run in every arm and clean up before PostgreSQL starts. PostgreSQL then reads back the selected GUC and durability settings over TLS before workload clocks, after the workload, and after the same-cluster stats restart. [Linux open(2)](https://man7.org/linux/man-pages/man2/open.2.html) · [Linux 6.8 ext4 writes](https://github.com/torvalds/linux/blob/v6.8/fs/ext4/file.c#L285-L306) · [Linux 6.8 sync helper](https://github.com/torvalds/linux/blob/v6.8/include/linux/fs.h)

Replay checks positive instrumented WAL writes for both methods and checks the method-specific sync counters for consistency with the configured path. Use combined WAL write and sync milliseconds, the device's inner and outer write and flush brackets, and end-to-end durations to interpret the result. The PostgreSQL total includes late publication and shutdown activity, and the device counters cover the entire disk. Neither apportions durability work to individual transactions. The handler audit at `4cd77c00b` leaves flush and post-flush time unresolved. This pilot estimates the effect of changing the storage policy on the whole workload.

## Advance only after both pilot blocks pass

For submit-to-settlement, enqueue-commit-to-settlement, and the middle application write span, compare the candidate with the faster duplicate in each block. Each of the six savings must strictly exceed 5% of the faster duplicate, the absolute difference between the current duplicates, and the largest within-block duplicate difference from the released delay screen. The historical floors are 30.775 ms for submit, 39.415 ms for drain, and 47.351 ms for middle writes. The earlier 100 µs delay reduced the sync count and worsened all three clocks against the faster duplicate in both blocks. That result supplies a noise floor and establishes that lower sync count alone is insufficient. [Released delay replay](../2026-11-24-dense-group-commit/evidence/native/release-replay.json)

The prior two matched runs needed roughly 24% less submit time, 19–20% less drain time, and 60–62% less middle-write time for Mesh to reach River's measured times. Mesh already led Oban on submit and drain by roughly 2–6% and trailed on middle writes by 18–24 ms. The cumulative Mesh write-plus-sync totals of 845 and 830 ms are cluster process sums. They do not establish a removable share of those gaps. [Prior raw reduction](../2026-11-24-dense-wal-cost/evidence/native/final-replay.json)

A qualifying pilot allows a separate pricing booking. Run four engines under each method, in stock-then-candidate and candidate-then-stock rotations. Run Mesh, River, Oban, and Apalis in that order in rotation 1 and reverse engine order in rotation 2. Each engine gets the same setting within each sub-block, with its own isolated cluster and both phases. Compare Mesh with each peer under the same setting. Report both rotations. Two rotations support a follow-up decision on this frozen build and host.

## Replay and controller intake

From the checkout root, run the correctness-only checks.

```sh
d=tools/durable-jobs-bench/results/2026-11-25-dense-wal-write-method
python3 -B "$d/controller.py" check
python3 -B "$d/test_packet.py"
```

The sole controller admits the sealed packet under `2026-11-25-dense-wal-write-method-pilot` after existing bookings. Under its normal fences, it runs the following.

```sh
seal=$(shasum -a 256 "$d/request.json" | cut -d' ' -f1)
python3 -B "$d/controller.py" intake pilot "$seal"
python3 -B "$d/controller.py" replay pilot
```

The packet retains one attempt per sub-block under `evidence/native/pilot`. Review any failure before another intake. After a successful pilot release and a positive replay, the controller can independently admit the same seal under `2026-11-25-dense-wal-write-method-pricing` and use `intake pricing` and `replay pricing`. Pricing evidence goes under `evidence/native/pricing`. The controller keeps the grant records and private staging namespaces scoped to the campaign.
