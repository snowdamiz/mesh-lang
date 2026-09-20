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

- **The main thread never collected**, so whatever `main` allocated stayed for the life of the program (`string_build` peaked at 705 MB). Fixed later; see "Lifting the remaining limits" below.
- The global process table is one `RwLock` taken about eight times per actor lifecycle and on every send; under `actor_ping` most kernel time is that lock's slow path. Sharding it is contained to `scheduler.rs` but touches about thirty sites.
- The worker loop re-polled every suspended actor on each cycle (a table read and a process lock apiece), which scales with idle actors. Fixed later, once it turned out to be a cliff rather than a slope; see "Lifting the remaining limits" below.
- `Map` is an insertion-ordered vector with linear `get`/`put`; an index would need to preserve iteration order.
- String interpolation with k parts performs k−1 `mesh_string_concat` calls, each allocating an intermediate.
- `meshc build` defaults to `--opt-level 0`, and the target CPU is `generic`.

# Cross-actor heap references — 2026-09-19

A memory-safety fix rather than an optimization, recorded here because the pass above changed how often the bug shows.

Each actor's collector scans only that actor's stack and heap, yet every path that hands a value to another actor passed pointers into the sender's heap:

- `spawn(actor, value)` packed its arguments into a buffer on the spawner's heap and passed pointer-typed arguments by reference, while the scheduler queued the raw buffer pointer where no collector could see it.
- `send(pid, value)` and `Timer.send_after` ship the message's own bytes, which for anything but a scalar are a pointer or a struct full of them.
- Service call and cast arguments, service replies, and `Job.async` / `Job.map` results did the same. A job's result is the sharpest case: the job actor exits the moment it has sent it.

Once the owner dropped its reference and collected, or exited, the other actor read freed, reused memory. Against the unmodified baseline a spawn reproduction that should print `payload-42` printed `garbage-` or `180052`, the eleven-shape message fixture segfaults, and the job fixture dies with SIGBUS before printing anything. For a plain `String` message the baseline happened to print the right answer 12 times out of 12, while the tree after the optimization pass above printed an empty line 8 times out of 12: hoisting the message slot let LLVM tail-call out of the sending frame, which removed a stale stack slot the conservative scan had been treating as a root, and exact-size bins recycle the freed block sooner. The defect was already there; that pass made it visible.

## Shape-guided copy

A value that crosses actors is now deep-copied, guided by its static type.

- **Lowering** (`mesh-codegen/mir/lower.rs`, `MsgShape`) works out what the value holds from the type checker's full type. It has to happen there: MIR erases `List<String>` to a bare pointer. Generic structs and sum types are instantiated, recursive types become cycles, and whatever a type cannot describe — a closure's environment, an opaque runtime handle or resource, an unresolved type variable — is marked `Shared`.
- **Code generation** (`mesh-codegen/codegen/msg_shape.rs`) maps the shape onto the value's real representation, using the same LLVM types the rest of codegen builds values with: field offsets, variant overlays, payloads that generic `Option`/`Result` box, collection elements (aggregates always boxed) versus tuple fields and argument slots (an aggregate of one word or less travels inline, a larger one in a box). The result is one constant `u32` table per send site.
- **The runtime** (`mesh-rt/actor/msg_shape.rs`) walks the table at send time, under the sender's process lock, and takes every reachable object out of the sender's heap (`capture`); the receiver rebuilds them in its own heap when it takes the message (`materialize`). Shared structure stays shared, and pointer chains are walked from an explicit stack, so a 200,000-cell list does not overflow a 512 KiB actor stack.

A wrong table cannot corrupt memory. Objects are copied verbatim by their `GcHeader` size, a slot is followed only when it holds the exact start of a live object in the sender's heap, and every offset is bounds-checked, so the worst a mistake can do is leave a value shared — which is what happened to every value before. Static string literals, the global arena and other actors' heaps fail that test and stay where they are.

What is `Shared` is **lent** instead: the owning heap treats it as a root (`ActorHeap::lend`) and the borrower pins the owner's process, so the heap stays mapped if the owner exits first. A message to oneself is never a loan, or a process would own itself.

| Path | Runtime entry point |
|---|---|
| `send` | `mesh_actor_send_shaped` |
| `spawn` arguments | `mesh_actor_spawn_shaped` (`adopt_spawn_args`) |
| `Timer.send_after` | `mesh_timer_send_after_shaped` — copied when the timer is set, not when it fires |
| service call / cast arguments, replies | `mesh_service_call_shaped`, `mesh_service_cast_shaped`, `mesh_service_reply_shaped` |
| `Job.async` / `Job.map` results | `mesh_job_async_shaped`, `mesh_job_map_shaped` |

