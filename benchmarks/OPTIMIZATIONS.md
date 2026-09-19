# Measured optimizations — 2026-09-19

Optimized JSON collection construction, map projections, collection formatting, module dependency sorting, replica ordering, and continuity snapshot creation. No runtime dependencies or public ABI changes. Existing working-tree changes were retained, including the edits in routing.rs.

## Results

Median of three independent process-run medians, in microseconds per operation. Each process performs warmup calls; graph/routing use seven calibrated batch samples and JSON/formatting use nine samples. Raw run medians, minima, maxima, and allocation observations are in [optimization-results.csv](optimization-results.csv).

| Operation | Size | Before (µs) | After (µs) | Speedup |
|---|---:|---:|---:|---:|
| Module sort (independent) | 4096 | 12,424.46 | 73.51 | 169.02× |
| Module sort (chain) | 4096 | 25,580.88 | 346.02 | 73.93× |
| Module sort (layered) | 4096 | 48,038.67 | 745.95 | 64.40× |
| JSON array parse | 2048 | 1,179.50 | 103.67 | 11.38× |
| JSON object parse | 2048 | 10,995.62 | 688.88 | 15.96× |
| JSON object encode | 2048 | 3,974.25 | 641.17 | 6.20× |
| Map keys | 2048 | 1,038.67 | 2.25 | 461.63× |
| Map values | 2048 | 1,051.12 | 1.92 | 548.32× |
| List → JSON | 2048 | 1,106.75 | 41.00 | 26.99× |
| JSON → List | 2048 | 1,102.67 | 42.42 | 26.00× |
| List formatting | 2048 | 2,124.08 | 202.67 | 10.48× |
| Map formatting | 2048 | 10,779.62 | 415.67 | 25.93× |
| Set formatting | 2048 | 2,155.67 | 212.67 | 10.14× |
| Replica selection | 512 | 1,296.15 | 140.22 | 9.24× |

Snapshot generation includes the in-memory SQLite read, serialization, chunk assembly, and hashing (seven samples per process). Database insertion and output verification are outside timing. It does not measure disk or network replication.

| Records | Chunk limit | Before (ms) | After (ms) | Speedup |
|---:|---:|---:|---:|---:|
| 100 | 4 KiB | 1.501 | 0.820 | 1.83× |
| 100 | 1 MiB | 11.648 | 0.820 | 14.20× |
| 1000 | 4 KiB | 15.137 | 8.828 | 1.71× |
| 1000 | 1 MiB | 691.458 | 8.572 | 80.67× |

These are operation microbenchmarks, not end-to-end compilation, HTTP throughput, actor scheduling, or production latency measurements. Small 16-element JSON cases are mixed: aggregate parse-array, parse-object, and encode-object medians changed from 1.042/3.500/2.750 µs to 1.125/4.000/3.041 µs. Across individual runs those ratios vary in both directions; timer granularity and desktop background work make small-case conclusions unreliable. The improvements above target repeated copying/scanning that grows with input size. No blanket claim of improvement for every workload is made.

At 2,048 entries, JSON object parsing requests 39,346,336 → 401,760 bytes from the system allocator; map formatting requests 122,777,600 → 208,890 bytes. Replica ordering at 512 candidates performs 22,055 → 11 allocations. Bytes count cumulative allocator requests, including full replacement sizes for reallocations and arena pages, **not peak memory or live object bytes**. Zero system bytes for optimized map projections means they fit in existing arena capacity, not that no Mesh list was allocated.

## Changes

- `mesh-common/module_graph`: index reverse dependency edges once. Preserve FIFO ready batches, alphabetical ordering within each batch, and cycle diagnostics.
- `mesh-rt/json`: reuse exact-capacity GC list builders for parsed arrays and list conversions; reuse the existing string-entry map constructor for parsed objects. Serde already handles duplicate object keys. Callback conversions remain in GC-managed storage and retain first-error propagation.
- `mesh-rt/collections/map`: use list builders for keys/values, preserving insertion order for every caller, including JSON encoders.
- `mesh-rt/collections/{list,map,set}`: append formatting text to one Rust String, then allocate the final Mesh string once. Preserve callbacks, Unicode, nesting, punctuation, and immutability.
- `mesh-rt/dist/routing`: borrow failure-domain/node strings in the existing sort key. Keep the same ordering and selection rules.
- `mesh-rt/dist/continuity_store`: serialize each snapshot record once instead of cloning and reserializing the growing chunk. Valid payloads, greedy boundaries, IDs, and checksums stay byte-for-byte equivalent. Oversized records now fail consistently in every position; the previous implementation sometimes emitted an oversized chunk after a flush. A live stack sample during the full performance gate led to this additional fix.

## Reproduction

