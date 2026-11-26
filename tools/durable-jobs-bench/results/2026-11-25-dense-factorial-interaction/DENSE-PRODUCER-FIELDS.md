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

## How much of the Direct4 submit gap could wake publication recover?

The two instrumented public 8×8 arms recorded 1,024 calls to `publish_wake_v1` and eight layout checks each. All the wake calls together took **8.444 and 11.607 ms**. The layout function took 27.950 and 32.702 ms separately. Both runs missed their observer stability gates. The figures describe the cost recorded with tracing on. They give no direct measurement of a replacement on Direct4.

The six private 2×32 Direct4 controls took **156.078–173.063 ms** from submit to enqueue commit. Even matching warm River's later effect cadence would leave a modeled **44.255–75.293 ms** of enqueue savings needed for a 20% submit-throughput lead. Giving Direct4 the full larger wake measurement as an elapsed saving leaves **32.648–63.686 ms**. Also crediting the best common-fields producer saving of **15.601 ms** leaves **17.047–48.085 ms**. The latter calculation deliberately assumes transfer and addition of two results from separate releases. It does not qualify either change. [Direct4 comparison and assumptions](DIRECT4-PEER-DEADLINES.md)

The matched input on all six warm River arms was 1,158,216 bytes. The Go producer took **0.893–1.031 ms to read it**, **8.641–9.630 ms to decode it**, and **61.255–77.863 ms from the enqueue call to the database commit record**. The last interval includes opening the transaction, the product insert, eight 128-row inserts returning jobs and application-side handling of the rows. Its scope is wider than PostgreSQL executor time. The adapter calls the ordinary `InsertManyTx` API, and its pinned PGX driver inserts from typed columns and returns the job row, including args. [Go producer](../../go/db.go)

The Direct4 backend counters measured **120–140 CPU-ms** on the single authenticated producer backend in each control. Every archive also contains **six observation replies after the enqueue receipt** on that connection. The closing CPU sample began **783.589–860.845 ms after acknowledgement**. The sums therefore cannot identify how much CPU the backend used during enqueue. Capturing the broker and backend counters as soon as the ID receipt arrives, before any subsequent observation on that connection, would give a narrower bracket. Its start/end skew, time from commit to receipt, clock resolution and command mix still need to be reported.

Run `python3 -B direct4_producer_budget.py EXPOSURE_DIR` with the same released event-park directory as the Direct4 replay. It checks the six Mesh control archives, six warm River archives, both function-tracked archives, and the six-arm common-fields export. It verifies the original product image, the shared Mesh/River input digest, receipt IDs, backend start ticks, wake call counts and recorded commit clocks.
