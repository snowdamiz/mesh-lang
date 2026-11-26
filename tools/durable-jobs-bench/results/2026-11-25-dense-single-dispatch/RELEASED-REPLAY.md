# Retain two 32-slot dispatchers

Both measured `1×64` fixed-four arms finished slower than both same-block `2×32` fixed-two controls on both end-to-end clocks. The middle 820 second writes took longer, and the last second write arrived later relative to enqueue commit in all four comparisons. The original exit-1 release and closed peer gate stand. No repeat or smoke was requested from the controller. [Original release](evidence/native/controller-release.json) · [Reconciled measurements](evidence/released-replay.json)

| Block | Lane | Submit → final terminal ms | Commit → final terminal ms | Middle writes ms | Commit → last write ms | Runner CPU-ms | Queue backend CPU-ms | All paired PG CPU-ms | Claims | Claim-to-terminal peak |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 1 | Stock A | 1356.9 | 1190.6 | 911.1 | 1188.4 | 543.4 | 1090 | 1730 | 113 | 64 |
| 1 | Single four | 1579.3 | 1382.5 | 1010.9 | 1378.5 | 529.5 | 1130 | 1810 | 82 | 43 |
| 1 | Stock B | 1401.0 | 1235.6 | 921.6 | 1234.4 | 565.8 | 1150 | 1810 | 121 | 62 |
| 2 | Stock B | 1345.8 | 1188.2 | 891.2 | 1186.5 | 553.0 | 1100 | 1730 | 122 | 62 |
| 2 | Single four | 1464.6 | 1307.0 | 997.4 | 1300.5 | 494.6 | 1040 | 1690 | 79 | 43 |
| 2 | Stock A | 1435.3 | 1256.3 | 965.1 | 1255.4 | 555.0 | 1140 | 1810 | 117 | 63 |

The candidate eliminated 31–43 claim calls per batch. Against the lower-CPU duplicate it saved 13.9 and 58.4 runner CPU-ms, equivalent to 3–4% and 11–17% of the historical 337–512 CPU-ms submit target. Runner plus PostgreSQL CPU grew by 66.1 ms against the cheaper control in block 1 and fell by 98.4 ms against the cheaper control in block 2. The runner counter covers process lifetime. PostgreSQL CPU is the sum of paired process ticks in a bracket around submit through last terminal. The six brackets have 31–123 ms of endpoint slack and cover all PIDs in the two snapshots. Queue CPU is the subset from warmed `parity_subject` backends, which also execute settlement and other queue SQL. These observations do not isolate claim CPU. [Historical CPU target](../2026-11-25-dense-host-cpu-budget/README.md)

The six primary measured arms each reconciled 1,024 completed jobs, 2,048 independently committed application XIDs, 1,024 individual terminal XIDs, four hot-checkout health probes, and four TLS application backends with writes in their final statement records. Both topologies shared a global capacity of 64 and the same PostgreSQL limit of 96, CPUs 0–1, and producer. The stock controls used eight queue and 32 application TLS sockets plus the broker, for 41. The candidate used four queue and 32 application TLS sockets plus the broker, for 37. Live PID/thread frames recorded eight threads per stock runner and ten for the candidate, with three broker threads throughout. The candidate's claim-to-terminal occupancy peaked at 43 in both runs even though four backends showed real writes. The archives do not identify the cause of that occupancy difference.

## SQL export and roles

The sealed runtime compiles the 5,987-byte `claim_batch_set_v1` body from public source `1e70f18d`. All 14 exit-zero archives contain the literal string `[redacted]` in `claim_body`. Its digest begins `017ad733`. Applying the sealed export code to the public body reproduces that exact sentinel because the redaction pattern matches the public SQL expression `token = s.grant_token`. Consequently the archives cannot independently establish the live body digest. The emitted lock configuration is exactly `search_path=pg_catalog, pg_temp` and `lock_timeout=100ms` in every completed arm. The same sealed ELF and migration path, observed set-based calls, and absence of cursor fallback reconcile across the six primary arms. [Frozen worker exporter](../../worker.py) · [Public schema](../../../../compiler/mesh-rt/src/jobs/schema.sql)

All 16 arms archived the same role limits of 8 worker, 32 application, and 1 producer. Every grant record showed that only the worker acquired set-claim execution after migration. Producer and application remained denied. The worker retained cursor execution and could not insert jobs directly. The public lock contract serializes the queue capacity check with a row lock, uses a shared try-lock for the namespace, and skips locked candidate rows. The archives support the config and caller privileges. No exact native lock-wait duration was captured.

For a future capture, query `encode(sha256(convert_to(prosrc, 'UTF8')), 'hex')` in PostgreSQL and export `claim_body_sha256` plus `proconfig`. An untimed PostgreSQL 17.11 schema check returned the full expected digest and both settings, and the actual worker redactor preserved both fields. This repairs the export format without changing redaction, the function, or any privilege. It cannot reconstruct the discarded native bytes.

## Secondary and warmup failures

- Reverse-order `single-two` warmup and measurement exited at the readiness check. They saw 2 and 3 worker TLS sessions, respectively, against the adapter minimum of 4. Both retained 32 application sessions and 1 producer, drained with zero claims, and produced no ledger or score.
- Stock B warmup in block 1 and stock A warmup in block 2 exited zero but failed the stricter replay readiness count. They saw 6 and 4 worker TLS sessions at readiness against 8 expected, then reached 41 total connections in the timed samples. This establishes an early snapshot versus later occupancy difference. The archives do not identify which queue connections were late.
- Forward-order `single-two` measured 1395.5 ms from submit, 1238.8 ms from enqueue commit, and 944.5 ms for the middle writes, using 425.2 runner CPU-ms. Only this order yielded a secondary measurement, so repeatability at fixed two remains untested.

## Reproduce from the checkout

```sh
d=tools/durable-jobs-bench/results/2026-11-25-dense-single-dispatch
python3 -B "$d/replay-released.py" | cmp - "$d/evidence/released-replay.json"
python3 -B "$d/test_offline.py"
```

The supplemental replay checks the original grant, release, source pins, staged inventories and rejection, then reuses the sealed scorer's remaining checks. It fixes the scorer's incorrect module reference for the occupancy helper within the replay. The original scorer and timing request stay byte-exact. The six measured primary observations do not constitute a qualified two-block campaign.