The unshaped entry points remain for messages that hold no references and for Rust callers. Rust callers of `Scheduler::spawn` that pass a `Box` the entry function takes ownership of never match a GC object and are handed over untouched. The String-only `mesh_service_call_typed`, `mesh_service_cast_typed` and `mesh_service_reply_string` are replaced by the shaped ones; nothing else referred to them.

## Defects this exposed

Covering every shape end to end turned up bugs that had nothing to do with garbage collection. All of these fail on the unmodified baseline too:

- `receive` loaded only the first 8 bytes of a message, so a by-value struct or sum type wider than a word arrived truncated.
- A struct wider than a word passed to a service `call`/`cast` kept only its first field; it now travels in a box, like a tuple field of that size.
- Generic annotations on actor and service parameters (`items :: List<String>`) were resolved by name only and lost their arguments, so such programs failed type checking.
- Struct- and closure-typed actor parameters failed LLVM verification in the actor wrapper.
- `Timer.send_after` evaluated its arguments twice.
- A job returning a `String` crashed the caller: the runtime boxed every result, but `Ok(text)` expects the pointer itself. A job returning a struct came back through the wrong calling convention (the runtime calls `fn(env) -> i64`). Both `Job.async` and `Job.map` now go through the uniform-slot adapter `List.map` already used, and the compiler tells the runtime when the result word is a reference.

### Function values

A job that returned a closure crashed for a reason of its own, and so did the same closure in plain single-threaded code. A function-typed *value* had two representations: a closure `{fn, env}` where a closure literal was written, but a bare code pointer wherever the type alone decided — a pattern binding, a struct or variant field, a tuple element. Storing a closure and binding it back (`Some(f) -> f(9)`) therefore loaded one word of the two and called it without its environment, and a named function could not be passed to a `Fun(...)` parameter at all: it failed LLVM verification.

- `mesh-codegen/mir/types.rs` — a function type now always resolves to `Closure`; the flag that chose between the two is gone, along with the duplicate `resolve_range_closure`. `FnPtr` remains only as the type lowering gives a function it calls or hands to the runtime *by name* (`lower_callee`, `as_fn_item`), so direct calls, `spawn`, HTTP handlers and `List.map(xs, double)` compile exactly as before.
- `mesh-codegen/codegen/expr.rs` — a named function evaluated as a value is `{fn, null}`, and a closure call with a null environment calls `fn(args)` rather than `fn(env, args)`. That is the convention the runtime's higher-order functions already used, which is also why a closure literal with no captures still gets a non-null dummy environment. A named function stored through a uniform `i64` slot (`Map.put`, `List.append`) is stored as that closure value, matching how it is read back.
- Tuple literals are built by a synthetic `__mesh_make_tuple` call, and the rule that splits a closure into `(fn, env)` for runtime intrinsics fired on it: `(f, 5)` became the three-element tuple `[fn, env, 5]`. Tuple elements are values and are no longer split.
- An indirect `Call` through a closure-typed local (`41 |> f`) made the *compiler* panic; it is now a closure call.

`tests/e2e/fn_values.mpl` covers a named function and a closure passed, returned, captured, stored in `Option`, `Result`, a user variant, a struct, a tuple, a list and a map, bound by patterns, piped into, handed to the runtime and sent to an actor. The baseline compiler panics on it.

Two more turned up while probing that area, both also present on the baseline:

- **Closures as HTTP handlers and middleware** crashed the server on the first request (SIGBUS). The router stores an environment per handler and the server calls `fn(env, request)` when it is set, but `mesh_http_route*` and `mesh_http_use_middleware` never received one, so the compiler boxed the closure and passed the box as the function pointer. They now take `(fn, env)` like `mesh_ws_serve`, which the existing rule for runtime calls fills in: `(fn, env)` for a closure, `(fn, null)` for a named function. The environment is pinned for the life of the program (`pin_closure_env`), because a router is a Rust allocation no collector scans and is itself never freed. This was a tracked limitation: `e2e_route_closure_runtime_failure` asserted that a closure route fails at request time, and is now `e2e_route_closure_handler`, asserting that it serves.
- **Ordering strings, and with it the whole REPL.** `"a" < "b"` type-checks (String is `Ord`) and lowering always generates `Ord__compare__String` with `<`, but codegen had no case for it: `Unsupported binop type: String`. A build only failed if it used one, because unreachable functions are pruned; the REPL has no entry point to prune from, so *every* evaluation failed, even `42`. There is now a runtime `mesh_string_compare` and codegen for `<`, `>`, `<=`, `>=`. With evaluation working again three more REPL defects showed: results were labelled with the wrapper function's type (`<() -> Int at 0x2a>` for `1 + 41`), floats were read from the integer return register, and a `let` was stored as a top-level definition the compiler does not keep, so later lines could not see it (bindings are now replayed inside the wrapper). `mesh-repl` had no test that evaluated anything; it has one now.
- **Interpolating a value with no `Display`** (`"${compare(1, 2)}"`) failed with `Undefined variable 'to_string'`, lowering's last-resort call to a function that does not exist. It now says which type lacks the implementation. The error stays in codegen so unreachable code is still pruned before it can complain.
- **`Tuple.first` / `Tuple.second` / `Tuple.nth`** are declared `-> Int`, all an untyped `Tuple` can promise, so on a tuple holding anything else they were a type error (`expected String, found Int`) or returned the element's address. A call that knows the tuple's type now takes the element's type from it (`tuple_accessor` in `mesh-typeck`), and codegen decodes the slot by the tuple convention rather than the collection one. A tuple behind a parameter whose type is not yet inferred still reads as `Int`, as before. A nested tuple read this way used to be given a by-value slot although every tuple is a heap pointer; its type is normalised like other runtime results now.

