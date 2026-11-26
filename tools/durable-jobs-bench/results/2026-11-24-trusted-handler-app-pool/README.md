# Trusted Jobs application-pool handoff

The isolated candidate adds `Jobs.start_with_app_pool(queue_pool, config, handler, app_pool)`. The handler has type `fn(String, PoolHandle) -> String`. The caller supplies a distinct, open application pool and retains ownership through `Jobs.stop` and `Jobs.join`. The runtime checks the explicit `trusted_shared_process` execution mode before binding the handle. The handler is absent from the fresh-process registry. Each attempt receives its runner’s handle inside the existing trusted actor entry. The exit callback, lease checks, and terminal settlement use the exit-confirmed `db5937e7` substrate. [Source patch](candidate.patch) · [Build capsule](candidate-source.tar.gz)

The frozen V1 fixture starts `ApplicationPool` with the application-owned handle and checks the return from `Process.register("parity-app-pool", owner)`. Each job makes one synchronous `ApplicationPool.current(Process.whereis("parity-app-pool"))` call after decoding its arguments and before writing its attempt row. The observer confirms that the name resolves to the newly started owner and that `Current` replies before readiness. The full ledger verifies the handle’s application role and the two independently committed writes. River and Rust hand the pool to workers in their state. The five archived equal-topology dense V1 runs show four used application sessions out of 32 warmed. Those ledgers contain entry and commit boundaries. They cannot isolate service waits or the first write. [Archived fingerprints](historical-fingerprints.json)

## Linode comparison

Each group runs six lanes in balanced order. Frozen V1 retains its original binary. The two service controls use the identical rebuilt exit-confirmed runner and byte-identical V1 handler source. The direct lane takes the typed application handle. The two observed variants add handler stamps and a single log record after the second successful write. All six use the pinned V1 broker and migrator. All four exit-confirmed binaries use one compiler and runtime. D2, the completion writer, producer normalization, and Bind are outside this candidate.

| Profile | Input | Runners | App sessions | Queue sessions | Peak with broker | PostgreSQL cap |
| --- | ---: | --- | ---: | ---: | ---: | ---: |
| Dense `noop` | 1,024 | 2 × 32 | 32 | 8 | 41 | 96 |
| `short` | 128 | 1 × 16 | 8 | 4 | 13 | 20 |

Both workloads use PostgreSQL 17.11 with verified TLS and durable commits. The adapter and database use CPUs 0 and 1, with two Mesh scheduler workers per runner. Each arm uses a fresh cluster. Dense retains a separate warmup arm per lane. The first timing grant runs groups 1 and 2. A second grant runs groups 3 through 6 after raw replay of the first two. The full request caps work at 108 serial arms. The controller owns all three fences and keeps partial and failed receipts.

The observer records entry, immediately before and after the live lookup, and both write returns. Replay joins every marker to the job ID, attempt, runner PID and commit XIDs. It reports the observed service interval and the observer’s effect relative to the two unobserved controls and the unobserved direct lane. The score counts all successful jobs and checks 2 application insert calls and 2 health probes per job, along with slot limits, actual TLS sessions, scalar producer calls, actor drain, and independent terminal XIDs. Adoption needs matching entry-to-first-commit, full-submit, and drain gains beyond duplicate spread in at least five of six dense groups, plus a median dense drain gain of at least 62.4 ms against the faster duplicate. That is one fifth of the historical 312 ms residual. A full gain alone cannot establish the portion attributable to the service.

## Replay

From the owning checkout, run the offline check.

```sh
packet=tools/durable-jobs-bench/results/2026-11-24-trusted-handler-app-pool
python3 -B "$packet/replay/controller.py" check
```

The request gives the controller separate SETUP and TIMING commands. SETUP builds the four AMD64 executables and runs the compiled two-runner pool isolation witness, without a benchmark. The local seal checks 19 archived native ledgers and preserves their existing origins. The [local gate receipt](evidence/local-gates.json) records workspace builds, named-handler checks, the scheduler teardown test, and the untimed compiled PostgreSQL pool witness. Candidate speed and attribution await the controller’s results.
