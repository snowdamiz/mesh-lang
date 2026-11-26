# PostgreSQL storage on the compiled ordinary slow workload

Cluster-wide LZ4 materially reduced the compiled Mesh producer cost. It preserved the true bulk path and saved a median **105 ms of producer PostgreSQL CPU** and **159 ms of submit-to-commit time** against the faster of two identical PGLZ controls in each of six local rotations. The full submit-to-last-settlement window beat both controls in two of six rotations. Commit-to-last-settlement also beat both in two of six. The post-commit spread prevents a drain speedup claim.

## Canonical workload

Every arm uses the pinned compiled ordinary trusted V1 fixture, one warm producer transaction, 512 × 16 KiB canonical arguments, one runner with 64 slots and eight application lanes, real 100 ms holds, two independent application commits per job, verified TLS, PostgreSQL 17.11 commit timestamps, synchronous commit, fsync, full-page writes, and a 68-connection cap. The six rotations use every order of two fresh default clusters and one fresh LZ4 cluster. The same Mesh executable runs all three. The macOS arm is a functional and local performance comparison on a shared development host with variable load and no two-CPU affinity.

| Metric | PGLZ A median [min–max] | PGLZ B median [min–max] | LZ4 median [min–max] | LZ4 below both |
| --- | ---: | ---: | ---: | ---: |
| Submit to commit, ms | 452 [250–708] | 594 [349–860] | 252 [188–518] | 6/6 |
| Submit to acknowledgment, ms | 473 [260–720] | 612 [373–867] | 291 [194–529] | 5/6 |
| Producer process CPU, ms | 58 [41–60] | 58 [55–63] | 57 [45–62] | 2/6 |
| Producer PostgreSQL CPU, ms | 210 [151–222] | 214 [202–229] | 99 [79–123] | 6/6 |
| Commit to last settlement, ms | 1,289 [1,102–3,169] | 1,358 [1,085–1,528] | 1,302 [1,056–1,781] | 2/6 |
| Submit to last settlement, ms | 1,741 [1,351–3,877] | 1,938 [1,706–2,177] | 1,575 [1,244–2,299] | 2/6 |
| Physical payload column bytes | 8,431,616 | 8,431,616 | 8,171,728 | 6/6 |
| TOAST heap bytes | 8,740,864 | 8,740,864 | 8,404,992 | 6/6 |

Both PGLZ controls stored all 512 canonical values without compression. LZ4 compressed all 512 and lowered stored column bytes by 3.1%. All 18 arms recorded eight `enqueue_batch_v1` calls returning 504 rows and eight scalar calls returning eight rows. The producer process CPU overlaps across all three configurations. The measured CPU saving is in the PostgreSQL producer backend, and the parent’s exact-SQL component independently sees the same direction and scale.

[Canonical scores and paired differences](evidence/canonical/summary.json) · [Exact batch and cap](frozen/canonical-input.json) · [Local executable and runtime pins](frozen/local-pins.json)

## Column-policy fallback and compressible control

Column-level EXTERNAL and column-level LZ4 both changed the exact layout digest. In one canonical diagnostic per policy, each batch-shaped statement returned the stock fallback marker and wrote zero WAL. The eight fallback calls were followed by 512 successful scalar calls. The switch-off PGLZ control made 512 scalar calls without a layout probe. EXTERNAL left canonical column and TOAST heap bytes equal to PGLZ. Explicit column LZ4 hit the same physical sizes as cluster LZ4 while giving up the batched inserts. The diagnostic order was fixed, with one observation per configuration.

The five compressible rotations kept the 16 KiB argument size and replaced each payload with repetitions of `abcd`. Both codecs compressed 512 values in every arm and kept them inline. LZ4 reduced stored column bytes from 143,329 to 83,957 (41.4%) and saved a paired median 20 ms of PostgreSQL producer CPU against the faster PGLZ control, winning all five rotations. EXTERNAL stored 8,431,616 column bytes in its single compressible diagnostic and moved them out of line. Its diagnostic used the scalar fallback.

[Canonical DDL diagnostics](evidence/ddl-fallback/summary.json) · [Compressible rotations](evidence/compressible/summary.json) · [Compressible EXTERNAL diagnostic](evidence/compressible-external/summary.json)

## Native comparison already on record

