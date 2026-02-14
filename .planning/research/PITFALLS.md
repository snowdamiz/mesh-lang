# Domain Pitfalls

**Domain:** Mesher monitoring/observability SaaS platform -- dogfooding Mesh language for full backend, high-volume event ingestion, real-time streaming, alerting, multi-node clustering
**Researched:** 2026-02-14
**Confidence:** HIGH (Mesh runtime source analysis + monitoring domain research + dogfooding experience patterns)

---

## Critical Pitfalls

Mistakes that cause rewrites, data loss, or block progress for days.

---

### Pitfall 1: The Dual-Bug Problem -- Is It the App or the Compiler?

**What goes wrong:** When the Mesher application produces incorrect behavior, the developer cannot immediately tell whether the bug is in the application logic or in the Mesh compiler/runtime. A monitoring dashboard showing wrong event counts could be a SQL query bug, a Map iteration bug in Mesh, a codegen bug in the compiler, or a GC bug in the runtime. Every debugging session starts with a triage question -- "is this Mesh or is this me?" -- that doubles investigation time.

**Why it happens:** Mesher is the first large Mesh application. The compiler has zero *known* bugs, but zero known bugs means zero large-application testing. Every Mesh language feature has been validated with small E2E tests (typically <50 lines), never with a multi-file application doing concurrent database writes, WebSocket broadcasts, and actor supervision simultaneously. The interaction between features under real load will surface compiler/runtime bugs that isolated tests never trigger.

**Consequences:**
- Debugging time doubles because every bug requires ruling out compiler/runtime issues first
- Workarounds for compiler bugs leak into application code, creating technical debt that must be unwound when the compiler is fixed
- Developer morale drops when "simple" features take 3x longer due to unknown compiler limitations
- The project stalls if a critical compiler bug blocks a core feature (e.g., a codegen bug that prevents pattern matching on database query results)

**Prevention:**
1. **Minimal reproduction discipline:** When a bug is encountered, immediately create a minimal standalone `.mesh` file that reproduces the behavior in isolation. If the minimal file works, the bug is in the app. If it fails, the bug is in the compiler.
2. **Compiler bug journal:** Maintain a running log of every compiler/runtime issue discovered during Mesher development. Each entry: symptom, minimal repro, workaround, fix status. This becomes the v9.0 bug fix backlog.
3. **Fix compiler bugs immediately, do not work around them long-term.** A workaround in the app creates two problems: the workaround itself, and the eventual removal of the workaround. Short-term workarounds (<1 day) are acceptable. If a workaround would be more than a few lines, fix the compiler first.
4. **Never assume the compiler is correct.** Zero known bugs means "zero bugs found by small tests." Treat every unexpected behavior as potentially a compiler bug until the minimal repro proves otherwise.

**Detection:** Any behavior that works in a minimal file but fails in the multi-module Mesher project points to a compiler issue (likely in module system, name mangling, or cross-module monomorphization). Any behavior that fails even in a minimal file is definitely a compiler bug.

**Phase mapping:** Every phase. This is the meta-challenge that persists throughout the entire milestone.

---

### Pitfall 2: Timer.send_after Spawns an OS Thread Per Timer

**What goes wrong:** The alerting system needs timer-driven evaluation -- checking alert rules every N seconds. The natural Mesh pattern is `Timer.send_after(self(), interval, :check_alerts)`. But `Timer.send_after` spawns a dedicated OS thread that sleeps for the duration and then sends the message. If the alerting evaluator runs every 10 seconds across 100 alert rules, that is 100 OS threads created and destroyed every 10 seconds, or 600 threads per minute. At scale, this exhausts OS thread limits and causes thread creation failures.

**Why it happens:** `Timer.send_after` was designed for occasional delayed messages, not for recurring timers. The implementation in `mesh_timer_send_after` (actor/mod.rs:585) spawns `std::thread::spawn` with a deep-copied message. There is no timer wheel or recurring timer facility in the Mesh runtime. Each `send_after` creates a new OS thread, which has ~8MB default stack on Linux and ~512KB on macOS.

**Consequences:**
- Thread exhaustion under high alert rule count (thousands of threads for hundreds of rules)
- OS-level errors: `pthread_create` failures causing actor panics
- High memory overhead from thread stacks (100 threads x 8MB = 800MB on Linux just for timer stacks)
- Timer inaccuracy under load -- thread scheduling jitter means timers fire late

**Prevention:**
1. **Use a single recurring timer actor** instead of individual `send_after` per rule. One actor calls `Timer.send_after(self(), interval, :tick)` and on receiving `:tick`, evaluates ALL alert rules, then schedules the next tick. This creates exactly one OS thread per tick, not one per rule.
2. **Batch timer operations.** Group alerts by evaluation interval (e.g., all 10s alerts together, all 60s alerts together). Use one timer per interval group, not one per rule.
3. **If the compiler is extended:** Consider adding a `Timer.send_interval(pid, ms, msg)` runtime function that uses a single background thread with a timer wheel instead of spawning a thread per invocation. This would be a runtime improvement triggered by dogfooding.
4. **Monitor OS thread count.** Track the number of OS threads in the Mesher process. If it grows unboundedly, timer usage is the likely cause.

**Detection:** `ps -M <pid> | wc -l` on macOS or `ls /proc/<pid>/task | wc -l` on Linux shows OS thread count. If it correlates with alert rule count, Timer.send_after is the cause.

**Phase mapping:** Alerting phase. Must be designed correctly from the start -- retrofitting a timer architecture is expensive.

---

### Pitfall 3: Map Linear Scan Becomes O(n^2) for Large Event Metadata

