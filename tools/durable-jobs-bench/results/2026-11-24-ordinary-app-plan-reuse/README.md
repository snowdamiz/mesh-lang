# Ordinary Pool.execute plan reuse

The Linode component is staged for the sole matrix controller. The workstation diagnostic ended with zero timed arms. The first launch stopped at a statistics-reset setup error, the second was canceled while refining the source-text match, and the last exited while waiting for both timing locks after the routing change. The interrupted receipts remain archived. The untimed PostgreSQL correctness check passed both wire paths and verified four separately committed XIDs. [Interrupted workstation receipts](interrupted-workstation.tar.gz)

The five archived native Mesh V1 2 × 32 runs finished their application writes 373.2–489.3 ms later than River, measured from each arm's own enqueue commit. This is the scale for the component decision. The new benchmark has no measured result yet. [Native stock comparison](../2026-11-23-dense-matched-topology/evidence/stock/scorecard.md)

## Experiment

The control calls `PQexecParams` for each INSERT and the candidate uses one protocol `PQprepare` per statement per physical connection and then `PQexecPrepared`. Both use text bindings, the same two byte-for-byte SQL strings, scoped role, row-level policies and indexes, and a blocking call through `ReadyForQuery` for each write. Each checkout in the timed workload issues the stock `SELECT 1` before the INSERT. libpq adds a portal description in both arms. The component isolates the libpq path and PostgreSQL work from the Mesh pool, scheduler, and queue traffic. [Stock pool](../../../../compiler/mesh-rt/src/db/pool.rs) · [Stock wire path](../../../../compiler/mesh-rt/src/db/pg.rs) · [Frozen V1 source](../2026-11-23-ordinary-trusted-v1/ordinary-trusted-v1.tar.gz)

The inputs are the 1,024 job keys, UUIDs, attempt numbers, entry timestamps and runner PIDs from measured V1 2 × 32 block one. The namespace comes from its application ledger, and the generation is each archived job UUID followed by `/1`, as emitted by V1. Two sessions replay each original runner PID's jobs. The same assignment, attempt/effect order and four physical TLS backends repeat in every arm. Every INSERT must return exactly one affected row. After the clock stops, a separate observer compares stored values and verifies all 2,048 distinct committed row XIDs and attempt/effect commit order. [Native inputs](../2026-11-23-dense-matched-topology/evidence/stock/blocks/01-noop.1795480034353792000/measured-mesh-v1-2x32/measured-mesh/raw.tar.gz)

On the pinned Linux AMD64 Linode, the client and PostgreSQL run on CPU IDs 0 and 1. A private PostgreSQL 17.11 cluster listens only on loopback and requires TLS. Both session groups use `sslmode=verify-full`. Synchronous commit, fsync, full-page writes, commit timestamp tracking, planner statistics and WAL I/O timing are enabled for every arm. Six permutations rotate two identical unnamed controls with the named candidate. A table truncate, statistics reset, and new TLS sessions precede each arm. Named Parse is measured separately. The warm wall clock starts with the four writer threads ready and stops after all writes return. Process CPU covers the Python/libpq client. Linux scheduler counters measure the four backend processes and are calibrated against the client process CPU clock. PostgreSQL planner time captures the planner portion of repeated statement work. The CPU and wall deltas include the effect of skipping Parse and reusing plans. WAL writes and sync times help interpret the remaining work. Host samples bracket each arm. A builder at either boundary or more than 1% measured steal disqualifies the campaign.

The implementation threshold was fixed before timing. The named arm must save at least 75 ms at the paired median against the faster duplicate and beat the duplicate spread in at least four of six rotations. The threshold is about one sixth of the native dense gap. A qualifying component result would authorize a separate ordinary Pool.execute implementation and native A/B. Its correctness checks must cover names and errors, aborted statements, DDL, DEALLOCATE and DISCARD, connection replacement, SQL role and search path, and transport outcomes where a write may have committed. [Native stock ledgers](../2026-11-23-dense-matched-topology/evidence/stock/scorecard.md)

## Peer application SQL

- **River 0.51.2 / pgx 5.13.0** uses named statements on demand for these two parameterized worker `conn.Exec` calls. The worker passes no mode override, the harness supplies a connection URL without cache settings, and the pinned pgx code defaults to `QueryExecModeCacheStatement`. Its Exec path prepares a generated name on a miss and executes that prepared statement on the same connection. This is inferred from the pinned driver and adapter code. [Worker](../../go/worker.go) · [Pool setup](../../go/db.go) · [Harness DSN](../../go/run.py) · [pgx defaults](https://github.com/jackc/pgx/blob/v5.13.0/conn.go#L156-L201) · [pgx Exec](https://github.com/jackc/pgx/blob/v5.13.0/conn.go#L499-L516)
- **Oban 2.24.2 / Ecto SQL 3.14.0 / Postgrex 0.22.4** sends both app INSERTs through raw `Repo.query!` with no `cache_statement` option. The Ecto PostgreSQL adapter forwards those queries to Postgrex. Postgrex selects the unnamed extended-protocol path for that call. This is inferred from the pinned driver and adapter code and is specific to these writes. [Worker](../../../../scripts/fixtures/jobs-parity-oban/lib/worker.ex) · [Dependency lock](../../../../scripts/fixtures/jobs-parity-oban/mix.lock) · [Ecto forwarding](https://github.com/elixir-ecto/ecto_sql/blob/v3.14.0/lib/ecto/adapters/postgres/connection.ex#L118-L122) · [Postgrex query](https://github.com/elixir-ecto/postgrex/blob/v0.22.4/lib/postgrex.ex#L268-L305)

## Controller and replay

[Controller request](request.json) pins the driver, historical row sources, and all six orders. The controller's active grant and the local timing fence, remote active-arm fence and native fence must precede dispatch. The staged command authenticates the expected boot, kernel, disk and PostgreSQL binary before writing any measured rows. Raw receipts and logs return to `target/app-plan-reuse/native-01`. The controller records the actual release. Repository consumers can verify the result offline.

```sh
packet=tools/durable-jobs-bench/results/2026-11-24-ordinary-app-plan-reuse
python3 -B "$packet/native.py" check
python3 -B "$packet/replay.py" target/app-plan-reuse/native-01/raw.tar.gz
```

The offline check verifies the archived dense statements, job parameters, six rotations and staging shell without starting PostgreSQL. The following workstation command only checks write correctness. Benchmark execution is restricted to the pinned native host, behind the controller grant.

```sh
PG_BIN_DIR=/opt/homebrew/opt/postgresql@17/bin python3 -B "$packet/verify-wire.py"
python3 -B -m unittest discover -s tools/durable-jobs-bench -p 'test_matrix.py'
(cd "$packet" && shasum -a 256 -c SHA256SUMS)
```
