# Compiled Mesh parity fixture

## Build the executable

Stage the hidden `.parity` project in a scratch directory inside the checkout.
`meshc build tests/e2e/durable_jobs` skips hidden sources and builds the
existing proof. From a source build, provide both `meshc` and its matching
`mesh-rt` static library.

```bash
mkdir -p "$project"
cp -R "$repo/tests/e2e/durable_jobs/.parity/." "$project/"
target=$(rustc -vV | sed -n 's/^host: //p')
cc -std=c11 -O2 -c "$project/worker_identity.c" -o "$project/worker_identity.o"
ar rcs "$project/libworker_identity.a" "$project/worker_identity.o"
digest=$(shasum -a 256 "$project/libworker_identity.a" | cut -d ' ' -f 1)
sed -e "s/@TARGET@/$target/" -e "s/@SHA256@/$digest/" \
	"$project/mesh.toml.in" > "$project/mesh.toml"
"$meshc" build "$project" --target "$target" --output "$binary"
```

Use `sha256sum` in place of `shasum -a 256` when needed. The fixture's
native binding reads the OS process ID and the local realtime clock.

## Driver contract

Call the compiled binary with one argument, `migrate`, `enqueue`, or `run`.
Each action requires `JOBS_PARITY_DB_URL` and `JOBS_PARITY_NAMESPACE`.
Use namespaces that match `[a-z][a-z0-9_]{0,62}`.

- `migrate` runs `Jobs.migrate` and configures the default queue's database-wide
  limit from `JOBS_PARITY_TOTAL_SLOTS`.
- `enqueue` reads the absolute path in `JOBS_PARITY_INPUT`. A single
  PostgreSQL transaction writes the business row and enqueues the jobs.
  Larger manifests split across calls of up to 128 specs and 1 MiB within
  that transaction.
  Standard output carries one line, `{"count":N,"xid":"..."}`, after commit or
  intentional rollback. The count is the input job count in both cases.
  The xid comes from `pg_current_xact_id()` inside that transaction.
- `run` starts a named isolated handler using `JOBS_PARITY_SLOTS` local
  slots and the fallback interval in `JOBS_PARITY_POLL_MS`. When startup
  succeeds, it prints `READY` to standard output. Send `SIGTERM` for shutdown.
  The process prints the final `Jobs.join` report after a graceful stop.

`JOBS_PARITY_MODE` accepts `dense`, `sparse`, or `fault`. The runner uses a
30,000 ms lease in dense and sparse modes and a 1,500 ms lease in fault mode.
Every runner has a 15,000 ms shutdown grace and a pool of one to four
connections. Each child opens one connection to write its attempt and effect.
The driver controls the enqueue schedule. The run action requires a
`JOBS_PARITY_RUNNER` label for error messages.

```json
{"jobs":[{"key":"job-1","payload":"sample","hold_ms":0,"fail_first":false}],"rollback":false,"request_id":"batch-1"}
```

Each job key also serves as the Mesh enqueue request key within the namespace.
Use distinct job keys for distinct jobs and reuse the same key and payload
when replaying an enqueue. The job key must be nonblank and at most 255
UTF-8 bytes. The driver creates the public tables before launching the
compiled fixture.

- `mesh_jobs_parity_product(engine, namespace, request_id)` stores business
  writes. Give the triple a unique constraint to deduplicate replay.
- `mesh_jobs_parity_attempts(engine, namespace, job_key, effect_key, attempt,
  entered_at, worker_pid)` stores one row per handler entry. The handler
  captures wall-clock time before decoding the job or connecting to
  PostgreSQL. The worker PID identifies the isolated Mesh process.
- `mesh_jobs_parity_effects(engine, namespace, job_key, effect_key, effected_at)`
  stores the idempotent write. Give `(engine, namespace, effect_key)` a unique
  constraint. The Mesh engine value is `mesh`.

With `fail_first: false`, the handler records the attempt, sleeps for
`hold_ms`, writes the effect, and completes. With `fail_first: true`, the
first attempt records the attempt and writes the effect before sleeping.
It then requests a retry in 1,000 ms. Later attempts skip the hold and try the
same effect key again. Each hold starts after its barrier row commits.
The driver can wait on that row before signaling the worker.