**What goes wrong:** Mesh's Map implementation uses a vector of `(u64, u64)` pairs with linear scan for lookups (collections/map.rs). For small maps (5-20 keys), this is fine. But monitoring events often carry metadata maps with 50+ keys (HTTP headers, tags, labels, context). Looking up a key in a 50-entry map requires scanning 50 entries. If the ingestion pipeline does 10 map lookups per event (extracting fingerprint fields, checking for required tags, etc.), that is 500 comparisons per event. At 10,000 events/second, that is 5 million comparisons/second -- all linear scans that could be hash lookups.

**Why it happens:** The Map implementation was designed for "small maps typical in Phase 8" (as stated in the map.rs header comment). It has no hash table, no tree structure, just a sorted or unsorted vector. This was a reasonable tradeoff when maps had 3-5 entries. Monitoring event metadata maps are much larger.

**Consequences:**
- Ingestion throughput bottleneck from O(n) map lookups instead of O(1)
- CPU-bound processing when the bottleneck should be I/O-bound
- The performance cliff is non-linear: going from 10-key maps to 50-key maps makes lookups 5x slower, not just 5x more data

**Prevention:**
1. **Limit metadata map sizes.** Enforce a maximum number of tags/labels per event (e.g., 32). This is a product decision that also limits the linear scan cost. Sentry limits custom tags.
2. **Avoid repeated map lookups in hot paths.** Extract all needed fields from an event map once (destructure into local variables) rather than doing repeated `Map.get` calls.
3. **If performance is unacceptable:** This is a strong signal that the Mesh runtime needs a hash-based Map implementation. Log the dogfooding finding and consider adding it as a runtime improvement.
4. **Profile before optimizing.** The linear scan may be fast enough for the actual event sizes Mesher encounters. Profile ingestion throughput before assuming this is the bottleneck.

**Detection:** Profile the ingestion pipeline. If `mesh_map_get` appears in the top 10 of a CPU profile, map linear scan is the bottleneck.

**Phase mapping:** Ingestion pipeline phase. Design event data structures to minimize map lookups.

---

### Pitfall 4: PostgreSQL Schema Without Time-Based Partitioning

**What goes wrong:** The events table is created as a single unpartitioned table. After a week of ingestion at even modest throughput (1,000 events/second), the table has 600 million rows. Queries for "events in the last hour" scan the entire table because there is no partition pruning. Index maintenance on writes becomes the bottleneck: every INSERT updates B-tree indexes across 600M rows. Autovacuum runs for hours and still cannot keep up with the write rate. The database becomes unusable.

**Why it happens:** PostgreSQL's default behavior is a single heap table with B-tree indexes. The getting-started experience is fast: `CREATE TABLE events (...)` works immediately. The problem only surfaces after days or weeks of production ingestion, by which time the table is too large to partition without downtime (partitioning existing tables requires `pg_partman` or manual migration with data copying).

**Consequences:**
- Query latency grows linearly with table size (no partition pruning)
- Write throughput degrades as indexes grow (B-tree splits, WAL amplification)
- Autovacuum cannot keep pace with dead tuples from high write + delete patterns
- Disk space grows without bound because old data is expensive to delete from an unpartitioned table (DELETE + VACUUM vs. DROP PARTITION)

**Prevention:**
1. **Partition the events table by time from day one.** Use PostgreSQL native range partitioning by a `created_at` timestamp column. Create daily or hourly partitions depending on expected volume.
2. **Use BRIN indexes instead of B-tree for timestamp columns.** BRIN indexes are orders of magnitude smaller for time-series data and have negligible write overhead because the data arrives in roughly sorted order.
3. **Design for partition-based retention.** Deleting old data is `DROP TABLE events_2026_02_13` (instant) rather than `DELETE FROM events WHERE created_at < '2026-02-13'` (hours of vacuum).
4. **Batch inserts.** The COPY protocol is up to 10x faster than individual INSERT statements. Mesh's `Pg.execute` uses the Extended Query protocol (one row at a time). Buffer events in memory and flush in batches.
5. **Tune PostgreSQL for write-heavy workloads:** increase `checkpoint_timeout` to 10-15 minutes, increase `max_wal_size` to 2GB+, reduce `random_page_cost` for SSD storage, configure `autovacuum_naptime` for aggressive cleanup.

**Detection:** Monitor `pg_stat_user_tables.n_dead_tup` and `pg_stat_user_tables.last_autovacuum`. If dead tuple count grows faster than autovacuum can clean, or if `seq_scan` count is high on the events table, the schema needs partitioning.

**Phase mapping:** Database schema phase. Must be designed into the schema from the first migration. Retrofitting partitioning is a major migration.

---

### Pitfall 5: Alert Storm from Cascading Rule Triggers

**What goes wrong:** The alerting system fires rules independently. When a systemic failure occurs (e.g., database goes down), every error rate rule, every response time rule, and every availability rule fires simultaneously. An operator receives 200 alerts in 30 seconds. They dismiss all notifications because the volume is overwhelming, and miss the one alert that identifies the root cause. Alert fatigue sets in, and operators begin ignoring alerts entirely.

**Why it happens:** Each alert rule evaluates independently without knowledge of other rules. A single root cause (database failure) manifests as multiple symptoms (high error rate, slow response times, connection timeouts, queue buildup), each triggering its own alert. Without correlation or grouping, every symptom generates a separate notification.

**Consequences:**
- Operators receive hundreds of alerts for a single incident, leading to alert fatigue
- Root cause identification is buried under symptom noise
- Operators learn to ignore alerts, defeating the purpose of the alerting system
- High notification volume can overwhelm delivery channels (email/webhook rate limits)

