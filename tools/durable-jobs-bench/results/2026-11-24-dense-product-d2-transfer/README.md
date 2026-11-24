# Dense product checkout transfer

Run this build-free request after the existing native SETUP for product
commit `b3557273` and in the reserved order behind the fixed product
sequence. It has its own grant, evidence directory and advancement decision.
The short-smaller decision and every all-five selected-config request keep
their existing flags and inventory.

The two checked controls and the hot arm use one SETUP-pinned Linux product
ELF for migration, producer and both runners. All three select ordinary
trusted shared-process execution, checked admission and grouped successful
settlement. The hot arm adds `mesh_pool_hot_checkout=1` to the application
pool URL in each runner. For a connection idle for less than 500 ms that
switch skips the checkout probe. Older connections still probe. Both
controls leave the option unset. The queue and producer remain fully
checked. A recent silent connection can wait on the first application
statement under the product's ordinary read semantics.

The profile uses 1,024 jobs, CPUs 0 and 1, two 32-slot runners, two
scheduler workers and 16 warmed application TLS sessions per Mesh runner.
PostgreSQL 17.11 uses the pinned executable, default storage, 4 MB
work_mem and a physical cap of 96. The River, Apalis and Oban lanes
reuse the exact released 2×32 inventories and engine settings. The
replay checks the two live Mesh runner identities, TLS sessions and
actual jobs by process, partitioning both app commits and terminal XIDs.
Only successful terminal settlements may share an XID, with a maximum of
eight. Every job must finish once with two separately committed
application writes. The two checked arms must each actually send 2,048
checkout probes. Hot must send fewer and the report keeps its actual
counts and SQL execution time. The group report also keeps the actual
physical peak, elapsed clocks and P99.

The archived product SETUP proof uses the same product runtime, handler
writes, grouped path and per-pool switch. The binder checks the exact
product and witness manifests and the native SQL cut, application,
claim and grouped reply-loss, cancel/retry, and normal and forced
teardown receipts. It refuses timing if these receipts are absent or
wrong. Use the existing native semantic proof on the same ELF before
seeking a grant if the old evidence cannot bind. The configured two
runners and warm application sessions are verified on every arm,
including all six warmup lanes before any measured arm of a rotation.

## Frozen advancement gate

On both submit-to-terminal and enqueue-commit-to-terminal, the hot
arm's elapsed saving versus the faster checked control must exceed
both 5% of that control and the absolute spread between the controls.
Its enqueue-commit-to-last-effect interval must also beat both
controls. At least four of six fully qualified rotations must pass.
All measured arms are required. Peer wins and probe counts are
reported independently.

Every lane appears once in every launch position and every ordered
adjacent pair occurs once. Each Mesh triplet order and each peer
triplet order occurs once. Groups 1 and 2 run before 3 through 6.
Use fresh clusters for each warmup and measured arm. The dispatcher
keeps each attempted arm, including prior and partial exports and
import failures. The replay lists all raw digests and selects complete
rotations independently of transport success. Preserve failures and
reconcile the three fences and host before any retry.

The R2 replay verified all six rotations, 120 successful exports and
the original failed import. Its hot lane cleared the gate in groups
1, 2, 4 and 6. The signed evidence records every original digest and
pairs the historical scores with their R2 artifacts. These historical
measurements are the entry condition for this product experiment.

## Controller commands

Verify the signed request and original evidence offline. Compare
the request SHA and public-key fingerprint with the controller handoff.

```sh
p=tools/durable-jobs-bench/results/2026-11-24-dense-product-d2-transfer
python3 -B "$p/transfer.py" check
```

Bind the existing archived product SETUP release and raw export.
The resulting JSON has the exact ELF, manifest, runtime and setup
receipt digests and can be regenerated from relocated archive inputs.
Keep the original raw export and binding alongside series evidence.
An active grant and the signed inbox copy are required for timing.

```sh
python3 -B "$p/transfer.py" bind "$release" "$setup_raw" > "$binding"
python3 -B "$p/transfer.py" timing "$release" "$setup_raw" "$binding" 1 2
python3 -B "$p/transfer.py" timing "$release" "$setup_raw" "$binding" 3 4 5 6
python3 -B "$p/transfer.py" replay "$release" "$setup_raw" "$binding"
```

Only the designated controller may invoke timing. Binding, replay,
and schedule checks do not contact the host or execute a workload.
