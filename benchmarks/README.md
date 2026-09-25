# Mesh HTTP Benchmark

For compiler and runtime operation benchmarks, and whole-program language benchmarks (`lang/`), see [Measured optimizations](OPTIMIZATIONS.md).

Compares HTTP throughput and latency of four language implementations of a minimal `Hello, World!` HTTP server: **Mesh**, **Go**, **Rust**, and **Elixir**.

See [RESULTS.md](RESULTS.md) for published numbers and [METHODOLOGY.md](METHODOLOGY.md) for the full measurement methodology.

## Directory Layout

```
benchmarks/
├── mesh/               Mesh HTTP server (main.mpl)
├── go/                 Go HTTP server (net/http)
├── rust/               Rust HTTP server (axum + tokio)
├── elixir/             Elixir HTTP server (plug_cowboy)
├── two-machine/        Two-machine images (any provider)
│   ├── Dockerfile.servers  Server image (builds all 4 languages)
│   ├── Dockerfile.loadgen  Load generator image (hey)
│   ├── start-servers.sh    Server entrypoint
│   └── run-benchmarks.sh   Load generator entrypoint
├── run_benchmarks.sh   Local runner (wrk-based, approximate)
├── RESULTS.md          Published benchmark results
└── METHODOLOGY.md      Measurement methodology details
```

## Endpoints

| Endpoint | Response |
|----------|----------|
| `GET /text` | `200 text/plain` — `Hello, World!\n` |
| `GET /json` | `200 application/json` — `{"message":"Hello, World!"}` |

---

## Option 1 — Local Run (Quick / Approximate)

Runs all four servers on your local machine and load-tests them with `wrk`. Takes about 15 minutes.

**Results will differ from published numbers** because:
- Your hardware is different from the published runs' dedicated 2 vCPU / 4 GB machines
- All four servers share your CPU (published results use a dedicated server VM with no load generator co-located)
- The local script uses `wrk`; published results use `hey`

### Prerequisites

