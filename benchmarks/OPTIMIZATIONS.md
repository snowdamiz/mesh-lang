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

# Language runtime and code generation — 2026-09-19

A second pass, aimed at compiled Mesh programs rather than individual runtime operations: the per-actor garbage collector and allocator, the reduction check the compiler inserts at every call and loop back-edge, and two code generation defects. No language semantics, public ABI, or `GcHeader` layout changed.

## Results

End-to-end wall time of whole Mesh programs built with `--opt-level 2`, in milliseconds. Median and minimum of seven runs per build, alternating before/after order each round, after one discarded warm-up per binary. Both builds must print identical output (`same`). Programs are in [lang/programs](lang/programs).

| Program | What it stresses | Before med | After med | Median | Min | Peak RSS before → after |
|---|---|---:|---:|---:|---:|---:|
| `actor_gc_live` | 3,000 live strings + 300,000 garbage strings in one actor | 20,253.6 | 59.3 | 341× | 465× | 50.6 → 47.4 MB |
| `actor_alloc_long` | one 5,000,000-iteration loop building a struct per trip | crashed (SIGBUS) | 131.8 | — | — | — → 8.0 MB |
| `actor_alloc_parallel` | eight actors allocating at once | 1,841.5 | 135.3 | 13.6× | 20.0× | 53.5 → 10.7 MB |
| `actor_alloc` | the same churn in one actor, restarted every 5,000 trips | 298.7 | 135.9 | 2.20× | 2.27× | 17.1 → 8.0 MB |
| `actor_loop_sum` | 300,000,000-trip arithmetic loop inside an actor | 1,473.9 | 668.2 | 2.21× | 2.19× | 7.6 → 7.6 MB |
| `string_build` | interpolation and concatenation, main thread | 1,272.6 | 722.2 | 1.76× | 1.79× | 969.3 → 704.8 MB |
| `actor_ping` | 200,000 spawn + send | 1,944.5 | 1,364.6 | 1.43× | 1.13× | not comparable |
| `loop_sum` | the same arithmetic loop on the main thread | 1,105.9 | 1,084.8 | 1.02× | 0.88× | 7.3 → 7.4 MB |
| `fib` | `fib(35)`, main thread | 173.2 | 173.3 | 1.00× | 0.97× | 7.4 → 7.4 MB |
| `list_pipeline` | `List.map`/`filter`/`reduce` over 100,000 elements | 347.9 | 349.9 | 0.99× | 1.00× | 247.0 → 253.1 MB |
| `map_ops` | 4,000-key `Map.put`/`Map.get` | 537.1 | 536.1 | 1.00× | 1.01× | 150.0 → 152.7 MB |

Read the last four rows as "unchanged". They run on the main thread, which was not a target, and their generated code is byte-identical between builds (`sum_to` was diffed). A separate twelve-run comparison using CPU time put `loop_sum` at 1.08 s vs 1.09 s and `fib` at parity; the 0.88× minimum above is noise. The benchmark host was a shared desktop with a load average between 20 and 110 on 10 cores throughout, so treat differences under about 15% as noise and do not read the absolute times as representative of an idle machine. `actor_ping` peak RSS ranged from 23 to 282 MB across runs of *both* builds because it depends on how many actors happen to be alive at once, so no memory claim is made for it.

`actor_alloc_long` is a correctness fix that happens to be measurable: the baseline overflows the actor's 512 KiB stack after roughly 30,000–100,000 iterations of any loop whose body builds a struct, variant, closure, tuple or field temporary. `actor_alloc` is the same workload restructured so the baseline survives.

## Changes