**Prevention:**
1. **Alert deduplication window.** When an alert fires, suppress duplicate alerts for the same rule for a configurable period (e.g., 5 minutes). Only re-alert if the condition persists after the window.
2. **Alert grouping by source.** Group alerts that fire within the same time window (e.g., 30 seconds) into a single notification: "5 alerts fired for project X" with a link to the details.
3. **Tiered severity levels.** Distinguish between `critical` (pages immediately), `warning` (batched notification), and `info` (logged but not notified). Start with conservative severity assignments.
4. **Cooldown periods.** After an alert fires, do not re-fire the same alert for a configurable cooldown period even if the condition remains true. Use "resolved" notifications when the condition clears.
5. **Start with fewer, broader rules.** It is better to have 5 well-tuned alert rules than 50 noisy ones. Add granularity only when operators request it.

**Detection:** Track the ratio of actionable alerts (operator took action) to total alerts. If the ratio is below 30%, the system has too much noise. Track alert volume per incident -- if a single incident generates more than 10 alerts, grouping is needed.

**Phase mapping:** Alerting phase. Deduplication and cooldown must be part of the initial alerting design, not bolted on later.

---

### Pitfall 6: List.find Option Pattern Matching LLVM Verification Error

**What goes wrong:** The developer writes natural Mesh code to find an event in a list:
```
case List.find(events, fn(e) do e.severity == "error" end) do
  Some(event) -> handle(event)
  None -> skip()
end
```
This triggers a known LLVM verification error because `List.find` returns `Option<T>` through the FFI boundary, and pattern matching on FFI-returned Options has a pre-existing codegen gap. The compiler crashes or produces incorrect code.

**Why it happens:** This is a documented pre-existing limitation: "List.find Option return pattern matching triggers LLVM verification error with case expression (pre-existing codegen gap)." The bug exists because FFI functions that return `Option<T>` produce a different LLVM IR layout than user-defined functions that return `Option<T>`, and the pattern match compiler expects the user-defined layout.

**Consequences:**
- A common monitoring pattern (finding specific events) cannot use the natural `case List.find(...) do` syntax
- Developers must use workarounds (e.g., `List.filter` + `List.length` checks, or `List.any` followed by `List.filter`)
- The workaround is less readable and less efficient than the intended pattern
- New developers hitting this bug will think the language is broken

**Prevention:**
1. **Fix this codegen bug before or during Mesher development.** It is the single most impactful pre-existing limitation for application code. `List.find` with pattern matching is a bread-and-butter operation for any data processing application.
2. **If not fixed immediately:** Document the workaround prominently. The workaround is:
   ```
   let results = List.filter(events, fn(e) do e.severity == "error" end)
   if List.length(results) > 0 do
     let event = List.head(results)
     handle(event)
   else
     skip()
   end
   ```
3. **Track this as a high-priority compiler fix** triggered by dogfooding. The fact that Mesher needs it and cannot use it is exactly the kind of finding dogfooding is meant to produce.

**Detection:** Any `case List.find(...) do Some(...) -> ...` pattern in Mesh code will trigger the LLVM verification error at compile time. The compiler will crash with an LLVM error, not a user-friendly Mesh error.

**Phase mapping:** Should be fixed in the first phase or as a pre-Mesher compiler fix. Blocking for any phase that processes lists of events.

---

### Pitfall 7: Single-Line Pipe Chains Make Complex Data Pipelines Unreadable

**What goes wrong:** Mesh's parser does not support multi-line pipe continuation. A monitoring event processing pipeline that chains multiple transformations must be written as a single line:
```
let result = events |> List.filter(fn(e) do e.level == "error" end) |> List.map(fn(e) do fingerprint(e) end) |> List.group_by(fn(f) do f.group_key end)
```
This line is 150+ characters and unreadable. In Elixir, this would be a clean multi-line pipeline:
```elixir
events
|> Enum.filter(&(&1.level == "error"))
|> Enum.map(&fingerprint/1)
|> Enum.group_by(&(&1.group_key))
```

**Why it happens:** The Mesh parser does not support `|>` as a continuation operator at the start of a new line. This is a documented limitation: "Single-line pipe chains only (parser does not support multi-line |> continuation)."

**Consequences:**
- Code readability degrades severely for any non-trivial data transformation
- Developers break pipelines into intermediate `let` bindings, which is verbose but readable
- The language's Elixir-inspired syntax promise ("expressive, readable") is undermined for the exact use case (data pipelines) where pipes are most valuable
- Code review becomes harder because long lines are hard to diff

**Prevention:**
1. **Accept intermediate let bindings as the standard pattern for now.** Instead of one long pipe, use:
   ```
   let errors = List.filter(events, fn(e) do e.level == "error" end)
   let fingerprinted = List.map(errors, fn(e) do fingerprint(e) end)
   let grouped = group_by(fingerprinted, fn(f) do f.group_key end)
   ```
   This is more verbose but readable and debuggable (each intermediate value can be inspected).
2. **Consider fixing the parser during Mesher development** if pipe-heavy code becomes a constant pain point. Multi-line pipe continuation is likely a parser change of moderate scope.
3. **Enforce a line length limit** in the project (e.g., 120 characters) to force developers into the intermediate-binding pattern rather than writing 200-character pipe chains.

**Detection:** Any pipe chain longer than 100 characters signals that the single-line limitation is being hit.

**Phase mapping:** All phases with data processing. Particularly impactful in ingestion pipeline and error grouping phases.

---

## Moderate Pitfalls

---

### Pitfall 8: WebSocket Actor-Per-Connection Memory Pressure Under Dashboard Load

