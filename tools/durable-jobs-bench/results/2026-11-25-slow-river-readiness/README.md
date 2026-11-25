# SLOW 2×32 River readiness sensitivity

This qualification asks how much of the ordinary unfused Mesh versus River gap survives when River starts with one available shared-pool connection per worker slot. It is a separate PGLZ sensitivity. The stock four-engine codec study supplies the released symmetric proof and matched inventory. The codec study and the cap-71 fused HOLD retain their own controller dispositions.

PGLZ is fixed for every lane. It passed the released two-codec proof and matches all eight independent ordinary cap-71 V1 blocks used below. All positive arms use PostgreSQL 17.11 on CPUs 0 and 1, 512 canonical 16 KiB jobs, two runners with 32 slots each, cap 71, one durable enqueue, a 100 ms hold between independent application commits, and one attempt and effect per job. The cap-71 scorer retains the original transaction, physical session, role, listener policy, CPU-steal, builder exclusion and disk observations. Mesh reuses the frozen unfused V1 cursor, eight warm application TLS sessions, the qualified bulk producer and scalar terminal path. Oban supplies a matched ordinary peer cutoff.

## Qualification

| Lane | Image and startup policy |
| --- | --- |
| Mesh | Exact retained unfused V1 inventory |
| Retained River | Exact SLOW cap-71 stock ELF, SHA-256 `a23130f74210946291ed3609797e5b218ece5a7bd70df318851cf91e634da1e1`, with stock lazy pools |
| Lazy A and B | Two fresh arms from one rebuilt ELF, with the same stock lazy policy and equal observation hooks |
| Warm River | Same rebuilt ELF, with explicit shared-pool checkout before each runner's ordinary River client start |
| Oban | Exact matched ordinary cap-71 inventory |
| Under-ready River | Same rebuilt ELF, runner 1 holds one fewer session. The proof expects the specific physical readiness rejection before the first submit. |

Each warm runner verifies all 33 TLS backend identities while retaining its actual checkouts. A two-runner barrier keeps both sets held concurrently. The checkouts return to the original 34-connection pools at the barrier so ordinary River can detach one of those same connections per runner for its notifier. The 64 shared sessions, two detached listener identities and original producer/observer must survive from the barrier through submit. Pool minima and maxima, listener lifecycle, poll interval and fetch cooldown retain their stock values. The startup census uses the existing producer/observer connection. It checks the empty trial at readiness, mid-idle and before submit. The replay reconstructs the live roster independently from PostgreSQL authorization and disconnect events. It rejects a connection change within the proof window or in an ambiguous millisecond boundary. The under-ready arm requires both real listeners, 33/32 held sessions, an empty enqueue log, and the exact startup gate failure. Stock retains its own binary and launch behavior. Its idle markers and physical log prove the seven-client bridge. The 67/7 readiness contrast precedes either workload clock.

The proof preserves the original submit-before-batch-read origin for River and the durable enqueue origin for all engines. It exports both clocks at first-write ranks 64, 448 and 512, at the last effect and at the final terminal commit. It also reports the rank-64-to-448 interval, the terminal tail, and the recorder and PostgreSQL startup envelopes. Rebuilt River reports producer connection setup and first runner launch through readiness and settlement. Retained River has a wrapper dispatch observation that includes Go migration. Mesh and Oban have a first-fork observation. These launch boundaries are reported with their actual scope.

## Prospective price

The table uses released raw ledgers from the ordinary SLOW V1 PGLZ study. Positive `River − Mesh` is how much faster River would have to become to erase a Mesh win, with Mesh fixed. Positive `River − 1.2 × Mesh` is how much River could improve before Mesh falls below a 20% throughput lead. The Oban columns show the corresponding Mesh deadline slack at `Oban / 1.2`. All values are milliseconds. [Raw hashes and exact microseconds](pricing.json)

