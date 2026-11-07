# Independent implementation review — round 1

**REQUEST_CHANGES for the bounded invocation primitive, including at mask1. Scheduler capability4 is not supportable on this snapshot.** Two concrete blockers follow. This does not reopen the accepted amendment3 API design, revoke the accepted base R1 implementation, or claim product/native integration qualification.

Reviewed source is exactly `review-input-r1/source-pin.json`, base `9b844dbab34e61f6de575af2b25b0ed0f321059e`. All 31 frozen file hashes match. Unchanged dependencies were taken from `git show` of that base; the copies used here are in `base/`. [source-hashes.json](source-hashes.json) records both sets. File:line references below refer to the frozen `review-input-r1/source/` unless expressly marked **base**. Live follow-up logs are evidence about those runs, not replacement source for this review.

The parent released the public API hold for exact amendment3 `544325a5eb2c656741e40da7524edc1ed645a1b7aeaea7219814e2a7bfbe79dc`; this review applies its B1/B2/N1/N2 dispositions. No additional design approval is requested.

## Blocking findings

### F1 — P1 — Repo.transaction bypasses the PostgreSQL refusal and strands a lease

**Locations:** `compiler/mesh-rt/src/db/pool.rs:216–225`, `:139–178`, `:334–337`; **base** `compiler/mesh-rt/src/db/repo.rs:757–765`; guarded destination `compiler/mesh-rt/src/db/pg.rs:1802`.

`mesh_repo_transaction` calls internal `pool::with_connection` directly. That function and its `checkout` have no scoped refusal. The newly guarded `mesh_pool_checkout` is `#[cfg(test)]` and is not this production route. Before reaching the guarded `mesh_pg_transaction`, checkout can call `open(&url)` and complete a real PostgreSQL handshake, or send `SELECT 1` on an idle connection through `health_check` (`pool.rs:63–68`). The PG connect worker has no native invocation ownership (**base** `db/transport.rs:269–345`). Refusing the eventual transaction callback does not undo this earlier unadmitted network work.

There is a second consequence in the same path: after `mesh_pg_transaction` returns refusal, `with_connection` calls `lease.take()` and transfers the handle to `mesh_pool_checkin`. The new scoped checkin guard returns before removing it from `pool.inner.leased`. The RAII lease has already been disarmed, so the connection and capacity slot remain leased. A subsequent ordinary caller can exhaust the pool. No invalid pointer or forged native authority is necessary: an existing unscoped pool is sufficient.

**Independent reproduction:** [probe_repo_refusal.py](probe_repo_refusal.py) ran successfully against the retained generated library `generated-abi-1794016967512686000/libprogress.dylib`. Its SHA-256 remained `a767b82c80a3189274b0e2a0e80dc76210086a878301d59803b17c74b2a988af` before/after. The probe created a legitimate empty one-slot pool outside a scope, then used the existing scoped ABI with a native test export:

- Direct scoped `Pg.connect`: `native_effect_refused`, no loopback connection.
- Scoped `Repo.transaction`: the loopback peer received a 44-byte PostgreSQL startup payload, then the operation returned `native_effect_refused`.
- The next unscoped query on the same pool: `pool checkout timeout`.

[repo-refusal-probe.json](repo-refusal-probe.json) records the result. This is an actual retained-runtime/native-scope boundary probe, **not** a generated scheduled-entry or full PostgreSQL integration pass. The frozen source independently establishes the same route; artifact execution does not establish that every byte of that earlier generated family was built from the final frozen manifest.

**Required fix:** put refusal before any checkout/dereference/protocol work at the shared production leasing boundary (or equivalently cover this caller before leasing). Keep ownership intact until a lease is actually returned or discarded; a refusal must not discard its cleanup obligation. Do not weaken checkin into an authorization path or silently permit PG under scoped execution.

