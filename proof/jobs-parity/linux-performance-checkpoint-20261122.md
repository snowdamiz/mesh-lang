# Linux durable Jobs performance checkpoint · 2026-11-22

Work is paused with a verified opt-in named-effects runtime and partial
canonical comparisons. Ordinary `Jobs.start` keeps its existing path.
The retained runtime was built from `d8741576fcceae76b4ef93cfedd0805e414a8bfa`.
The compiler and runtime tree match after `35fa53ff`, which removed the
experimental scheduler and watchdog changes. The public warm path is
`Jobs.start_effects` with `Jobs.effect` on Linux GNU ARM64.

| Retained environment | Pin |
| --- | --- |
| Release ELF | `fe719b77c033f0c08e61138291eb119487c796476e23fe9fb544711dd50233c8` |
| Release runtime archive | `f99dbbdf5cb0fd53bf85aa9609dfce335d5fada770ae3217e24b02ec818ae4e6` |
| Guest base CPIO | `bdfda1a2a78378e57d2067412c2f296bb443d1f671757b80054fe33a8f67bc4b` |
| Guest kernel | `ee66670649107e2efe319a436c9b9caf84b093d471f73577d54292e5c081b80b` |
| Host and guests | Apple M5 Pro, 15 logical CPUs, macOS 27.0.1. Linux ARM64, 2 vCPUs, 2 GiB or 4 GiB for large. PostgreSQL 17.11, verified TLS and SCRAM. |
| Canonical comparison | Driver `e8281fea`, Oban Basic 2.24.2 Isolated, OTP 29.1.1, Elixir 1.20.4. Each Oban runner verified live `+S 2:2 +SDcpu 1:1 +SDio 1 +A 2`. |

## Completed canonical controls

These are medians over the recorded groups, with Mesh / Oban order in
each cell. Each pair used the same frozen input and physical cap. All
10,080 measured jobs reconciled two separate ordered writes. The shared
producer/observer used one backend per arm. Peaks include
`compiled_caller`. The series keeps arm-level host load and outliers.

| Profile | Complete pairs | Mesh throughput wins | Jobs/s | Runner CPU-ms/job | Sampled RSS, MiB | Max sampled workload connections |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Dense 8r/64 | 3 | 0 | 1,003 / 2,284 | 0.94 / 3.45 | 319 / 925 | 65 / 81 |
| Slow 1r/64 | 3 | 2 | 490 / 452 | 1.56 / 1.32 | 330 / 146 | 13 / 67 |
| Sparse 1r/1 | 2 | 2 | 2.87 / 2.03 | 8.68 / 21.15 | 28.7 / 115.8 | 5 / 4 |
| Short 1r/16 | 2 | 2 | 775 / 357 | 1.32 / 4.01 | 84.6 / 119.9 | 13 / 19 |
| Large 8r/64 | 2 | 2 | 293 / 211 | 8.58 / 72.54 | 245 / 999 | 65 / 81 |

Dense trails on throughput. Slow exceeds the 1.50× RSS limit. The
sparse Oban tree sampled four processes against the limit of two. Five
matched pairs per profile remain the gate for scoring. The dispatcher
stopped after `slow-03` exported a clean receipt. Resume at `sparse-03`
and retain all twelve completed groups.
[Resume and exact commands](../../target/jobs-perf-work/RESUME.md)
[Canonical partial receipt](../../target/jobs-perf-work/canonical/checkpoint-receipt.json)

The previous five-pair Linux diagnostics use the reconstructed
two-scheduler Oban launcher. An unscored process audit confirmed its
other emulator defaults. The original hard-cap guest receipts pin the
profile file but lack live flag evidence. The macOS ordinary path and
the historical Linux comparator remain separate series.
[Warm diagnostics](linux-warm-jobs-20261122.json)
[Preserved-profile diagnostics](linux-regression-screens-20261122.json)
[Emulator provenance](linux-oban-emulator-modes-20261122.json)

## Continue each lane from its checkpoint

- **Dense due work** in `target/dense-due-work`. The canonical
  same-binary pilot measured 1,068 / 901 / 1,133 / 2,464 jobs/s for
  control A / deferred probe / control B / Oban. The candidate reduced
  due calls from 415 and 401 to 236 while losing throughput to both
  controls. The next pending comparison is the reverse-order pilot.
  Scheduled and retry crossing, COMMIT loss, and workspace verification
  remain open for the patch. [Resume](../../target/dense-due-work/RESUME.md)
- **Slow runtime RSS** in `target/linux-queue-perf`. Candidate ELF
  `cfe7a9de98bf6c4fb4ff07fcd2341af5b4e6efea0ff9c100188132b41c8a060a`
  has three canonical warm groups with throughput wins and RSS ratios
  of 1.667, 1.811 and 1.608. Groups 04 and 05 remain unstarted. The
  held proof reached 64 workers at both effect boundaries and verified
  exact reaps and fallback. The package applies to an audited ELF and
  DSO set. The mapping check and advice lack atomicity across arbitrary
  application code. [Resume](../../target/linux-queue-perf/RESUME.md)
- **Large scalar and spill** in `target/large-context-quantum`.
  The 1 MiB quantum lost durable drain in both canonical rotations.
  Scalar projection beat Oban twice with mixed stock-control results.
  The separate TLS wire and cursor proof passed. The next open work is
  the component check for temporary I/O and claim COMMIT acknowledgement
  using the exact Rust projection, with timings for actual runtime
  acknowledgements still missing. [Resume](../../target/large-context-quantum/RESUME.md)
- **Large inline and scoped roles** in `target/large-claim-perf`.
  Both canonical owner-role rotations and the scoped replay failed the
  promotion gate. Scoped lease, capacity, ACL and stale-cursor checks
  completed on a disposable PostgreSQL 17.11 server. Resume the four
  pending ordinary and batch COMMIT cuts after correcting the verified
  TLS relay setup. Additive migration and mixed-version support remain open.
  [Resume](../../target/large-claim-perf/RESUME.md)

The [final five-cohort dense CPU attribution](../../target/dense-cpu-sampling/REPORT.md)
uses the diagnostic launcher. Sampled whole-guest busy medians were
3.264 versus 0.991 CPU-ms/job. The narrow due-like stack was 0.081
CPU-ms/job and the broad signature was 0.143. These nested signatures
lack statement IDs and profiler overhead remains inconclusive.
The [failed split-row screen](../../target/narrow-job-row/RESULTS.md)
lost to the faster stock arm in all five triplets. Its lower-load
median combined CPU penalty was 1.270 ms/job against 0.424 ms/job
stock A/A spread. Its SQL proof covered fresh diagnostic databases.

The retained source passed the workspace, compiled CLI, runtime, fuzz,
law and solver checks and the PostgreSQL 17.11, 18.7 and 19.0 matrix
at the recorded revisions. The compiled Linux public API proof on the
retained release runtime passed polling, LISTEN, deadline and reaping,
and retry after session retirement. Those checks apply to the retained
runtime. The experimental patches retain the narrower proof boundaries
listed above. [Verification evidence](../../docs/durable-jobs-parity-evidence.md)