**What goes wrong:** Mesher streams real-time events to dashboards via WebSocket. Each WebSocket connection spawns an actor (64 KiB stack) plus a reader thread (OS thread). With 100 concurrent dashboard users watching real-time event streams, that is 100 actors + 100 OS threads. Each actor's per-actor heap grows as events are buffered for sending, and the 256 KiB GC threshold means each actor heap can hold up to 256 KiB of event data before collection. Total memory for 100 dashboard connections: 100 * (64 KiB stack + 256 KiB heap + ~512 KiB reader thread stack) = ~80 MB. At 1,000 connections, it is 800 MB just for WebSocket overhead.

**Why it happens:** The actor-per-connection model with a reader thread per connection is a correct design for isolation but expensive per connection. Each connection's reader thread is a full OS thread (`std::thread::spawn` in ws/server.rs:579). The per-actor GC threshold of 256 KiB means event data accumulates in the heap between GC cycles.

**Consequences:**
- Memory usage scales linearly with connected dashboard users
- OS thread exhaustion possible at high connection counts (1,000+ dashboards)
- GC pauses in individual actor heaps may cause message delivery delays to dashboard users
- Reader threads blocked on Mutex contention (Arc<Mutex<WsStream>>) during broadcasts

**Prevention:**
1. **Limit concurrent WebSocket connections per project/user.** A monitoring dashboard does not need 1,000 simultaneous WebSocket connections. Implement connection limits (e.g., 50 per project).
2. **Use WebSocket rooms for broadcast efficiency.** Send events to room channels, not individual connections. The room broadcast (Ws.broadcast) sends once per room, not once per connection, reducing the work done per event.
3. **Consider connection timeouts.** Dashboard tabs left open but unfocused should not maintain active WebSocket connections indefinitely. Implement idle timeouts that close connections after no client-side heartbeat pong.
4. **Monitor per-actor heap sizes.** If WebSocket actors' heaps grow large (many events buffered for slow clients), implement backpressure: drop older events if the send buffer exceeds a threshold.

**Detection:** Monitor total OS thread count and memory usage. Correlate with WebSocket connection count. If memory grows faster than expected per connection, check per-actor heap sizes.

**Phase mapping:** Real-time streaming phase. Connection limits should be designed into the WebSocket handler from the start.

---

### Pitfall 9: Error Grouping Over-Groups or Under-Groups Events

**What goes wrong:** The error grouping algorithm assigns a fingerprint to each error event based on its stack trace, error message, and type. If the fingerprint is too coarse (e.g., based only on error type), unrelated errors with the same type are grouped together: a `NullPointerException` from module A and a `NullPointerException` from module B appear as the same issue, masking the true error count for each. If the fingerprint is too fine (e.g., includes line numbers), the same logical error appears as multiple issues every time the code changes, creating hundreds of duplicate issues.

**Why it happens:** Error grouping is inherently subjective. Sentry's developers acknowledge: "It is not possible for Sentry to always group errors correctly, as 'correct' is subjective to the developer." The challenge is compounded by:
- Stack traces vary by deployment (different line numbers, inlined functions)
- Error messages often include dynamic data (user IDs, timestamps) that make every occurrence unique
- Different callers hitting the same bug produce different stack traces

**Consequences:**
- Over-grouping: Issues show inflated counts, mixing unrelated errors. Operators cannot tell which errors are actually frequent.
- Under-grouping: The issues list has hundreds of entries for what is logically the same bug. Operators waste time triaging duplicates.
- Users lose trust in the error counts and stop using the grouping feature

**Prevention:**
1. **Start simple, evolve incrementally.** Use a three-part fingerprint: `hash(error_type + top_3_stack_frames + normalized_message)`. Strip dynamic data (numbers, UUIDs, timestamps) from error messages before hashing.
2. **Normalize stack traces.** Remove line numbers (which change on every deploy). Use only function names and module paths for fingerprinting. This is the approach Sentry uses for most platforms.
3. **Allow user-defined fingerprints.** Provide an API field (`fingerprint`) that lets users override automatic grouping. This is the escape hatch for when automatic grouping is wrong.
4. **Version the grouping algorithm.** When the algorithm changes, apply the new version only to new events. Existing issues keep their original grouping. This prevents a grouping algorithm update from splitting all existing issues into new ones.
5. **Display grouping metadata.** Show users what fields contributed to the fingerprint so they can understand why events are grouped together and provide feedback.

**Detection:** Track the issue creation rate relative to the event rate. If 90% of events create new issues (very high unique fingerprint rate), the algorithm is under-grouping. If one issue has 10,000 events with clearly different error messages, it is over-grouping.

**Phase mapping:** Error grouping phase. The initial algorithm should be conservative (slight over-grouping is better than under-grouping, because users can split issues but cannot merge them).

---

### Pitfall 10: Middleware Parameter Type Annotation Requirement Breaks Ergonomics

**What goes wrong:** Mesh middleware requires explicit `:: Request` parameter type annotations due to incomplete type inference. Every middleware function must annotate its parameter:
```
fn log_middleware(req :: Request, next) do
  IO.println("Request: " <> req.path)
  next(req)
end
```
Forgetting the `:: Request` annotation causes a type error that is confusing because the parameter type should be inferrable from the middleware registration context. In a monitoring platform with multiple middleware layers (auth, rate limiting, logging, CORS), every middleware function needs this boilerplate annotation.

**Why it happens:** Documented limitation: "Middleware requires explicit `:: Request` parameter type annotations (incomplete inference)." The type inference engine does not propagate the expected parameter type from the middleware registration site (`HTTP.use(router, fn)`) back to the closure parameter.