**Required regression:** an existing empty pool and an existing idle connection, scoped `Repo.transaction` under live and retired originals, no connector queueing/connection/health-query/callback, unchanged pool capacity, followed by a successful unscoped query. Exercise the real public Repo route. A test of the test-only checkout wrapper would miss this bug again.

### F2 — P2 — the compiler adds an intrinsic without its JIT registration

**Locations:** `compiler/mesh-codegen/src/codegen/intrinsics.rs:55–69`; **base** `compiler/mesh-repl/src/jit.rs:1282–1289`, `:3085–3108`; `compiler/mesh-rt/src/library.rs:6–7`.

`declare_intrinsics` now declares `mesh_library_submit_bounded_v2` for every codegen module. The JIT registry remains the base registry and only maps the old library invoke functions. Thus the existing source-derived registration invariant fails. The follow-up `evidence/workspace-lib-bins2.log` records exactly:

```text
not registered with the JIT: ["mesh_library_submit_bounded_v2"]
test result: FAILED. 69 passed; 1 failed
```

A copy of the observed log is retained in [evidence/evidence/workspace-lib-bins2.log](evidence/evidence/workspace-lib-bins2.log). This is an actual required workspace regression, not merely an absent test receipt. It does not imply that every ordinary REPL expression crashes; unused declarations need not be executed.

**Required fix/test:** register the real bounded-submit implementation with the JIT, exposing the Rust path through `library` as needed, or explicitly restrict declaration to codegen modes that own the symbol without weakening the registry invariant. Keep the already-approved native C header separation. Rerun `mesh-repl`'s existing `every_declared_runtime_function_is_registered` test and the affected workspace gate, against a newly pinned candidate. Later live fixes do not change this frozen verdict.

## Prior findings and accepted API

The complete prior source audit and API review rounds1–3 were read, together with the selected parent design and original provenance inventory.

| Prior item | Frozen implementation assessment |
|---|---|
| Audit F1, WS rooms | Source-addressed: `ws/rooms.rs:235,251,284` refuse before membership mutation or local/cluster broadcast. The socket positive/refusal test is meaningful. It does not independently exercise a live cluster frame. |
| Audit F2, remote controls | Source-addressed: MONITOR/DEMONITOR/MONITOR_EXIT/LINK/EXIT inspect target invocation under the target lock (`dist/node.rs:2211,2241,2256,2274,2295`). The added TestPeer test drives those protocol branches and checks mutation/mailbox/output. It is not a full network/session qualification. |
| Audit F3, late HTTP buffers | Source-addressed for completed owned buffers: body/header collections and queued errors use `Zeroizing` (`http/client.rs:90–99,772–807,888–902`). Direct disposal/erasure instrumentation remains absent; this does not prove erasure of request/provider/TLS/OS copies. |
| Audit F4, router capture | Source-addressed: router creation, route registration and middleware refuse before pinning (`http/router.rs:190,222,298`). The retained weak-root test checks eventual reclamation. |
| Audit F5, delayed send | Source-addressed through explicit refusal before capture/thread creation (`actor/mod.rs:1052–1061`). `Timer.apply_after` remains the owned-actor route. No support for scoped delayed sends is inferred. |
| B1 | Static code pinning, original retained context, normal-stack lifetime callbacks, callback markers and lifecycle-unlocked shutdown are implemented/documented. Exact disposal/executor and cleanup ordering still need the gates below. No unload permission exists. |
| B2 / N1 | One caller reference; clear valid outputs; null-before-reentry precedence; Pending/Ready/Taken; once-only terminal results; cancel/take completion-mutex ordering are implemented in `library/invocation.rs:41–167`. Release neither cancels nor joins. Rejected take/cancel do not mutate state. |
| N2 | `in_owned_callback` protects init/enable/register, while shutdown rejects all callback reentry (`library.rs:210–291`). Standalone V1 callback init/register compatibility has a focused positive test. |
| Accepted R1 fixes | SQLite source remains the exact accepted base, including open/prepare recovery admission and per-statement/commit admission. Host failure provenance remains distinct. Timer and 4100-request cleanup cases remain in the reused matrix. No R1 header change was introduced. |