Machine: Apple M4, macOS 26.5.2 (25F84), rustc 1.97.0 (2d8144b78 2026-07-07). Cargo bench optimized profile. Allocation-counting harnesses delegate to the System allocator, with counters disabled during timing. Baselines were compiled and copied before the respective implementation edits, using the same benchmark harness and toolchain. The checkout was dirty, so this compares implementations within the working tree rather than claiming an unmodified released revision. The starting diff was retained locally in `target/performance-initial.patch`.

```sh
cargo bench -p mesh-common --locked --bench module_graph
cargo bench -p mesh-rt --locked --features fuzzing --bench json_collections
cargo bench -p mesh-rt --locked --features fuzzing --bench collection_format
cargo bench -p mesh-rt --locked --features fuzzing --bench routing
cargo bench -p mesh-rt --locked --features fuzzing --bench snapshot
```

For before/after comparisons, build the same harness against each implementation and copy the emitted executable before rebuilding. Do not switch or reset a dirty working tree. This run saved executables as `target/performance/{module-graph,json-collections,collection-format,routing,snapshot}-{before,after}` and ran them serially three times, alternating before/after order on the second run. Our compilation/test jobs were stopped during measurement; this was a shared desktop with other development activity, not an isolated benchmark host.

Input construction, correctness assertions, and cleanup are outside timing. Allocation counting is enabled only in a separate untimed sample. `black_box` consumes results. JSON/formatting use the runtime’s global arena fallback and its existing `fuzzing` reset between samples; all Mesh pointers are discarded before each reset. Do not enable that reset in a production or concurrent process.

## Verification

Fresh verification after the snapshot change (local logs: `target/performance/verification/`):

| Check | Result |
|---|---|
| `cargo test -p mesh-common --locked` | 24 passed |
| Authoritative parser/typeck/codegen `--lib --locked` gates | 17 / 76 / 253 passed |
| `cargo build --workspace --locked` | Passed |
| Full runtime library suite, one test thread | 1,016 passed, 2 ignored |
| JSON and collection end-to-end filters | 6 + 1 passed |
| `proof autonomous-chaos` | Passed, five rounds |
| `proof fly-driver-conformance` | Passed, 14 tests |
| `proof continuity-soak --duration-seconds 10 --cycle-millis 10 --allow-short` | Smoke passed; not a 24-hour release proof |
| `proof autonomous-performance` | Passed all budgets on isolated retry |
| `proof docker-autoscaling` | Failed twice: burst/latency + scale-down first; managed-worker readiness on isolated retry |

The first performance attempt recorded 14,679 ms snapshot import (budget ≤10,000 ms) and 0.964 MiB/s (budget ≥1 MiB/s); all other assertions passed. Snapshot creation and snapshot import are different paths. The isolated retry passed with 3,391 ms snapshot import and 4.173 MiB/s (`target/proof/autonomous-performance/1789804117118/summary.json`). Budgets have not been changed.

The first Docker run returned 874/1,000 successful burst responses and 126 HTTP 503 reply timeouts. Burst p99 was 9.302 seconds (budget 6 seconds); failure-load p99 was 14.107 seconds (budget 10 seconds). Scale-down timed out with `consensus_commit_timeout` and incomplete load reports. Cleanup passed. Evidence: `target/proof/docker-autoscaling/1789803027547/summary.json`. No before-change Docker baseline was captured, so this failure cannot be labeled pre-existing or attributed to the optimization patch.

The isolated Docker retry also failed, at `proof_managed_worker_readiness_timeout`; its cleanup completed (`target/proof/docker-autoscaling/1789804307000/summary.json`). The full cluster release proof remains unresolved. Further investigation should measure distributed queue/transport delays, load-report freshness, and consensus commit timing; final first-run telemetry showed incomplete reports, dispatch timeouts, and queue rejections. No provider fencing, durability settings, or proof budgets were relaxed.

## Broader audit coverage and deferred candidates

The scan included dirty and clean compiler/runtime modules, parser/lexer, type checking, code generation, actors, HTTP/WebSocket/database/distributed paths, formatter, LSP, package tooling, registry, and website/editor surfaces. It was a pattern-driven audit, not an exhaustive profile of every workload. These further candidates remain unimplemented and unbenchmarked:

- Legacy PostgreSQL/SQLite query row lists still use repeated immutable append; bulk list construction needs row/duplicate-column/wire fixtures.
- Scheduler polling rebuilds its suspended vector; retention in place needs wakeup/concurrency measurements.
- Codegen monomorphization performs repeated function-name scans; type checking repeats resource-containment scans. Resource ownership/security semantics need dedicated regression evidence.
- LSP reparses/typechecks the full project per change; caching needs explicit invalidation tests.
- Formatter recursively remeasures nested groups; package resolution can fetch shared Git dependencies before deduplication.
- Registry version listings fetch historical README text that the response discards. Database profiling should establish the benefit.

No speculative frontend changes were made: the inspected request, highlighter, and search paths already use parallel requests, shared initialization, and debouncing.
