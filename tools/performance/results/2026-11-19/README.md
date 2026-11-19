# Full Mesh cloud benchmark

[Interactive chart](chart.html) · [Download the comparison as CSV](comparison.csv)

Mesh `25a991af` ran from 2026-11-19 16:01 to 16:30 UTC on the dedicated Linode G6 server in Washington, DC. The server and load generator each had four AMD EPYC 7713 vCPUs. Server CPU affinity was CPU 0 or CPUs 0–3. The hosts, fixture checksum, k6 version, CPU budgets and load settings match the recorded November 18 Elixir, Go and Rust/Tokio matrix at `c1d546d6`. The prior Mesh column uses the November 19 full matrix at `33cad8ed`. The peer and prior Mesh measurements were taken at different times.

Native Linux release build using Rust 1.99.0, LLVM 21.1.8, and Mesh optimization level 2. All four smoke trials passed. The complete matrix contains 66 trials. Actor and scoring figures are throughput. Recovery is the median time from sending the failure request until the replacement answers a state read. The public Mesh worker panics and issues a diagnostic, Elixir requests a supervised shutdown, and Go and Rust signal failure explicitly.

Cells are medians [minimum–maximum] across five shuffled blocks. The sustained trial is one five-minute measurement. HTTP values are per-block p99 for valid replies using one connection per request, with 15 seconds of warm-up and 30 seconds of measurement in short trials.

| Workload | Configuration | Unit | Mesh | Elixir | Go | Rust/Tokio | Prior Mesh |
|---|---|---|---:|---:|---:|---:|---:|
| Actor ping-pong | 1 CPU · 1 pair | M round trips/s | 1.73 [1.7–1.73] | 1.49 [1.42–1.51] | 1.96 [1.93–1.96] | 3.16 [3.15–3.18] | 1.75 [1.74–1.76] |
| Actor ping-pong | 1 CPU · 64 pairs | M round trips/s | 2.34 [2.33–2.35] | 1.55 [1.52–1.56] | 1.97 [1.96–1.97] | 2.99 [2.96–3] | 2.32 [2.3–2.34] |
| Actor ping-pong | 4 CPUs · 1 pair | M round trips/s | 1.7 [1.65–1.7] | 1.64 [1.53–1.66] | 1.8 [1.8–1.81] | 1.43 [1.42–1.47] | 1.7 [1.68–1.74] |
| Actor ping-pong | 4 CPUs · 64 pairs | M round trips/s | 7.05 [6.45–7.22] | 1.59 [1.52–1.65] | 5.62 [5.57–5.63] | 8.49 [8.47–8.55] | 6.5 [5.45–6.7] |
| CPU scoring | 1 CPU | M records/s | 7.12 [7.09–7.13] | 0.783 [0.78–0.784] | 5.75 [5.74–5.76] | 7.68 [7.67–7.69] | 6.99 [6.96–7.04] |
| Supervisor recovery | 1 CPU | µs | 26.1 [14.6–137] | 48.3 [47.6–56.7] | 2.72 [2.44–44.7] | 30.6 [27.4–36.7] | 229 [148–357] |
| Supervisor recovery | 4 CPUs | µs | 43.2 [25.4–47.8] | 55.8 [52.5–89.6] | 3.41 [2.46–79] | 72.8 [54.8–142] | 205 [180–374] |
| HTTP p99 | 1 CPU · 10 req/s | ms | 1.51 [1.41–1.63] | 1.64 [1.59–1.79] | 1.64 [1.61–2.56] | 1.37 [1.28–1.52] | 1.47 [1.32–1.76] |
| HTTP p99 | 1 CPU · 400 req/s | ms | 0.951 [0.945–0.987] | 1.19 [1.18–1.23] | 1.13 [1.05–1.16] | 0.927 [0.908–0.946] | 1.08 [1.07–1.09] |
| HTTP p99 | 1 CPU · 1,500 req/s | ms | 1.01 [0.989–1.04] | 149 [108–205] | 1.18 [1.13–1.2] | 1.03 [1.01–1.04] | 1.21 [1.07–1.24] |
| HTTP p99 | 4 CPUs · 10 req/s | ms | 1.43 [1.34–1.58] | 1.77 [1.58–1.89] | 1.75 [1.69–1.86] | 1.48 [1.32–6.89] | 1.61 [1.43–1.87] |
| HTTP p99 | 4 CPUs · 400 req/s | ms | 0.971 [0.929–0.988] | 1.2 [1.18–1.28] | 1.23 [1.18–3.28] | 0.921 [0.901–0.962] | 1.07 [0.999–1.15] |
| HTTP p99 | 4 CPUs · 1,500 req/s | ms | 1.01 [0.996–1.05] | 84.5 [1.4–156] | 1.26 [1.23–1.27] | 0.991 [0.968–1.05] | 1.12 [1.09–1.14] |
| Five-minute HTTP p99 | 4 CPUs · 400 req/s | ms | 0.949 | 1.2 | 1.18 | 0.943 | 1.08 |

The expected Mesh panic diagnostic appeared in 10/10 recovery trial stderr logs. Each CPU configuration has five recovery samples. Use these figures for median recovery comparisons. A recovery p95 needs a larger sample.

An earlier [focused recovery run on the same Mesh commit](recovery.md) measured 303 fresh processes per runtime on one worker and 101 on four. P95 was 39.4 µs for Mesh and 71.6 µs for Elixir on one worker, and 43.2 µs and 79.8 µs on four. The focused one-worker run pinned CPU 2. The full suite pinned CPU 0.

## HTTP correctness

SLO requires p99 ≤50 ms and fewer than 0.1% failed arrivals, including unsent load. Counts below cover measured blocks and the sustained trial.

| Runtime | Valid replies | Failed arrivals | SLO passes |
|---|---:|---:|---:|
| Mesh | 693,023 | 0 | 31/31 |
| Elixir | 685,955 | 7,070 | 23/31 |
| Go | 693,021 | 0 | 31/31 |
| Rust/Tokio | 693,021 | 0 | 31/31 |

## Host sampling

Server host had usable samples in 31/31 HTTP windows. The lowest window median CPU idle was 86.9%. Maximum observed CPU steal over the matrix was 0.2%.

Load host had usable samples in 31/31 HTTP windows. The lowest window median CPU idle was 66.0%. Maximum observed CPU steal over the matrix was 0.8%.
