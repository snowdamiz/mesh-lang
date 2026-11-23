# Preparing PostgreSQL argument text for a large Mesh claim

Caching the canonical argument text saved PostgreSQL a median **32.6 CPU-ms when claiming 64 jobs** in four matched rotations. Preparing and storing that text at enqueue added **314.7 CPU-ms** against the faster duplicate stock control. All four batches used more backend CPU through settlement. Keep the stock JSONB representation for this workload.

Each local PostgreSQL 17.11 arm submitted the same 64 × 512 KiB input under one transaction, using 64 scoped enqueue calls and savepoints. It claimed the global ordered prefix in eight groups of eight, checked every returned context byte against the stock SQL expression on the same tuple and checked the parsed arguments against the frozen input. Each job had its own durable terminal transaction. This SQL component omits handlers, application effects, and runner startup. Timing used a Unix socket on macOS ARM64. The scored Linux TLS workload is recorded separately. [Five native comparisons at equal work_mem](../2026-11-23-iteration-comparison/four-engine-scorecard.md)

## Two ways of maintaining a cache

Both campaigns used two stock controls, 16 MiB work_mem, EXTERNAL JSONB storage, synchronous commits, full-page writes, one fresh database per arm, and four rotated orders. Live text assembles the stock output using the locked argument's JSONB text and the stock expression for the remaining keys. The generated column and the explicit-argument trigger each add an EXTERNAL text column.

| Campaign and arm | Enqueue CPU ms | Claim CPU ms | Settle CPU ms | Enqueue / claim / settle WAL MiB | Final TOAST heap MiB |
|---|---:|---:|---:|---:|---:|
| Generated column, stock A / B | 257 / 252 | 128 / 131 | 31 / 30 | 34.33 / 0.04 / 0.04 | 32.88 |
| Generated column, live text | 241 | 125 | 29 | 34.33 / 0.04 / 0.04 | 32.88 |
| Generated column, cached | 512 | 236 | 204 | 68.59 / 35.20 / 35.20 | 131.50 |
| Explicit argument trigger, stock A / B | 336 / 328 | 112 / 120 | 25 / 24 | 34.33 / 0.04 / 0.04 | 32.88 |
| Explicit argument trigger, live text | 314 | 121 | 26 | 34.33 / 0.04 / 0.04 | 32.88 |
| Explicit argument trigger, cached | 636 | 79 | 24 | 68.59 / 0.04 / 0.04 | 65.75 |

Values are per-arm medians across four rotations. The headline differences are computed within each rotation against its faster stock control. Per-stage medians and total CPU medians are computed independently. One-minute host load ranged from 56–65 in the first series and 84–153 in the second. Backend CPU is from raw Mach task ticks calibrated against process CPU and getrusage at 125/3 ns per tick. These captures support the physical write-cost comparison. Native throughput remains a separate measurement. [Per-arm medians, paired differences and archive digests](summary.json)

A separate physical witness followed one job's actual TOAST chunks. With the generated column, the JSONB chunk ID survived enqueue, claim, settlement and an unrelated priority self-assignment. The text received a new chunk ID at each update despite keeping the same 524,359 bytes, 263 chunks, and SHA-256. The explicit trigger kept both IDs across all three updates. Editing the actual arguments changed both digests and yielded the exact updated text. This fixed the claim and settlement write amplification. The trigger still wrote a second copy at enqueue and increased total backend CPU by a paired median of 284.9 ms per 64 jobs. The trigger is an experimental schema modification scoped to these writes.

## Replay

From a clean checkout, verify both archives, all 32 accepted arms, the original partial attempt and both physical witnesses.

```sh
python3 -B tools/durable-jobs-bench/results/2026-11-23-large-arg-render/replay.py
```

To repeat the live experiments, use macOS with PostgreSQL 17.11 and its matching `PG_BIN_DIR`. Supply fresh output directories. The timing screen takes the existing local queue-performance lock.

```sh
root=tools/durable-jobs-bench/results/2026-11-23-large-arg-render
python3 -B "$root/screen.py" target/arg-render-generated-repeat
python3 -B "$root/toast.py" target/arg-render-generated-toast
python3 -B "$root/screen.py" --on-args-write target/arg-render-write-repeat
python3 -B "$root/toast.py" --on-args-write target/arg-render-write-toast
```

The first generated-column attempt stopped after preserving one stock arm because the progress printer tried to round the user/system CPU dictionary. The retry reports the sum in that printout and uses the original per-stage counters in the receipts. [First campaign and failed attempt](receipts.tar.gz) · [Explicit-trigger campaign](args-write.tar.gz)
