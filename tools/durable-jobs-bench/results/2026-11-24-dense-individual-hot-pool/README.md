# Public dense Jobs with individual settlement

The Mesh arms bind revision `1e70f18d624a841740d3bbb3d0fd09b269257f68` through the successful [Jobs proof](../2026-11-24-qualified-hot-pool/README.md). The [previous six rotations](../2026-11-24-dense-product-d2-transfer/NATIVE-RESULTS.md) used grouped Mesh settlement. This transfer uses the current public trusted shared-process handler, protected checked set-based admission, and one terminal transaction per Mesh job.

All three Mesh arms use the same staged ELF for migration, producer and both runners. Both checked controls leave the application pool URL unchanged. The hot arm adds `mesh_pool_hot_checkout=1` to that URL in each runner. The producer and queue pools use the same settings in all three arms.

All six lanes use the same 1,024-job input, PostgreSQL 17.11, CPUs 0/1, and two 32-slot runners. Mesh uses two scheduler workers and 16 warm application TLS connections per runner. The physical PostgreSQL connection cap is 96. River, Oban and Apalis use the same stock peer binaries and 2×32 adapter inventories as the previous six-rotation comparison. River and Apalis retain their stock terminal batching.

Each rotation uses fresh clusters for all six warmup and all six measured arms. The launch orders balance position, adjacency and Mesh and peer triplet order. Every archive must reconcile the canonical batch and two independent application writes per job. A Mesh archive also requires 1,024 distinct terminal XIDs, protected set admission with zero observed cursor fallback, matching per-runner cap and drain receipts, staged binary identity on both runner children and the broker, eight bulk submissions in one transaction, and 2,048 checked probes or fewer in the hot arm. In each qualifying rotation hot must beat the faster checked control in both full clocks by more than 5% and the duplicate spread. Its last application effect also has to commit earlier. Four of six complete rotations must qualify.

The replay retains every raw archive digest and reports median and range for submit-to-terminal, enqueue-commit-to-terminal, and the 103rd to 922nd second application commit and terminal commit. It reports within-rotation deltas and wins against both controls and all three peers.

## Controller commands

Run from the repository root. The offline check and historical controls make no native dispatch. The designated controller registers the byte-exact setup and timing request files in its inbox. After Jobs proof release and replay, the build-free SETUP installs the adapter in a new directory and executes the already-staged ELF in two diagnostic arms under the existing three fences. The setup grant, raw export, verification and release are retained under `evidence/setup`. Each diagnostic uses the same topology as timing and verifies the stage as the benchmark UID. The successful setup release goes in `evidence/setup/controller-release.json`.

```sh
d=tools/durable-jobs-bench/results/2026-11-24-dense-individual-hot-pool
python3 -B "$d/controller.py" check
python3 -B "$d/offline_test.py"

# After the Jobs proof archive and verification receipt exist
python3 -B "$d/controller.py" bind

# Controller only, during the matching SETUP and TIMING grants
python3 -B "$d/controller.py" setup
python3 -B "$d/controller.py" timing 1 2
python3 -B "$d/controller.py" timing 3 4 5 6

# Once all rotations have reconciled
python3 -B "$d/controller.py" replay > "$d/evidence/final-replay.json"
python3 -B "$d/report.py" < "$d/evidence/final-replay.json" > "$d/NATIVE-RESULTS.md"
```

The binding reads the original Jobs proof export, checks the raw archive against its verification receipt, and validates both short individual-settlement proofs. The adapter keeps the original product bytes in place and uses a separate staging directory and scratch directory. Preserve setup or timing failures and reconcile host and lock state before granting any retry.
