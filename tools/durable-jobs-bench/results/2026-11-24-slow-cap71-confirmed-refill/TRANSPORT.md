# Shared producer transport on the held slow pilot

The original transport failure was a timeout waiting for the public
`mesh session` broker to return an enqueue receipt. The first two replies
were `SESSION_READY` and the TLS, role, and database-cap witness. The raw
broker output ended after those replies and stderr was empty. The frozen
driver's `reply()` waits for a complete stdout line for at most 15 seconds
after the request flush, within the enclosing fixture deadline.
The public producer uses one connection and runs the whole batch inside its
synchronous `session_loop` before it sends that line.
[Initial attempt summary](evidence/local-initial-probe.json)

The first failing command below ran concurrently with the same command on
the public schema. It uses the retained Darwin candidate to migrate the
database and the retained Darwin public executable for the inline broker.
These are untimed proof commands.

```sh
p=tools/durable-jobs-bench/results/2026-11-24-slow-cap71-confirmed-refill
env TMPDIR="$PWD/target/slow-cap71-confirmed-refill/tmp" \
    PG_BIN_DIR=/opt/homebrew/opt/postgresql@17/bin \
    PRODUCT_SOURCE="$PWD/target/short16-fused-refill/source" \
    PRODUCT_FROZEN_HARNESS="$PWD/target/ordinary-completion-writer/combined/bulk-base/ordinary-v2" \
    python3 -B "$p/proof.py" target/slow-cap71-confirmed-refill/evidence/local-candidate-03 \
    target/short16-fused-refill/artifact-03/mesh \
    target/2026-11-24-trusted-jobs-product-composition/local-fixture-sealed/mesh
```

The error was `harness.FixtureError: timed out waiting for shared
producer/observer response`. It occurred during the canonical enqueue
request before the first enqueue receipt was available to the driver.
Use a fresh destination to replay the command. The local executions used
only the owned disposable PostgreSQL database.

A later public session with the original watchdog showed the live broker
inside `Jobs.enqueue_in` and Rust job-spec parsing and serialization after the
inline request was sent. That session completed. A serial candidate replay
also completed with the original watchdog, and the next instrumented
concurrent pair both passed. The first timeouts therefore remain
intermittent. The original failed runs did not retain live PostgreSQL state
at the deadline, so the exact trigger remains unresolved.
[Public session proof](evidence/shared-session-public.json) · [Candidate session proof](evidence/shared-session-candidate.json)

The separate direct-file proof uses the public executable's `enqueue`
command with the canonical file. It persisted 512 uncompressed 16 KiB jobs
in one outer transaction on both schemas. The candidate schema also
retained its 64 executing rows across two scoped refill transactions and
honored queue controls and rollback on lock timeout.
[Public direct-file proof](evidence/direct-file-public.json) · [Candidate direct-file proof](evidence/direct-file-candidate.json)

A controlled lock proved how the same timeout can occur after the broker
consumes the inline request. The diagnostic held an exclusive lock on
`mesh_jobs_parity_product` before the producer's first INSERT. At the
watchdog deadline the broker was alive and PostgreSQL showed an open
producer transaction waiting on the relation lock. No jobs were committed
and stdout still contained only the two setup replies. The diagnostic
killed the blocked producer before it released the lock and verified that
zero jobs remained. This proves the pre-receipt failure mode. The first
timeouts had no corresponding PostgreSQL lock record.
[Controlled lock receipt](evidence/controlled-marker-lock.json)

The separate slow-cap71 pilot remains held. The public same-seal native
series has its own controller request and is outside this diagnostic.
