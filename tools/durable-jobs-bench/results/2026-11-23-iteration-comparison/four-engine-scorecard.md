# Four native queue engines on the same two CPUs

## Public trusted Mesh, Go River, Elixir Oban and Rust Apalis

Each row gives the median jobs/s and matched wins over five Linode rotations. All rows use one Mesh executable and the policy selected on short jobs. The trusted handler runs inside its application process. PostgreSQL 17.11 shares two CPUs with each queue. Every handler commits its two application writes independently. Submit includes producer work. Drain starts at the first enqueue commit. Both clocks end at the final durable settlement. [Interactive results](chart.html#public-short-selection)

| Workload and deployment | Clock | Mesh | River | Oban | Apalis | Mesh wins vs River / Oban / Apalis |
|---|---|---:|---:|---:|---:|---|
| [Short, 1 × 16](chart.html#public-short-selection) | Submit | 570.01 | 561.11 | 277.41 | 854.46 | 3 / 5 / 0 |
| [Short, 1 × 16](chart.html#public-short-selection) | Drain | 713.61 | 613.50 | 313.31 | 924.81 | 3 / 5 / 0 |
| [Dense, 8 × 8](chart.html#public-dense-stock) | Submit | 470.19 | 841.96 | 634.18 | 1,112.67 | 0 / 0 / 1 |
| [Dense, 8 × 8](chart.html#public-dense-stock) | Drain | 512.48 | 904.54 | 698.45 | 1,162.19 | 0 / 0 / 1 |
| [Sparse, 1 slot](chart.html#public-sparse) | Submit | 2.66 | 2.08 | 2.04 | 2.48 | 5 / 5 / 5 |
| [Sparse, 1 slot](chart.html#public-sparse) | Drain | 2.66 | 2.08 | 2.04 | 2.48 | 5 / 5 / 5 |
| [Slow, 1 × 64](chart.html#public-slow) | Submit | 262.23 | 287.43 | 242.05 | 354.62 | 2 / 5 / 0 |
| [Slow, 1 × 64](chart.html#public-slow) | Drain | 366.99 | 391.01 | 321.85 | 455.00 | 2 / 5 / 0 |
| [Large, 8 × 8](chart.html#public-large) | Submit | 25.36 | 20.69 | 24.95 | 37.78 | 5 / 4 / 0 |
| [Large, 8 × 8](chart.html#public-large) | Drain | 90.72 | 70.90 | 109.05 | 339.78 | 5 / 0 / 0 |
| [Dense, Mesh 2 × 32 and peers 8 × 8](chart.html#public-dense-mixed) | Submit | 682.04 | 843.03 | 621.83 | 1,119.75 | 0 / 5 / 0 |
| [Dense, Mesh 2 × 32 and peers 8 × 8](chart.html#public-dense-mixed) | Drain | 770.33 | 900.16 | 684.68 | 1,165.95 | 0 / 5 / 0 |
| [Dense, all engines 2 × 32](chart.html#public-dense-equal) | Submit | 688.41 | 997.17 | 762.22 | 1,407.67 | 0 / 0 / 0 |
| [Dense, all engines 2 × 32](chart.html#public-dense-equal) | Drain | 772.65 | 1,079.96 | 855.47 | 1,493.00 | 0 / 0 / 0 |
| [Large, all engines EXTERNAL](chart.html#public-large-external) | Submit | 36.24 | 27.13 | 36.76 | 72.61 | 5 / 3 / 0 |
| [Large, all engines EXTERNAL](chart.html#public-large-external) | Drain | 89.70 | 72.92 | 119.88 | 301.41 | 5 / 0 / 0 |

Stock profiles use the original runner counts and default storage. The additional dense and large comparisons identify the changed runner counts or PostgreSQL argument storage.

The five stock profiles cover 35,040 measured jobs and 70,080 independent application commits for the selected Mesh policy and its three peers. Warmups, the alternate short policy, and deployment sensitivities are additional runs.

All four engines reached 64 simultaneous jobs between their first and second committed writes in every slow rotation. The canonical slow workload allowed 68 PostgreSQL connections.

In Dense, 8 × 8, Apalis group 2 settled its final job, `job_001009`, 25.92 s after the last application write committed. The observer recorded one job executing and all others terminal in 169 samples spanning 25.67 s. The rates and paired wins include this run.

## Public large jobs with the file-session producer

5 matched rotations on CPUs 0 and 1. Each lane completed 64 jobs and committed two ordered application writes per job. The file lane and both inline duplicates used the same runner and broker ELFs. Public stock used the same runner ELF and its stock broker. [Interactive results](chart.html#public-large-file-session)

Elapsed time in milliseconds. Cells show median (min–max).

| Lane | First submit to enqueue commit | Submit to last settlement | Enqueue commit to last settlement |
|---|---:|---:|---:|
| Mesh file | 1506.15 (1462.08–1581.29) | 2207.51 (2113.27–2245.72) | 664.43 (651.18–723.39) |
| Mesh inline A | 1805.74 (1791.91–1879.43) | 2450.55 (2424.11–2543.21) | 658.65 (618.36–691.83) |
| Mesh inline B | 1816.52 (1769.41–1872.76) | 2502.47 (2423.34–2578.87) | 683.88 (649.41–706.11) |
| Public stock Mesh | 1776.66 (1771.11–1879.45) | 2472.69 (2412.09–2575.59) | 677.95 (635.82–696.14) |
| Go River | 2235.09 (2214.02–2261.20) | 3085.32 (3049.35–3213.71) | 850.24 (835.13–952.51) |
| Elixir Oban | 1969.86 (1914.72–2250.24) | 2467.80 (2450.78–2859.58) | 506.30 (497.94–609.34) |
| Rust Apalis | 1494.77 (1446.38–1643.14) | 1820.20 (1614.35–26775.85) | 205.21 (167.97–25282.02) |

Each cell below gives the median paired difference in milliseconds and the count of rotations where file was faster. Positive differences favor file.

| Reference | First submit to enqueue commit | Submit to last settlement | Enqueue commit to last settlement |
|---|---:|---:|---:|
| Mesh inline A | +290.17 (5/5) | +288.09 (5/5) | -5.78 (1/5) |
| Mesh inline B | +332.40 (5/5) | +298.55 (5/5) | -10.50 (2/5) |
| Public stock Mesh | +309.03 (5/5) | +311.50 (5/5) | +1.70 (3/5) |
| Go River | +715.23 (5/5) | +936.08 (5/5) | +185.81 (5/5) |
| Elixir Oban | +466.78 (5/5) | +304.75 (5/5) | -153.25 (0/5) |
| Rust Apalis | +31.74 (3/5) | -387.31 (1/5) | -483.79 (1/5) |
| Faster inline duplicate | +290.17 (5/5) | +288.09 (5/5) | -10.50 (1/5) |

Within-rotation absolute difference between the two inline controls, in milliseconds.

| Rotation | First submit to enqueue commit | Submit to last settlement | Enqueue commit to last settlement |
|---|---:|---:|---:|
| 1 | 76.44 | 132.86 | 56.42 |
| 2 | 6.84 | 37.88 | 31.04 |
| 3 | 22.50 | 27.22 | 4.72 |
| 4 | 2.01 | 5.93 | 7.94 |
| 5 | 62.91 | 37.15 | 25.75 |

Apalis's final settlement in rotation 4 came 25.11 s after its last application commit. The measured comparison includes that rotation.

The original SETUP export failed. The controller recovered the same native SETUP archive. The first TIMING attempt failed before the file and duplicate inline adapters could launch and yielded zero qualified rotations. All scores above use the released path-only retry. [Execution orders, per-arm clocks and evidence digests](scores.json)

## Public large-file producer with symmetric EXTERNAL storage

5/5 matched rotations. Seven lanes complete the same 64 jobs with 512 KiB payloads per rotation on CPUs 0 and 1. Every lane uses eight 8-slot runners and EXTERNAL payload storage. Mesh verifies 4 MiB work_mem. Rates and matched throughput changes are medians. [Interactive results](chart.html#public-large-file-external)

| Lane | Submit jobs/s | Drain jobs/s | FILE paired submit change (wins) | FILE paired drain change (wins) |
|---|---:|---:|---:|---:|
| Mesh FILE | 46.96 | 97.10 | — | — |
| Mesh inline A | 37.50 | 92.88 | +28.48% (5/5) | +5.28% (3/5) |
| Mesh inline B | 37.92 | 94.97 | +23.47% (4/5) | +0.22% (3/5) |
| Public stock Mesh | 37.58 | 98.05 | +21.95% (5/5) | -0.97% (2/5) |
| Go River | 27.56 | 73.96 | +75.22% (5/5) | +31.28% (4/5) |
| Elixir Oban | 38.48 | 125.96 | +20.32% (5/5) | -25.16% (0/5) |
| Rust Apalis | 73.89 | 324.61 | -37.69% (0/5) | -72.67% (0/5) |

Against the faster same-broker inline control per clock, FILE saved a paired median 293 ms to enqueue commit and 320 ms from submit through settlement. It won 5/5 producer comparisons and 4/5 full submissions. Oban drained sooner in all five rotations. [Durable application-write intervals](../2026-11-25-large-external-write-progress/README.md)

The original grant parser failed before staging. These five rotations came from the released [retry](../2026-11-25-public-large-file-external/grant-parser-retry/RESULTS.md). [Original failure](../2026-11-25-public-large-file-external/evidence/native/README.md)

## Original V1 and protected workloads

The five workloads below use the original trusted Mesh V1 runner and its protected Mesh control. Each completed rotation contains both Mesh modes and all three stock peers on the same Linode. Rates are median jobs per second. The last column counts the rotations in which trusted Mesh was faster than each peer.

## Submission through last settlement

| Workload | Rotations | Protected Mesh | Trusted Mesh | Oban | River | Apalis | Trusted wins vs Oban / River / Apalis |
|---|---:|---:|---:|---:|---:|---:|---|
| Dense no-op | 5/5 | 174.69 | 358.77 | 648.15 | 843.91 | 1,139.36 | 0 / 0 / 0 |
| Short | 5/5 | 200.58 | 442.10 | 280.80 | 719.93 | 843.41 | 4 / 1 / 0 |
| Slow | 5/5 | 163.20 | 243.52 | 245.29 | 286.93 | 344.19 | 1 / 0 / 0 |
| Sparse | 5/5 | 2.64 | 2.66 | 2.04 | 2.08 | 2.49 | 5 / 5 / 5 |
| Large · EXTERNAL | 5/5 | 41.04 | 48.63 | 37.19 | 27.54 | 73.98 | 5 / 5 / 0 |

## First enqueue commit through last settlement

| Workload | Rotations | Protected Mesh | Trusted Mesh | Oban | River | Apalis | Trusted wins vs Oban / River / Apalis |
|---|---:|---:|---:|---:|---:|---:|---|
| Dense no-op | 5/5 | 193.49 | 452.62 | 717.91 | 912.01 | 1,190.70 | 0 / 0 / 0 |
| Short | 5/5 | 224.74 | 593.21 | 319.70 | 782.92 | 918.83 | 5 / 2 / 0 |
| Slow | 5/5 | 206.13 | 364.41 | 321.21 | 391.57 | 445.15 | 5 / 1 / 0 |
| Sparse | 5/5 | 2.64 | 2.66 | 2.04 | 2.08 | 2.49 | 5 / 5 / 5 |
| Large · EXTERNAL | 5/5 | 76.80 | 106.61 | 116.07 | 70.62 | 331.07 | 1 / 5 / 0 |

## Public trusted runner on 128 short jobs

5/5 matched rotations. The same b3557273 executable and batch producer ran both Mesh policies. The candidate win columns count paired wins for the selected policy. The frozen rule requires grouped commits to win four pairs on each clock with no increase in median p99.

| Lane | Submit jobs/s | Drain jobs/s | Candidate wins from submit | Candidate wins from commit |
|---|---:|---:|---:|---:|
| Individual terminal commits (selected) | 570.01 | 713.61 | — | — |
| Grouped terminal commits | 591.24 | 715.56 | 1/5 | 2/5 |
| Oban | 277.41 | 313.31 | 5/5 | 5/5 |
| River | 561.11 | 613.50 | 3/5 | 3/5 |
| Apalis | 854.46 | 924.81 | 0/5 | 0/5 |

## Dense jobs with confirmed teardown and hot checkout

6/6 matched rotations. All ten arms use two 32-slot runners on the same two CPUs. The hot and checked checkout arms use the same Mesh executable and bulk producer. The separate writer comparison uses checked checkout throughout.

| Lane | Submit jobs/s | Drain jobs/s | Candidate wins from submit | Candidate wins from commit |
|---|---:|---:|---:|---:|
| Mesh · hot checkout | 747.37 | 844.06 | — | — |
| Mesh · checked checkout A | 659.93 | 755.10 | 6/6 | 6/6 |
| Mesh · checked checkout B | 674.92 | 759.01 | 6/6 | 6/6 |
| Mesh V1 · scalar producer | 532.38 | 741.66 | 6/6 | 6/6 |
| Oban | 741.75 | 827.67 | 4/6 | 5/6 |
| River | 1,008.06 | 1,090.44 | 0/6 | 0/6 |
| Apalis | 1,369.28 | 1,447.14 | 0/6 | 0/6 |

## Dense jobs with overlapping terminal writes

6/6 matched rotations. The zero-hold writer settles successful jobs while the dispatcher continues. Both writer-off controls use the same runner and bulk producer.

| Lane | Submit jobs/s | Drain jobs/s | Candidate wins from submit | Candidate wins from commit |
|---|---:|---:|---:|---:|
| Mesh · zero-hold writer | 665.89 | 749.11 | — | — |
| Mesh · writer off A | 690.55 | 775.52 | 1/6 | 1/6 |
| Mesh · writer off B | 689.79 | 772.98 | 1/6 | 2/6 |

Hot checkout cleared the prespecified threshold in 4/6 rotations and the zero-hold writer in 0/6. Advancement required four. Each qualifying rotation needed an earlier final application write and savings on each durable clock above both 5% of the faster control's elapsed time and the time difference between the two controls.

## Slow jobs with equal runner topology and a 71-session cap

8/8 matched rotations. All four engines run 512 jobs with two 32-slot runners and a 100 ms hold between independent application commits. Every engine shares two CPUs with PostgreSQL.

| Lane | Submit jobs/s | Drain jobs/s | Candidate wins from submit | Candidate wins from commit |
|---|---:|---:|---:|---:|
| Trusted Mesh V1 | 283.98 | 408.03 | — | — |
| Oban | 267.49 | 362.07 | 8/8 | 8/8 |
| River | 255.57 | 337.72 | 6/8 | 7/8 |
| Apalis | 353.32 | 461.60 | 0/8 | 0/8 |

| Engine | Peak overlapping committed holds, range | Peak physical PostgreSQL sessions, range |
|---|---:|---:|
| Mesh | 64 | 17 |
| Oban | 64 | 69 |
| River | 64 | 68–69 |
| Apalis | 64 | 69 |

Mesh returns its application connection during the hold. The three peer handlers retain theirs until the second write. Session counts cover the timed interval from submit through settlement. [Connection accounting and execution policy](../2026-11-24-ordinary-slow-equal-topology-cap71/README.md)

If Mesh kept each first application commit at its observed time and finished each job exactly 100 ms later, its finish time minus Apalis’s actual finish would range from +80.0 to +270.3 ms, measured from each engine’s enqueue commit. A positive value requires earlier first writes to match Apalis on drain.

## Slow jobs with two Mesh runners against stock peers

6/6 matched rotations. Each Mesh arm uses the same V1 bulk executable. The control copies give one runner 64 slots. The candidate gives two runners 32 slots each. All three peers use one 64-slot runner. Every arm shares two CPUs with PostgreSQL and has a 68-connection cap.

| Lane | Submit jobs/s | Drain jobs/s | Candidate wins from submit | Candidate wins from commit |
|---|---:|---:|---:|---:|
| Mesh V1 · 2 × 32 | 273.58 | 392.95 | — | — |
| Mesh V1 · 1 × 64 A | 261.53 | 362.39 | 5/6 | 5/6 |
| Mesh V1 · 1 × 64 B | 261.07 | 356.20 | 5/6 | 5/6 |
| Oban | 240.91 | 318.49 | 5/6 | 5/6 |
| River | 253.35 | 333.40 | 5/6 | 5/6 |
| Apalis | 342.61 | 443.12 | 0/6 | 0/6 |

## Dense jobs against peers using eight 8-slot runners

5/5 matched rotations. Both Mesh lanes use the same V1 runner and 64 total slots. The peer engines share the same CPU and PostgreSQL limits.

| Lane | Submit jobs/s | Drain jobs/s | Candidate wins from submit | Candidate wins from commit |
|---|---:|---:|---:|---:|
| Mesh V1 · 2 × 32 | 547.01 | 751.82 | — | — |
| Mesh V1 · 8 × 8 | 362.63 | 452.45 | 5/5 | 5/5 |
| Oban | 633.50 | 695.27 | 0/5 | 5/5 |
| River | 821.00 | 895.20 | 0/5 | 0/5 |
| Apalis | 1,113.61 | 1,161.49 | 0/5 | 0/5 |

## Dense jobs against peers using two 32-slot runners

5/5 matched rotations. Both Mesh lanes use the same V1 runner and 64 total slots. The peer engines share the same CPU and PostgreSQL limits.

| Lane | Submit jobs/s | Drain jobs/s | Candidate wins from submit | Candidate wins from commit |
|---|---:|---:|---:|---:|
| Mesh V1 · 2 × 32 | 545.94 | 766.91 | — | — |
| Mesh V1 · 8 × 8 | 359.43 | 446.38 | 5/5 | 5/5 |
| Oban | 780.50 | 880.61 | 0/5 | 0/5 |
| River | 1,002.35 | 1,091.82 | 0/5 | 0/5 |
| Apalis | 1,352.34 | 1,425.24 | 0/5 | 0/5 |

## Short jobs with set-based claims and grouped successes

5/5 matched rotations. The candidate uses set-based admission and can settle up to eight successful jobs in one transaction. The ordinary control uses stock admission and one terminal transaction per job.

| Lane | Submit jobs/s | Drain jobs/s | Candidate wins from submit | Candidate wins from commit |
|---|---:|---:|---:|---:|
| Trusted Mesh candidate | 502.74 | 691.00 | — | — |
| Trusted Mesh control | 437.54 | 585.93 | 3/5 | 3/5 |
| Protected Mesh | 199.68 | 223.55 | 5/5 | 5/5 |
| Oban | 298.04 | 335.35 | 5/5 | 5/5 |
| River | 715.12 | 772.55 | 0/5 | 2/5 |
| Apalis | 925.66 | 999.87 | 0/5 | 0/5 |

## Slow jobs with set-based enqueue

5/5 matched rotations. The first two lanes use the same executable with set-based enqueue enabled or disabled. The original V1 trusted runner and its protected control use their standard producer.

| Lane | Submit jobs/s | Drain jobs/s | Candidate wins from submit | Candidate wins from commit |
|---|---:|---:|---:|---:|
| Trusted Mesh · batch enqueue | 261.68 | 356.65 | — | — |
| Same binary · scalar enqueue | 250.39 | 366.52 | 5/5 | 1/5 |
| Original trusted Mesh | 251.24 | 365.48 | 5/5 | 0/5 |
| Protected Mesh | 164.77 | 209.05 | 5/5 | 5/5 |
| Oban | 247.27 | 331.48 | 4/5 | 5/5 |
| River | 288.66 | 387.57 | 1/5 | 1/5 |
| Apalis | 355.45 | 457.84 | 0/5 | 1/5 |

## Slow jobs with DEFAULT payload storage

5/5 matched rotations. All four engines use one 64-slot runner and two CPUs shared with PostgreSQL. Mesh uses set-based enqueue and individual terminal commits.

| Lane | Submit jobs/s | Drain jobs/s | Candidate wins from submit | Candidate wins from commit |
|---|---:|---:|---:|---:|
| Trusted Mesh | 260.33 | 359.31 | — | — |
| Oban | 242.41 | 314.17 | 5/5 | 5/5 |
| River | 252.97 | 337.72 | 3/5 | 3/5 |
| Apalis | 340.34 | 441.72 | 0/5 | 0/5 |

## Slow jobs with EXTERNAL payload storage

5/5 matched rotations. All four engines use one 64-slot runner and two CPUs shared with PostgreSQL. Mesh uses set-based enqueue and individual terminal commits.

| Lane | Submit jobs/s | Drain jobs/s | Candidate wins from submit | Candidate wins from commit |
|---|---:|---:|---:|---:|
| Trusted Mesh | 286.63 | 361.58 | — | — |
| Oban | 278.03 | 336.48 | 5/5 | 5/5 |
| River | 286.87 | 347.54 | 3/5 | 5/5 |
| Apalis | 401.49 | 452.36 | 0/5 | 0/5 |

## Slow jobs with grouped successes

2/5 matched rotations. This pilot stopped after two opposite orders. The V2 lanes can settle up to eight successes together. V1 keeps one terminal transaction per job. Both policies used the same 512-job input and 100 ms hold. [Extension decision](../2026-11-23-bulk-enqueue-v2-slow/README.md)

| Lane | Submit jobs/s | Drain jobs/s | Candidate wins from submit | Candidate wins from commit |
|---|---:|---:|---:|---:|
| V2 · batch enqueue | 260.90 | 363.53 | — | — |
| V2 · scalar enqueue | 254.10 | 383.25 | 2/2 | 0/2 |
| V1 · batch enqueue | 263.05 | 360.88 | 1/2 | 1/2 |
| V1 · scalar enqueue | 243.40 | 351.32 | 2/2 | 1/2 |
| Oban | 236.63 | 310.37 | 2/2 | 2/2 |
| River | 273.80 | 368.18 | 1/2 | 1/2 |
| Apalis | 342.64 | 450.95 | 0/2 | 0/2 |

## Dense jobs with hot PostgreSQL checkout

6/6 matched rotations. All engines use two 32-slot runners. Each Mesh lane runs the same executable with grouped successes.

| Lane | Submit jobs/s | Drain jobs/s | Candidate wins from submit | Candidate wins from commit |
|---|---:|---:|---:|---:|
| Mesh hot checkout | 757.42 | 859.78 | — | — |
| Checked A | 693.10 | 780.50 | 6/6 | 6/6 |
| Checked B | 691.05 | 776.14 | 6/6 | 6/6 |
| Oban | 762.02 | 854.02 | 2/6 | 3/6 |
| River | 1,001.10 | 1,083.72 | 0/6 | 0/6 |
| Apalis | 1,360.67 | 1,437.05 | 0/6 | 0/6 |

## Public dense jobs with hot checkout and individual settlement

6/6 matched rotations. Two 32-slot runners per engine complete 1,024 jobs on CPUs 0 and 1. The three Mesh lanes share the logger-off ELF and commit each terminal transition individually. Each job commits two ordered application writes. Rates and paired percentage changes are medians. [Interactive results](chart.html#public-dense-individual-hot-checkout)

| Lane | Submit jobs/s | Drain jobs/s | Hot paired submit change (wins) | Hot paired drain change (wins) |
|---|---:|---:|---:|---:|
| Mesh hot checkout | 756.56 | 857.00 | — | — |
| Mesh checked A | 687.87 | 772.46 | +10.12% (6/6) | +11.57% (6/6) |
| Mesh checked B | 684.05 | 772.62 | +9.85% (6/6) | +11.47% (6/6) |
| Go River | 1,006.19 | 1,083.52 | -24.96% (0/6) | -21.38% (0/6) |
| Elixir Oban | 765.81 | 863.85 | -0.09% (3/6) | +0.97% (4/6) |
| Rust Apalis | 1,404.75 | 1,479.38 | -47.06% (0/6) | -42.52% (0/6) |

Against the faster checked duplicate in each rotation, hot checkout gained a median 9.61% from submit and 11.25% from enqueue commit. Rotations 1, 3, 4, 6 cleared the preregistered margin. [Release receipts and raw replay](../2026-11-24-dense-individual-hot-pool/logger-off-retry/evidence/native/README.md)

## Dense experimental fixed8 and d388 against stock peers

5/5 matched rotations. Five fresh groups run 1,024 jobs with two 32-slot runners per engine on CPUs 0 and 1. The public-derived experimental Mesh ELF groups successful settlements in fixed8ms mode and uses logical-key grant SQL d388. Rates and matched changes are medians. [Interactive results](chart.html#dense-fixed8-logical-peers)

| Lane | Submit jobs/s | Drain jobs/s | Mesh paired submit change (wins) | Mesh paired drain change (wins) |
|---|---:|---:|---:|---:|
| Mesh fixed8 + d388 | 1,020.66 | 1,216.96 | — | — |
| Stock Go River | 1,002.75 | 1,083.14 | +2.11% (4/5) | +12.23% (5/5) |
| Stock Elixir Oban | 774.68 | 864.22 | +32.52% (5/5) | +41.19% (5/5) |
| Stock Rust Apalis | 1,384.17 | 1,458.74 | -25.61% (0/5) | -15.19% (0/5) |

Stock River opened application sessions during drain. Its sampled total rose from 7 idle connections to peaks of 67–69. Mesh started with 32 warmed application TLS sessions and held 41 total connections in the timed samples. A prewarmed River comparison is a separate sensitivity. The product ELF requires its own proof and timing.

The two reversed factorial pilot rotations passed their advancement gates. Their combined lane used the same image, grant SQL and launch inventory as the five fresh peer rotations. [Same-image pilot](../2026-11-25-dense-fixed8-logical-factorial/PILOT-RESULTS.md) · [Peer release](../2026-11-25-dense-fixed8-logical-factorial/evidence/native-retry/peers-release.json)

## Dense fixed8 and River connection readiness

6/6 matched rotations. Each lane uses two 32-slot runners for 1,024 jobs on CPUs 0 and 1. Rebuilt River lazy and prewarmed modes share an ELF. This sensitivity uses six fresh rotations. [Interactive results](chart.html#dense-fixed8-river-readiness) · [Original stock comparison](chart.html#dense-fixed8-logical-peers)

| Lane | Submit jobs/s | Drain jobs/s | Prewarmed River paired submit change (wins) | Prewarmed River paired drain change (wins) |
|---|---:|---:|---:|---:|
| Prewarmed Go River | 1,313.97 | 1,466.60 | — | — |
| Lazy rebuilt Go River | 1,002.52 | 1,079.50 | +31.16% (6/6) | +34.53% (6/6) |
| Stock Go River | 998.75 | 1,081.39 | +31.55% (6/6) | +36.25% (6/6) |
| Experimental Mesh fixed8 + d388 | 980.45 | 1,194.85 | +35.33% (6/6) | +22.76% (6/6) |

The sealed gate requires all six gaps to have the same sign and each absolute gap to exceed the larger of 5% of prewarmed River elapsed time and the absolute stock versus rebuilt-lazy River gap. The replay reports **Mesh deficit** from submit and **Mesh deficit** from enqueue commit.

| Group | Prewarmed River submit lead (ms) | Submit threshold (ms) | Drain lead (ms) | Drain threshold (ms) |
|---|---:|---:|---:|---:|
| 1 | 210.34 | 39.36 | 122.24 | 35.76 |
| 2 | 279.69 | 38.93 | 168.08 | 34.92 |
| 3 | 275.90 | 38.53 | 149.71 | 34.91 |
| 4 | 269.38 | 38.65 | 194.75 | 34.23 |
| 5 | 720.18 | 39.01 | 635.66 | 34.72 |
| 6 | 187.94 | 39.23 | 102.35 | 35.66 |

Paired elapsed-time differences, median (min to max) in milliseconds.

| Comparison | Submit | Drain |
|---|---:|---:|
| Time saved by warming the same River ELF | +241.358 (+217.835 to +276.158) | +243.539 (+204.746 to +277.431) |
| Frozen stock minus rebuilt-lazy River | +9.333 (-31.587 to +31.488) | +12.260 (-22.756 to +24.957) |

Mesh had **41 idle total sessions**, including 32 warmed application TLS sessions. Prewarmed River had **67 idle total sessions**, with 32 shared-pool sessions and one listener per runner, plus the observer. Stock and rebuilt-lazy River each started with seven idle sessions.

River runner launch to readiness took a median 56.4 ms in lazy mode and 314.7 ms in prewarmed mode. Warming added a paired median 267.002 ms to readiness and 29.466 ms from runner launch through settlement, including the common idle window. Mesh has no matching runner-launch timestamp. The cross-engine gates use the submit and drain clocks.

Group 5 remains scored. Mesh recorded 11.71% I/O wait and 0.00% CPU steal in that rotation. [Native result and independent release audit](../2026-11-25-dense-fixed8-river-readiness/NATIVE-RESULTS.md)

## Canonical public dense eight-runner fixed8 and d388

2/2 matched rotations. Each lane runs eight 8-slot runners for 1,024 jobs on CPUs 0 and 1. Mesh B uses the canonical public 4924b588 ELF with d388 grants, fixed8 grouped settlement and hot checkout. A1 and A2 use hot checked-individual settlement on the same ELF. Rates and paired changes are medians. [Interactive results](chart.html#public-dense-8x8-fixed8)

| Lane | Submit jobs/s | Drain jobs/s | Mesh B paired submit change (wins) | Mesh B paired drain change (wins) |
|---|---:|---:|---:|---:|
| Public Mesh B | 700.71 | 799.57 | — | — |
| Public Mesh A1 | 501.03 | 548.51 | +40.95% (2/2) | +47.00% (2/2) |
| Public Mesh A2 | 528.79 | 582.46 | +32.52% (2/2) | +37.27% (2/2) |
| Stock Go River | 827.42 | 891.32 | -15.30% (0/2) | -10.30% (0/2) |
| Stock Elixir Oban | 637.13 | 702.71 | +10.05% (2/2) | +13.85% (2/2) |
| Stock Rust Apalis | 1,158.38 | 1,211.75 | -39.44% (0/2) | -33.94% (0/2) |

B led Oban on both clocks in both rotations. River and Apalis led B on both clocks in both. The preregistered margin qualified Apalis's lead. The River and Oban comparisons remained inconclusive because the measured gaps fell below the duplicate-control spread in rotation 2.

The mirrored admission pilot used hot checked-individual duplicates A1/A2 and hot candidate B. Pilot C kept application checkout checks on B's grouped route and shared the ELF and warmed session count. Both pilot gates passed. The scored forward peer block came from the 06:04 exit-1 capture. The seven arms from its interrupted reverse block remain archived, including B's warmup flagged for overlapping apt-get. The 06:15:10 exit-0 continuation supplied the entire fresh reverse block. [Pilot and custody](../2026-11-25-public-dense-8x8/path-recovery/README.md) · [Peer clock and roster audit](../2026-11-25-public-dense-8x8/path-recovery/peer-stage-recovery/reverse-continuation/evidence/peers/clock-and-roster.json) · [Reverse continuation and retained failure](../2026-11-25-public-dense-8x8/path-recovery/peer-stage-recovery/reverse-continuation/README.md)

## Short jobs with combined settlement and refill

2/5 matched rotations. The initial two rotations use opposite orders. The three additional rotations require both fused copies to improve both clocks against their paired and public controls in each pilot rotation.

| Lane | Submit jobs/s | Drain jobs/s | Candidate wins from submit | Candidate wins from commit |
|---|---:|---:|---:|---:|
| Fused A | 574.62 | 687.04 | — | — |
| Fused B | 538.50 | 636.18 | 2/2 | 2/2 |
| Grouped control A | 569.81 | 683.52 | 1/2 | 1/2 |
| Grouped control B | 554.96 | 679.59 | 1/2 | 1/2 |
| Public grouped control | 599.03 | 723.29 | 1/2 | 0/2 |
| Oban | 264.47 | 301.68 | 2/2 | 2/2 |
| River | 472.81 | 501.65 | 2/2 | 2/2 |
| Apalis | 740.17 | 790.03 | 0/2 | 0/2 |

## Short jobs with stock and prewarmed River

6/6 matched rotations. Prewarmed River opened its PostgreSQL connections before submit. Cold and prewarmed River used the same executable.

| Lane | Submit jobs/s | Drain jobs/s | Candidate wins from submit | Candidate wins from commit |
|---|---:|---:|---:|---:|
| Mesh hot checkout | 591.59 | 728.85 | — | — |
| Mesh checked checkout | 577.07 | 684.50 | 4/6 | 4/6 |
| Stock River | 473.94 | 503.08 | 4/6 | 5/6 |
| River cold control | 472.13 | 501.32 | 4/6 | 4/6 |
| River prewarmed | 881.94 | 978.10 | 1/6 | 2/6 |
| Stock Oban | 278.86 | 316.04 | 6/6 | 6/6 |
| Stock Apalis | 827.62 | 887.74 | 1/6 | 1/6 |

## Short jobs with hot checkout and confirmed actor teardown

5/5 matched rotations. Both trusted runners use the same bulk producer and wait for teardown before submitting successes for settlement. Recent pooled connections skip the checkout probe in the candidate. Each job makes two independently committed application writes.

| Lane | Submit jobs/s | Drain jobs/s | Candidate wins from submit | Candidate wins from commit |
|---|---:|---:|---:|---:|
| Trusted Mesh · hot checkout | 626.59 | 762.06 | — | — |
| Trusted Mesh · checked checkout | 600.51 | 720.80 | 3/5 | 3/5 |
| Protected Mesh | 193.59 | 221.68 | 5/5 | 5/5 |
| Oban | 285.49 | 321.83 | 5/5 | 5/5 |
| River | 566.19 | 612.64 | 3/5 | 3/5 |
| Apalis | 808.73 | 874.10 | 0/5 | 0/5 |

## Short jobs with fewer PostgreSQL checkout probes

5/5 matched rotations. V2 runners use the original producer. Hot checkout sent two probes across 256 writes, versus 256 probes in each identical control. All writes used their own committed transactions.

| Lane | Submit jobs/s | Drain jobs/s | Candidate wins from submit | Candidate wins from commit |
|---|---:|---:|---:|---:|
| Mesh · hot checkout | 485.58 | 687.68 | — | — |
| V2 control A | 492.77 | 689.04 | 2/5 | 2/5 |
| V2 control B | 506.03 | 714.41 | 1/5 | 2/5 |
| River | 563.09 | 606.95 | 0/5 | 3/5 |

## Short jobs with bulk enqueue and fewer checkout probes

5/5 matched rotations. Both Mesh lanes share the V2 bulk producer. This experimental runner can report success before actor teardown. The comparison below it measures the separate exit-confirmed runner.

| Lane | Submit jobs/s | Drain jobs/s | Candidate wins from submit | Candidate wins from commit |
|---|---:|---:|---:|---:|
| Mesh · bulk and hot checkout | 650.47 | 788.43 | — | — |
| Mesh · bulk | 552.38 | 646.56 | 4/5 | 4/5 |
| River | 712.77 | 769.02 | 2/5 | 4/5 |

## Short jobs with bulk enqueue and all four engines

5/5 matched rotations. This run reuses the two experimental V2 builds and the stock River binary from the three-lane series above. Both V2 runners can settle before actor teardown.

| Lane | Submit jobs/s | Drain jobs/s | Candidate wins from submit | Candidate wins from commit |
|---|---:|---:|---:|---:|
| Mesh · bulk and hot checkout | 634.90 | 763.67 | — | — |
| Mesh · bulk | 600.04 | 716.20 | 3/5 | 4/5 |
| Protected Mesh | 208.39 | 234.67 | 5/5 | 5/5 |
| Oban | 309.54 | 352.13 | 5/5 | 5/5 |
| River | 473.74 | 504.50 | 5/5 | 5/5 |
| Apalis | 894.70 | 976.44 | 0/5 | 0/5 |

## Short jobs with confirmed actor teardown

5/5 matched rotations. Zero hold sends ready completions to a writer as soon as they are available. The writer-off control runs the same executable and also waits for actor teardown. The earlier bulk V2 runner could settle while teardown was still running.

| Lane | Submit jobs/s | Drain jobs/s | Candidate wins from submit | Candidate wins from commit |
|---|---:|---:|---:|---:|
| Mesh · zero-hold writer | 614.76 | 745.43 | — | — |
| Same runner · writer off | 597.33 | 727.77 | 3/5 | 4/5 |
| Earlier bulk V2 | 638.54 | 766.58 | 1/5 | 2/5 |
| Oban | 287.89 | 324.38 | 5/5 | 5/5 |
| River | 555.75 | 602.45 | 2/5 | 2/5 |
| Apalis | 871.13 | 957.50 | 0/5 | 0/5 |

## Short jobs with both producer and runner changes

5/5 matched rotations. Both Mesh lanes use the V2 grouped-success policy. Only the producer's set-based enqueue switch changes.

| Lane | Submit jobs/s | Drain jobs/s | Candidate wins from submit | Candidate wins from commit |
|---|---:|---:|---:|---:|
| Trusted Mesh · batch enqueue | 596.54 | 714.01 | — | — |
| Same binary · scalar enqueue | 498.16 | 700.18 | 5/5 | 4/5 |
| Oban | 308.13 | 353.14 | 5/5 | 5/5 |
| River | 697.01 | 754.41 | 2/5 | 2/5 |
| Apalis | 742.12 | 790.22 | 0/5 | 1/5 |

## Large jobs with equal PostgreSQL work_mem

5/5 matched rotations. All four engines use EXTERNAL argument storage and 16 MiB work_mem. Both extra Mesh controls use the same executable at 4 MiB.

| Lane | Submit jobs/s | Drain jobs/s | Candidate wins from submit | Candidate wins from commit |
|---|---:|---:|---:|---:|
| Trusted Mesh · 16 MiB | 47.54 | 105.85 | — | — |
| Same binary · 4 MiB A | 47.26 | 101.40 | 3/5 | 4/5 |
| Same binary · 4 MiB B | 43.67 | 101.65 | 5/5 | 4/5 |
| Oban | 36.89 | 116.70 | 5/5 | 1/5 |
| River | 27.33 | 72.61 | 5/5 | 5/5 |
| Apalis | 75.52 | 321.68 | 0/5 | 0/5 |

## Large jobs with one JSON encode per specification

5/5 matched rotations. The three Mesh lanes use the same V1 runner. The candidate producer retains each encoded specification across batch boundaries. All six lanes use EXTERNAL storage and 16 MiB work_mem.

| Lane | Submit jobs/s | Drain jobs/s | Candidate wins from submit | Candidate wins from commit |
|---|---:|---:|---:|---:|
| Mesh · encode once | 47.63 | 110.96 | — | — |
| Stock producer A | 47.79 | 115.93 | 3/5 | 3/5 |
| Stock producer B | 47.53 | 110.01 | 3/5 | 3/5 |
| Oban | 34.98 | 104.22 | 5/5 | 4/5 |
| River | 25.29 | 67.12 | 5/5 | 5/5 |
| Apalis | 76.19 | 334.72 | 0/5 | 0/5 |

## Public trusted execution

The db5937e7 product build waits for actor teardown before settling. The earlier V2 and initial completion-writer builds could settle before teardown. The product comparison reuses the frozen V1 producer and includes both native executable identities in the arm data. [Teardown witness](../2026-11-23-ordinary-completion-writer/combined/README.md#actor-teardown) · [Native product qualification](../2026-11-23-ordinary-trusted-product-db5937e7/README.md)

| Workload | Rotations | Product submit jobs/s | V1 submit jobs/s | Product drain jobs/s | V1 drain jobs/s |
|---|---:|---:|---:|---:|---:|
| Short | 1 | 456.34 | 383.80 | 614.56 | 509.53 |
| Dense | 1 | 366.31 | 359.95 | 462.60 | 448.68 |

Dense, short, slow and sparse use each engine's standard producer. Large uses the direct-file Mesh producer with both Mesh modes and PostgreSQL EXTERNAL argument storage on every engine. The sparse workload includes the prescribed gaps between arrivals. Every completed job has two independently committed application writes.

Protected Mesh executes a fresh process and fences its two application writes. Trusted Mesh runs the handler in the application's process with its database credentials. The five-workload comparison uses the V1 binaries. The short grouped-success comparison uses its own frozen V2 inventory. [Per-rotation chart](chart.html) · [Data, binary identities and raw digests](scores.json) · [Handler entry and the remaining backlog](handler-progress.md) · [Sparse per-job response times](sparse-latency.md) · [Slow-job slot use](slow-slots.md) · [How each engine groups durable completions](../2026-11-23-native-completion-batches/README.md) · [Original four-engine baseline](../2026-11-23-baseline/README.md)
