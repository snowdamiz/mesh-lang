# Public LARGE progress with symmetric EXTERNAL storage

The public Mesh FILE producer settled 64 × 512 KiB jobs sooner than River and Oban in all five matched Linode rotations. All lanes used PostgreSQL 17.11, EXTERNAL payload storage and two CPUs shared by PostgreSQL and the workload. The stock public inline comparator, same-broker inline duplicates, and three stock peers ran alongside FILE. The [controller replay](../2026-11-25-public-large-file-external/grant-parser-retry/README.md) covers all 70 warmup and measured arms.

| Peer | Median FILE / peer submit-to-settlement throughput, paired | FILE wins | Rotations with ≥20% FILE throughput advantage |
|---|---:|---:|---:|
| River | 1.752× | 5/5 | 5/5 |
| Oban | 1.203× | 5/5 | 3/5 |
| Apalis | 0.623× | 0/5 | 0/5 |

Median full-submit throughputs were 46.96 jobs/s for Mesh, 27.56 for River, 38.48 for Oban and 73.89 for Apalis. Ratios in the table use matched rotations. The 20% column is descriptive.

### Which clock still needs to move on public FILE

At fixed drain, Mesh needed 43.3 ms and 254.8 ms of producer savings to reach 1.20× Oban's full-submit throughput in rotations 1 and 4. The other three already cleared that mark. At fixed enqueue, each rotation still required substantial drain savings to reach the same throughput ratio **on the drain clock**. A component timing in CPU-ms cannot be substituted for either elapsed saving.

| Rotation | Additional submit saving needed, ms | Additional drain saving needed, ms | Jobs whose claim stamp was still ahead at the drain deadline | Durable effects reached, of 64 |
|---|---:|---:|---:|---:|
| 1 | 43.3 | 279.6 | 25 | 32 |
| 2 | 0 | 244.2 | 28 | 32 |
| 3 | 0 | 208.7 | 25 | 36 |
| 4 | 254.8 | 439.1 | 32 | 24 |
| 5 | 0 | 212.3 | 20–21 | 40 |

The deadline starts at Mesh's producer commit and allows five sixths of the same rotation's Oban drain. In rotation 4 the final Mesh claim stamp was still **380.5 ms ahead** at that cutoff. Four different runner PIDs later handled the 32 jobs whose stamps definitely missed it. Even instant handling at those stamps would still miss that deadline on the recorded schedule. Across the five rotations, Mesh had three to six productive runner PIDs out of eight; the run with six had the largest deficit.

The [replay](peer_budgets.py) joins the released ledgers and raw hashes for FILE, River, Oban and Apalis. It bounds each millisecond-rounded claim stamp by the earlier of handler entry and the end of its rounding interval. The [receipt](peer-budgets.json) keeps signed headroom for all three peers. These are stock, same-rotation deadlines. The prepared consumer and optimized producer each need their own matched controls.

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

## Last confirmed grant and handler work

The four measured prepared-consumer arms expose the client timestamp after each runner's last positive grant COMMIT. The replay joins runners to job ledgers by PID and reconciles the groups to eight or nine activation calls. In the reverse combined arm, one runner took two turns. Its earlier eight-job receipt is bounded by the latest claim stamp and earliest handler entry in that turn. Either end of the bound gives the same final effect in the calculations below.

| Combined EXTERNAL arm, ms from durable enqueue | Forward | Reverse |
|---|---:|---:|
| Last confirmed grant | 480.122 | 478.987 |
| Last durable effect | 531.631 | 523.107 |
| Last effect with each receipt-to-entry interval removed | 521.201 | 515.697 |
| Last effect with each entry-to-first-write interval removed | 521.829 | 510.384 |
| Last effect with each receipt-to-first-write interval removed | 490.377 | 493.833 |

Each illustration moves a job's later events by its own removed interval and takes the latest adjusted effect across all 64 jobs. The grant schedule and downstream durations stay fixed. The entry-to-first-write interval includes application SQL and scheduling as well as parsing. Moving work off the shared CPUs could also advance later grants in a new run.

Even instant completion at every confirmed grant misses the most permissive historical Oban deadline of 446.855 ms by 33.267 and 32.132 ms. At the tightest historical deadline the corresponding gaps are 75.439 and 74.304 ms. These are minimum advances in final confirmation for a 20% drain-throughput lead against those historical times. An actual peer comparison still requires fresh matched runs.

On the released runtime, the PostgreSQL row reader validates UTF-8 and copies the context into an owned string. Preparation moves that string into the checked batch. The trusted worker copies it into its actor heap before handler entry. Parsing a JSON string copies the decoded value into the heap, while a successful `Json.as_string` returns the stored pointer. Charging that accessor for another payload copy would overstate the opportunity. The source pins are `f3b3612d9` in `compiler/mesh-rt/src/db/pg.rs`, `src/jobs/prepared_claim.rs`, `src/jobs/worker.rs` and `src/json.rs` under the same runtime crate.

```sh
python3 -B confirmed_handler.py "$external_pilot_evidence" | cmp - confirmed-handler.json
```

## How many runners need to advance on LARGE

On the two EXTERNAL combined arms, moving every job on just the slowest of the eight runners arbitrarily early, while holding the other seven timelines fixed, could shorten drain by at most **12.173 and 10.665 ms**. The second-latest runner settled at 530.630 and 521.732 ms after enqueue. An improvement to prepared context rendering can affect several runners, since all 64 contexts pass through that code.

| Historical Oban comparison | Forward combined | Reverse combined |
|---|---:|---:|
| Drain saving required to tie | 6.577–57.183 ms | −3.829 to +46.777 ms |
| Drain saving required for a 20% throughput lead | 95.948–138.120 ms | 85.542–127.714 ms |
| Runners with a durable effect after the 20% deadline | 3–4 | 3–4 |
| Runners with a terminal commit after the 20% deadline | 4–7 | 4–6 |

Each range uses all five historical stock Oban drains. A negative saving means that combined arm had already met that particular historical time. The five blocks and two arms came from different runs. A prospective treatment still needs same-image controls, an eligible original bridge, and fresh matched peers for a competitive result.

Across eight sessions PostgreSQL recorded 1,907 and 1,770 ms for the top-level PREPARE SELECT, including 154 and 162 calls respectively and 64 returned rows in each arm. The aggregate spans polling, SQL, result handling and waits. It cannot assign a latency or CPU saving to removal of an intermediate JSONB object. The [per-deadline replay](render-budget.json) binds the two original combined captures, all eight runner reports in each, the released pilot score, and the historical peer series. Run `python3 -B render_budget.py | cmp - render-budget.json` from this directory with commit `39f7fae80` available locally.
