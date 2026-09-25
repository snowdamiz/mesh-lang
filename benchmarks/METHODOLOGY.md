# Benchmark Methodology

## Goal

Compare HTTP throughput and latency of four language implementations of a minimal "Hello, World!" HTTP server:

- **Mesh** — this language (meshc 0.1.0 compiled with LLVM 21, mesh-rt built-in HTTP server)
- **Go** — Go 1.21.6, stdlib `net/http`
- **Rust** — stable toolchain (Feb 2026), axum 0.7 / hyper 1 / tokio
- **Elixir** — Elixir 1.16.3 / OTP 24 erts-12.2.1, plug_cowboy 2.8

## Hardware Topology

Two dedicated machines in one datacenter (Chicago), on a private network:

| Machine | Size | Purpose |
|---------|------|---------|
| `bench-servers` | 2 dedicated vCPU, 4 GB RAM | Runs all 4 language servers |
| `bench-loadgen` | 2 dedicated vCPU, 4 GB RAM | Runs `hey` load generator |

**Why two machines in one datacenter?**
- Dedicated CPUs eliminate CPU sharing between load generator and application servers
- An intra-datacenter private network gives sub-millisecond RTT, minimising network noise
- Dedicated (not burstable) CPUs make results reproducible

## Benchmark Tool

[**hey**](https://github.com/rakyll/hey) — a Go HTTP load testing tool supporting duration-based runs and IPv6.

```
hey -c 100 -z 30s -t 30 <url>
```

- `-c 100` — 100 concurrent connections (HTTP/1.1 keep-alive)
- `-z 30s` — run for 30 seconds
- `-t 30` — 30-second per-request timeout

## Protocol

HTTP/1.1 over the private network (IPv6 in the published runs). All servers bind `[::]:PORT` for dual-stack compatibility.

## Server Ports

| Port | Language |
|------|----------|
| 3000 | Mesh |
| 3001 | Go |
| 3002 | Rust |
| 3003 | Elixir |

## Endpoints

| Endpoint | Response |
|----------|----------|
| `GET /text` | `200 text/plain` — `Hello, World!\n` |
| `GET /json` | `200 application/json` — `{"message":"Hello, World!"}` |

## Measurement Procedure

For each language × endpoint combination:

1. **Warmup pass:** 30 seconds (`-z 30s`) — discarded. Matches timed run duration to ensure all JIT compilation, code-cache population, and OS-level TCP stack warmup complete before measurement begins.
2. **Timed runs:** 5 × 30-second runs (`-z 30s`). Run 1 is logged but excluded from the average (belt-and-suspenders warmup for runtimes that continue optimising past the warmup pass).
3. **Average req/s** across runs 2–5 reported.
4. **p50 / p99** from the last timed run's hey latency distribution.

Servers are benchmarked sequentially (Mesh → Go → Rust → Elixir) to avoid cross-interference.

## Memory Measurement

RSS (Resident Set Size) logged from `/proc/PID/status` (VmRSS field) on the server VM at 2-second intervals throughout the server lifetime. Values reported as peak across all observations.

Baseline values captured at server startup (before any load):

| Language | Startup RSS |
|----------|------------|
| Mesh     | ~4.9 MB    |
| Go       | ~1.5 MB    |
| Rust     | ~3.4 MB    |
| Elixir   | ~1.6 MB    |

## Servers Under Test

All servers implement identical logic: read the HTTP path, return a static text or JSON body. No database, no middleware overhead beyond what the framework requires.

**Mesh** (`benchmarks/mesh/main.mpl`):
```
HTTP.serve(router |> HTTP.on_get("/text", ...) |> HTTP.on_get("/json", ...), 3000)
```
Compiled to native binary by meshc (LLVM 21 backend), linked against mesh-rt.

**Go** (`benchmarks/go/main.go`):
```go
http.ListenAndServe("[::]:3001", mux)
```
Standard library net/http, `GOMAXPROCS = runtime.NumCPU()`.

**Rust** (`benchmarks/rust/src/main.rs`):
Axum 0.7 / hyper 1 / tokio with multi-threaded runtime.

**Elixir** (`benchmarks/elixir/`):
Plug + Cowboy (plug_cowboy 2.8), MIX_ENV=prod.

## Reproducibility

All server and load generator code is in `benchmarks/` and `benchmarks/two-machine/`. The two-machine setup can be reproduced on any provider or on your own hardware by following Option 2 in [README.md](README.md).

## Caveats

- All four language servers run co-located on the **same machine** (2 dedicated vCPUs, 4 GB RAM). Under sustained load they share these resources; reported throughput reflects realistic multi-tenant conditions, not the isolated peak each server could sustain alone.
- Mesh's first timed run for `/text` was 4,041 req/s (JIT warmup). It is excluded from the reported average (19,718 req/s); subsequent runs stabilised at ~19,500–20,000 req/s.
- Mesh p50/p99 are absent from the recorded results — the original run predated the latency parser fix in `two-machine/run-benchmarks.sh`; a fresh run will populate these values.

## Isolated Peak Throughput

The co-located results above reflect realistic multi-tenant conditions (all four servers sharing 2 vCPUs). To measure each server's actual peak throughput, a separate isolated run benchmarks each language alone on the server machine.

### Procedure

For each language (Mesh → Go → Rust → Elixir):

1. Start the server image with `LANG=<language>` and the isolated entrypoint — only that language's server starts.
2. Run the same benchmark protocol: 30s warmup + 5 × 30s timed runs, Run 1 excluded, runs 2–5 averaged.
3. Stop the server container before starting the next language.

This gives each server exclusive access to both dedicated vCPUs and full 4 GB RAM, eliminating cross-language interference entirely.

### Running Isolated Benchmarks

Build the images as in [README.md](README.md) Option 2. Then, for each language, start its server alone on the server machine and run the load generator against it:

```bash
# Server machine
docker run -d --name bench-servers --network host -e LANG=Mesh \
  bench-servers /app/benchmarks/two-machine/start-server-isolated.sh

# Load generator machine
docker run --rm --network host -e SERVER_HOST=10.0.0.10 bench-loadgen

# Server machine, before the next language
docker rm -f bench-servers
```

Results are printed to stdout in the same table format as the co-located runner.