The five retained Linux stock-policy blocks put bulk Mesh’s median window at 521 ms to producer commit, 1,436 ms from commit to final settlement, and 1,957 ms from submit to final settlement. In paired blocks Mesh trailed River by a median 185 ms from submit and 110 ms after commit. It trailed Apalis by 533 ms from submit and 313 ms after commit. Mesh finished 123 ms ahead of Oban from submit. Apalis entered its p99 job about 299 ms earlier from commit when comparing engine medians. Most of the Mesh–Apalis gap occurs after commit, with a corresponding difference in the time it takes committed jobs to reach the handler. The workload alone requires at least 800 ms of hold time at 64 continuously occupied slots. The native Mesh blocks recorded 64 peak slots, about 46 conservatively counted occupied slots on average, and at least 129 ms average slot residence. That identifies post-commit slot supply and residence as a separate measured constraint. The producer CPU saving remains relevant to submission and cannot establish a change in the native drain or rankings.

The compiled comparison starts before the warmed adapter reads and submits its batch, and its acknowledgment follows durable commit and receipt assembly. The producer and producer-backend CPU brackets cover that request through acknowledgment. Settlement windows use PostgreSQL commit timestamps. Runner CPU includes startup and idle. Physical layout probes happen before submission and after the final durable terminal row, outside both workload windows. The stock workload switches and SQL schema remain in force.

[Five retained native blocks and raw hashes](evidence/retained-native-v1-slow.json)

## Four-engine transfer qualification and native hold

All eight local transfer smokes completed the exact canonical workload. Each reconciled 512 full argument records, 512 attempt commits, 512 independent effect commits, and 512 terminal receipts. Both Mesh arms used the expected 504 true-batch rows plus eight scalar rows. River’s local run used its stock raw verifier. Its Linux-only RSS sampler was unavailable on Darwin, so these are functional smokes. Peer timing remains unmeasured.

| Engine | PGLZ compressed rows | LZ4 compressed rows | PGLZ column bytes | LZ4 column bytes |
| --- | ---: | ---: | ---: | ---: |
| Mesh | 0 | 512 | 8,431,616 | 8,171,728 |
| River | 0 | 512 | 8,431,616 | 8,171,728 |
| Oban | 0 | 512 | 8,431,616 | 8,171,728 |
| Apalis | 0 | 512 | 8,422,400 | 8,168,956 |

The native PostgreSQL 17.11 binary used in the retained inventory was built without LZ4. The bootstrap log and the binary pin match exactly. The preselected native comparison is five slow rotations of all four engines under both PGLZ and LZ4, using the same cluster build across the eight lanes, CPUs 0–1 and the existing artifact, durability, and topology pins. One per-lane warmup precedes each measurement block. The controller has a hold receipt. A future run would require a new LZ4-capable PostgreSQL pin and controller qualification with both policies on that same build. The existing native default results remain separate.

[Local smoke details and raw ledger hashes](evidence/local-peers/summary.json) · [Native PostgreSQL identity and configure evidence](frozen/native-postgres-capability.json) · [Frozen native inventory and rotation order](frozen/native-config.json) · [Controller hold](request.json)

## Replay

The frozen packet includes exact canonical and compressible batch bytes, the V1 source archive plus the `ce5e551d` enqueue overlay, the linked Darwin binary, the harness, and the Linux artifact and adapter inventory. The earlier local producer-screen executable rejected the V1 worker’s `execution` config. The linked local artifact here uses the V1 runtime with the exact bulk changes and the retained release compiler, which is how the separately pinned Linux artifact was assembled.

Replay from the repository root. The replay commands rescore the persisted per-job payload and committed-write evidence. They do not start PostgreSQL.

```sh
result=tools/durable-jobs-bench/results/2026-11-23-slow-storage-transfer
lane=target/ordinary-slow-storage-transfer
for series in canonical compressible ddl-fallback compressible-external; do
  python3 -B "$result/replay/local_series.py" replay "$lane" "$series"
done
python3 -B "$result/replay/verify_local_peers.py"
python3 -B "$result/replay/native.py" check
(cd "$result" && shasum -a 256 -c SHA256SUMS)
```

To recreate the isolated local runner, extract `frozen/local-harness.tar.gz` into `$lane/base` and `frozen/local-compiled-v1-darwin-arm64.tar.gz` into `$lane/artifacts-v1`. Set `PG_BIN_DIR` to PostgreSQL 17.11, and run `replay/local_mesh.py "$lane" /absolute/path/to/new/evidence lz4`. The runner requires a fresh evidence directory. It verifies the exact batch before timing and reconciles the committed rows afterward.
