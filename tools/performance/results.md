# Linode performance results

The full Mesh run took 29 minutes and recorded **66 of 66 trials** with zero
runner errors. All four Mesh smoke trials passed before timing.
Mesh completed 6.80 million actor round trips per second with 64
pairs on four CPUs, compared with 2.85 million in the previous
full run. The scoring loop completed 3.51 million records per second
on one CPU. The HTTP measurement validated 693,021 replies and
counted 0 failed arrivals.

## Run provenance

| Results | Source commit | Start | Trial count |
| --- | --- | --- | ---: |
| Mesh current | `8112fada878032daf979eff415f4b57facdc0944` | 2026-11-18 20:51 UTC | 66 |
| Mesh previous, Elixir, Go, Rust/Tokio | `c1d546d6a97ff9cee9d930a7345e510ed18b703d` | 2026-11-18 15:41 UTC | 264 |

Both runs used the same two Linode Dedicated G6 instances in Washington, DC.
Each had four AMD EPYC 7713 vCPUs with one thread per core. The runner pinned
each subject to CPU 0 or CPUs 0–3. The load host sent HTTP traffic over the
private network using k6 1.6.1. The native Linux release builds used Mesh
at `-O2`, Rust 1.99.0, and LLVM 21.1.8. The earlier matrix used Go 1.26.4,
Elixir 1.20.4 with OTP 29.1.1, and Tokio 1.54.1.

The Elixir, Go, and Rust results come from the earlier matrix. Both runs use
the same fixtures and workload settings. Separate measurement windows can
affect cross-run differences. The tables show five-block medians with
minimum and maximum in brackets unless specified. [Workload contract](contract.md)

## Actor throughput

Throughput counts completed request and reply pairs, in millions per second.
The trial timed 160,000 round trips per pair after 100 untimed warmup
operations. Mesh used ordinary actor placement.

| Actor pairs · CPUs | Mesh current | Mesh previous | Elixir | Go | Rust/Tokio |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1 · 1 | 1.76 [1.74–1.76] | 1.50 [1.50–1.51] | 1.49 [1.42–1.51] | 1.96 [1.93–1.96] | 3.16 [3.15–3.18] |
| 64 · 1 | 2.40 [2.38–2.41] | 1.72 [1.71–1.74] | 1.55 [1.52–1.56] | 1.97 [1.96–1.97] | 2.99 [2.96–3.00] |
| 1 · 4 | 1.70 [1.70–1.72] | 1.27 [1.25–1.62] | 1.64 [1.53–1.66] | 1.80 [1.80–1.81] | 1.43 [1.42–1.47] |
| 64 · 4 | 6.80 [6.53–6.90] | 2.85 [1.73–2.95] | 1.59 [1.52–1.65] | 5.62 [5.57–5.63] | 8.49 [8.47–8.55] |

### Sampled actor latency

Each cell shows sampled median and p99 round-trip latency in µs. Each value
is the median of the corresponding percentile across five trials.

| Actor pairs · CPUs | Mesh current | Mesh previous | Elixir | Go | Rust/Tokio |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1 · 1 | 0.58 / 0.69 | 0.67 / 0.82 | 0.61 / 1.49 | 0.54 / 0.66 | 0.32 / 0.55 |
| 64 · 1 | 30.52 / 84.58 | 37.15 / 65.14 | 42.98 / 76.02 | 0.54 / 0.71 | 0.85 / 56.67 |
| 1 · 4 | 0.58 / 0.83 | 0.81 / 0.93 | 0.63 / 1.16 | 0.59 / 1.95 | 0.41 / 5.75 |
| 64 · 4 | 9.78 / 23.88 | 18.00 / 44.66 | 38.00 / 81.16 | 0.74 / 1.05 | 0.68 / 25.66 |

## CPU scoring and supervisor recovery