**Consequences:**
- Every middleware function has boilerplate type annotations
- Error messages when annotations are forgotten are not helpful (generic type error, not "add :: Request")
- Developers coming from Elixir/Ruby expect parameter types to be inferred

**Prevention:**
1. **Establish a project convention:** All middleware functions use `:: Request` annotations. Document this in a project style guide.
2. **Use a single middleware file** that collects all middleware definitions. This contains the boilerplate in one place and makes it easy to copy the pattern.
3. **Consider fixing this inference gap** if it is encountered frequently. The fix would be to propagate the expected function type from `HTTP.use` to the closure parameter during type inference.

**Detection:** Compilation errors in middleware functions that fail to compile without type annotations.

**Phase mapping:** HTTP API phase. Will be encountered early when setting up the ingestion API.

---

### Pitfall 11: Conservative Stack Scanning Retains Garbage Under High Allocation

**What goes wrong:** Mesh uses conservative stack scanning for garbage collection -- every 8-byte word on the stack is treated as a potential pointer. Under high allocation rates (e.g., an ingestion actor allocating a struct per incoming event, processing 1,000 events/second), false positives from conservative scanning retain dead objects that look like valid pointers but are actually stale integer values on the stack. The per-actor heap grows beyond the GC threshold because GC runs but fails to reclaim memory due to false pointer retention.

**Why it happens:** Conservative GC (documented in PROJECT.md key decisions) does not have type maps for the stack. An integer value that happens to match a heap address prevents that heap object from being collected. At high allocation rates, the probability of false positive retention increases because there are more objects to accidentally "point to."

**Consequences:**
- Per-actor heap grows larger than expected, consuming more memory
- GC cycles run but reclaim less memory than precise scanning would
- Long-running ingestion actors accumulate retained garbage over hours/days
- Memory usage appears to "leak" slowly even though GC is running

**Prevention:**
1. **Minimize live variables in hot loops.** The fewer variables on the stack, the fewer false positive pointers. Extract processing into helper functions that return values rather than keeping many live bindings.
2. **Monitor per-actor heap sizes.** If an actor's heap grows monotonically despite GC runs, conservative scanning false positives are the likely cause.
3. **Consider periodic actor restart.** For long-running ingestion actors, a supervised restart every N hours resets the heap to zero. This is the BEAM "let it crash" philosophy applied to memory management.
4. **Profile GC effectiveness.** Track bytes reclaimed per GC cycle vs. total heap size. If the ratio declines over time, false retention is growing.

**Detection:** Per-actor heap size monitoring. A healthy actor should have heap size oscillating between ~50% and 100% of the GC threshold. If heap size grows monotonically past the threshold and GC runs show diminishing returns, conservative scanning is the cause.

**Phase mapping:** All phases with high-throughput actors. Particularly the ingestion pipeline.

---

### Pitfall 12: Map.collect Assumes Integer Keys -- Breaks String-Keyed Aggregations

**What goes wrong:** A natural monitoring operation is grouping events by a string key:
```
let counts = events
  |> Iter.from()
  |> Iter.map(fn(e) do {e.source, 1} end)
  |> Iter.collect()  # Tries to collect into Map<String, Int>
```
But `Map.collect` assumes integer keys (documented limitation: "Map.collect assumes integer keys"). Collecting into a `Map<String, Int>` produces a map with broken key comparisons or incorrect behavior because the runtime's `mesh_map_new()` defaults to integer key type tag.

**Why it happens:** The `Iter.collect()` for Map calls `mesh_map_new()` which defaults to `KEY_TYPE_INT` (tag 0). String keys require `KEY_TYPE_STR` (tag 1), but the collect path does not propagate the key type from the iterator's element type to the map constructor.

**Consequences:**
- Aggregating events by string fields (source, level, project name) produces incorrect results
- String keys are compared as integer values (pointer comparison), so different strings that happen to have different addresses are always "not equal"
- The bug is silent -- the map appears to work but contains duplicate entries for the same logical key

**Prevention:**
1. **Do not use `Iter.collect()` to build string-keyed maps.** Build maps manually:
   ```
   let counts = Map.new()
   for event in events do
     let key = event.source
     let current = Map.get(counts, key)
     Map.put(counts, key, current + 1)
   end
   ```
2. **Track this as a compiler/runtime fix.** The fix is to propagate the key type from the iterator element type to the map constructor in the collect codegen path.
3. **Use `List.group_by` or manual accumulation** for string-keyed aggregations until the fix lands.

**Detection:** Any `Iter.collect()` call that produces a `Map<String, V>` will silently produce incorrect results. Test by inserting the same string key twice and checking if the map has 1 entry or 2.

**Phase mapping:** Error grouping and analytics phases. Any phase that aggregates events by string fields.

---

### Pitfall 13: Multi-Node Split Brain Without Consensus Protocol

**What goes wrong:** Mesher runs on multiple nodes using Mesh's distributed actor system. When a network partition occurs between nodes, each partition continues operating independently. Both partitions accept events, fire alerts, and update state. When the partition heals, the system has divergent state: different event counts, different alert histories, conflicting global registry entries. There is no consensus protocol or conflict resolution mechanism.

**Why it happens:** Mesh's distributed actor system provides transparent remote send, global process registry, and node monitoring, but does not include a consensus protocol (Raft, Paxos). The global registry uses cluster-wide broadcast for registration, which means during a partition, each side of the partition has its own registry state. Node monitoring delivers `:nodedown` signals, but there is no quorum mechanism to decide which partition is "authoritative."

**Consequences:**
- Duplicate alert notifications (both partitions fire the same alert rule)
- Inconsistent event counts between nodes after partition heals
- Global registry conflicts: same name registered on different nodes in different partitions
- Dashboard shows different data depending on which node it connects to