## Ceilings

- **Lending retains.** A lent heap stays whole until the borrower's process is dropped, and two actors that lend to each other keep each other's process alive after both exit. Closures were the common case and are copied now (see below); what is still lent is what no type describes: opaque runtime handles and unresolved type variables.
- **The main thread did not collect** at this point. It does now; see below.
- `mesh_supervisor_start_child` kept the caller's argument buffer by pointer for later restarts. It now keeps its own copy, but the compiler never emits a call to it, so what the arguments *point at* is still the caller's: whoever gives dynamic children a language surface has to pass a shape, as `spawn` does.

## Verification

Every garbage-collection fixture builds at `--opt-level 2`. That matters: at the default `-O0` a dead stack slot still holds the value, the conservative scan keeps it alive, and the unfixed runtime passes. Conservative scanning also means an end-to-end run cannot be made to fail for every shape, so the shape tables themselves are asserted in `mesh-codegen` (a `String`, and a one-word struct inside a tuple, which fails if it is described as a box) and the copy in `mesh-rt` (scalars not mistaken for pointers, shared structure, a 200,000-cell chain, a deliberately wrong table).

| Fixture (`tests/e2e/`) | Covers |
|---|---|
| `actors_spawn_arg_survives_gc`, `actors_spawn_arg_relayed` | a spawn argument outlives the spawner's collections, and a relay that exits at once |
| `actors_spawn_arg_shapes` | closure, struct and one-word struct arguments |
| `actors_send_string_survives_gc`, `actors_send_shapes_survive_gc` | twelve message shapes: variants, tuples, structs, lists, maps, `Option`, `Result`, nested |
| `actors_timer_send_string_survives_gc` | `Timer.send_after` |
| `service_heap_values_survive_gc` | list, struct and one-word struct arguments, list and string replies, a cast |
| `job_heap_results_survive_gc` | thirteen job results including a closure, `Job.map` direct and piped |
| `fn_values`, `tuple_accessors_typed`, `string_ordering`, `stdlib_http_closure_handler` (not GC-related) | function values in every position, typed tuple accessors, string ordering, closures as HTTP handler and middleware; see above |

| Check | Result |
|---|---|
| `cargo build --workspace --locked` | Passed |
| `cargo test --lib --locked` for mesh-common / mesh-lexer / mesh-parser / mesh-typeck / mesh-codegen | 24 / 20 / 17 / 76 / 254 passed |
| `cargo test -p mesh-rt --lib --locked -- --test-threads=1` | 1,042 passed, 2 ignored |
| `cargo test -p mesh-repl --locked` | 46 passed, plus 1 doc-test (one added: the first to evaluate anything) |
| `cargo test -p meshc --locked --no-fail-fast`, all 28 targets | 27 passed in one run: `e2e` 366, `e2e_actors` 18, `e2e_concurrency_stdlib` 14, `e2e_supervisors` 6, `e2e_entrypoints` 8, `e2e_secret` 11, `tooling_e2e` 30, `e2e_cluster_declarations` 5, unit tests 63, and the remaining eighteen package, database, client, formatter and LSP targets. `e2e_stdlib` failed on one test, the one asserting that closure routes fail, and passes (109, 2 ignored) with that test updated |
| After the last runtime change: mesh-rt, `e2e`, `e2e_stdlib`, `e2e_actors`, `e2e_concurrency_stdlib`, `e2e_supervisors` | 1,042 / 366 / 109 / 18 / 14 / 6 passed |
| `rustfmt --check --edition 2021` on every touched file; `git diff --check` | Clean |

