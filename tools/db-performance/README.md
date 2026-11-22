# Database driver benchmarks

Build the same Mesh fixture against each revision's release runtime. Run the
commands below from the checkout containing this fixture. Set
`BASE_CHECKOUT` and `CANDIDATE_CHECKOUT` to the two source trees.

```sh
mkdir -p target/db-performance

for checkout in "$BASE_CHECKOUT" "$CANDIDATE_CHECKOUT"; do
  (cd "$checkout" && cargo build --locked --release -p meshc -p mesh-rt)
done

MESH_RT_LIB_PATH="$BASE_CHECKOUT/target/release/libmesh_rt.a" \
  "$BASE_CHECKOUT/target/release/meshc" build tools/db-performance/mesh \
  --opt-level 2 -o target/db-performance/baseline
MESH_RT_LIB_PATH="$CANDIDATE_CHECKOUT/target/release/libmesh_rt.a" \
  "$CANDIDATE_CHECKOUT/target/release/meshc" build tools/db-performance/mesh \
  --opt-level 2 -o target/db-performance/candidate

python3 tools/db-performance/bench.py \
  --baseline target/db-performance/baseline \
  --candidate target/db-performance/candidate
```

The runner needs Python 3.11 or later. For PostgreSQL modes it uses `psql`,
`initdb` and `pg_ctl`. Set `PG_BIN_DIR` if these tools are outside `PATH`
and `pg_config` cannot find them. With no `MESH_TEST_DATABASE_URL`, it starts
a loopback-only cluster under `.tmp/` and removes it on exit. To use an
existing **test** server, point `MESH_TEST_DATABASE_URL` at a role that can
create a database. The runner creates and drops a uniquely named database
for each run. The PostgreSQL wire counter needs a test server that accepts
non-TLS connections.

Each block runs both binaries in shuffled order for each combination of mode
and actor count. The default actor counts are 1, 16, and 64.

| Mode | Each actor does |
| --- | --- |
| `pg_point` | Read a row by primary key using `Pool.query` |
| `pg_typed` | Read the same row using `Pool.query_values` |
| `pg_write` | Insert one row per call using `Pool.execute` |
| `pg_rows` | Read and decode 10,000 rows of eight columns using `Pool.query_as` |
| `pg_sleep` | Read a row after `pg_sleep(0.05)` |
| `sqlite_point`, `sqlite_write`, `sqlite_rows` | Run the equivalent work on a fresh SQLite file shared by the actors in each trial |
| `sqlite_mixed` | Read on separate connections while a writer holds an exclusive transaction. Select with `--mode sqlite_mixed --actors 16`. |
| `idle` | Measure a ticker without database work when selected with `--mode idle` |

The fixture checks results, measures each call with the monotonic clock, and
records lateness for a ticker that sleeps for 1 ms. Setup, warmup, and the
post-trial check of persisted writes are outside the timing window.
The proxy counts PostgreSQL frontend `Query` and `Sync` messages as round
trips and counts `Parse` separately. The runner excludes warmup traffic.

`run.json` holds individual samples, trial failures, workload settings, and
both executables' SHA-256 digests. `comparison.csv` has the median and range
of each per-trial metric. Use `--out` to keep runs in separate directories.
On macOS and Linux, the runner also records each fixture process's peak resident
memory in bytes, including setup and warmup.
The runner exits nonzero if a scenario has fewer than five valid blocks for
either binary. Check the ticker sample count before comparing its p99.

For the SQLite WAL check, run `--mode sqlite_mixed --actors 16 --workers 1`.
The fixture counts every read error. It verifies that every successful reader
finished before the writer released the transaction and checks that the
writer's row committed. A rollback-journal baseline can exit with failed
trials due to `database is locked`. The receipts retain the error counts for
each side.

For an A/A calibration, check host load with `uptime`, pass the same
executable on both sides and add `--check-noise`. The command exits nonzero
when the median trial duration, call p50, call p99, or ticker p99 differs by
more than 5% in any scenario.
Use `--mode` repeatedly to select a subset and `--workers 1` for scheduler
experiments. `--sqlite-write-work` adds recursive SQL work to each
`sqlite_write` so that the ticker spans a long write on one worker.

For the D1 ticker check, compare `--mode idle --actors 16 --workers 1 --iterations 6400`
with `--mode pg_sleep --actors 16 --workers 1 --iterations 4 --pg-sleep-spacing-ms 100`.
Use separate output directories. The loaded run schedules 64 calls across
the 16 actors, with 100 ms between their scheduled start times. Pacing gives
the ticker time to wake between blocking 50 ms SQL calls. The reported call
latencies exclude the pacing delay. Every ticker sample comes from an
observed wake.

```sh
python3 tools/db-performance/bench.py --self-test
bash scripts/verify-db-drivers.sh
```

The driver proof exercises the PostgreSQL and SQLite runtime and compiled
programs. Its disposable PostgreSQL 17 cluster also enables the TLS
certificate and MD5 authentication tests. For an existing compatible test
server, use a disposable database with the session time zone set to UTC.
Set `MESH_TEST_DATABASE_CA` to the URL-encoded CA path and
`MESH_TEST_DATABASE_MD5=1` if the server uses MD5 authentication for the
`mesh_md5_test` role. Each of these two tests is skipped with a message
when its prerequisite is absent.