CPU scoring times 8 million records after reading and decoding the fixture.
Recovery times the failure request through the replacement's first valid
reply and checks that the sibling kept its state.

| Workload | Mesh current | Mesh previous | Elixir | Go | Rust/Tokio |
| --- | ---: | ---: | ---: | ---: | ---: |
| Scoring, one CPU (million records/s) | 3.51 [3.50–3.53] | 0.98 [0.97–0.98] | 0.78 [0.78–0.78] | 5.75 [5.74–5.76] | 7.68 [7.67–7.69] |
| Recovery, 1 CPU (µs) | 179.2 [167.3–356.2] | 934.7 [862.4–1073.1] | 48.3 [47.6–56.7] | 2.7 [2.4–44.7] | 30.6 [27.4–36.7] |
| Recovery, 4 CPUs (µs) | 409.3 [388.4–587.8] | 888.4 [359.9–1129.4] | 55.8 [52.5–89.6] | 3.4 [2.5–79.0] | 72.8 [54.8–142.0] |

## HTTP latency and service-level objective

Each row shows valid-reply p99 latency in ms with its block range, followed
by the number of blocks that met the service-level objective (SLO).
Each block has 15 seconds of warmup and 30 seconds of measurement. The SLO
requires p99 at or below 50 ms and fewer than 0.1% failed arrivals. The
runner counts errors, dropped arrivals, interruptions, and missing arrivals.
One TCP connection carries each request.

| Offered requests/s · CPUs | Mesh current | Mesh previous | Elixir | Go | Rust/Tokio |
| --- | ---: | ---: | ---: | ---: | ---: |
| 10 · 1 | 1.41 [1.38–1.60], 5/5 | 1.54 [1.43–2.05], 5/5 | 1.64 [1.59–1.79], 5/5 | 1.64 [1.61–2.56], 5/5 | 1.37 [1.28–1.52], 5/5 |
| 400 · 1 | 1.05 [0.97–1.11], 5/5 | 1.00 [0.98–1.04], 5/5 | 1.19 [1.18–1.23], 5/5 | 1.13 [1.05–1.16], 5/5 | 0.93 [0.91–0.95], 5/5 |
| 1,500 · 1 | 1.13 [1.06–1.16], 5/5 | 1.10 [1.03–1.28], 5/5 | 148.64 [107.76–204.85], 0/5 | 1.18 [1.13–1.20], 5/5 | 1.03 [1.01–1.04], 5/5 |
| 10 · 4 | 1.46 [1.38–1.67], 5/5 | 1.49 [1.39–1.87], 5/5 | 1.77 [1.58–1.89], 5/5 | 1.75 [1.69–1.86], 5/5 | 1.48 [1.32–6.89], 5/5 |
| 400 · 4 | 1.00 [1.00–1.10], 5/5 | 1.01 [0.98–1.20], 5/5 | 1.20 [1.18–1.28], 5/5 | 1.23 [1.18–3.28], 5/5 | 0.92 [0.90–0.96], 5/5 |
| 1,500 · 4 | 1.09 [1.06–1.12], 5/5 | 1.10 [1.06–1.17], 5/5 | 84.45 [1.40–156.40], 2/5 | 1.26 [1.23–1.27], 5/5 | 0.99 [0.97–1.05], 5/5 |

The current Mesh warmup validated 292,526 replies with zero failed arrivals.
Mesh met the SLO in all 30 HTTP blocks and the sustained trial. The grid
ends at 1,500 requests/s.

In the earlier matrix, Elixir recorded 6,782 dropped arrivals and 288
missing arrivals at 1,500 requests/s. The earlier Mesh, Go, and Rust
measurements recorded zero failed arrivals.

Host CPU sampling began 45 seconds after the current run started. It
covered almost all HTTP measurement windows. The load host had at least
66% median idle CPU in each fully sampled HTTP window.

### HTTP server CPU use and startup