Two failures seen along the way were the environment's, not the change's, and are recorded because they will recur on a busy machine. `e2e_http_server_runtime` reached a different process: the server binds `[::]:18080`, and another program listening on `127.0.0.1:18080` answers first, so the test now takes a free port. `e2e_http_crash_isolation` once failed to link (`malformed archive ... libmesh_rt.a`) because something rebuilt the runtime archive while the linker was reading it; it passes on its own.

Two documented limits were left as they were at this point, and both are lifted below: the main thread never collected, and `Timer.send_after` to a *service* delivers a plain message rather than something a `cast` handler receives (`e2e_timer_service_cast_known_dispatch_limit` froze that). A third, noted beside `e2e_deriving_sum_type` — derived `Display`/`Eq` on variants that carry values — turned out to be stale, so the fixture now covers it.

# Waiting on the main thread, and a final re-run — 2026-09-20

Writing a benchmark for the message copy turned up a cost that had nothing to do with copying. `main` is not a coroutine, so when it waits for a service reply or a message it polls its mailbox, and it slept 10 µs after every miss. An OS takes 60 µs or more to honour a sleep that short, so every service call made from `main` cost about 85 µs, against 2.5 µs for the same call made from inside an actor.

- `mesh-rt/actor/mod.rs` — **`MainThreadWait`**: spin through the first 2,000 misses (a few hundred microseconds) and only then fall back to the 10 µs sleep. A reply from a running actor arrives within microseconds, so the common case never sleeps; a long wait costs one brief spin and then behaves as before. Used by both main-thread waits, `mesh_service_call_shaped` and `mesh_actor_receive`.

Median and minimum of seven alternating runs, in milliseconds, the same tree with and without the change:

| Program | What it does | Before med | After med | Median | Min |
|---|---|---:|---:|---:|---:|
| `service_call_main` | 100,000 service calls from `main`, integer arguments | 5,136.6 | 331.9 | 15.5× | 8.6× |
| `actor_send_payload` | the same with two heap strings in and one out per call | 4,588.8 | 374.3 | 12.3× | 14.7× |

Run on their own rather than under the harness, the two builds of `service_call_main` took 8.58 s and 0.17 s. The string-carrying version is within a tenth of a second of the integer one on the new build, so deep-copying three strings costs about a microsecond per call.

`actor_send_payload` uses calls rather than casts on purpose. A mailbox holds 1,024 messages and rejects the rest (`send` returns `2`), so a sender that does not wait for replies loses messages. A first version of the benchmark fired 300,000 casts, printed a total that changed from run to run, and that was the mailbox working as documented, not a bug.

## All programs, original baseline against the final tree

Same method as above, after every change on this page. The host was loaded throughout (load average between 8 and 24 on 10 cores), so absolute times are higher than in the first table and differences under about 15% are noise.

| Program | Before med | After med | Median | Min | Peak RSS before → after | Output |
|---|---:|---:|---:|---:|---:|---|
| `actor_gc_live` | 36,610.4 | 79.6 | 460× | 798× | 50.6 → 47.5 MB | same |
| `actor_alloc_long` | crashed (SIGBUS) | 107.5 | — | — | — → 8.0 MB | baseline crashed |
| `actor_send_payload` | crashed (SIGBUS) | 542.7 | — | — | — → 30.9 MB | baseline crashed |
| `service_call_main` | crashed (SIGBUS) | 362.8 | — | — | — → 11.9 MB | baseline crashed |
| `actor_alloc_parallel` | 1,736.9 | 250.1 | 6.95× | 13.0× | 54.1 → 10.7 MB | same |
| `actor_loop_sum` | 3,803.8 | 1,604.4 | 2.37× | 2.33× | 7.6 → 7.6 MB | same |
| `actor_alloc` | 380.2 | 187.2 | 2.03× | 2.17× | 17.1 → 8.0 MB | same |
| `string_build` | 964.3 | 591.1 | 1.63× | 1.78× | 969.3 → 704.8 MB | same |
| `actor_ping` | 3,435.6 | 2,762.2 | 1.24× | 1.37× | not comparable | same |
| `map_ops` | 416.5 | 399.2 | 1.04× | 0.95× | 150.1 → 152.9 MB | same |
| `list_pipeline` | 278.5 | 270.4 | 1.03× | 1.09× | 247.0 → 253.2 MB | same |
| `closure_call` | 930.5 | 952.3 | 0.98× | 1.09× | 7.4 → 7.4 MB | same |
| `fib` | 120.3 | 122.8 | 0.98× | 0.96× | 7.4 → 7.4 MB | same |
| `loop_sum` | 768.7 | 809.4 | 0.95× | 0.97× | 7.4 → 7.5 MB | same |

