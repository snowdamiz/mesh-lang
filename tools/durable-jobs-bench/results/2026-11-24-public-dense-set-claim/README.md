# Public dense set-based claim audit

The five equal-topology runs leave **244–400 ms** against River after giving Mesh free enqueue. This credits Mesh’s entire submit-to-enqueue time and keeps River’s full submit-to-last-settlement clock. Mesh’s equal drain itself took **1,307–1,360 ms**. Every selected arm reconciled 1,024 jobs and two independently committed application writes per job. The 15 Mesh measured arms also had 1,024 distinct terminal XIDs, one durable enqueue transaction with eight calls to the bulk SQL writer, and zero observed cursor fallbacks. [Per-block data](evidence/audit.json) · [Frozen batch](evidence/batch_000000.json.gz)

| Selected groups | Mesh layout | Claim SELECTs between observer snapshots | Extra SELECTs in the broader SQL totals | Whole-trial SELECT elapsed sum |
|---|---:|---:|---:|---:|
| Equal 1–5 | 2×32 | 92–115 | 25–26 | 1.446–1.667 s |
| Mixed 1–5 | 2×32 | 92–109 | 24–26 | 1.427–1.653 s |
| Stock 1–5 | 8×8 | 264–274 | 104–107 | 14.509–15.149 s |

Mixed and equal repeat the same Mesh configuration with different peer layouts and distinct raw captures. Each block contains a separate full-input warmup. The first stock group 3 attempt completed its measured Mesh arm, but the warmup failed while reading the shared broker’s final reply. Its raw arm and dispatch are preserved here. The selected retry is a complete eight-arm block. Stock group 1 recorded 30 queue-row lock timeouts and matching contention hints, while the other 14 captured none. The 8×8 statement sums remain above 14 s even in the four clean stocks. [Original failure](evidence/original-failure/raw.tar.gz)

## What the archives establish

The SQL timing sums cover recorded successful top-level executes across startup, about three seconds of deliberate pre-submit idle, the workload, and teardown up to the final query. Equal group 1 recorded 123 executes, with 98 between the harness’s after-idle and after-terminal observer reads. The broader count therefore has 25 extra calls. Those extra calls cover both ends of the workload and have no separate elapsed subtotal. Idle queries include other runner operations. Wake counters are lifetime counts, with one notification and one subscription scan per runner, approximately eleven to twelve fallback scans per runner, and no due scans. They cannot assign each wake scan to a particular SQL invocation.

