# Large Job enqueue on PostgreSQL 17.11

Sending 16 half-megabyte jobs in one genuine set-based write increased PostgreSQL CPU in every default-storage rotation. Four writes for the 64 jobs cost 894 ms of backend CPU at the median. The two stock arms cost 701 and 632 ms. The set-based arm also committed later than both controls in all five rotations. Keep the current large producer for the native comparison.

| Argument storage | Submission path | Backend CPU, ms | BEGIN through COMMIT, ms | Faster wall than both stock controls |
|---|---|---:|---:|---:|
| Default | Stock A / B | 701 / 632 | 988 / 1,006 | — |
| Default | Scalar writes, four savepoints | 715 | 926 | 3/5 |
| Default | Four calls to an ordered SQL loop | 704 | 1,306 | 0/5 |
| Default | Four set-based writes | 894 | 1,361 | 0/5 |
| EXTERNAL | Stock A / B | 178 / 200 | 1,068 / 653 | — |
| EXTERNAL | Scalar writes, four savepoints | 174 | 479 | 2/5 |
| EXTERNAL | Four calls to an ordered SQL loop | 210 | 717 | 2/5 |
| EXTERNAL | Batch request with scalar fallback | 259 | 941 | 1/5 |

Each of the 50 trials ran on a fresh database and submitted the original 64 × 512 KiB payloads in one synchronous transaction. Stock made 64 scalar calls with a savepoint per job. The diagnostic passed individually bounded values to the ordered loop. On the default layout the set-based path called the frozen Mesh bulk SQL four times. On the EXTERNAL layout its catalog check requested scalar fallback each time. This screen bypassed the public 1 MiB per-call input limit to measure the proposed database work.

The collector checked every stored payload and receipt ID against the input, insertion order by durable position, and each row’s commit timestamp against the top transaction. All variants stored 33,559,808 bytes across 64 uncompressed argument datums. Timings include the SQL exchanges over a local Unix socket, with inputs prepared before the transaction. Host one-minute load ranged from 30 to 71. The original native queue timings include producer encoding, TLS, workers, and terminal settlement. [Native scorecard](../2026-11-23-iteration-comparison/four-engine-scorecard.md)

## Replay

From the repository root, regenerate the paired differences, win counts, and medians from all original arm receipts. The script also checks the calibrated Mach timebase, argument storage, and per-arm SQL call counts. To run the SQL screen again, give it a fresh directory inside the checkout.

```sh
python3 -B tools/durable-jobs-bench/results/2026-11-23-large-enqueue-groups/replay.py
PG_BIN_DIR=/opt/homebrew/opt/postgresql@17/bin python3 -B tools/durable-jobs-bench/results/2026-11-23-large-enqueue-groups/screen.py target/large-enqueue-groups/fresh
```

[Original receipts](receipts.tar.gz) · [Paired results in milliseconds](summary.json)