**Prevention:**
1. **Design for eventual consistency, not strong consistency.** Monitoring data is append-only and idempotent. Duplicate events are better than lost events. Design the system so that both partitions can ingest events independently, and deduplicate when the partition heals.
2. **Use PostgreSQL as the source of truth, not in-memory actor state.** Alert state, event counts, and grouping should be durably stored in PostgreSQL. When a partition heals, nodes re-read from the database rather than trying to reconcile in-memory state.
3. **Alerting deduplication via database.** Before sending an alert notification, check the database for a recent alert with the same fingerprint. If one exists within the deduplication window, skip the notification. This prevents duplicate alerts during partitions.
4. **Designate a primary node for alerting.** Only one node evaluates alert rules. If that node goes down (`:nodedown`), a standby node takes over. This avoids dual-fire during partitions.
5. **Defer multi-node to a later phase.** Build Mesher as a single-node application first. Multi-node adds complexity that is not needed for the initial dogfooding goal. Add clustering once the single-node version is stable.

**Detection:** Simulate a network partition (block traffic between nodes with `iptables` or `pfctl`). Check if both nodes fire the same alerts. Check if event counts diverge. Check if the global registry has conflicting entries after the partition heals.

**Phase mapping:** Multi-node clustering phase. Should be the last phase, after single-node stability is proven.

---

### Pitfall 14: Ingestion Pipeline Backpressure Missing -- Events Dropped Under Load

**What goes wrong:** The ingestion HTTP endpoint accepts events and spawns an actor per request. Under burst traffic (e.g., a client application suddenly sends 10,000 errors/second), the system spawns 10,000 actors simultaneously. Each actor attempts a database write. The connection pool has a max of N connections (e.g., 20). The remaining 9,980 actors block on pool checkout, consuming 9,980 * 64 KiB = 624 MB of memory just for actor stacks. If the checkout timeout expires, 9,980 actors get timeout errors and the events are lost.

**Why it happens:** The HTTP server is actor-per-connection, which means each request gets its own actor that runs until the response is sent. There is no queue between the HTTP acceptance and the database write. The connection pool provides bounded database access, but the backpressure signal (checkout timeout) is too late -- the actor has already been spawned and is consuming memory.

**Consequences:**
- Memory spikes during traffic bursts (thousands of actors waiting on the pool)
- Event loss when pool checkout times out
- Database connection pool contention causes cascading slowdowns
- The system appears to "crash" under load because all resources are consumed by waiting actors

**Prevention:**
1. **Buffer events in memory before writing to the database.** Use a dedicated ingestion actor that receives events via its mailbox and batches them. Instead of each HTTP handler writing directly to the database, it sends the event to the buffer actor and immediately returns `202 Accepted`. The buffer actor flushes to the database in batches (e.g., every 100 events or every 1 second, whichever comes first).
2. **Return 429 Too Many Requests when the buffer is full.** If the in-memory buffer exceeds a configurable size (e.g., 10,000 events), reject new events with 429 status. This provides backpressure to the client SDK.
3. **Use batch INSERT.** Instead of one INSERT per event, buffer events and write them in a single multi-row INSERT or COPY operation. This reduces connection pool contention because one checkout handles 100 events instead of 1.
4. **Monitor buffer depth.** Track the size of the in-memory event buffer. If it approaches the maximum, emit a metric/log that can trigger autoscaling or client-side rate limiting.

**Detection:** Monitor connection pool checkout wait time and timeout rate. If checkout timeouts exceed 1% of requests, the system needs buffering.

**Phase mapping:** Ingestion pipeline phase. The buffering architecture must be designed from the start, not added after the naive approach fails under load.

---

### Pitfall 15: TyVar Resolution Failure for Chained Method Calls on Collected Iterators

**What goes wrong:** The developer writes a pipeline that collects an iterator into a List, then calls `.to_string()` on the result:
```
let names = events |> Iter.from() |> Iter.map(fn(e) do e.name end) |> Iter.collect()
IO.println(names.to_string())
```
The `.to_string()` call on the collected List fails with a type error because the `TyVar` from the `Ptr -> List<T>` conversion remains unresolved for the `.to_string()` method. This is a documented tech debt item: "TyVar from Ptr->List<T> remains unresolved for .to_string() on collected collections (use string interpolation instead)."

**Why it happens:** The `Iter.collect()` return type is `Ptr` in the type system (opaque pointer), which gets coerced to `List<T>`. But the type variable `T` in the resulting `List<T>` is not fully resolved when the `.to_string()` trait method is dispatched, causing a type error.

**Consequences:**
- Cannot call `.to_string()` directly on iterator results
- Must use string interpolation `"${names}"` as a workaround
- Confusing error message that does not suggest the workaround

**Prevention:**
1. **Use string interpolation** (`"${collected_list}"`) instead of `.to_string()` on iterator results. This is the documented workaround.
2. **Bind to intermediate variable with explicit type** if possible to help the type checker.
3. **Consider fixing this type resolution** if it is encountered frequently during Mesher development.

**Detection:** Type error on `.to_string()` calls after `Iter.collect()`.

**Phase mapping:** Any phase that formats collected iterator results for logging or API responses.

---

## Minor Pitfalls

---

### Pitfall 16: Database Connection Pool Blocking in Actor Context

**What goes wrong:** The connection pool uses `parking_lot::Mutex` + `Condvar` for synchronization. When an actor calls `Pool.checkout` and all connections are busy, the actor's OS worker thread blocks on the Condvar. This blocks the entire M:N scheduler worker thread, preventing other actors scheduled on that worker from running.