Equal first grants were stamped 2.1–4.1 ms after enqueue commit. Median time from that grant stamp to handler entry was 32.1–34.9 ms across the five equal blocks. The claim stamp is evaluated inside the SELECT. The runtime checks the returned rows, including ordinality and bytes, before sending COMMIT, and dispatches only after receipt of idle ReadyForQuery. The ledger does not time SELECT start, queue lock acquisition, validation, COMMIT, or client/scheduler delay independently. Summed elapsed overlaps between backends and includes lock waits. Claim work in executions cancelled by a lock timeout is also outside the successful statement sum. [Public function and checked claim source at b3557273](https://github.com/snowdamiz/mesh-lang/commit/b3557273d72be3802bf189b284e7caaf5f3caa0f)

The candidate to price is the grant CTE’s second access to `mesh_jobs.jobs`. The ordered CTE physically locks up to 16 eligible rows, renders context after that limit, and then rejoins the job relation by namespace and ID for UPDATE. A broad update-side scan repeated under the shared queue lock could cost materially more than a bounded indexed or tuple lookup. The accompanying capture also checks broad scans or residual heap work in the picked prefix, capacity count, empty-eligibility check, and the pause lookup called by every row fence. Context JSON is built once by the speculative CTE for each picked row. The audit shows no legacy cursor calls, and the sealed bench batch has small no-op payloads. Those properties make repeat large-context rendering implausible here. The exact update plan, filtered and visited rows, trigger cost, lock wait distribution and backend CPU are absent from these archives.

Prior bounded-cursor captures used the legacy cursor, with committed ordered ready-index plans and no dense scan saving from the limit. Durable lane partitioning tested the serialization mechanism in a local V1 adapter, found little paired CPU or full-submit gain, and introduced settlement lock misses. Named statements priced outer Parse/Bind planning and scalar settlement on a different shape. The protected product SETUP was a correctness proof on its own ELF. The booked hot worker, public D2, permits, WAL and short producer runs change or observe different paths. These reports do not contain an exact granted CTE plan for this public native queue. [Legacy plan study](../2026-11-23-claim-cursor-plan/README.md) · [Prior lane and named evidence](evidence/preceding-experiments.txt) · [Separate WAL diagnostic](../2026-11-24-dense-wal-cost/README.md)

## Controller diagnostic

One build-free window, two complete reverse orders with duplicate trace-off controls for both 2×32 and 8×8. The primary lane is 2×32. The 8×8 result is a separate contention diagnostic. Each arm uses a fresh PostgreSQL 17.11 cluster, canonical no-op input, the selected public ELF, two CPUs and the original application, TLS, durability, queue policies and XID checks. The prior archives remain the matched peer performance reference. Added settings apply to the worker PostgreSQL role in trace arms only. All arms reuse the existing idle and terminal SQL round trips. The idle snapshots have about three seconds without submits. Full statement/function/table readback and log reduction happen after the workload. Each trace arm is compared against the two adjacent trace-off controls in the same topology, preserving the raw wall difference as an instrumentation cost.

Trace arms log only the worker role’s SQL durations, record acquired lock waits above 1 ms, and collect nested statement rows, buffers, WAL and function counters. If the existing PostgreSQL installation has `auto_explain.so`, trace arms also sample 25% of parent queries and their nested plans with actual rows, buffers, and node timing off. The replay preserves plan shape, rows times loops, reported filters, index name, sort and spills. Stock prefix logging is shared across both trace states. SQL bind parameter logging is disabled in trace arms. The replay reports:

- Existing idle and submit-to-observer SQL deltas, and post-observer overhang. These are overlapping per-statement elapsed sums, with nested totals kept separate from their parents.
- Log-derived server Execute intervals by idle, submit-to-enqueue, committed drain, and boundary. Acquired lock waits are a thresholded lower bound. The SELECT-to-COMMIT server log gap includes client/protocol/scheduling time, and the server COMMIT record does not measure client receipt. Sampled queue-lock statement end to COMMIT end approximates post-acquisition lock residence to millisecond clock precision.
- CPU runtime deltas from `/proc/*/schedstat` for stable worker PostgreSQL PID/start-time pairs between the existing observer boundaries. This includes all operations on those backends. The replay marks missing endpoints and leaves per-statement CPU unresolved.
- Off-clock whole-trial role function statistics and relation statistics. Function elapsed includes wait and may be inclusive of child calls. Without the installed extension, nested statement and relation counters show aggregate work, leaving per-node scan counts unresolved. [PostgreSQL auto_explain](https://www.postgresql.org/docs/17/auto-explain.html) · [Statement statistics](https://www.postgresql.org/docs/17/pgstatstatements.html) · [Protocol logging](https://www.postgresql.org/docs/17/runtime-config-logging.html)

Advance the one candidate only if both equal trace orders reproducibly show substantial removable locked work after the duplicate spread, with a material share of the 244–400 ms residual. For example, a repeated full-relation UPDATE scan would motivate a bounded lookup of the locked candidate rows under the same queue lock. First check original prefix/lock behavior, concurrent legacy and admin capacity, fallback, and independent application/terminal XIDs. Then take a private, matched native comparison under a fresh controller grant. A scan that fits the locked prefix, or elapsed dominated by wait/COMMIT without substantiated removable executor work, leaves the SQL unchanged.

```sh
d=tools/durable-jobs-bench/results/2026-11-24-public-dense-set-claim
python3 -B "$d/audit.py" | diff - "$d/evidence/audit.json"
python3 -B "$d/offline.py"
python3 -B "$d/controller.py" check
# Controller only, during the exact TIMING grant
python3 -B "$d/controller.py" timing
# Once the controller exports the arms
python3 -B "$d/controller.py" replay
```
