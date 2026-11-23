# Protected native combined source

The combined local commit is `7bdc8f721e65f6c0ab5fb28cb6362506acfcc68e`. It applies the four-file measured lean runtime patch to the exact independent admission candidate `bd7851b9398198955b4c8df630310a29b38cddd4`. The clean source archive is staged at `target/jobs-admission-v3/combined/source.tar.gz`. The Linux ARM64 runtime type check passed on the merged source. The controller's AMD64 build and combined witness passed. The combined runner ELF SHA-256 is `df6abf1fe511e161358c1024fcd8cdfecd02406025d83e84f61464f477709993`. [Native build pins](amd64-build.json)

The AMD64 runner uses the already emitted `runner.o` and unchanged `libworker_identity.a`. The object contains exactly one occurrence of both the frozen handler fingerprint `e03da7787d8c4083:Main__handle` and the exact measured worker SHA-256 as embedded bytes. Rebuild only the runtime archive from the merged source and use the pinned link arguments in [pins.json](pins.json). Put the exact `mesh-lean-worker` beside the runner to exercise compiled lean mode. Omit the companion to exercise full self-exec from the same linked image. Admission mode is independent and selected by `MESH_JOBS_ADMISSION_V3=strict`. Stock and strict both require a separate terminal XID for each job.

The pinned stock compiler and actual lean worker were built with the recorded LLVM and Rust versions. Both measured code-generation inputs and the native identity archive are under `target/jobs-admission-v3/combined/inputs` in the shared checkout. [Combined pins](pins.json) · [Exact lean patch](lean-runtime-v1.patch.gz)

The separately linked lean witness passed its real PostgreSQL 17.11 relay proof. Its four attempts include three sealed companion processes, one digest-mismatch full self-exec, exact consuming waits, and two fenced writes with independent commits and full resets for every attempt. The combined AMD64 witness independently repeated all four cases. It recorded 28 v3 admission calls, zero cohort calls, four unique worker PIDs, four exact consuming waits and four individual terminal XIDs. The first native four-arm rotation also reached both strict selectors, with full work and cap reconciliation. [Combined fenced proof](combined-fenced-proof.json) · [Strict SQL receipt](combined-strict-proof.json) [Lean fenced proof](lean-fenced-proof.json) · [Admission wire proof](../receipts/wire.json)

The original lean component promotion threshold was 1.5 CPU-ms/job. The measured component saved 1.045 child CPU-ms/job and 1.018 runner-tree CPU-ms/job. The protected four-arm integration is exploratory. The controller owns the separate stock, lean-only, strict-only, and combined comparison, with frozen no-op and short inputs, identical physical caps and known worker image selection. Cohort settlement uses its separately disclosed terminal transaction contract.

The signed four-arm package passed offline Ed25519 signature verification and all 24 member hashes, including the merged source archive, measured runner object, measured companion, and separate fenced proof. The controller request is `lean-admission-v3`. [Signature and member receipts](signed-package-verification.json)


## First native timing rotation

One matched rotation per profile, with full no-op warmups on the same two AMD64 CPUs. The rates below run from committed enqueue to the last terminal commit.

| Profile | Stock | Lean only | Admission only | Combined |
| --- | ---: | ---: | ---: | ---: |
| No-op, jobs/s | 187.64 | 204.59 | 207.67 | 226.66 |
| Short, jobs/s | 228.50 | 256.98 | 238.47 | 275.98 |

Each arm reconciled its frozen input, distinct terminal XIDs, separate effect commits, and the same physical limits. Both strict lanes recorded full resets and zero cohort calls. This first rotation is exploratory. The matched series and the independent admission comparison determine repeatability. [Reduced first rotation](first-native-rotation.json) · [Controller series](../../2026-11-23-lean-admission-pilot/)