`closure_call` makes 200 million calls through a closure held in a parameter; it is there because every call through a function value now tests the environment for null, and it shows that test costs nothing measurable. The two service programs crash on the baseline for the reason `actor_alloc_long` does: the service's message loop leaked stack on every trip, and its 512 KiB actor stack ran out after about 32,000 calls (a 64 MB stack for `main` changes nothing). The last five rows are unchanged.

One thing found here, and fixed below: an idle worker thread backed off to sleeping 100 µs and then 1 ms between polls, and nothing woke it when an actor became ready, so the *first* message after a quiet spell could wait up to a millisecond.

# Lifting the remaining limits — 2026-09-20

Five limits the sections above recorded as left alone, and three defects found while lifting them — one of them hiding inside a limit that looked merely documented. A fourth, a miscompile with no connection to any of them, is recorded near the end.

## The main thread collects

`main` already allocated from a collectable heap of its own; it never collected because it is not a coroutine, so it had no recorded stack to scan and never reached a yield, which is where an actor collects.

- `mesh-rt/actor/stack.rs`, `actor/mod.rs` — the main process records its thread's stack base at start-up (`pthread_get_stackaddr_np`, `pthread_getattr_np`, `GetCurrentThreadStackLimits`; on a platform with none of these it simply keeps not collecting).
- `mesh-rt/gc.rs` — when the main heap is due, the allocator raises a thread-local flag instead of collecting. It must not collect there: runtime functions keep fresh objects in Rust-side temporaries that no scan can see. `mesh_reduction_check`, which compiled code already calls at every call site and loop back-edge, is a point where every live value is on the stack or in a register, so `main` collects there. The check's fast path gains one load of a flag in the thread-local it already reads.
- `MESH_GC_STRESS=1` makes every heap collect at every opportunity. It exists to find roots the collector cannot see, and found the defect in the next section on its first run.

Roots held outside any heap were audited rather than assumed. A closure environment is retained by three Rust-owned structures: the HTTP router (pinned when the route is registered), the streaming HTTP client (its callback runs on its own thread while the caller carries on; it now holds a loan for as long as the stream lasts), and the WebSocket server (whose caller is blocked for its whole life). Iterator adapters hold theirs in collected objects, and channels carry only `Int`.

| Program (all on `main`) | Peak RSS before → after | Median time before → after |
|---|---:|---:|
| `string_build` | 704.8 → 8.2 MB | 537.1 → 602.6 ms |
| `list_pipeline` | 253.2 → 10.7 MB | 246.3 → 290.4 ms |
| `map_ops` | 152.9 → 153.0 MB | 356.2 → 339.8 ms |
| `loop_sum`, `fib`, `closure_call` | unchanged | unchanged: 696.8 vs 678.9 ms CPU for `loop_sum` over fourteen alternating runs |

The 12–18% those two programs now spend collecting is what reclaiming their memory costs. `map_ops` does not shrink because each map it builds is 16 bytes larger than the last, so a freed block never fits the next request: a non-moving free-list allocator does not help that pattern, on `main` or in an actor.

## A `for` over an iterator wrote past its result list

`mesh_list_builder_push` wrote `data[len]` with no check. Every other producer sizes its builder exactly, but a `for` over an iterator cannot know how many results it will collect and started from capacity 0, so every element landed on whatever the loop body had just allocated. Stress mode showed it as a corrupted object header in the collector's sweep; without a sweep walking the list it only corrupted data (the baseline dies with SIGBUS on `tests/e2e/for_in_iterator_builds_result.mpl`). The builder now grows by doubling and `push` returns it, since it moves when it grows; the eleven Rust callers and the five places codegen emits it all keep what comes back.

The same shape of mistake — size a result from its inputs' length fields, then copy more than that into it — is possible anywhere a buffer is sized up front, so `tests/e2e/concat_large_list_and_string.mpl` audits the two other places that do it, `List.concat` and string interpolation. Both are correct. It is kept because an overrun does not show up in the value that overran: the fixture reads back an object allocated *after* each concat, which is where the damage lands.

## A test that spawned an actor the wrong way

Running the whole workspace, rather than each package's `--lib`, turned up `mesh-rt`'s `library_memory` failing. It fails identically on the previous commit, and the fix below makes it pass there too, so the defect is the test's alone and none of the work on this page is implicated: it has been broken since spawn arguments started being copied. Nothing noticed because `authoritative-verification` runs `cargo build --workspace` but no `mesh-rt` test at all, and this defect lives in an integration target that `--lib` does not build.