The original provenance memo is still a baseline research document, not a complete candidate closure ledger. Its pool/Repo classification must now include F1 explicitly.

## G1–G8 assessment and remaining evidence

These are qualification limits and required evidence, distinct from F1/F2's concrete failures.

| Gate | Supported by source/retained evidence | What is still missing or limited |
|---|---|---|
| G1 admission/startup | Owned input/context/completion is installed before queue publication; bounds 1..1 enforced; lifecycle serializes submit against Stopping. Pre-init/post-stop, injected preparation and coroutine-stack failure controls exist. | The preparation hook fails early, before real queue preparation; it is not an injected failure of each preparation stage. No complete init/submit/shutdown race or worker-start-failure receipt tied to this exact pin was established. Ordinary allocation abort is explicitly outside the contract. |
| G2 unwind/GC/Ready | Scheduler installs original context/PID before forced coroutine unwind. Terminate, SQLite close and secret cleanup precede `owned.finish`. Root/child loans are retained separately. Generated GC, actor/timer and existing native memory/GC gates provide useful coverage. | Strict Ready ordering remains unproven: `scheduler.rs:1216–1222` publishes Ready before table removal; `resume_process:979` still holds a process Arc, and its caller still owns the coroutine. The capture test polls for reclamation after terminal take (`invocation.rs:671–681`). Establish the exact promised cleanup boundary with a barrier test; terminal take alone cannot prove every root-owned destructor ran. Preserve legitimate descendant loans rather than waiting for descendant settlement. |
| G3 cancel/finite work | Cancel sets monotone refusal under the same mutex as take; process stop avoids cross-thread destruction of scheduled secrets. The real blocked Host write test checks Pending until return, one accepted write, no second write, and queued-root nonexecution. | Full cancel/take race, cancel after Taken with live descendants, descendant spawn versus cancel and accepted-SQL cancellation races are not closed by the reviewed receipts. Sequential Ready-before-cancel and take-before-cancel tests are useful but not the whole race matrix. |
| G4 waits/resources | Final yield, cooperative receives, actor/job receives, sleep, service calls and HTTP have runtime guards. SQLite transaction count and platform storage resource scope are present. The HTTP transaction test checks no dispatch, unwind and rollback count 0. | No complete behavior matrix for receive/service/job/reduction-yield/sleep/resource-mutex refusal and cleanup was established. A generated old call opens a DB across HTTP, but retirement then refuses its next query; this is not a positive successful read-handle reuse after a permitted wait. |
| G5 Host/HTTP | Original callback table is selected; worker context is retained; ActiveRequestGuard unregisters on unwind; lifetime callbacks use a native-stack trampoline and markers. New all-nine and after-shutdown late-HTTP release tests are meaningful in source. | STATUS says the latest extensions were pending. An earlier six-test receipt does not execute those additions; the first follow-up compile log fails and the later workspace run stops at F2. No complete exact-pin pass for the additions was established here. Lock probes cover lifecycle/table, not every process/completion last-owner path. No direct queued/send-failure buffer-erasure probe or real JNI attach/global-ref test. |
| G6 transfers | Same-original actor/job/timer capture and local target identity checks; unsupported services/channels/supervisors/remote/PG/WS/server paths mostly refuse. Remote controls and rooms now have actual boundary tests. | F1 disproves full refusal coverage. Same-original service/message positive controls plus cross-original call/cast/reply/control rejection and cleanup need a candidate ledger. Original native post-wait observations remain an adapter/composition gate; node scanning is not ProcessOwner revalidation. |
| G7 real progress | Fresh retained red shows load/status BUSY 3 while old HTTP is held. Both generated green receipts show `loaded`/`fresh-status` before release, then old refusal 10, no bytes, rows 0, only `/hold`. These are real generated fixture observations. | Actual Core bootstrap refuses accepted R1 PRAGMAs before the selected RPC. No same-Core held-RPC/concurrent-load composition passed. The fixture cannot qualify native global serializer removal, shared Core services or whole-product progress. Final coherent source/tool/artifact receipt remains necessary. |
| G8 ABI/regression | Separate one-way invocation header, fixed compiler thunk/limits, native-only companion metadata on the same rows, collision checks, retained 31-case matrix in two modes and C/C++ include-order checks. Legacy Core output equality independently verified below. | F2 leaves the required workspace gate red. Broad runtime counts and earlier fixture receipts must retain their exact candidate provenance. Complete N1 null/reentry/state matrix, bounds/error/allocation and native-platform checks are not inferred from ABI parsing. Fresh review required after fixes. |