- `mesh-codegen/codegen/mod.rs` — **`hoist_static_allocas`**: every fixed-size `alloca` is moved to its function's entry block before verification. An `alloca` outside the entry block is a dynamic stack allocation that re-executes each loop trip and is released only on return, so tail-call loops, `for` and `while` leaked stack per iteration. Entry-block slots are also the only ones LLVM's mem2reg/SROA promote, so at `-O2` the loop in `actor_alloc` lost its per-trip `sub sp`, the stored-and-reloaded variant tag, and the `case` on a value known to be `Some`. An existing helper covered `let`/`if`/`match`/`receive` inside tail-call functions only; about fifty other sites were unprotected. At `-O0`, where every slot now belongs to the fixed frame, the two example projects' frames shrank in total and the largest growth in any compared function was 128 B (against a 512 KiB actor stack).
- `mesh-codegen/codegen/expr.rs` — **static string literals**: a literal is emitted as a constant `{ i64 len, [N x i8] }` in `MeshString` layout and used by address, instead of calling `mesh_string_new` (allocate + copy) on every evaluation. Safe because the runtime never mutates a string in place and the collector ignores pointers outside its own pages.
- `mesh-rt/actor/heap.rs` — **mark phase**: each page keeps a bitmap of object starts, set at bump-allocation time. Resolving a candidate word is now a binary search over pages plus a short backward bit scan, instead of walking the entire all-objects list for every word on the stack and in every live object (the cause of the 20 s run). Blocks are never split, merged or moved, so bits are only added.
- `mesh-rt/actor/heap.rs` — **free bins**: freed blocks are segregated into one bin per exact byte size below 256 B and one per power of two above, with a bitmap of non-empty bins, replacing first-fit over a single linked list. Reuse keeps its previous contract — full-capacity wipe, unchanged `header.size`, `alloc_exact` reuses equal sizes only, alignment honoured — and adds a bound: a reused block is at most 4× the request, so a 22-byte string no longer wipes and accounts for a freed 24 KiB list.
- `mesh-rt/actor/heap.rs` — **collection trigger**: reused blocks now count toward the trigger, and after each sweep the threshold becomes `max(256 KiB, 2 × live bytes)` with `total_allocated` reset to the exact surviving total. Previously reuse was free and the threshold fixed, which had two effects: under steady garbage churn every cycle recycled all freed blocks and then grew the heap by another 256 KiB (heap size grew with the square root of bytes ever allocated), and any actor whose live set exceeded 256 KiB collected at every yield.
- `mesh-rt/actor/heap.rs` — **lazy pages**: a heap allocates its first 64 KiB page on first use, so an actor that never allocates owns no heap memory.
- `mesh-rt/gc.rs`, `actor/stack.rs` — **allocation path**: the running actor's process handle is cached per thread and validated by PID, so `mesh_gc_alloc_actor` no longer takes the global process-table `RwLock`, hashes the PID and clones an `Arc` on every allocation. That lock is shared by every worker; it is what serialized `actor_alloc_parallel`.
- `mesh-rt/actor/mod.rs`, `actor/stack.rs` — **`mesh_reduction_check`**: the coroutine yielder and the reduction countdown share one thread-local, the fast path is kept small enough to inline, and `yield_current` is kept out of line so its register-saving prologue is not paid on every call. The main-thread path is unchanged in cost (one thread-local read).
- `mesh-rt/string.rs` — `mesh_int_to_string` formats into a stack buffer instead of allocating and freeing a `String`.

## Reproduction

Machine: Apple M4, macOS 26.5.2, rustc 1.92.0, LLVM 21. The baseline `meshc` and `libmesh_rt.a` were release builds of the working tree copied aside before any edit; the checkout was dirty, so this compares implementations within the working tree rather than against a released revision.

```sh
cargo build --release --locked -p meshc -p mesh-rt
bash benchmarks/lang/build.sh base <baseline meshc> <baseline libmesh_rt.a>
bash benchmarks/lang/build.sh new target/release/meshc target/release/libmesh_rt.a
python3 benchmarks/lang/run.py 7 base new
```

`MESH_RT_LIB_PATH` selects the runtime, so compiler and runtime changes can be measured separately. The first execution of a freshly built binary on macOS is scanned by the system and can take 0.4 s or more; the runner discards it.

## Verification

| Check | Result |
|---|---|
| `cargo test --lib --locked` for mesh-common / mesh-lexer / mesh-parser / mesh-typeck / mesh-codegen | 24 / 20 / 17 / 76 / 253 passed |
| `cargo build --workspace --locked` | Passed |
| `cargo test -p mesh-rt --lib --locked -- --test-threads=1` | 1,024 passed, 2 ignored (1,016 before; eight tests added) |
| `cargo test -p meshc --test e2e` | 363 passed |
| `--test e2e_actors` / `e2e_supervisors` / `e2e_concurrency_stdlib` / `e2e_result_error_propagation` | 10 / 6 / 14 / 1 passed |
| `--test e2e_stdlib` | 108 passed, 2 ignored |
| `--test e2e_entrypoints` | 8 passed on three isolated reruns; one 15 s run timeout when run straight after the suites above on the saturated host |
| `cargo test -p mesh-repl --locked` | 45 passed |
| Output equality, before vs after, all eleven benchmark programs | Identical |

Three existing heap tests were adjusted to the new contract rather than left to pass vacuously: the large-allocation test allocates once first (pages are lazy), the reuse test subtracts the freed block as a real sweep does (reuse is now accounted), and two code generation tests assert the static literal instead of the mere presence of the `mesh_string_new` declaration. The distributed proofs (`autonomous-chaos`, `docker-autoscaling`, soak) were **not** rerun for this pass.

## Found but not changed