Spawn arguments are a block of one 8-byte slot per argument; the runtime copies `args_size` bytes for the child and keeps alive whatever the slot words point at. The test passed the 1 MB object *as* the block, declared it 8 bytes long, and had the actor read byte 8 — one past the copy, so it read the copy's neighbour, which was 0. Every real spawner, in codegen and in `dist/node.rs`, names the object with a slot; the test now does too. The runtime is unchanged: it cannot tell a short `args_size` from an honest one, which is exactly why `lend_words` keeps the pointed-to objects alive instead of guessing at sizes.

Worth doing separately: CI should run `cargo test -p mesh-rt --locked` (about six seconds beyond the build it already does). It is not added here because nothing in this tree can prove that suite green on Linux, and a red pipeline would be the worse trade.

## `Timer.apply_after`

`Timer.send_after` delivers a plain message, which a service's `cast` handler never sees: a service dispatches on a tag only its generated functions know. Rather than teach timers about one kind of receiver, there is now the general form, `Timer.apply_after(milliseconds, function)`, which runs a zero-argument function after the delay in an actor of its own. A delayed cast is `Timer.apply_after(5000, fn () -> Writer.flush(pid) end)`. `e2e_timer_service_cast_known_dispatch_limit`, which froze the old limit, is now `e2e_timer_apply_after_reaches_a_service_cast`.

## Waking a worker, and the cliff behind it

An idle worker slept 100 µs, then 1.1 ms, between looks at its actors, and `wake_process` was a no-op.

- Each process records the worker that runs it (a coroutine never leaves the thread that created it). Whoever makes a process Ready puts its PID on that worker's wake queue and unparks that worker, if it is parked. A first version unparked *every* idle worker and cut call throughput to 0.17×; waking one costs nothing measurable.
- A worker no longer looks at its blocked coroutines on every pass. They wait in a map until the wake queue names them. That polling, a table lookup and a lock per blocked actor per pass per worker, was a cliff: once `main` got briefly ahead of the workers (a cold start is enough), the backlog made every pass slower and the backlog longer. `actor_ping`, 1.3 s normally, was seen to run past 90 s on the first run of a fresh binary.
- About thirty places make a process Ready and not all of them say so (links, supervisors, shutdown), so each worker still sweeps its blocked coroutines, no more often than every millisecond and for at most about a tenth of its time. A path that forgets to notify costs latency, not liveness.

| | Before | After |
|---|---:|---:|
| Service call from `main` after 5 ms of quiet (`wake_latency`, mean of 300) | 103–122 µs | 5–11 µs |
| The same after 250 ms of quiet | 775–1,142 µs | 11–48 µs |
| `actor_ping`, six consecutive runs | 1.1–1.5 s, or more than 90 s | 0.80–1.04 s |
| `service_call_main` / `actor_send_payload` (median of seven) | 192.4 / 338.8 ms | 168.3 / 290.8 ms |

## Closures are copied, not lent

A closure's type says nothing about what it captured, so an environment could be described only where the closure is made, not where it is sent. Each environment now starts with a pointer to a shape table the compiler emits for it (null when no capture holds a reference), and the copier follows that table when it meets the new `CLOSURE` kind. Captures get their shapes from the full type of the captured variable, like message payloads. An environment that is not an object in the sender's heap, such as the `next` the HTTP server hands to middleware, is lent as before.

## Tuple accessors in generic helpers

`fn head(p) do Tuple.first(p) end` pinned `p` to the untyped `Tuple` and returned `Int`, so `head(("a", 1))` was a type error. An accessor applied to a value whose type is not known yet now says only what it needs — a tuple with at least that many elements (`Ty::tuple_row`, a row that unifies with any long-enough tuple and becomes that tuple once its tail is known) — and `head` generalizes like any other function. Lowering needed the other half: a function with unannotated parameters is checked once and lowered once per usage type, but inner expressions kept their type variables, so `Tuple.first(p)` inside `head` had no concrete type. Each specialization now lowers against its own substituted types (`specialize_types`).

Writing the "still true" note about computed indices turned it into the third defect. `Tuple.nth(t, i)` on `(1, "two", 3)` did not merely read as `Int`; it printed `4308377600`, the string's address, because the declared return type is what the slot gets read as. A computed index can land on any element, so the result type is knowable only when every element shares one — where the tuple's type is known, its elements are now unified with each other, `("a", "b", "c")` indexed by a variable is a `String`, and `(1, "two", 3)` is a type error at the call. A literal index is unaffected, and an index past the end still panics at run time, where the length is known.

What that does not reach is a computed index on a tuple whose type is *not* known at the accessor — `fn pick(p, i) do Tuple.nth(p, i) end` — which still reads as the declared `Int` and so still hands back an address. Closing it needs a constraint the type language cannot currently write: "a tuple, of any arity, all of whose elements are this type". A row fixes the arity of what it names, so it cannot say it, and the alternatives are both worse than the hole — rejecting every unannotated tuple parameter with a computed index, or narrowing `Con("Tuple")` so it no longer accepts a concrete tuple, which is what lets `Tuple.size(("a", 1))` and `Queue.pop` work at all. It is left as it was, deliberately, rather than half-fixed.

