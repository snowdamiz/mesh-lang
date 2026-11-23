# Run the Go River durable jobs benchmark

Use Go 1.26.4, Python 3.11 or later, and a dedicated PostgreSQL 17.11 cluster.
The Go module pins the open-source River client and pgx driver at v0.51.2
and pgx at v5.13.0. `go.sum` verifies the module contents.

## Prepare the binary and inputs

Run from the repository root. Set `profile` to `noop`, `slow`, `sparse`,
`short`, or `large`. Build and generate the inputs before the timing window.

```sh
profile=short
work="$PWD/target/linode-river"
mkdir -p "$work/build"
(
	cd tools/durable-jobs-bench/go
	GOTOOLCHAIN=local CGO_ENABLED=0 go build -trimpath -mod=readonly -buildvcs=false \
		-o "$work/build/river-bench" .
)
python3 -B tools/durable-jobs-bench/go/prepare_input.py "$profile" "$work/input-$profile"
```

The generator calls the parity harness used by the
[frozen Linux regression proof](../../../proof/jobs-parity/linux-regression/README.md).
Both the generator and the runner check the ordered batch digest. For a
remote trial, stage the Linux binary and the generated profile input directory
in advance.

## Run a profile

Match the PostgreSQL cluster's `max_connections` and database
`CONNECTION LIMIT` to the profile. Use zero reserved connections and
enable `track_commit_timestamp`, `synchronous_commit`, `fsync`, and
`full_page_writes`. The runner checks these settings over TLS before it
creates a schema. Use a dedicated database with no other client sessions.
The benchmark role needs permission to create and own schemas in that database.

| Profile | Runner processes | Total slots | PostgreSQL cap | Jobs |
| --- | ---: | ---: | ---: | ---: |
| noop | 8 | 64 | 96 | 1,024 |
| slow | 1 | 64 | 68 | 512 |
| sparse | 1 | 1 | 5 | 24 |
| short | 1 | 16 | 20 | 128 |
| large | 8 | 64 | 96 | 64 |

Set `RIVER_BENCH_DATABASE_URL` in the environment using the benchmark role
and the cluster's CA file. The URL uses a loopback IP, an explicit port
and database, `sslmode=verify-full`, and an absolute `sslrootcert` path.
The certificate needs a matching IP subject alternative name.

Run the built binary on the PostgreSQL host. Set CPU affinity and host
limits through the matrix launcher. The controller and each River process
use `GOMAXPROCS=2`. Use a fresh evidence directory for every attempt.

```sh
"$work/build/river-bench" -profile "$profile" -input "$work/input-$profile" \
	-out "$work/short-01"
```

For a setup smoke, add `-smoke` and choose a fresh output directory.
That adds a separate rolled-back enqueue and a one-retry job with a
deduplicated effect. The no-op, slow, and large profiles run one full
unmeasured warmup before the measured arm. Every arm migrates a fresh
schema and drops it after evidence capture. Each arm also has a
three-second idle window before enqueue.

River uses its stock PostgreSQL listener, 250 ms fallback polling,
and a 5 ms worker fetch cooldown. Each runner's pool allows its
worker slots plus two connections, including the listener. The warm
producer and observer share one connection. The producer commits the
application product row and `InsertManyTx` batches in one transaction.
Each job writes the attempt, holds the same connection during the fixture
delay, then writes the effect in a second autocommit transaction. The
worker returns and River persists the terminal state.

## Archive the evidence

Keep the full output directory from each attempt. `status.json` records
whether the run passed. A failing arm keeps enqueue receipts, observation
samples, runner logs, and the ledger when the database is reachable.

- `config.json` records the binary digest, module versions, and runtime settings.
  `database.json` records the PostgreSQL version, TLS, and connection limits.
- `measured/enqueue.jsonl` records request IDs, transaction IDs, inserted job
  IDs, and enqueue commit timestamps.
- `measured/ledger.json` contains payload hashes, job order and state, attempt
  numbers, and both application write identities and commit timestamps.
  The runner checks that the writes use distinct transactions and commit
  in order before the terminal job commit.
- `measured/samples.jsonl` contains database counts and runner resource
  samples. The controller samples while waiting for settlements, at
  100 ms intervals, and twice during idle. Linux runner RSS and thread
  counts come from `/proc`.
  River can keep a job row in `running` after its handler returns while
  the stock completer waits to persist a batch. Each runner records its
  own handler high-water mark in `runner-N-capacity.json`. The controller
  checks it against that runner's slot allocation and reconciles handler
  calls with the durable attempt rows.
- `measured/metrics.json` uses the earliest PostgreSQL enqueue commit and
  latest PostgreSQL terminal row commit for throughput. Pickup starts
  at each job's enqueue commit and ends at worker entry. Claim latency
  uses River's `attempted_at`. Settlement latency ends at the terminal
  row's commit. Runner CPU includes startup, idle, and shutdown.

Archive `warmup/` and `faults/` alongside `measured/` when present. Retain
the exact input directory or its frozen digest with the matrix run.

## Run through the native matrix controller

Use the adapter entry point when the matrix provides PostgreSQL 17.11,
the CPU mask, and a fresh output directory. Run under an unprivileged
user that can write the output and scratch directories. PostgreSQL needs
`pg_stat_statements` available for the shared fixture cluster.

```sh
build=/var/lib/durable-jobs-bench/river/builds/river-0.51.2-e00cbd00-b
source=/var/lib/durable-jobs-bench/river/builds/river-0.51.2-e00cbd00-verifier-f208a4ca/source
"$source/tools/durable-jobs-bench/go/run" \
	--binary "$build/river-bench" \
	--pg-bin /var/lib/durable-jobs-bench/native/toolchain/pgsql-17.11/bin \
	--scratch /var/lib/durable-jobs-bench/river/scratch \
	short "$output"
```

Pass an existing empty absolute directory as `output`. The adapter provisions
a capped TLS cluster in scratch, runs one arm, and writes the matrix
`ledger.json` at the output root. The matrix controller schedules warmup.
The ledger includes submit, enqueue commit, pickup, and durable settlement
timestamps in epoch microseconds. Its `peak_executing` is the sum of per-runner
handler high-water marks, an upper bound across runner processes.

The SETUP-only Linux x86-64 binary has SHA-256
`e00cbd00137964ddda85f6eb96835b6b8d0b347ec91e68147c99d28eaf2d7131`.
Native setup checks passed for both wrappers on short and sparse. The revised
wrapper verifies raw write counts, transaction identities, versions, and slot
receipts before it translates results. Short passed matrix ledger
reconciliation. Sparse verified rollback, retry, and effect deduplication.
Matched-submit timing is on HOLD. See
`target/linode-river/READY.json` for the setup handoff. The
[smoke bundle](../results/2026-11-23-river-smoke/README.md) preserves the
source, build pins, earlier attempts, raw evidence, and offline replay.
