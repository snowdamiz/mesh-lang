# Batch enqueue with EXTERNAL argument storage

For the frozen 512 × 16 KiB slow-job submission, enabling the same set-based writer on EXTERNAL argument storage reduced median PostgreSQL CPU to **90.6 ms**, from **224.4 and 221.8 ms** on duplicate default-storage controls. It used less CPU and committed earlier than both defaults and the original EXTERNAL scalar path in all four rotations. This experiment measures the producer transaction on a local Mac. Native end-to-end timing is still needed to establish a queue improvement.

| Argument storage and writer | Backend CPU, median ms | BEGIN through COMMIT, median ms | Batch / scalar rows |
|---|---:|---:|---:|
| Default, stock layout A | 224.4 | 635.8 | 504 / 8 |
| Default, stock layout B | 221.8 | 593.7 | 504 / 8 |
| EXTERNAL, original layout check | 204.8 | 730.6 | 0 / 512 |
| EXTERNAL, compatible layout check | 90.6 | 145.9 | 504 / 8 |

PostgreSQL stored 8,431,616 uncompressed argument bytes in every arm. EXTERNAL lets PostgreSQL store these externally without first attempting compression. Mesh’s original layout signature also selected the scalar writer when an operator configured EXTERNAL. The candidate treats EXTERNAL and the original EXTENDED setting as equivalent only for the storage field of the jobs table’s args column in that signature. The other attributes still participate. Changing metadata storage, selecting MAIN for args, enabling row security, adding an index, or adding a constraint each caused the candidate to decline the batch path. These checks ran with the schema owner. Scoped-role and compiled-queue behavior require their own proofs.

The SQL workload used PostgreSQL 17.11, pglz as the configured default codec, synchronous commit, and fsync. Each trial used a fresh database. The collector verified every argument, the 512 distinct receipt IDs, and agreement between the jobs’ commit timestamps and the producer transaction. Inputs were constructed before timing. The local collector used a Unix socket. One-minute load ranged from 53 to 57 during the trials.

## Replay

From the repository root, regenerate the summary or run the original collector in a fresh output directory.

```sh
python3 -B tools/durable-jobs-bench/results/2026-11-23-large-enqueue-groups/replay.py
PG_BIN_DIR=/opt/homebrew/opt/postgresql@17/bin python3 -B tools/durable-jobs-bench/results/2026-11-23-external-bulk-sql/screen.py target/external-bulk-sql/fresh
```

[Original receipts](receipts.tar.gz) · [Paired differences](summary.json) · [Compiled LZ4 comparison](../2026-11-23-slow-storage-transfer/README.md)