**Prevention:**
1. Size the connection pool to match the number of scheduler worker threads (one connection per CPU core is a good starting point).
2. Keep checkout timeout short (1-2 seconds) to prevent long worker thread blocking.
3. Use the batch buffer pattern (Pitfall 14) to reduce the number of actors competing for pool connections.

**Phase mapping:** Database integration phase.

---

### Pitfall 17: WebSocket Room Broadcast Amplification Across Nodes

**What goes wrong:** In a multi-node setup, a `Ws.broadcast(room, message)` sends the message to all local connections AND forwards it to all other nodes via `DIST_ROOM_BROADCAST`. If multiple nodes receive the same event and each broadcasts to the same room, the message is sent N times (once per node). With 3 nodes, each dashboard user receives the event 3 times.

**Prevention:**
1. Designate event broadcasting responsibility to a single node (the node that ingested the event).
2. Use `Ws.broadcast_except` to avoid echo when forwarding between nodes.
3. Include a message ID in broadcast messages so clients can deduplicate.

**Phase mapping:** Multi-node streaming phase.

---

### Pitfall 18: No String Concatenation in Pattern Matching for Log Parsing

**What goes wrong:** Monitoring often requires parsing log messages by pattern. In Elixir, you can match `"ERROR: " <> rest` to extract the rest of a message. Mesh supports string pattern matching via `snow_string_eq` but does not support string prefix/suffix destructuring in patterns. Log parsing must use `String.split`, `String.starts_with`, or similar stdlib functions instead of pattern matching.

**Prevention:**
1. Use `String.split`, `String.starts_with`, and `String.contains` for log parsing.
2. Design the event schema so that structured fields (level, message, source) are sent as separate fields, not as a single log line that needs parsing.
3. Encourage clients to send structured JSON events rather than raw log lines.

**Phase mapping:** Ingestion pipeline and error grouping phases.

---

### Pitfall 19: Missing `List.group_by` in Stdlib

**What goes wrong:** Error grouping requires grouping events by fingerprint. The natural function is `List.group_by(events, fn(e) do e.fingerprint end)` which returns `Map<String, List<Event>>`. But Mesh's stdlib does not include `List.group_by`. The developer must implement grouping manually with a fold/reduce over the list.

**Prevention:**
1. Check if `List.group_by` exists in the current stdlib before assuming it does. If not, implement it as a Mesh function in the Mesher codebase.
2. Track missing stdlib functions discovered during Mesher development. These become candidates for stdlib additions in a future language version.

**Phase mapping:** Error grouping phase.

---

### Pitfall 20: Clock Skew Between Nodes Causes Event Ordering Issues

**What goes wrong:** In a multi-node setup, events ingested on different nodes get timestamps from different system clocks. If node A's clock is 2 seconds ahead of node B's clock, events ingested on node A appear to be 2 seconds in the future relative to events on node B. Dashboard queries for "last 5 minutes" miss recent events from node B or include extra events from node A.

**Prevention:**
1. Use NTP synchronization on all nodes (ensure `ntpd` or `chrony` is running).
2. Use the client-provided timestamp as the canonical event time, not the server ingestion time. This makes event ordering independent of server clock skew.
3. For server-generated timestamps (alert fire time, ingestion time), accept that clock skew of a few milliseconds is unavoidable and design queries with tolerance (e.g., add 5-second buffer to time range queries).

**Phase mapping:** Multi-node clustering phase.

---

### Pitfall 21: Forgetting to Handle WebSocket Disconnection Cleanup

**What goes wrong:** When a dashboard WebSocket disconnects (browser tab closed, network drop), the WebSocket actor receives an `on_close` callback. If the actor has registered state (room memberships, active query subscriptions, per-connection metrics), failing to clean up this state causes resource leaks. Room memberships are automatically cleaned up by the Mesh runtime (`RoomRegistry` dual-map with `conn_rooms`), but application-level state (e.g., "this connection was watching project X") is not automatically cleaned.

**Prevention:**
1. Implement all cleanup in the `on_close` callback.
2. Use the actor supervision tree to ensure cleanup happens even if the WebSocket actor crashes (the supervisor can run cleanup logic).
3. Use the room system for all broadcast needs -- it handles cleanup automatically.

**Phase mapping:** Real-time streaming phase.

---

## Phase-Specific Warnings

| Phase Topic | Likely Pitfall | Mitigation |
|-------------|---------------|------------|
| **All phases** | Dual-bug confusion: app vs compiler | Minimal repro discipline, fix compiler bugs immediately (Pitfall 1) |
| **All phases** | Single-line pipe chains unreadable | Use intermediate let bindings (Pitfall 7) |
| **Ingestion pipeline** | No backpressure, events dropped | Buffer actor + batch writes + 429 response (Pitfall 14) |
| **Ingestion pipeline** | Map linear scan bottleneck | Limit metadata size, minimize lookups (Pitfall 3) |
| **Ingestion pipeline** | Conservative GC false retention | Minimize live variables, periodic actor restart (Pitfall 11) |
| **Ingestion pipeline** | Log parsing without string destructuring | Use structured events, String.split (Pitfall 18) |
| **Database schema** | Unpartitioned events table | Time-based partitioning from day one (Pitfall 4) |
| **Database integration** | Pool blocking scheduler workers | Right-size pool, batch buffer pattern (Pitfall 16) |
| **Error grouping** | Over/under grouping | Simple fingerprint, user overrides, versioned algorithm (Pitfall 9) |
| **Error grouping** | Map.collect integer key assumption | Manual map building for string keys (Pitfall 12) |
| **Error grouping** | Missing List.group_by | Implement manually or add to stdlib (Pitfall 19) |
| **Alerting** | Alert storms from cascading rules | Deduplication window, grouping, cooldown (Pitfall 5) |
| **Alerting** | Timer.send_after thread explosion | Single recurring timer actor, batch evaluation (Pitfall 2) |
| **Real-time streaming** | WebSocket memory pressure | Connection limits, room broadcasts, idle timeouts (Pitfall 8) |
| **Real-time streaming** | Disconnect cleanup leaks | on_close cleanup, supervision tree (Pitfall 21) |
| **Multi-node** | Split brain without consensus | PostgreSQL as truth, single alerting node (Pitfall 13) |
| **Multi-node** | Broadcast amplification | Single-origin broadcast, message deduplication (Pitfall 17) |
| **Multi-node** | Clock skew ordering | NTP, client timestamps, query tolerance (Pitfall 20) |
| **Compiler interaction** | List.find pattern match crash | Fix codegen bug or use filter workaround (Pitfall 6) |
| **Compiler interaction** | Collected iterator .to_string() failure | Use string interpolation (Pitfall 15) |
| **HTTP middleware** | Missing type annotations | Convention: always annotate :: Request (Pitfall 10) |