## Two modules exporting the same `pub fn`

Unrelated to the limits above, and the worst thing on this page. `Lowerer::qualify_name` leaves a `pub` function's name unqualified so importers can call it bare, and `merge_mir_modules` keys merged functions by that bare name, keeping the first it sees. Two modules exporting the same name collapsed into one symbol: the later body was dropped and every call site, `from B import f` included, ran `A`'s body while the type checker went on believing it had `B`'s signature. Where the signatures differed that reinterprets a value across types — an `Int` returned where a `String` was expected is a wild pointer — with no diagnostic anywhere.

Until `pub` symbols are module-qualified, `meshc` refuses the ambiguous program, naming every module that defines the symbol and saying why one of them would silently win (`reject_duplicate_pub_functions`, run after type checking so export keys are final). `compiler/meshc/tests/e2e_duplicate_pub_functions.rs` is the regression test.

## Still true

- `Tuple.nth` reads as `Int` where the tuple's type is not known at the accessor — an untyped `Tuple`, or an unannotated parameter with a computed index. For a tuple of references that `Int` is an address; see above for why closing it needs a constraint the type language cannot write.
- What no type describes is still lent rather than copied: opaque runtime handles and unresolved type variables.
- A monotonically growing object (`map_ops`) is not helped by collection.
- The periodic sweep stays: about thirty places make a process Ready and not all of them name the worker.
- The main thread's stack base is read through a platform call. macOS is verified by every run here; the Linux (glibc and musl) and Windows branches are compiled by the `compatibility-matrix` workflow but no run on this machine exercises them. A platform with none of the three keeps the old behaviour — `main` does not collect — rather than scanning a guessed range.

## Verification

Each lifted limit has a test that fails without it. Two needed care: the collector is conservative, so a dead stack slot can keep a value alive and an end-to-end run passes against a broken shape table — the tables are asserted in `mesh-codegen` instead; and the list-builder overflow only corrupts memory the collector later walks, so its fixture needs 200 elements (12 were not enough to reach past the following allocation) and `--opt-level 2`.

| Test | Fails without |
|---|---|
| `tests/e2e/for_in_iterator_builds_result.mpl` | the builder growing; the baseline dies with SIGBUS |
| `mesh-rt` `test_list_builder_push_grows_a_full_builder_instead_of_overrunning_it` | the same, directly |
| `tests/e2e/concat_large_list_and_string.mpl` | an overrun in `List.concat` or string interpolation, read through a later allocation |
| `e2e_timer_apply_after_reaches_a_service_cast` | `Timer.apply_after` (replaces the test that froze the old limit) |
| `mesh-rt` `a_closure_is_copied_through_the_table_its_environment_names`, `a_plain_function_or_a_foreign_environment_is_not_followed` | closures being copied, and foreign environments still being lent |
| `mesh-codegen` `test_closure_environment_describes_itself` | the environment's table pointer, which no run can observe |
| `tests/e2e/tuple_accessors_typed.mpl` (`head`, `label`) | tuple rows and per-specialization lowering |
| `mesh-typeck` `test_computed_tuple_index_needs_one_element_type`, `..._takes_the_shared_element_type` | a computed index handing back an element's address as an `Int` |
| `MESH_GC_STRESS=1` over `e2e`, `e2e_actors`, `e2e_supervisors`, `e2e_concurrency_stdlib`, `e2e_stdlib` | main-thread collection reaching a root it cannot see; this is what found the builder overflow |
| `mesh-rt` `library_memory` | an embedded host's spawned actor borrowing the call's managed input |
| `e2e_duplicate_pub_functions` | two modules exporting one `pub fn` collapsing into a single symbol |

| Check | Result |
|---|---|
| `cargo build --workspace --locked` | Passed |
| `cargo test --workspace --locked --exclude meshc` — **every** target, not each package's `--lib` | All 16 targets passed. Running all of them is what surfaced `library_memory`; `--lib` does not build it |
| within that: mesh-common / mesh-lexer / mesh-parser / mesh-typeck / mesh-codegen libs | 24 / 20 / 17 / 79 / 255 |
| within that: `mesh-rt` lib, `library`, `library_memory`; `mesh-repl` | 1,045 (2 ignored) / 1 / 1; 46 plus 1 doc-test |
| `cargo test -p meshc --locked --no-fail-fast`, all 31 targets | 672 passed in one run: `e2e` 368, `e2e_stdlib` 109 (2 ignored), `tooling_e2e` 30, `e2e_actors` 18, `e2e_concurrency_stdlib` 14, `e2e_secret` 11, `e2e_entrypoints` 8, `e2e_supervisors` 6, `e2e_cluster_declarations` 5, `e2e_duplicate_pub_functions` 1, unit tests 63, and the remaining package, database, client, formatter and LSP targets |
| The five actor-facing suites under `MESH_GC_STRESS=1` | `e2e` 368, `e2e_stdlib` 109 (2 ignored), `e2e_actors` 18, `e2e_concurrency_stdlib` 14, `e2e_supervisors` 6 |
| `cargo fmt --all --check`; `git diff --check`; `scripts/verify-no-sidecar-files.sh` on the diff range | Clean |

