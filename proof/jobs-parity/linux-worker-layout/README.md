# GNU ARM64 worker section layout proof

The full Jobs ELF `ba80441a7cef12ba78bf0900fe9294c56c7ae6d3bd32fd9bb555e097f8afea95` passed the frozen Linux slow64 score in five fresh boots. The shutdown-inclusive score also passed. The separate full-occupancy check recorded 61 committed holds for the candidate on slow-04 and 64 for both controls. All five boots and their original peaks remain in the score.

This is qualification for the frozen 512-job slow workload on the pinned GNU ARM64 guest. The artifact is a compiler layout candidate. Product rollout and other workloads have their own acceptance requirements.

## Recheck the five pairs

Run the portable replay from a checkout.

```sh
python3 -B proof/jobs-parity/linux-worker-layout/replay.py
```

Python 3.10 or newer is sufficient. The replay reads the three checked-in inputs and runs without a guest, build toolchain, or performance archive. Add `--compare-checkout` to check the result against `validate_pairs` and `profile_summary` in the repository harness. This option requires the harness hash recorded in the receipt. The standalone replay also works when the owning harness has changed.

The frozen score uses median throughput, pickup p99, CPU per job, and peak RSS. It uses the largest connection, process, and thread counts per scored arm and the candidate's peak queued context. The replay reads the original settings, including the strict throughput win over Oban and the 1.5× RSS limit. It preserves the 4–1 throughput win count and scores the shutdown envelope separately. The stock Mesh arm provides same-cohort context and contributes no measurements to the candidate-versus-Oban median gates.

[Replay](replay.py) · [Frozen settings](frozen-profiles.json) · [Qualification inputs](qualification.json)

## Measured results

Each measured arm runs 512 jobs with one runner, 64 slots, 100 ms holds, PostgreSQL 17.11, and a full warmup. Mesh arms use the retained worker-only THP policy. The observer verified `THP_enabled=0` on the held workers and `THP_enabled=1` on their runner. The pinned GNU ARM64 guest has 4 KiB base pages and a 65,536-byte fault window. Oban uses the `8526a5bd` inetrc with live lookup `["file"]` and effective flags `+S 2:2 +SDcpu 1:1 +SDio 1 +A 2`.

| Boot | Stock Mesh jobs/s | Candidate jobs/s | Oban jobs/s | Candidate peak RSS MiB | Oban peak RSS MiB | Stock / candidate / Oban committed hold peak |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| slow-01 | 525.99 | 510.40 | 474.48 | 202.25 | 142.59 | 64 / 64 / 64 |
| slow-02 | 466.76 | 487.17 | 452.48 | 203.05 | 143.35 | 64 / 64 / 64 |
| slow-03 | 227.72 | 372.78 | 390.24 | 220.66 | 141.86 | 64 / 64 / 64 |
| slow-04 | 381.97 | 257.55 | 235.27 | 190.98 | 151.27 | 64 / 61 / 64 |
| slow-05 | 539.67 | 545.39 | 349.22 | 203.05 | 147.03 | 64 / 64 / 64 |

| Five-boot median | Stock Mesh | Candidate | Oban |
| --- | ---: | ---: | ---: |
| Jobs/s | 466.76 | 487.17 | 390.24 |
| Peak RSS MiB | 328.69 | 203.05 | 143.35 |
| Peak RSS through shutdown MiB | 328.69 | 203.05 | 143.35 |
| Runner CPU ms/job | 1.671 | 1.506 | 1.807 |
| Pickup p99 ms | 979.84 | 940.10 | 1,204.85 |

The candidate's median RSS is 11.97 MiB below the frozen cap. Throughput and CPU vary in both directions against same-cohort stock Mesh. The RSS improvement also appears in the separate A/B/A held comparison. At 64 workers, stock summed RSS was 336.52–338.06 MiB and the candidate used 220.38 MiB. Main-image mappings fell by 116.25 MiB.

## Committed holds and exact waits

Each CSV row gives the first and second effect commit times for one completed job. Times are relative microseconds within one arm. The exporter keeps 512 rows for every arm. The replay sweeps the half-open commit intervals to derive the peak and integral of concurrent holds. It also checks the integral against the sum of individual hold durations. The compact receipt retains the database witnesses for ordered commits under distinct XIDs and the shutdown samples and runner wait4 totals.