---

## Sources

### Primary (HIGH confidence -- direct Mesh source analysis)
- `crates/mesh-rt/src/actor/mod.rs:581-600` -- Timer.send_after OS thread spawn implementation
- `crates/mesh-rt/src/actor/heap.rs:111-137` -- Per-actor heap, 256 KiB GC threshold, conservative scanning
- `crates/mesh-rt/src/actor/process.rs:220` -- DEFAULT_REDUCTIONS = 4000, scheduler preemption
- `crates/mesh-rt/src/collections/map.rs:1-60` -- Map linear scan implementation, KEY_TYPE_INT/STR tags
- `crates/mesh-rt/src/db/pool.rs:1-100` -- Connection pool with parking_lot Mutex, checkout timeout
- `crates/mesh-rt/src/ws/server.rs:579` -- WebSocket reader thread spawn per connection
- `crates/mesh-rt/src/dist/node.rs` -- Distributed actor system, no consensus protocol
- `.planning/PROJECT.md:234-243` -- Documented tech debt and known limitations
- `.planning/PROJECT.md:271` -- Conservative stack scanning design decision

### Secondary (MEDIUM confidence -- monitoring domain research, multiple sources agree)
- [Sentry Developer Docs: Grouping](https://develop.sentry.dev/backend/application-domains/grouping/) -- Error fingerprinting design challenges, platform variance
- [Sentry Developer Docs: Ingestion](https://develop.sentry.dev/ingestion/) -- Relay -> Kafka -> ClickHouse pipeline architecture
- [Sentry Fingerprint Rules](https://docs.sentry.io/concepts/data-management/event-grouping/fingerprint-rules/) -- Custom fingerprinting, `{{ default }}` extension
- [PostgreSQL Write-Heavy Tuning](https://www.cloudraft.io/blog/tuning-postgresql-for-write-heavy-workloads) -- WAL, checkpoint, index overhead
- [PostgreSQL Partitioning Scalability Bottlenecks](https://www.postgresql.org/message-id/510b887e-c0ce-4a0c-a17a-2c6abb8d9a5c@enterprisedb.com) -- Many-partition lock contention
- [Table Partitioning for Performance](https://medium.com/cubbit/table-partitioning-in-postgresql-performance-bloat-7c248dd2d604) -- BRIN vs B-tree, autovacuum impact
- [Scaling Data Ingestion](https://www.matia.io/blog/best-practices-and-pitfalls-of-scaling-data-ingestion-for-high-volume-sources) -- Batch vs row-by-row, schema drift
- [Sentry at Scale](https://www.mindfulchase.com/explore/troubleshooting-tips/devops-tools/sentry-at-scale-diagnosing-ingestion-and-alerting-challenges-in-enterprise-devops.html) -- Ingestion saturation, Kafka partitioning
- [Alert Fatigue Solutions 2025](https://incident.io/blog/alert-fatigue-solutions-for-dev-ops-teams-in-2025-what-works) -- Deduplication, tiered alerts, actionability metrics
- [Alert Fatigue in Monitoring](https://icinga.com/blog/alert-fatigue-monitoring/) -- Alert storms, parent-child dependencies, cooldown
- [Alert Fatigue Prevention](https://www.datadoghq.com/blog/best-practices-to-prevent-alert-fatigue/) -- 30-50% actionable rate target, continuous tuning
- [WebSocket Scale 2025](https://www.videosdk.live/developer-hub/websocket/websocket-scale) -- Connection limits, memory per connection, OS tuning
- [Scaling WebSockets](https://ably.com/topic/the-challenge-of-scaling-websockets) -- Sticky sessions, connection lifecycle, memory leaks
- [WebSocket Performance Issues](https://oneuptime.com/blog/post/2026-01-24-websocket-performance/view) -- Reconnection storms, buffer bloat
- [Split-Brain in Distributed Systems](https://dzone.com/articles/split-brain-in-distributed-systems) -- Quorum, fencing, STONITH
- [Clock Skew in Distributed Systems](https://braineanear.medium.com/taming-clocks-in-distributed-systems-unraveling-the-complexity-of-time-307867b8caf9) -- NTP, logical clocks, vector clocks
- [The Difficulty of Dogfooding](https://blog.codinghorror.com/the-difficulty-of-dogfooding/) -- Developers as unrepresentative users
- [Understanding Compiler Bugs](https://www.cs.sjtu.edu.cn/~zhonghao/paper/understandingbug.pdf) -- Workaround patterns, bug propagation
