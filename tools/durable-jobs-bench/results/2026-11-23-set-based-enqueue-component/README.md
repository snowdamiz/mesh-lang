# Set-based durable enqueue on PostgreSQL 17.11

Four rotated blocks compared two copies of stock Mesh enqueue SQL, one ordered call to the existing function per job inside a single SQL statement, and a set-based insert for independent jobs. The 1,024-job set-based path took a median **46 ms** through durable commit. The two stock controls took **248 and 285 ms**. With 128 short jobs, the corresponding figures were **9, 39 and 44 ms**.

| Producer operation | 128-job wall, ms | 128-job backend CPU, ms | 1,024-job wall, ms | 1,024-job backend CPU, ms |
| --- | ---: | ---: | ---: | ---: |
| Stock A | 38.8 | 22.7 | 247.9 | 160.5 |
| Ordered scalar calls in a single statement per batch | 16.9 | 13.7 | 129.2 | 97.3 |
| Set-based inserts and receipts | 8.6 | 6.9 | 45.8 | 34.0 |
| Stock B | 43.6 | 22.8 | 285.3 | 155.5 |

Each arm used a fresh database on macOS ARM64 and the canonical 128-job short or 1,024-job dense input. The measured window included the outer transaction, namespace admission and savepoint per batch of up to 128 specs, all SQL calls, response decoding, and COMMIT confirmation. Backend CPU used a calibrated Mach timebase. The client encoded the new parameter envelope within the window. All arms started with previously validated canonical JSON and request fingerprints and used a Unix socket. The price of parsing the original Mesh input, TLS, worker pickup and job completion falls outside this experiment.

Every measured arm reconciled the physical positions, generated IDs in receipt order, request fingerprints, durable payload hashes, effect keys and commit timestamps of job and receipt rows. Receipt replay, uniqueness and replacement policies, mixed-version clients and fault recovery still need verification in the compiled product. The native ordered-wire experiment that held after a short-job regression used one SQL statement per job and supplies a separate comparison. [Prior native experiment](../2026-11-23-ordered-enqueue-repeat/README.md)

Two preliminary checks exposed mistakes in the component harness. The first assumed a row inserted in a savepoint carried the top-level transaction XID. PostgreSQL used the subtransaction XID. The corrected check uses the matching commit timestamp. The second sorted a projected ordinal as text. The corrected query orders by the numeric source ordinal and validates the receipt for each input position. The original failures and SQL are in the [evidence archive](evidence.tar.gz).

From the checkout root, verify all 32 measured arms and reproduce the medians and paired differences.

```sh
python3 -B tools/durable-jobs-bench/results/2026-11-23-set-based-enqueue-component/replay.py
```

For a fresh local run, set `PG_BIN_DIR` to the macOS PostgreSQL 17.11 bin directory and append `--run target/bulk-fresh`. The replay uses the frozen libpq support from the [payload storage component](../2026-11-23-large-storage-component/README.md). [Full reduction](summary.json)
