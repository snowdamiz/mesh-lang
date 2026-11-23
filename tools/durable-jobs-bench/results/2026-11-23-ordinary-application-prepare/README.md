# Reusing plans for ordinary Mesh handler writes

Reusing the two application statements saved a median **0.0564 PostgreSQL CPU-ms per write pair** against the lower-CPU duplicate control in each rotation. The controls differed by 0.0505 CPU-ms per pair at the median. Elapsed time improved in three of six rotations. These results support keeping the two-CPU Linode slot for the wider queue experiments. [Raw per-backend counters, plan counts and committed rows](measurements.tar.gz)

| PostgreSQL 17.11 component | Stock A | Named statements | Stock B |
|---|---:|---:|---:|
| Backend CPU per pair, median ms | 0.3976 | 0.3339 | 0.4371 |
| Elapsed per 1,024 pairs, median ms | 604.8 | 638.3 | 794.7 |

Eight TLS connections executed the trusted handler's two original parameterized INSERTs under its scoped login and row-level policies. Every write had its own synchronous commit. All arms retained the pool's two health queries per pair. In the candidate, each connection prepared both writes once, inside the measurement window. PostgreSQL counted five custom plans and 123 generic plans per statement on every candidate connection. The stock sessions issued an unnamed extended query for each write. [Original compiled fixture and policy](../2026-11-23-ordinary-trusted-v1/README.md)

The six permutations used identical stock controls and fresh connections. The replay verifies 18,432 pairs, including row identities, ordered commit times and distinct transaction IDs. The host ran macOS ARM64, with one-minute load between 49.3 and 68.0. The client drove libpq directly. The CPU column charges only the eight PostgreSQL processes and the wall column includes the client. A compiled runtime, pool checkout implementation, queue claim and settlement would add work outside this component. The separate Linux comparison measures the complete queue stacks. [Native matched comparisons](../2026-11-23-iteration-comparison/chart.html)

## Replay

From this directory, verify the archive and recompute both rows.

```sh
shasum -a 256 -c SHA256SUMS
python3 -B replay.py
```

To repeat the live macOS component from the checkout root, set `PG_BIN_DIR` to the PostgreSQL 17.11 installation. Use a fresh output directory under `target/`. The runner calibrates the Mach CPU clock, acquires the shared local performance lock and removes its disposable cluster after the run.

```sh
PG_BIN_DIR=/opt/homebrew/opt/postgresql@17/bin \
  python3 -B tools/durable-jobs-bench/results/2026-11-23-ordinary-application-prepare/measure.py \
  target/ordinary-application-prepare-repeat
```
