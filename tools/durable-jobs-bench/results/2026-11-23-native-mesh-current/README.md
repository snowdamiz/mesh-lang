# Current Mesh AMD64 release artifact

The release build used `3828f20d2d6beb683868cce33e6dd1958365914b`. It contains
the due-range change at `446e413c` and the String.join change at `aec101f2`.
Rust 1.99.0 and LLVM 21.1.8 built the compiler and runtime from the same
source. The compiler linked the frozen benchmark fixture with opt level 2
and an explicit `MESH_RT_LIB_PATH`. The build used the default compiled Jobs
execution path and the benchmark's existing listener, batching, and worker THP settings.

The runner SHA-256 is
`3dc20c730bd269da86c9c98f225d5f85bdc0dd3d3a31908e0eac90aa50d8dfd2`.
The compiler, runtime, executable, manifest, frozen fixture, and build logs
are in `target/linode-jobs-stepchange/stock-3828f20d-amd64.tar.gz`. The
controller verified each manifest entry after download. The archive hash
and compiler identity are in this directory. Native reuse paths are under
`/var/lib/durable-jobs-bench/native/builds/mesh-stock-3828f20d/`.

The pinned boot was `90ec1c24-08c8-4bc1-aa28-79b6db1bd13a`. The source
built and linked from 19:25:31 to 19:37:12 UTC on November 23, 2026.
The two native diagnostic arms reconciled 1,152 jobs and 2,304 independent
application commits on PostgreSQL 17.11. TLS was verified on the producer,
runner, and compiled application paths. The runner and postmaster shared
CPUs 0 and 1.

The current workspace tests passed on Darwin ARM64 with 4,816 passed and
63 ignored. The local PostgreSQL 17.11 compatibility, parity driver,
scoped roles, and compiled durable Jobs checks also passed. The Linux
smokes qualified the exact runner for end-to-end timing under the frozen
execution contract.

- [Build driver and exact flags](build-driver.sh)
- [Build and compiler identity](mesh-toolchain.txt)
- [Binary manifest](mesh.json)
- [Short ledger replay](stock-smokes/short/reconciled.json)
- [Dense no-op ledger replay](stock-smokes/noop/reconciled.json)