Process CPU use is in cores during the 1,500 requests/s measurement.
Startup columns show the first successful health response's upper time
bound and the preceding probe window in ms. Each startup median combines
the 15 HTTP trials for that CPU budget.

| Metric | Mesh current | Mesh previous | Elixir | Go | Rust/Tokio |
| --- | ---: | ---: | ---: | ---: | ---: |
| Process CPU use · 1 CPU | 0.351 [0.346–0.354] | 0.402 [0.398–0.405] | 0.615 [0.606–0.620] | 0.449 [0.443–0.452] | 0.216 [0.213–0.217] |
| Process CPU use · 4 CPUs | 0.409 [0.408–0.417] | 0.724 [0.715–0.727] | 0.684 [0.654–0.730] | 0.710 [0.697–0.718] | 0.274 [0.270–0.276] |
| Startup bound / window · 1 CPU | 5.69 / 2.81 | 5.80 / 2.82 | 1205.19 / 6.60 | 5.87 / 2.88 | 5.70 / 2.73 |
| Startup bound / window · 4 CPUs | 5.88 / 2.84 | 5.88 / 2.93 | 930.51 / 7.46 | 6.13 / 3.01 | 5.73 / 2.79 |

## Five-minute HTTP trial

Each subject ran once at 400 requests/s on four CPUs. The memory figures
use Linux RSS samples taken every 250 ms. RSS change subtracts the first
measurement sample from the last.

| Runtime | Valid replies | p99 (ms) | CPU cores | Peak RSS (MiB) | RSS change (MiB) | SLO |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Mesh current | 120,000 | 1.00 | 0.141 | 6.14 | +0.00 | Pass |
| Mesh previous | 120,000 | 1.00 | 0.400 | 5.84 | +0.00 | Pass |
| Elixir | 120,001 | 1.20 | 0.163 | 147.83 | +0.05 | Pass |
| Go | 120,000 | 1.18 | 0.200 | 12.78 | +0.37 | Pass |
| Rust/Tokio | 120,001 | 0.94 | 0.081 | 3.78 | +0.16 | Pass |

## Reproduction

Run the Mesh executable built from the dated source with Rust and LLVM
versions above. Set `BENCH_LOAD_HOST` to the load host SSH destination and
`BENCH_SERVER_ADDRESS` to the server host private IPv4 address. Install k6
1.6.1 on both hosts for this smoke and load sequence. [Benchmark guide](README.md)

```sh
cargo build --locked --release -p meshc -p mesh-rt
mkdir -p .tmp/performance/bin
MESH_RT_LIB_PATH="$PWD/target/release/libmesh_rt.a" \
  target/release/meshc build tools/performance/mesh --opt-level 2 \
  --output .tmp/performance/bin/mesh

python3 tools/performance/run.py smoke --languages mesh \
  --bin-dir .tmp/performance/bin --out .tmp/performance/mesh-smoke \
  --require-k6 --k6-version 1.6.1

python3 tools/performance/run.py run --languages mesh \
  --bin-dir .tmp/performance/bin --out .tmp/performance/mesh-linode \
  --seed 26084 --blocks 5 --pairs 64 --actor-iters 160000 \
  --cpu-iters 8000000 --warmup-ops 100 \
  --load-host "$BENCH_LOAD_HOST" --server-address "$BENCH_SERVER_ADDRESS" \
  --port 18080 --http-rates 10 400 1500 --k6-version 1.6.1 \
  --warmup-seconds 15 --measure-seconds 30 \
  --sustained-rate 400 --sustained-seconds 300

python3 tools/performance/run.py report .tmp/performance/mesh-linode
```

The Mesh executable SHA-256 was
`086bad13a74a728bc4ff041d1bd6362bc3d3471670da0252cbd8d71d1767c809`.
The fixture SHA-256 was
`86cf426419cbce838ca6220da4eeda531ad3ee09dba4c37ff13a225f6ce5eb2f` in both runs.
