# Public LARGE FILE with symmetric EXTERNAL storage

The Linode packet compares the public `b3557273` FILE producer with two identical inline controls on the recovered broker ELF. A fourth Mesh lane uses the original public EXTERNAL inline executable. River, Oban and Apalis use their released public EXTERNAL inventories, including the original storage-policy wrappers and hashes. Each of five blocks includes a warmup and a measured trial for all seven lanes. Blocks 2 and 4 reverse blocks 1 and 3, respectively.

All arms use 64 jobs with 512 KiB each, eight runners with eight slots each, CPUs 0 and 1, the original 96-connection cap, and the original host, disk and driver pins. The runner and migrator are the public `af2aca86…` image and the experimental producer is the recovered and qualified `a5a2d566…` broker. The inherited trial records first submit, first durable enqueue commit and last durable settlement. Mesh applies and reads back `EXTERNAL` and `work_mem=4MB` at migration. The stock peer overlays prove EXTERNAL before submit and verify all 64 stored, uncompressed payloads after settlement. All ledgers reconcile payload hashes, durable commits, exactly one attempt and effect per job, and distinct application transaction IDs. The retained Mesh adapter supplies the column-setting readback and job ledger.

## Historical evidence

The local [historical replay](evidence/historical-replay.json) rebuilt scores from 70 default-storage arms and 40 public EXTERNAL inline arms, with no local performance run. In the default FILE series, the faster same-image inline duplicate took a paired median 290 ms longer through enqueue and 288 ms longer through settlement. The paired post-commit drain difference was 10.5 ms in favor of inline. Mesh FILE beat River and Oban from submit in all five default-storage pairs.

For the original EXTERNAL inline series, Mesh minus River from submit had a paired median of −582 ms (range −678 to −558 ms), with five wins. Mesh minus Oban had a median of −49 ms (−100 to +128 ms), with three wins. Mesh minus Apalis had a median of +896 ms (+760 to +955 ms), with zero wins.

Holding the historical EXTERNAL post-commit drain and peer clocks fixed, an enqueue reduction of 47.207 ms and 128.306 ms would tie the two Oban losses. The Apalis ties would require 759.615–954.511 ms, depending on the block. Mesh already led River by the historical submit margins above. The corresponding median post-commit gaps were −189 ms against River, +180 ms against Oban and +506 ms against Apalis. Those gaps would persist if only enqueue changed. The historical default FILE effect and the EXTERNAL thresholds come from independent series. The FILE effect under EXTERNAL remains unmeasured.

The checked-cursor EXTERNAL campaign changed the consumer. The older EXTERNAL direct-file experiments used the ordinary V1 or protected runtimes, and `f3b3612d9` concerns a separate default-storage concurrent claim experiment. A scan of released inventories, local request records and the controller timeline found no exact run or pending request for this packet.

## Controller handoff

From the checkout root, verify the [sealed packet](controller-packet.json) and reproduce the historical replay.

```sh
d=tools/durable-jobs-bench/results/2026-11-25-public-large-file-external
python3 -B "$d/matched.py" check
python3 -B "$d/historical.py" | cmp - "$d/evidence/historical-replay.json"
```

Only the sole Linode controller registers the exact packet bytes and opens its 24-minute TIMING grant after the existing bookings. Use the existing authenticated and lock-fenced controller. Reuse the retained broker, adapter, stock peer images and PostgreSQL. The existing matrix stages its driver and inventories, rehashes pinned files before each arm, and verifies no build overlap. A failed block stops dispatch and retains its receipts for review.

```sh
python3 -B "$d/matched.py" timing
python3 -B "$d/matched.py" replay
```

The replay requires the controller's successful release at `evidence/native/controller-release.json`, with `id`, `exit_status` and the `grant` object from `evidence/native/grant.json`. It reconciles all 70 raw arms and reports the unchanged clocks, within-block comparisons and the conservative improvement against the faster inline duplicate. Preserve raw exports and the original releases. Any stage cleanup is limited to this run's seven private matrix staging trees after the raw output is secured and the controller fences are clear.
