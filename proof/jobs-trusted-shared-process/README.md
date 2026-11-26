# Trusted shared-process Jobs proof

Run this from the checkout on macOS or Linux as an unprivileged user. Put PostgreSQL's `initdb`, `pg_ctl`, and `psql` on `PATH`, or set `PG_BIN_DIR` to their directory. The proof starts its own loopback PostgreSQL cluster.

```sh
cargo build --locked -p meshc
cargo build --release --locked -p mesh-rt
python3 -B proof/jobs-trusted-shared-process/prove.py target/debug/meshc \
  --runtime target/release/libmesh_rt.a
```

The proof builds the [Jobs guide example](../../website/docs/docs/concurrency/index.md#run-trusted-handlers-in-the-application-process) and runs it through `Jobs.start`. It checks that the tenant SQL login cannot read the Jobs table, that both forms of drain reject the opt-in, and that the default and explicit fresh-process handlers still send output through the worker's standard error.

The trusted handler exercises a live input across allocations and yields, retries an actor panic and timeout, and cancels a parked actor. The proof kills a runner after its SQL commit and confirms that a replacement settles the recovered job without changing the committed effect. It holds a separate SQL statement across cancellation and `Jobs.stop`, verifies that the old runner still tracks the attempt, then lets the SQL commit and observes the cancelled queue row and the runner's exit.

The replacement uses two slots, `claim_batch: true`, and `claim_batch_strategy: "set_based"`. PostgreSQL function statistics confirm it called the set-based claim function. Two queued jobs each succeed on their first attempt with separate terminal transaction IDs. The original cursor runner and the replacement share the same queue and recovery rules.

The example keeps its synchronous PostgreSQL connection use within one scheduling turn. In native integrations, keep yielding callbacks out of frames that cannot unwind. Application code owns the lifetime of its detached actors and native operations.

The native controller request uses the sealed V1 compiler, fixtures, and broker for a matched short and dense replay. [Native setup and pins](native/README.md)

## Verification scope

The Darwin ARM64 verification for `e4156740` exercised the set-based claim configuration and the compiled guide. The workspace recorded 4,820 passed, 71 ignored, and one compiler timing test reserved for Linode. PostgreSQL 17.11, 18.7, and 19.0 passed compatibility, scoped role, and effect-fence checks. The receipt identifies the product source and the checkout at the start of each PostgreSQL run. [Verification commands and log digests](set-based-verification.json) · [Parked Linux proof](../../tools/durable-jobs-bench/results/2026-11-24-e4156740-trusted-jobs-native/README.md)

The correctness runs use the product runtime at `db5937e7e8988f935e9be16b68a2561c2250cc27`. The local release fixture uses the five frozen benchmark inputs and handlers to check durable outcomes. The executable from the timed V1 archive retains its own identity and measurements. The receipt records the production runtime, fixture executable, and fault executable hashes separately. [Verification receipt](verification.json)

The four-engine comparison of trusted set-based claims and individual terminal commits used the Linux executable built from `b3557273`. It records that executable’s digest, all peer builds, and both durable timing windows. [Linode scorecard](../../tools/durable-jobs-bench/results/2026-11-23-iteration-comparison/four-engine-scorecard.md)

Local lifecycle runs use macOS ARM64. The GNU/Linux ARM64 release cross-build verifies the Linux runtime configuration. The runtime uses the existing platform actor stack and worker deadline clock, including the existing Windows clock implementation. Native x86-64 qualification is tracked by the controller request. Windows and Intel macOS received source review.