| Tool | Required for | Install |
|------|-------------|---------|
| `wrk` | Load generator | `brew install wrk` · `apt install wrk` |
| `go` | Go server | [go.dev/dl](https://go.dev/dl/) 1.21+ |
| `cargo` | Rust server | [rustup.rs](https://rustup.rs) (stable toolchain) |
| `mix` | Elixir server | [elixir-lang.org/install](https://elixir-lang.org/install.html) 1.16+ / OTP 24+ |
| `meshc` | Mesh server | see below |

Missing tools are detected at startup and those languages are skipped — you don't need all four.

**Installing meshc** (Rust required):

```bash
# From the repo root:
cargo install --path compiler/meshc
```

### Running locally

```bash
# From the repo root:
bash benchmarks/run_benchmarks.sh
```

What happens:
1. Each available server is built and started on its assigned port (Mesh :3000, Go :3001, Rust :3002, Elixir :3003).
2. For each language × endpoint: 10-second warmup (discarded), then 3 × 30-second timed runs.
3. A summary table is printed with req/s, p50, p99, and peak RSS per language.
4. All server processes are cleaned up on exit.

---

## Option 2 — Two-Machine Run (Reproduces Published Results)

Two dedicated machines (2 vCPU, 4 GB RAM each) on one private network, from any provider or on your own hardware. One machine runs all four servers; the other runs the `hey` load generator. This is the setup that produced the numbers in [RESULTS.md](RESULTS.md).

### Prerequisites

- **Two Linux machines** with Docker, reachable from each other on a private network
- **Docker with buildx** on the machine that builds the images — verify with `docker buildx version`

### Step 1 — Build the images

The server image builds `meshc` from source (Rust + LLVM 21). This takes **10–15 minutes** on the first build. Run from the repo root (the Dockerfiles need `compiler/` and `benchmarks/`); on Apple Silicon add `--platform linux/amd64`:

```bash
docker buildx build -f benchmarks/two-machine/Dockerfile.servers -t bench-servers --load .
docker buildx build -f benchmarks/two-machine/Dockerfile.loadgen -t bench-loadgen --load .
```

Copy each image to its machine (`docker save bench-servers | ssh server docker load`), or push both to a registry both machines can pull from.

### Step 2 — Start the servers

On the server machine:

```bash
docker run -d --name bench-servers --network host bench-servers
docker logs -f bench-servers
```

Wait for `=== All servers running ===`. All four servers must be ready before the load generator starts, or the benchmark reports `N/A` for servers that have not started yet:

```
Mesh ready on port 3000
Go ready on port 3001
Rust ready on port 3002
Elixir ready on port 3003
=== All servers running ===
```

### Step 3 — Run the load generator

On the load generator machine, point `SERVER_HOST` at the server machine's private address (an IPv6 address goes in brackets, `[fd00::10]`):

```bash
docker run --rm --network host -e SERVER_HOST=10.0.0.10 bench-loadgen
```

The benchmark runs sequentially (Mesh → Go → Rust → Elixir). For each language, it tests `/text` then `/json`, with a 30-second warmup followed by 5 × 30-second timed runs. Total run time is approximately 15–20 minutes. Progress looks like:

```
--- Benchmarking Mesh (port 3000) ---
  Endpoint: /text
  Warmup done. Running 5 timed runs...
    Run 1: 4041.23 req/s  p50=N/A  p99=N/A  [warmup — excluded]
    Run 2: 19914.11 req/s  p50=4.9 ms  p99=14.2 ms
    ...
```

When complete, the output ends with a formatted results table.

### Step 4 — Collect RSS memory data (optional)

Peak resident memory is logged by the server container throughout the run:

```bash
docker logs bench-servers | grep '^RSS,'
```

Each line: `RSS,<Language>,<unix_timestamp>,<VmRSS_kB>`. Take the maximum `VmRSS_kB` value per language and divide by 1024 for MB.

### Step 5 — Clean up

```bash
docker rm -f bench-servers
```

---

## Benchmark Parameters

| Parameter | Value | Notes |
|-----------|-------|-------|
| Connections | 100 | Concurrent HTTP/1.1 keep-alive connections |
| Warmup | 30s | Results discarded — ensures TCP stack and runtime caches are warm |
| Timed runs | 5 × 30s | Run 1 excluded from average (JIT/code-cache warmup) |
| Average | Runs 2–5 | 4 runs averaged for reported req/s |
| Latency | p50 / p99 | From the last timed run |
| Tool | `hey` | Go HTTP load tester; IPv6-capable |

To change parameters, edit `benchmarks/two-machine/run-benchmarks.sh`:

```bash
CONNECTIONS=100
WARMUP_DURATION=30
BENCH_DURATION=30
RUNS=5
DISCARD_RUNS=1
```

---

## Servers Under Test

All four servers implement identical logic: read the HTTP path, return a static body. No database, no middleware beyond what the framework requires.

| Language | Framework | Port | Source |
|----------|-----------|------|--------|
| Mesh | Built-in `HTTP.serve` | 3000 | `benchmarks/mesh/main.mpl` |
| Go | stdlib `net/http` | 3001 | `benchmarks/go/main.go` |
| Rust | axum 0.7 / hyper 1 / tokio | 3002 | `benchmarks/rust/src/main.rs` |
| Elixir | plug_cowboy 2.8 | 3003 | `benchmarks/elixir/` |

---

## Interpreting Results

- **Req/s** — higher is better. Published averages exclude Run 1 to eliminate cold-start artifacts.
- **p50 / p99** — lower is better. p99 shows worst-case latency tail.
- **Peak RSS** — lower is better. Memory footprint under sustained load.
- All four servers run on **one machine**. Results reflect co-located throughput, not each language's isolated maximum. See [METHODOLOGY.md](METHODOLOGY.md) for caveats.
