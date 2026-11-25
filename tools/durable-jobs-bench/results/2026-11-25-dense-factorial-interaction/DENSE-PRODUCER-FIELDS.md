# Common fields in the eight dense enqueue arguments

The compiled stock producer wrote 1,024 jobs in eight calls of 128 on PostgreSQL 17.11. All 15 normalized specification fields outside `args` had identical values within each call. Reconstructing the complete compact specification reproduced every SHA-256 fingerprint stored in the request receipts. The request key and fingerprint remain per job.

| UTF-8 JSON argument | Per 128 jobs | Per 1,024 jobs |
|---|---:|---:|
| Stock array of specification, request key and fingerprint | 187,521 B | 1,500,168 B |
| One common object plus per-row args, key and fingerprint | 154,639 B | 1,237,112 B |
| Difference | 32,882 B | 263,056 B (17.5%) |

The alternative encoding reconstructs the original normalized specs exactly for this input. The byte counts cover the JSON parameter inside the database call. External batch framing, the remaining protocol bytes, durable rows and client receipts sit outside this count. All ten measured Mesh and River archives across the five released late-capacity rotations had matching job keys and payload hashes.

A 30 ms producer saving would require removing 22.5–28.8% of the stock eight-call PostgreSQL execution time if it all came from the executors. A proportional saving on bytes alone gives no basis for that claim. Evaluating common fields once could also remove repeated server work. Its cost, the cost of forming the shared representation, and its behavior for mixed inputs require a separate test. The stock execution measurements are in commit `402459f7b` under `tools/durable-jobs-bench/results/2026-11-25-dense-producer-layout/`.

The replay binds the stored IDs and request keys to the compiled broker output and checks all full-spec fingerprints before counting bytes. It verifies both durable application writes per job in the ten matched native arms. From this directory run

```sh
python3 -B dense_producer_fields.py | cmp - dense-producer-fields.json
```
