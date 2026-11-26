# A claim cursor limit saves work only in the rollback warmup

Keep the stock cursor. In the committed 64-job large-context workload, stock
and `LIMIT 20` used the ordered ready index for every turn and recorded the
same scan counts and buffers. The bounds also saved zero executor work on
the 1,024-job dense data and the sparse fixtures. The one observed change
was a stock sort after repeated rolled-back claims. [Plan evidence](plans.json.gz),
[committed drain](durable.json.gz)

## Count the fetches

Let `R` be `remaining` when the cursor opens, after the 16-slot ceiling and
the queue capacity check. Immediately before each `FETCH`, at most
`R - 1` jobs have been claimed and at most three oversized markers have been
returned. The last fetch can claim the last slot, produce the fourth
marker, lock a row that cannot fit, or reach EOF. Thus even counting the
final fetch attempt, there are at most `R + 3` attempts and at most 19
successful fetches. The tested bounds were the conservative `LIMIT 20`
and `LIMIT (remaining + 4)`. PostgreSQL copies the cursor's SPI parameters
at `OPEN`, so the second expression uses the starting `remaining`.

Both limits sit above `LockRows`. The 24-lock fixture consumed 32 visible,
eligible index rows to return positions 25 through 32 in every arm. A
bound on candidate input below `LockRows` would count those skipped rows.
PostgreSQL 17.11 also stops propagating the executor sort bound at
`LockRows`. A legal limit can influence the chosen plan, while a Sort
under `LockRows` still has to read its input. The stock PL/pgSQL cursor
already uses PostgreSQL's fast-start planning with the default
`cursor_tuple_fraction = 0.1`.

## Inspect the inner cursor

The old `target/large-held-claim-cursor/proof-05/declare-plan.json` shows
the outer claim Function Scan and a two-row Sort for output phase order.
`target/jobs-due-range-integration/logs/plan-pg17.11.json` measures the
separate due lookup. The old dense fence `lock-plan.json` is a lookup for
one job token. Those plans cover different executor paths.

The new capture uses nested `auto_explain` with `ANALYZE`, `BUFFERS`,
and `TIMING OFF` on the actual cursor and runner query. The SQL changes
one clause in a disposable copy of the original function. PostgreSQL
17.11 uses READ COMMITTED, 128 MiB shared buffers, 4 MiB work_mem,
synchronous commit, fsync, and full-page writes. Each arm starts from a
clone of the same analyzed fixture. The payload generator reproduces the
three frozen input digests. The fixtures model completed rows and current
leases with actual heap and index updates. Autovacuum is disabled to keep
starter statistics and maintenance fixed. The receipts cover nine
fixtures, three query variants, and three cache modes for 81 confirmed
claims. The auto mode traces the sixth execution after five successful
queries rolled back to savepoints. [Fixture](fixture.sql), [plan replay](replay.py)

| Fixture | Visible Jobs rows at the scan | Jobs rows returned from LockRows | Cursor shared hits, fresh / reused |
| --- | ---: | ---: | ---: |
| Dense, early / middle / late | 8 / 8 / 1 | 8 / 8 / 1 | 12 / 114 / 189 fresh, 12 / 16 / 11 reused |
| Large, early / middle / late | 8 / 4 / 1 | 8 / 4 / 1 | 59 / 306 / 95 fresh, 562 / 282 / 71 reused for both bounds |
| Sparse | 1 | 1 | 4 / 3 |
| Mixed registrations and two locks | 9, including one filtered row | 3 | 11 / 10 |
| Dense with 24 locked head rows | 32 | 8 | 40 / 40 |

The table counts visible SQL rows at the Jobs scan, including rows
removed by its filter. PostgreSQL can visit invisible tuples or index
entries in addition to those rows. Shared counts include TOAST work
charged during cursor execution. The fresh early large arms also had
526 shared reads each. The remaining fresh and reused cases shown had
zero cursor shared reads.

In the early large rollback warmup, stock auto chose a custom sequential
scan, hash semi join, and external merge Sort under `LockRows`. The
scan and sort input consumed all 64 rows to emit the eight-row prefix.
The cursor recorded 4,431 shared hits, 1,368 temp blocks read, and 4,119
temp blocks written. Both limits chose a generic ordered
`mesh_jobs_ready` scan that consumed eight rows, used 562 shared hits,
and had zero cursor temp I/O. The two bounds also retained the normal
outer output path. The outer execution still wrote 513 temp blocks in
each bounded arm for the large result. Forced custom and generic plans
all used the streaming ready-index path, including stock. The auto
cache decision also changed in a few dense cases, where the underlying
work stayed identical. A fresh rollback-warmup replay reproduced the
large sort and its disappearance with both bounds. [Warmup repeat](warm-repeat.json.gz)

