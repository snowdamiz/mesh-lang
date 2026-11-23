# PostgreSQL enqueue cost for the 512-job slow workload

Changing the producer connection’s `default_toast_compression` from `pglz` to `lz4` saved **84–100 ms of backend CPU per 512-job transaction** against the faster duplicate control in all four rotations on macOS ARM64. The same 504 jobs used the set-based writer and the same eight used its scalar tail in every default and LZ4 arm. LZ4 beat both controls on wall time in three rotations. The fourth finished 2.75 ms behind the faster control. [32 trial receipts and clock calibration](receipts.json)

| Slow workload, 512 × 16 KiB | Backend CPU, median ms | BEGIN through COMMIT, median ms | Stored argument bytes |
|---|---:|---:|---:|
| Default PGLZ A | 180.12 | 275.36 | 8,431,616 |
| Default PGLZ B | 180.96 | 276.29 | 8,431,616 |
| LZ4 session default | 82.10 | 173.63 | 8,171,728 |
| EXTERNAL column storage | 174.42 | 419.89 | 8,431,616 |

PostgreSQL 17.11 stored all 512 canonical arguments uncompressed under PGLZ and used LZ4 for all 512 when selected. Setting the column to EXTERNAL changed its catalog layout. The candidate’s layout check then sent every row through scalar enqueue. Every arm validated the complete arguments and receipt IDs after the outer transaction committed.

For 512 equally long repeated-character payloads, LZ4 used 43.88 ms of backend CPU and 82,421 stored bytes. The duplicate defaults used 61.77 and 63.67 ms and 143,329 bytes. EXTERNAL occupied 8,431,616 bytes. Stored argument bytes sum `pg_column_size(args)`. Filesystem allocation and index overhead are separate.

The driver prebuilt the same nine chunk sizes observed in the compiled native batch producer. It executed the frozen V1 schema, both real enqueue functions, the namespace fence and the layout check on a local Unix socket. The capture includes one outer synchronous commit and per-chunk savepoints. The Python driver used the database owner and priced SQL execution on a single connection. Application encoding, transport to the producer, workers and settlements belong to the compiled comparison. Shared-host scheduling affects the wall clock. All native timings and peer rankings come from the separate matched campaign. [Frozen binary and SQL package](../2026-11-23-set-based-enqueue/native/bulk-enqueue-ce5e551d079e.tar.gz) · [Native scorecard](../2026-11-23-iteration-comparison/four-engine-scorecard.md)

## Replay

Run the reducer from the checkout root.

```sh
python3 -B tools/durable-jobs-bench/results/2026-11-23-slow-payload-storage-cost/reduce.py
```

For a fresh macOS trial with Homebrew PostgreSQL 17.11, choose a new evidence directory. The disposable cluster uses a private Unix socket and is removed after shutdown.

```sh
PG_BIN_DIR=/opt/homebrew/opt/postgresql@17/bin python3 -B tools/durable-jobs-bench/results/2026-11-23-slow-payload-storage-cost/screen.py target/slow-payload-storage-cost/new
```

The first setup attempt used a libpq helper that requires idle state after each command and rejected an open transaction. The second incorrectly expected one physical `xmin` across released savepoints. The accepted replay checks the parent transaction’s commit timestamp against all 512 rows and joins every returned ID to both the stored argument and its durable receipt. No failed arm entered the timing table.