| Block | River − Mesh submit | River − Mesh drain | River 20% margin submit | River 20% margin drain | Oban 20% deadline slack submit | Oban 20% deadline slack drain |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | +67.002 | +188.047 | −324.246 | −84.035 | −220.392 | −77.743 |
| 2 | −107.330 | −19.252 | −482.720 | −284.626 | −276.317 | −178.729 |
| 3 | +137.451 | +209.180 | −240.867 | −51.843 | −208.740 | −44.696 |
| 4 | +256.393 | +293.584 | −93.018 | +45.996 | −241.028 | −129.416 |
| 5 | +221.014 | +247.956 | −135.431 | −6.015 | −195.492 | −102.751 |
| 6 | +267.835 | +337.332 | −83.464 | +92.367 | −193.040 | −70.777 |
| 7 | −42.150 | +50.239 | −406.980 | −197.783 | −99.650 | +41.228 |
| 8 | +5.326 | +116.545 | −349.155 | −121.518 | −182.801 | −0.370 |

The stock median gaps are +102.226 ms from submit and +198.614 ms from enqueue commit. The smallest positive historical gaps are +5.326 and +50.239 ms. Even a small improvement can flip a close pair, and the 20% River drain margin survived in only two historical blocks. A readiness effect must be measured in fresh pairs. The prior SHORT and DENSE effects supply the probe design. Startup cost consumes any elapsed savings on a launch-to-settlement clock and is reported separately.

## Controller handoff

Only the sole authenticated controller can route the sealed SETUP request from the canonical checkout. It checks the original host, boot, disk and retained pins before staging. The new build runs under the local dispatch lock and both native fences. The native Go prerequisite is an existing Linux AMD64 Go 1.26.4 compiler and complete vetted module cache. The released evidence identifies the version and module graph. The controller supplies the exact native path and compiler digest after checking its existing inventory. The packet uses network-disabled Go settings and requires the cap-71 stock rebuild to reproduce the released ELF byte for byte before building the adapter. It also requires identical Go module and build metadata and pins the candidate ELF in every rebuilt lane's worker inventory.

The controller puts `2026-11-25-slow-river-readiness.toolchain.json` in its existing requests directory with `request_sha256`, `compiler`, `compiler_sha256` and `module_cache`. The inherited stock replay helper paths are already in the controller checkout. The exact blobs are also retained in Git commit `c10cc82938d3754cf071c3baa8ee357247679b66` under `tools/durable-jobs-bench/results/2026-11-23-slow-storage-transfer/replay`. The existing checkout lock and exact request inbox still apply. If the retained exact River ELF was collected, the controller can restore its same archive member to the pinned path through its existing custody process.

```sh
d=tools/durable-jobs-bench/results/2026-11-25-slow-river-readiness
python3 -B "$d/controller.py" check
python3 -B "$d/controller.py" proof
python3 -B "$d/controller.py" replay-proof --release /path/to/canonical-release.json
```

The controller release binds the request digest, all seven raw SHA-256 values from `raw-sha256.json`, and a `readiness_build_sha256` map keyed by `build.json`, `built-river.tar.gz`, `toolchain.json` and `source-sha256.txt`. Replay verifies that custody plus the actual per-arm inventories and evidence. Preserve partial captures on failure. The successful SETUP scores are diagnostic.

Any later timing needs a separate controller enrollment after an exit-zero proof, exact replay and retained build and file pins. The first two timing blocks would run `mesh retained-river lazy-a warm-river lazy-b oban` and its exact reverse. Each block would repeat its order for fresh unscored warmup and fresh measured arms. Exclude the shortage fault from timing. Price the stock-to-rebuilt bridge against both lazy duplicates, compute the cold-to-warm delta from the lazy midpoint, and report warm River versus Mesh and plain Oban on both official clocks. Require each proposed directional claim to clear 5% of warm elapsed, both absolute stock-to-lazy bridge gaps and the lazy spread in every mirrored measured block. Report the clocks and the inconclusive verdict when the differences fail that rule. Treat a 20% throughput lead as its own cutoff using 1.2 times Mesh elapsed against the matched peer. Timing remains gated on the same rebuilt ELF and plan, plus the existing campaign order and noise controls.
