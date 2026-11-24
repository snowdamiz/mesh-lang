# Large enqueue receipts on PostgreSQL 17.11

Keep the original scalar enqueue function. Projecting the receipt inside the insert left every arm at **25,243 TOAST heap hits and 33,431 TOAST index hits** for the 64 × 512 KiB submission. It saved backend CPU against both stock controls once in four rotations and reached durable commit ahead of both in zero. The server’s full-row return did not produce an extra observable TOAST read in this workload. [Rotations and counters](result.json)

| Insert receipt implementation | Enqueue backend CPU (ms) | Enqueue wall (ms) | TOAST heap hits |
|---|---:|---:|---:|
| Stock A | 324 | 1,613 | 25,243 |
| Build the receipt in RETURNING, call the installed effect-key function | 324 | 1,617 | 25,243 |
| Return only ID, run and state to PL/pgSQL | 354 | 1,819 | 25,243 |
| Stock B | 285 | 1,360 | 25,243 |

Medians cover four rotated groups on local macOS ARM64 with EXTERNAL storage, synchronous commits and 16 MiB work_mem. The controls were identical. The 16 arms each checked 64 original request-key fingerprints, full receipts, ordered context bytes and stored payloads. Each job reached terminal state at attempt one in its own settlement transaction. The component used a local database socket and prepared inputs. Native submission includes the compiled producer, verified TLS and application work. [Native four-engine scorecard](../2026-11-23-iteration-comparison/four-engine-scorecard.md)

A separate live probe installed a helper that included an application argument in the effect key. Stock and direct projection returned the actual argument. Passing only the three fields produced a different first receipt. All three cases exercised request-key replay, a unique replacement and rollback to a savepoint. The first timing attempt compared receipt subtransaction IDs with the top-level XID. The corrected check matches every receipt’s commit timestamp to the producer’s confirmed commit and verifies all fingerprints and native IDs. [Raw measurements, failed attempt and compatibility probe](receipts.tar.gz)

## Replay

Run `python3 -B tools/durable-jobs-bench/results/2026-11-23-enqueue-receipt-projection/replay.py` from the checkout. A fresh macOS run uses `screen.py target/enqueue-receipt-projection/fresh` and `semantics.py target/enqueue-receipt-projection/fresh-proof` with `PG_BIN_DIR` pointing to PostgreSQL 17.11.
