# Public dense bounded grant experiment

The archived PostgreSQL 17.11 replay recorded 146–147k `jobs` index tuples fetched per 1,024 completed jobs at equal 2×32 and 376–396k at stock 8×8. The nested grant CTE had about 132k shared buffer hits in one equal run. The archive lacks per-node plans. This packet tests whether the update side scans a broad set of job rows despite the locked pick limit. [Archived native replay](../2026-11-24-public-dense-set-claim/evidence/native/final-replay.json)

The candidate adds a membership predicate against the IDs already in the materialized `sized` CTE. The existing join still checks namespace and ID. PostgreSQL can use both columns of the primary key on the UPDATE side. The function's pick order, locks, sizing and fallback, triggers, ACL, invoker execution, tokens, wire validation, and client commit handling are inherited from the pinned public ELF and SQL.

## Controller intake

Run from this checkout as the sole Linode controller. The intake verifies the supplied packet SHA-256 and invokes the existing slot owner. The actual dispatch requires the latest live, unexpired matching TIMING record, sealed request bytes, and the existing host and native locks. The benchmark uses the pinned image, PostgreSQL 17.11 and CPUs 0–1. Every arm has a fresh cluster. Native failure archives and controller receipts stay in this directory. A retry for a failed or incomplete release needs a controller-issued grant.

```sh
d=tools/durable-jobs-bench/results/2026-11-24-public-dense-bounded-grant
python3 -B "$d/controller.py" check
python3 -B "$d/controller.py" intake <packet-sha256>
python3 -B "$d/controller.py" replay
# After next_peer_gate.passed, with a separate controller booking
python3 -B "$d/controller.py" intake-peers <packet-sha256>
```

1. Group 0 runs the role, order, contention, fallback, and lease checks on both function definitions in a clone of the migrated database. It then captures actual stock and candidate plans for the exact grant statement with typed binds for the PL/pgSQL variables. Each EXPLAIN runs as the real worker over verified TLS in a rolled-back transaction, under the queue row and namespace advisory locks, with the real transition trigger. The test captures the dense head, mid-drain at two remaining-slot values, tail and empty shapes. Each shape uses 1,024 seeded job rows and independently reseeded stocks and candidates. The fixtures express live row-state distributions. They approximate the native backlog and omit its heap churn and concurrent activity.
2. `force_custom_plan`, `force_generic_plan`, and `auto` with five prior executions cover both plan choices. The recorded plan counters describe the prepared replay session. Native timings use the unmodified PL/pgSQL cache setting. The captured plans are executor evidence for the exact extracted statement. The ordinary runner still calls the real function on the measured path. [PostgreSQL PREPARE](https://www.postgresql.org/docs/17/sql-prepare.html) · [PL/pgSQL plan caching](https://www.postgresql.org/docs/17/plpgsql-implementation.html)
3. Admit timing only if candidate update-side observed visits are bounded across all shapes and modes and, in aggregate across head and mid auto plans, fall by at least 75% with a 25% reduction in grant shared-buffer accesses. Report the pick scans so the replay can distinguish them from UPDATE. Visits are based on EXPLAIN rows and filters times loops, with PostgreSQL per-loop rounding. Running time from the plan probe is excluded from scoring.
4. Groups 1–6 run all six rotations of two control arms and one candidate at equal 2×32. The score uses first submit to last durable settlement. It also records committed drain, queue contention hints, and off-clock table/index counters. The existing reconciliation requires 1,024 independent terminal commits and two independently committed application writes per job. Release the peer gate only if candidate beats both controls in at least five groups, median savings clear 75 ms, 10% of control wall time and twice median duplicate-control spread, and aggregate index fetches fall at least 25%. Post-trial index counters cover the whole arm.
5. Groups 7–12 require a fresh grant and run the same candidate against the pinned equal-topology River and Oban lanes, with a full-input warmup per lane. The proposed end-to-end bar is at least a 20% median submit-to-last-settlement reduction against each peer and a paired win in all six groups. Report all raw clocks and failures even when the experiment misses either gate.

The candidate changes only the trial database. Group 0 may return a plan-gate failure with valid plan evidence. It also runs one held qualification arm using the stock function after successful probe checks. Its clock is excluded from all comparisons. Stock 8×8 is historical context for this experiment. The native end-to-end comparison uses equal 2×32 throughout.