## Confirm the lock prefix and price a committed drain

An independent session probed the locks before COMMIT. Both candidates
matched stock on ordered context digests, declared byte lengths, and
ordinals. In the non-fit case, rows 1 and 2 returned, row 3 remained
locked until COMMIT, and unvisited row 4 was free. The oversized case
returned four markers with one requested slot. The fifth row was free
before COMMIT and the next committed turn claimed it after skipping the
four committed markers. The existing Function Scan and ordinality query
delivered the full result for validation before the receiver confirmed
COMMIT. [Lock fixtures](edge.sql), [lock and drain receipts](durable.json.gz)

The local release-shaped SQL workload used the optimized Homebrew
PostgreSQL 17.11 build and the stock claim, application fence, two
independent application writes, and settlement SQL. It drained the
frozen 64-job large profile on a reusable claimant connection and a
separate application connection. Five committed one-slot claims warmed
the SPI cache, followed by eight-slot claims and the three-job tail.
Each arm reconciled 64 attempt-1 settlements and 128 ordered
application records with distinct committed transaction IDs for the two
writes. All six orders of stock A, `LIMIT 20`, and stock B ran once
with nested tracing disabled. Separate traced drains showed the custom
ready index plan for the first five turns, then the generic ready index
plan, in all three arms. Every traced turn matched on scan rows and
cursor buffers, with zero cursor sorts or spill. [Durable replay](durable.py)

| Local ms per 64-job drain, median and range | Stock A | LIMIT 20 | Stock B |
| --- | ---: | ---: | ---: |
| Wall | 220.0 (211.0–305.5) | 262.5 (212.3–365.0) | 226.9 (219.1–300.1) |
| Jobs backend CPU | 116.6 (109.0–152.7) | 136.4 (112.0–172.6) | 119.2 (110.2–150.6) |
| Application backend CPU | 12.0 (11.4–27.4) | 12.3 (11.2–27.4) | 13.5 (11.4–27.3) |

Relative to the midpoint of the two stock controls within each order,
the candidate's median wall change was +3.04%, with a range of −6.54%
to +69.42%. The paired Jobs CPU change was +1.40%, with a range of
−0.73% to +55.00%. The host load average was 7.6–8.0 on 15 logical
cores. The matched committed plans and wide local timing spread give
no evidence of a saving. The CPU clock control matched a known busy
interval to within 0.1% after conversion from Mach ticks.

The test uses a Unix socket, a serial SQL driver, and a preloaded
cohort on macOS ARM64. Its wall window includes the client and
confirmed claim, application, and settlement transactions. Its CPU
windows cover whole backends. It omits the native runner pool,
TLS, producer timing, and worker process launch, so the measurements
provide no Linux transfer estimate.

## Historical native observation

In the Linode six-arm capture, `03-mesh-on` attributed
2.840–2.902 CPU-ms/job to the 32 stable `parity_subject` backends
and 1.774–1.781 CPU-ms/job to the 32 stable `compiled_caller`
backends. The instrumented full drain took 6.91% longer than the
trace-off midpoint. These role totals cover all activity on those
backends and leave individual claim SQL CPU unresolved.
[Native receipt and perturbation screen](../2026-11-23-native-worker-cost/README.md)

## Replay locally

Run from the repository root with the installed PostgreSQL 17.11 binaries
and Python 3. The durable CPU receipt uses macOS. Both scripts start a
private socket-only server and remove its data and socket on exit. The
checked-in compressed receipts preserve the captured counters and exclude
the raw logs and database files.

```sh
receipt=tools/durable-jobs-bench/results/2026-11-23-claim-cursor-plan
pg_bin=/opt/homebrew/opt/postgresql@17/bin
(cd "$receipt" && shasum -a 256 -c SHA256SUMS)
python3 -B "$receipt/replay.py" --pg-bin "$pg_bin"
python3 -B "$receipt/replay.py" --pg-bin "$pg_bin" \
	--output target/jobs-claim-cursor-plan/repeat --cases large_early --modes auto
python3 -B "$receipt/durable.py" --pg-bin "$pg_bin"
```
