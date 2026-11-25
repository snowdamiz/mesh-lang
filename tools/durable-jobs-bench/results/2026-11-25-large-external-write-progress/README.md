# Public LARGE progress with symmetric EXTERNAL storage

The public Mesh FILE producer settled 64 × 512 KiB jobs sooner than River and Oban in all five matched Linode rotations. All lanes used PostgreSQL 17.11, EXTERNAL payload storage and two CPUs shared by PostgreSQL and the workload. The stock public inline comparator, same-broker inline duplicates, and three stock peers ran alongside FILE. The [controller replay](../2026-11-25-public-large-file-external/grant-parser-retry/README.md) covers all 70 warmup and measured arms.

| Peer | Median FILE / peer submit-to-settlement throughput, paired | FILE wins | Rotations with ≥20% FILE throughput advantage |
|---|---:|---:|---:|
| River | 1.752× | 5/5 | 5/5 |
| Oban | 1.203× | 5/5 | 3/5 |
| Apalis | 0.623× | 0/5 | 0/5 |

Median full-submit throughputs were 46.96 jobs/s for Mesh, 27.56 for River, 38.48 for Oban and 73.89 for Apalis. Ratios in the table use matched rotations. The 20% column is descriptive.

## Where the differences accrued

Elapsed times below are paired median Mesh minus peer values in milliseconds. Negative values mean Mesh took less time. Each arm starts at its own submission, and the application milestones count durable second writes in completion order. Intervals partition each individual run exactly. Medians across runs need not add to the median total.

| Peer | Submit to durable enqueue | Commit to eighth second write | Eighth through 56th second write | 56th second write to last settlement |
|---|---:|---:|---:|---:|
| River | −751 | −189 | +163 | −159 |
| Oban | −440 | +37 | +101 | +33 |
| Apalis | +53 | +34 | +403 | +42 |

Mesh's enqueue advantage carried all five full-submit wins over Oban. Oban completed its post-commit drain sooner in all five, by a paired median 163 ms. The middle 48 writes took Mesh 326–573 ms longer than Apalis across the five pairs. That interval includes both admission and handler execution. The ledger alone cannot allocate the difference between them.

Against the faster same-broker inline control, FILE saved a median 293 ms to enqueue in five wins and 320 ms through settlement in four wins. The remaining full-submit pair lost 13 ms. The file-path transport was exercised by the benchmark broker; these measurements apply to that producer and deployment.

Run `python3 -B replay.py | cmp - native.json` here to verify the 20 measured FILE and peer archive hashes and reproduce [all intervals](native.json). For the controls and storage checks, run `python3 -B ../2026-11-25-public-large-file-external/grant-parser-retry/retry.py replay`.

## Producer CPU availability before commit

Both storage configurations left CPU time available in the measured portion of the stock FILE enqueue. In EXTERNAL groups 1, 2, 4 and 5, CPUs 0 and 1 together recorded 130–160 idle CPU-ms (29.5–36.4% of their ticks) over windows of 233–234 ms. Group 3 had too few interior samples for a difference. The five default-storage windows recorded 32.1–33.5% idle over 934–1,168 ms. Each CPU read included here followed a sample timestamp inside the producer transaction and preceded the durable commit timestamp.

In the separate default-storage factorial, the producer-only lookahead arm recorded 27.4% and 27.0% idle in the two rotations. The combined arm recorded 29.0% and 29.8%. The same-image OFF duplicates spanned 32.2–33.6%. These are partial host-wide counts on the two pinned CPUs. They cannot identify the process using the CPU, establish that input preparation was ready during idle ticks, or predict EXTERNAL throughput. An EXTERNAL trial still needs a fresh matched stock bridge and both reversed controls.

The replay checks 22 measured raw hashes. Supply the pilot evidence directory from the factorial archive at commit `34b4403224f4880dad4710934801183189346e49`.

```sh
python3 -B producer_window.py "$pilot_evidence" | cmp - producer-window.json
```

## Where the EXTERNAL factorial still needs earlier writes

The EXTERNAL pilot at `39f7fae80` closed after the reverse stock FILE bridge missed calibration. The measured bridge spent 812.637 ms before durable commit. Its two same-image OFF references spent 691.590 and 679.854 ms. The bridge's 126.915 ms enqueue excess accounted for all but 4.275 ms of its 131.190 ms full-submit excess. Its commit acknowledgement arrived 12.477 ms after commit, outside the enqueue interval. The reverse warmup bridge took 731 ms to enqueue. The corresponding warmup OFFs took 724 and 704 ms. The raw CPU counters cover only portions of each enqueue window and lack per-process attribution.

The five historical Oban drains imply deadlines of **404.683–446.855 ms** for a 20% throughput lead on 64 jobs. The table applies the most permissive deadline to each released combined Mesh arm. These are design targets across two series. The pilot supplied no fresh matched peer result.

| Combined Mesh rotation | Last claim stamp after commit | Jobs with a later claim stamp than 446.855 ms | Jobs still missing their second durable write at 446.855 ms | Minimum last-effect advance even with immediate final settlement |
|---|---:|---:|---:|---:|
| Forward | 460.890 ms | 7 | 21 | 84.776 ms |
| Reverse | 473.236 ms | 7 | 24 | 76.252 ms |

At the tightest deadline, the minimum last-effect advances rise to 126.948 and 118.424 ms. The observed final settlement tails were 11.172 and 9.290 ms. A producer-only improvement can advance submit-to-settlement. To reach these drain deadlines, work between durable enqueue and the last application write also needs to finish earlier. The claim timestamp is recorded inside the transaction and floored to milliseconds. Later stamps give a lower bound on claims still outstanding.

Replay the 20 matched historical archives and 16 factorial captures with the archived EXTERNAL pilot evidence directory as the argument. The script verifies the release, sealed score and raw hashes and checks the measured clocks against each ledger.

```sh
python3 -B factorial_deadlines.py "$external_pilot_evidence" | cmp - factorial-deadlines.json
```
