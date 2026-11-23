# Mesh scheduled-work index range

The candidate changes the PostgreSQL due-time query shared by listening Jobs runners. It reads the same materialized clock through a scalar subquery, which lets PostgreSQL use `scheduled_at_ms` as a range condition on `mesh_jobs_queue_due`. [Source patch](runtime.patch)

## Product verification

The detached Darwin arm64 run at product commit `446e413c` recorded 4,814 passing workspace tests, zero failures, and 63 ignored. The product regression test uses an advancing clock to check that the cutoff and delay share one sample. It also covers the exact due boundary, worker eligibility, and queue and namespace pauses. [Product proof receipt](product-proof.json)

The compatibility gate passed on PostgreSQL 17.11, 18.7, and 19.0, with 118 passing tests, zero failures, and six ignored on each version. Each run passed the due-time regression, the parity-driver check, and the scoped-role checks for fresh and upgraded databases. The effect-fence proof passed on all three versions. The product receipt records per-version SHA-256 digests for the compatibility and effect-fence logs.

The PostgreSQL 17.11 plan replay matched the archived range witness. Both queries returned no delay for a ready-only backlog and a 2,000 ms delay for the first eligible retryable job when future work was present. The candidate pushed `scheduled_at_ms` into the `mesh_jobs_queue_due` index condition. The index scan emitted 1,024 rows for the baseline and zero for the candidate in the ready-only case. With future work, the scan emitted 1,026 and two rows, respectively. [Plan witness](plan-witness.json)

```sh
PG_BIN_DIR=/opt/homebrew/opt/postgresql@17/bin \
    python3 -B tools/durable-jobs-bench/results/2026-11-23-mesh-due-range/verify-plan.py
```

## Native benchmark evidence

The timed Linux candidate ELF used the pinned compiler from `3f69b150`, frozen worker sources, and a release runtime from `ee593226`. The runtime and product commits have byte-identical `store.rs` blobs. The product receipt records the SHA-256 digests for the baseline and candidate ELFs. [Compiler and runtime hashes](mesh-toolchain.txt)

`build.sh` stages the candidate under the matrix owner's host directory. `run-candidate.sh` calls the same native arm driver. [Baseline](../2026-11-23-baseline/README.md), [dense diagnostic](../2026-11-23-dense-diagnostics/README.md)

The earlier macOS run at `ee593226` passed 4,813 workspace tests, the PostgreSQL 17.11 compatibility and scoped-role gate, and the effect-fence proof. The native Linux dense and sparse diagnostic arms reconciled 1,048 jobs with two ordered writes per job. [Local proof receipt](local-proof.json), [full native repeat](../2026-11-23-due-range-repeat/README.md)

The five alternating dense pairs recorded median paired throughput gains of 1.92% from submit to settlement and 2.59% from commit to settlement for the pinned candidate ELF. [Five-pair comparison](../2026-11-23-due-range-focused/README.md)

Host verification matched 409 raw arm status records to 409 sanitized local archives. The build caches were removed. The sealed runner and scoped source stay available for replay. [Host archive verification](host-verification.json)