- **The main thread still never collects, deliberately.** `main` is the usual spawner and sender, and values other than `String` still cross actors by reference (see the next section). The cost is visible above: `string_build` peaks at 705 MB.
- The global process table is one `RwLock` taken about eight times per actor lifecycle and on every send; under `actor_ping` most kernel time is that lock's slow path. Sharding it is contained to `scheduler.rs` but touches about thirty sites.
- The worker loop re-polls every suspended actor on each cycle (a table read and a process lock apiece), which scales with idle actors.
- `Map` is an insertion-ordered vector with linear `get`/`put`; an index would need to preserve iteration order.
- String interpolation with k parts performs k−1 `mesh_string_concat` calls, each allocating an intermediate.
- `meshc build` defaults to `--opt-level 0`, and the target CPU is `generic`.

# Cross-actor heap references — 2026-09-19

A memory-safety fix rather than an optimization, recorded here because the pass above changed how often the bug shows.

Each actor's collector scans only that actor's stack and heap, but two paths handed another actor a pointer into it:

- `spawn(actor, value)` packed its arguments into a buffer on the spawner's heap and passed pointer-typed arguments by reference, while the scheduler queued the raw buffer pointer where no collector could see it.
- `send(pid, value)` ships the message's own bytes, which for a `String` is the pointer.

Once the owner dropped its reference and collected, the other actor read freed, reused memory. A reproduction that should print `payload-42` printed `garbage-` or `180052` on the unmodified baseline for the spawn case. For the send case the baseline happened to print the right answer 12 times out of 12, and the tree after the optimization pass above printed an empty line 8 times out of 12: hoisting the message slot let LLVM tail-call out of the sending frame, which removed a stale stack slot the conservative scan had been treating as a root, and exact-size bins recycle the freed block sooner. The defect was already there; that pass made it visible.

## Changes

- `mesh-rt/actor/scheduler.rs` — **`adopt_spawn_args`**. When the argument buffer is an object on the spawner's GC heap it is copied for the child. Rust callers that pass a `Box` the entry function takes ownership of, and buffers in the global arena, never match and are handed over untouched. Every heap an argument word points into — the spawner's, or an ancestor's when the spawner passes on something it was itself given — lends those objects to the child (`ActorHeap::lend`): they are marked like stack roots until the child's process is dropped, and the child pins the owner's process so the heap stays mapped if the owner exits first. It is type-agnostic, so it holds for any argument shape. Arguments that point nowhere (integers, static string literals) borrow nothing and pin nothing.
- `mesh-rt/actor/mod.rs`, `mesh-codegen/codegen/expr.rs` — **`mesh_actor_send_typed`**. A `String` message is copied into the message and re-created on the receiver's heap, reusing the mechanism typed service calls already had. This adds one runtime symbol.

Lending is wrong for messages — a long-lived receiver would pin every sender's heap for good — so a message needs a real copy, and only `String` has one. **Any other heap value in a message is still shared by reference**: a sum type carrying a string prints an empty line on the baseline and `blank` after these changes, 8 runs out of 8 each. That needs a type-directed copy of message payloads and is tracked separately, as are `Timer.send_after` with a string message and nested strings in service calls. Only scalar and `String` actor parameters compile today (struct- and closure-typed ones fail LLVM verification in the actor wrapper), so `String` is the reference-typed spawn argument that occurs in practice.

Retention is the price of lending: a lent heap stays whole until the borrower's process is dropped, so a chain of short-lived spawners that each pass a heap value to the next pins every ancestor. Integer-only chains are unaffected.

## Verification

Three end-to-end fixtures (`tests/e2e/actors_spawn_arg_survives_gc.mpl`, `actors_spawn_arg_relayed.mpl`, `actors_send_string_survives_gc.mpl`) build at `--opt-level 2`. That matters: at the default `-O0` a dead stack slot still holds the value, the conservative scan keeps it alive, and the unfixed runtime passes. Driven as the harness drives them, the two spawn fixtures print `180049` and `20007` against the unfixed runtime and `payload-42` against the fixed one.

| Check | Result |
|---|---|
| `cargo build --workspace --locked` | Passed |
| `cargo test -p mesh-rt --lib --locked -- --test-threads=1` | 1,030 passed, 2 ignored (six tests added) |
| `cargo test --lib --locked` for mesh-parser / mesh-typeck / mesh-codegen | 17 / 76 / 254 passed |
| `cargo test -p mesh-repl --locked` | 45 passed |
| `cargo test -p meshc --test e2e_actors` | 13 passed (three added) |
| `--test e2e_supervisors` / `e2e_concurrency_stdlib` | 6 / 14 passed |
| `--test e2e_stdlib` | 108 passed, 2 ignored |
| `--test e2e` | 363 passed |

Spawning 200,000 actors is not slower than the original baseline with the extra work on the spawn path: 1,514 → 1,258 ms with two integer arguments and 1,796 → 1,611 ms with a heap string (median of seven alternating runs).
