# Native evidence for supervised persistent Jobs workers

The x86_64 run passed the lifecycle proof and completed one measured rotation on CPUs 0 and 1. The rotation contains dense no-op and short workload blocks. Each has five arms and uses the same input and database limits within the block. Results describe this rotation. The planned additional rotations remain outside this packet. [Replayed outcomes](outcomes.json) · [Frozen plan](capture/series/series.json)

## Execution contracts

- **Mesh strict.** `Jobs.start_effects` uses a fresh credential-free executable per attempt by default. The child must exit and the runner must reap it before settlement. The named application writes use the attempt token and lease fence.
- **Mesh trusted persistent.** `trusted_persistent_workers` opts exact worker versions into sequential PID reuse after a clean completion. Each claim gets a fresh managed context and socket. Application writes keep the same named registry, token fence, verified TLS, independent commits, and full resets as strict. The child watchdog enforces attempt and lease deadlines across generations. Constructors and process-wide state live for the child lifetime, so the opt-in requires audited synchronous handlers. Unlisted versions and later claims of versions that trip the actor guard use fresh processes.
- **Ordinary trusted V1.** This separate experiment uses a shared runner process and trusts application writes under the application role. Its terminal token checks and two durable writes are distinct from the protected named-effect path. The matrix here includes the two Mesh arms above plus stock Oban, River, and Apalis. [Ordinary V1 contract](../2026-11-23-ordinary-trusted-v1/README.md)

## Native proof and outcomes

The first setup exited 1 during PostgreSQL startup. Its recorded directory implies a 122-byte Unix socket filename path. The retry moved the proof work directory, which reduced the path to 84 bytes. The controller recorded the same two proof binary SHA-256 hashes and exit 0 after the retry. The retry proved independent hard and renewed lease expiry under SIGSTOP, stale-generation rejection, pre-entry expiry, cancellation, crash and stop handling, strict fallback, and recovery of the original XIDs after a dropped post-commit receipt. Both no-op smokes passed. The first archive includes the original bootstrap transcript and exact proof binaries. Its server log was in the disposable database directory. [First setup](capture/native/first/) · [Successful retry](capture/native/retry/)

- **Dense no-op, 1,024 jobs per arm.** Commit-to-final-settlement throughput was 189 jobs/s for Mesh strict, 278 for Mesh trusted persistent, 727 for Oban, 904 for River, and 1,186 for Apalis. Mesh used 1,024 and 64 distinct worker PIDs, respectively. Mesh median claim-to-terminal time was 127.3 and 49.1 ms.
- **Short, 128 jobs per arm.** Throughput was 221 jobs/s for Mesh strict, 298 for Mesh trusted persistent, 339 for Oban, 780 for River, and 802 for Apalis. Mesh used 128 and 16 distinct worker PIDs, respectively. Mesh median claim-to-terminal time was 58.3 and 32.1 ms.

Every measured arm reconciled two independently committed application writes per job. The Mesh ledgers also witnessed distinct per-job terminal XIDs. From process startup through teardown, Mesh runner plus reaped-child CPU went from 5.81 to 2.89 ms/job for dense and 4.78 to 2.69 ms/job for short. In the sampled submit-to-settlement window, pinned CPU including PostgreSQL and driver work went from 11.20 to 8.13 ms/job and 9.77 to 7.27 ms/job, respectively. The two CPU metrics cover different windows. [Outcome and CPU replay](outcomes.json)

## Why summed RSS rose

The sampler sums resident pages across the runner process tree. It tracks runner roots and descendants separately. Dense peak RSS increased from 138.0 to 475.9 MiB. Root RSS at those peaks was 69.4 and 70.1 MiB, and descendant RSS was 68.6 and 405.8 MiB. Short peak RSS increased from 21.8 to 110.6 MiB. Root RSS was 9.4 and 9.3 MiB, and descendant RSS was 12.4 and 101.4 MiB. The persistent arms still had 64 and 16 resident children, respectively, at the drained sample. Both strict arms had zero. The descendant counts explain the summed RSS difference. Since the capture lacks PSS and shared versus private page data, it leaves the physical memory increase undetermined. [RSS samples and process counts](outcomes.json)

## Replay and source

Run the checksum and replay commands from this directory with Python 3.11 or later. Replay reads the local archives and uses the pinned matrix drivers and workload generator.

```sh
shasum -a 256 -c SHA256SUMS
python3 -B replay.py | cmp - outcomes.json
```

The source overlay and combined patch apply to Git base `3828f20d2d6beb683868cce33e6dd1958365914b`. The resulting tree is the source tree of candidate `67ec9bb9ba405f5f2cb830a338f77a8eed76d897`. The patch series also preserves the original commit breakdown. The fixture folder holds the original handler and the exact admission-only configuration used for both Mesh arms. The native build reused the compiler from the base revision and built the runtime from the candidate. The recorded compiler, runtime, worker executable, proof binary, peer-adapter, and input hashes pin those roles separately. [Source and fixture](source/) · [Pins](pins.json) · [Setup and retry recipes](recipes/)

The controller's live raw index remains in the original series directory. This packet copies the exact archives, request envelopes, reconciliations, and inventories for the completed rotation. The raw capture and proof bytes passed a credential-pattern check. The packet contains public test certificates and preserves the host identity needed for the matching checks. It carries no database files, private keys, or access material. [Raw index origin](../2026-11-23-worker-execution-v3/) · [Input paths and hashes](pins.json)