Every measured arm reconciled 512 ordered write pairs. The scored candidate and Oban arms account for 5,120 jobs. The extra stock arms account for 2,560 jobs. The 64 exact worker waits come from a separate traced held boot of the same executable. The replay checks the exec, zero-exit, consuming-wait, held, and ledger PID sets. The five performance boots used trace-off execution and verified normal runner drain and wait4 status in each arm.

[Committed hold intervals](hold-intervals.csv) · [Lifecycle receipt](qualification.json)

## RSS sample scope

At terminal count 359 in slow-03, 57 descendants summed to 181.95 MiB and the runner used 38.72 MiB. The descendant total was 36.22 MiB above the median per-descendant estimate from the other samples with at least ten processes. A single runner-sized process replacing one typical worker predicts 36.16 MiB of excess. The boot's 65-process sample totaled 202.20 MiB.

The raw sampler exported aggregate RSS. A pre-exec child sharing the runner's address space is a hypothesis. Per-PID attribution at the peak is unavailable. The original 220.66 MiB stays in both scores.

## Rebuild and find the full archive

The unchanged fallback is `fe719b77c033f0c08e61138291eb119487c796476e23fe9fb544711dd50233c8`. The original Zig/LLD command and frozen object, archives, and link-time DSOs reproduce it byte for byte. To rebuild the candidate, use the same input files and apply the recorded linker fragment. The fragment selects whole sections by object or archive member and orders regular read-only data. The four PT_LOAD roles, GNU interpreter, constructors, dynamic ABI, unwind entries, and original garbage-collection settings are preserved.

The provenance file lists repository-relative release link commands and hashes for the inputs, layout, maps, and audits. Place the pinned link inputs at those paths, then run the benchmark-specific relink in a fresh output directory. This command uses the archived macOS ARM64 Zig driver.

```sh
python3 -B proof/jobs-parity/linux-worker-layout/relink.py relink . target/worker-layout-rebuild
```

The command verifies the inputs, reproduces the stock ELF, and asks LLD for the normal symbol map with `--strip-debug`. It retains the resulting map and regenerates the original `order.ld` from the checked-in line indices. It checks the fragment digest and the final candidate digest before writing the receipt. Add `--stock-map PATH` to use a retained map. To extract the fragment without linking, run the following command with the stock map.

```sh
python3 -B proof/jobs-parity/linux-worker-layout/relink.py extract \
  target/worker-hot-section-layout/stock/link.map target/reconstructed-order.ld
```

The indices are one-based lines in the stock LLD map. The receipt pins the archived map and its normalized digest. Normalization removes the checkout prefix from input names so an otherwise identical LLD map can come from another checkout. The extractor preserves the original order of 1,666 read-only data selectors and 2,228 text selectors. The resulting 595,792 bytes hash to `a5b21e88de9e5b027ea34acaba13f1830c6e07871b41947bbfdab30808a857de`. These indices apply to this archived ELF. The verification run generated its own stock map and linker script. The map, script, stock ELF, and candidate ELF all matched the archived bytes.

If the full archive is available, `target/worker-hot-section-layout/REPLAY.md` shows how to derive the order from the pinned profiles and verify the normal and relocation sidecars. Run `export.py --archive PATH` to derive the compact score inputs from that archive. The portable score runs directly from the checked-in inputs.

The compiler link entry point is `compiler/mesh-codegen/src/link.rs`. The independent portable compiler experiment keeps its adapter and method under `target/portable-mesh-section-layout`. The compiler layout owner is developing `scripts/mesh-section-layout.py` from that work. Its replay and the source artifact hashes let the compiler layout owner compare the approaches without borrowing benchmark claims from a different ELF.

The optional archive root is `target/worker-hot-section-layout`. The `qualification/slow-01` through `qualification/slow-05` directories hold all five scored groups. The `canonical/slow-*` directories hold the unscored pilots. The first held diagnostic without the retained THP flag lives in `held-01`. The corrected held proof lives in `canonical-held-01`. Dense pilot timings have their own receipts in `canonical/noop-*`.

[Link provenance](link-provenance.json) · [Map indices](order-map-indices.json) · [Relink helper](relink.py) · [Archive exporter](export.py)