Cancellation is safe-point stopping, not rollback of an already accepted finite effect. No concrete final foreign release under the named locks was demonstrated by this review. Conversely, structural detachment and a small lock probe are not exhaustive proof. Blocking shutdown is an accepted contract; failure to cancel held HTTP before shutdown is not by itself a new API defect. File/stdin/environment remain explicit cap2-unqualified baseline effects; this review does not invent complete-effects coverage.

## Retained evidence checked independently

- All 31 frozen source hashes match, and the unchanged authority fragment hashes to `5bbaf429dbd3749924b041847aba40ce40ee33c5a13a5c97f454601d79f87843`.
- All 31 legacy-mode and 31 scheduled-mode case logs contain `status: Some(0)` and `PASS`. Some controls intentionally remain V1 or exit during unsupported activation; this is a 62-run regression matrix, not 62 distinct scheduled behaviors. The initial combined e2e run failed its sibling collision test; its main generated test passed. The corrected collision rerun passed separately.
- `runtime-audit-fixes2.log` reports 1668 passed / 20 ignored for its earlier runtime snapshot. `owned-lifetime-gates.log` reports six focused tests. Newer source/test additions do not inherit those passes automatically. Follow-up runtime integration logs record their own passes; the workspace follow-up records F2, not an all-suite pass.
- [compatibility-verified.json](compatibility-verified.json) independently compares actual retained baseline/candidate Core fixture files: C header, Swift, Kotlin, JNI C and TypeScript bytes are equal. Both manifests have 185 rows, including exactly 176 `mesh_messenger_` app operations; stripping the new `scheduledSymbol` reproduces every legacy export row. This proves those file comparisons, not app execution or platform linking. The parent's retained physical-symbol/desktop-parser receipt is separately hashed, not relabeled as a fresh platform test.
- The source still advertises numeric 1 (`authority.rs:276–278`); bits 2/4/8/16/32 stay off. CALL_LOCK and native serializers remain.

[evidence-hashes.json](evidence-hashes.json) identifies the receipts inspected. The R1 handoff and final family/review records were read as accepted historical evidence; no R1 family was rebuilt or modified.

## Audit actions and handback

Executed only bounded reads/hash comparisons and the single isolated loopback reproduction:

```text
git show 9b844dbab34e61f6de575af2b25b0ed0f321059e:<unchanged-file>
python3 -B .tmp/native-owned-invocations/implementation-review-r1/probe_repo_refusal.py
```

Source comparisons used Python SHA-256 and unified diffs of every pinned file versus the base. Retained-symbol inspection used `nm -gU`; no compiler/linker was invoked. The loopback probe used only the retained generated runtime and its own ephemeral localhost peer. Writes are confined to this review directory. No source/app edits, builds, children, Git mutations, push/PR/merge, deployment, or external network were performed.

Fix F1 and F2, produce a fresh frozen candidate and exact gate receipts, then request a new independent implementation round. Keeping mask1 is necessary but does not make this candidate acceptable with F1/F2 present. Cap4 remains held; full Core/native/JNI/services/publication/product qualification remains explicitly unclaimed.