The `malformed archive ... libmesh_rt.a` link failure recorded in the previous section recurred twice during an early stress run, on `e2e_http_middleware_inferred` and `e2e_http_path_params`. It is the linker reading the 300 MB runtime archive while something rewrites it, and it is environmental: those two pass on their own, all eleven `e2e_http_*` tests pass together under stress, and repeating the whole stress pass with nothing else building gave `e2e_stdlib` 109 of 109. This filesystem is the reason it is reachable at all: exFAT has no hard links, so cargo copies rather than links whatever it uplifts — it says so itself, once per crate, as `hard linking files in the incremental compilation cache failed. copying files instead` — and copying a 300 MB archive leaves a window a concurrent linker can fall into.

## All programs, original baseline against the final tree

Same method as the earlier tables — seven alternating runs of each pair, warm-up discarded, median and minimum in milliseconds — against the same frozen pre-change compiler and runtime. Load average was about 3 on 10 cores, so differences under about 10% are noise.

| Program | Before med | After med | Median | Min | Peak RSS before → after | Output |
|---|---:|---:|---:|---:|---:|---|
| `actor_gc_live` | 13,513.9 | 27.8 | 486× | 489× | 51.3 → 47.6 MB | same |
| `actor_alloc_parallel` | 2,898.9 | 37.4 | 77.5× | 65.8× | 55.5 → 10.8 MB | same |
| `actor_alloc_long` | crashed (SIGBUS) | 53.5 | — | — | — → 8.1 MB | baseline crashed; final printed `4999999` |
| `actor_send_payload` | crashed (SIGBUS) | 257.8 | — | — | — → 8.7 MB | baseline crashed; final printed `1588890 1688890` |
| `service_call_main` | crashed (SIGBUS) | 174.5 | — | — | — → 12.0 MB | baseline crashed; final printed `5000249995 5000249995` |
| `actor_loop_sum` | 870.1 | 409.7 | 2.12× | 2.17× | 7.6 → 7.8 MB | same |
| `actor_alloc` | 115.0 | 56.2 | 2.05× | 2.03× | 17.1 → 8.1 MB | same |
| `string_build` | 520.0 | 363.7 | 1.43× | 1.41× | 969.3 → 8.2 MB | same |
| `actor_ping` | 963.5 | 938.2 | 1.03× | 0.92× | 107.7 → 334.3 MB | same |
| `fib` | 72.9 | 71.3 | 1.02× | 1.01× | 7.4 → 7.4 MB | same |
| `loop_sum` | 459.7 | 459.4 | 1.00× | 1.12× | 7.4 → 7.5 MB | same |
| `map_ops` | 233.9 | 234.6 | 1.00× | 0.99× | 150.0 → 152.9 MB | same |
| `closure_call` | 564.8 | 563.4 | 1.00× | 1.00× | 7.4 → 7.5 MB | same |
| `list_pipeline` | 150.2 | 179.5 | 0.84× | 0.82× | 247.0 → 10.8 MB | same |
| `wake_latency` | 1,804.5 | 2,022.9 | 0.89× | 0.90× | 7.8 → 7.9 MB | see below |

`wake_latency` spends 1.5 of its 1.8 seconds inside `Timer.sleep(5)`, which on `main` is a plain `std::thread::sleep` in both builds and which this machine overshoots by 0.6–0.7 ms a time. That is the whole column: re-run alternately on a quieter host both builds take 1.70–1.76 s, and a sleep-only version of the program (300 sleeps, no calls) takes 1.66–1.71 s either way. The number the program exists to measure is on its stderr, and it is 87–107 µs before against 6–11 µs after.

`list_pipeline` is the one real regression, and it is the trade named above: it now reclaims its garbage (247 → 10.8 MB) and pays 16–18% for it. `actor_ping`'s RSS varies from run to run between roughly 100 and 340 MB on both builds; it is 100,000 messages in flight, not a leak. The three crashed rows are the baseline's leaked message-loop stack, the same defect as `actor_alloc_long`.
