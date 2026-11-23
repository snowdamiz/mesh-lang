# PostgreSQL STORAGE EXTERNAL peer qualification

The native SETUP passed for Oban, River, and Apalis on CPUs 0 and 1. Each peer used its pinned stock executable and PostgreSQL 17.11 with verified TLS, synchronous commit, fsync, full-page writes, and commit timestamps. Each raw archive reconciles 64 terminal jobs, 33,554,432 logical payload bytes, and 128 distinct committed application writes. PostgreSQL reported no compression on any of the 192 payload column values. The adapter recorded DDL before the first submission and inspected the persisted rows after the final settlement.

- [Symmetric inventory](large.json) pins the three peers and both staged Mesh EXTERNAL variants. One Mesh variant uses the stock broker, and the other uses the direct-file broker.
- [Untuned peer and Mesh-only inventory](attempt-456ed505cc9b49d38dbfb181d4ca8bfd/parent-large.json) preserves the exact input from the first Mesh-only series.
- [Native raw archives and release receipt](attempt-456ed505cc9b49d38dbfb181d4ca8bfd/) preserve the original attempt IDs, install log, host samples, driver exit codes, inventories, and per-engine PostgreSQL storage evidence.
- [Frozen overlay and installer](freeze/) reproduce the bytes staged for this SETUP. Their hashes are in the [frozen request](attempt-456ed505cc9b49d38dbfb181d4ca8bfd/request.json).

The setup changed `parity_*.oban_jobs.args`, `parity_*.river_job.args`, and `apalis.jobs.job` from `EXTENDED` to `EXTERNAL` on empty tables. Oban and Apalis ran the DDL as the existing table owner after migration. River required two database-local event triggers because the stock process creates and drops its randomly named schema. The guarded `CREATE TABLE` hook applied the DDL and removed itself during migration. The `DROP SCHEMA` hook captured the persisted rows during cleanup and then removed itself. Both hooks used the existing disposable cluster owner and left the peer login roles unchanged. [PostgreSQL 17 event trigger functions](https://www.postgresql.org/docs/17/functions-event-triggers.html)

The archived `ddl_us` for Oban and Apalis includes the verified TLS `psql` call. River records its server-side DDL and hook installation separately. The storage snapshot records its own start and finish in all three receipts. The controller owns timing and uses the standard whole-arm wall clock and separate submit-to-settlement and commit-to-settlement windows. These SETUP smokes qualify correctness, and the stock-policy and Mesh-only timing series retain their own inventories.

## Replay the native receipts

Run from the checkout root. The command opens the local archives and reconciles the pinned file identities, CPU mask, database settings, and durable ledger.

```sh
result=tools/durable-jobs-bench/results/2026-11-23-payload-storage-peers
attempt="$result/attempt-456ed505cc9b49d38dbfb181d4ca8bfd"
python3 -B "$result/replay-native.py" "$attempt" | diff "$attempt/replay.json" -
(cd "$result" && shasum -a 256 -c SHA256SUMS)
```

The local macOS archive contains two fresh PostgreSQL 17.11 trials per peer. Oban and Apalis passed the stock scoring gate. River finished both workloads and passed the stock raw ledger verifier. Both stock River adapters then exited with `invalid peak runner RSS` because the macOS binary has no Linux `/proc` sampler. The scored native River smoke passed. [Local replay results](local-replay.json)
