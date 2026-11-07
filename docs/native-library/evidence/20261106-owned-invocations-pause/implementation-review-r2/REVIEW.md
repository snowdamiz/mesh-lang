# Independent implementation review — round 2

**REQUEST_CHANGES for the bounded native invocation primitive, even while its advertised mask stays 1.** The previous Repo and JIT blockers are addressed, and the unborrowed-root cleanup-before-Ready correction is sound in the inspected path. Two remaining ownership defects are described below. They are source-derived findings, not executed reproductions. This review was expressly prohibited from running builds, tests, network requests or children.

**Capability 4: NOT READY. Whole goal: NOT COMPLETE.** Keep mask 1, the legacy CALL_LOCK and native serializers. Neither a passing generated fixture nor this review qualifies Core composition, shared services, JNI/native adapters, native policy, SDK effects or private publication. This verdict does not revoke the accepted bounded R1 source or reopen accepted amendment 3.

## Reviewed identity and method

- Base Git object: `9b844dbab34e61f6de575af2b25b0ed0f321059e`.
- Frozen source manifest: `../review-input-r2/source-pin.json`, SHA-256 `48fd7392c87319a97aed03489f1016525476209398ff27468ea4b0afde1cbee6`. All **33** member hashes independently match. Production-source references below mean this frozen `source/` tree; **base** references mean exact `git show` objects, copied under this review's `base/`.
- Accepted amendment 3: SHA-256 `544325a5eb2c656741e40da7524edc1ed645a1b7aeaea7219814e2a7bfbe79dc`, independently matched. Parent's release of the public implementation hold applies only to the exact separate native-only companion API.
- Unchanged R1 authority header, obtained from the base Git object: SHA-256 `5bbaf429dbd3749924b041847aba40ce40ee33c5a13a5c97f454601d79f87843`. SQLite and other unchanged owning dependencies were read from the same base, never from unpinned live source.
- Read the selected parent README/CONTRACT/SKETCH/ALLOCATIONS/API_CONSTRAINTS/PROOF_MATRIX and synthesis/final-contract R2 review; API reviews 1–3; complete source audit and implementation review 1, including its Repo reproduction; STATUS and PROVENANCE_LEDGER-r2; accepted R1 handoff/family/qualification/review records.

[verification-initial.json](verification-initial.json), [inputs.json](inputs.json), [evidence-pins.json](evidence-pins.json) and [verification-final.json](verification-final.json) identify the exact material inspected. Final source/tool/artifact family pins have **not** been supplied to this reviewer. I do not assign either earlier generated family to the final frozen source merely from a directory name, timestamp or passing log.

## Remaining defects

### F1 — P2 — a same-original SHARED reply can permanently retain both completed heaps

**Locations:** `compiler/mesh-rt/src/actor/scheduler.rs:373–379,879–899,1231–1235`; `actor/service.rs:159–174`; `actor/mod.rs:595–609,1128`; `actor/process.rs:328–340`. Owning unchanged dependency: **base** `actor/heap.rs:865–874`, `actor/msg_shape.rs:450–459`.

Every descendant of a library root holds a strong `library_heap_owner` reference to that root, including a child spawned with no arguments. A same-original child is allowed to reply with a value whose shape requires lending. `detach_from_sender` turns a SHARED pointer into a `HeapBorrow` owning a strong reference to the child's Process. The root receives that loan into its `heap_borrows`. `addressed_to` strips only a direct loan from the receiver's own heap; it does not strip a reference to a child which in turn owns the receiver.

The resulting strong-reference graph is:

```text
root Process.heap_borrows -> child Process
child Process.library_heap_owner -> root Process
```

This does not require two actors to exchange separate loans: the unconditional child-to-library-root edge already supplies the reverse edge. A valid native test entry can reproduce the graph by replying with one actor-heap object described by the valid `[2, SHARED]` shape. SHARED is an implemented compiler/runtime representation, not an invalid native pointer: **base** `mesh-codegen/src/codegen/msg_shape.rs:54–70` emits it for opaque pointer representations; **base** `mesh-codegen/src/mir/lower.rs:1490–1561` has explicit shared fallbacks. Compiler-created self-describing closures that copy successfully are not evidence for this failing case.

Root removal and Ready publication do not break either edge. Child cancellation unwinds its stack and removes its table entry, but finalization takes only invocation/completion ownership; it does not release `library_heap_owner` or `heap_borrows`. Once both table entries and native handles are gone, the two Arcs keep both private heaps alive indefinitely. The existing comment at `process.rs:336–339` acknowledges cyclic loans in the underlying actor implementation; the new API admits that representation without a scoped restriction.

The new service/descendant test at `library/invocation.rs:285–365` replies with scalar 42 and no shape. It verifies the useful acyclic case, so its success does not establish cleanup of this graph. This is an inherited actor ownership limitation exposed through the newly supported scheduled-root path, not a claim that the R2 diff first invented the actor cycle. It is also **not** evidence that the cycle retains Host context: finalization removes `invocation` and `owned_invocation`, so the demonstrated retained objects are the Process heaps and their loan data.

**Required bounded correction:** either make scoped loans reclaimable after all relevant processes settle, or refuse the unsupported loan-forming transfer before it installs a cycle. Preserve a live child's legitimate access to a root heap. Clearing every loan when the root finishes would introduce use-after-free and is not a fix. No blanket service capability or source-wide GC redesign is required to bound this unit.

**Required regression, not run here:** extend the actual scheduled same-original service test with a child-heap SHARED result, weak references to both processes, root terminal take while the child is live, then cancel after Taken and release all external references. Require both weak references to expire after child cleanup, without a timer deadline or shutdown being mistaken for reclamation. Keep the scalar and genuinely live-borrower positive controls. If refusing this route, require refusal before installing the loan and verify no retained graph.

### F2 — P2 — cancellation before HTTP send strands prepared requests in the global registry

**Locations:** `compiler/mesh-rt/src/http/client.rs:302–310,379–389,419–427,523–544,848–876,1111–1118`; `library/invocation.rs:59–71`; `actor/scheduler.rs:1133–1165,1231–1235`.

`mesh_http_build` inserts a `MeshRequestData` into the process-wide REQUESTS map. Body/header setters copy the request's data into that entry. It has neither a creating process/invocation owner nor a cleanup registration. The coroutine holds only the integer handle. The ActiveRequestGuard is created later, after `activate_request` removes that entry to begin send.

Consider an otherwise live scheduled entry that builds and fills a request, then performs a legal `Timer.sleep` or waits for an actor result before sending. Native cancellation while it is suspended forces unwind. The subsequent Mesh send/cancel instructions never execute, there is no Rust guard owning the pending entry, and the scheduler finalizer closes SQLite/secrets but never visits pending HTTP requests. Terminal take can therefore return refusal 10 and the process can be reclaimed while its prepared request remains in REQUESTS.

The registry has a global limit of 4,096. Repeating this valid prepare/wait/cancel path consumes its slots; eventually a fresh permitted invocation's `mesh_http_build` returns 0 before any new network attempt. This is a deterministic source-level capacity/cleanup failure, not a claim of post-retirement transmission. The entry also retains its copied request data, but this finding does not demand erasure of all HTTP/provider copies or reopen the prior completed-response-buffer finding.

The retained R1 fix remains correct: refused **send/stream** removes the pending request before returning. The current active-request guard also correctly removes an **activated** request on unwind. Neither runs when cancellation prevents send from being reached. The new nonlocal cancellation API needs an owner for this in-between state; an explicit `Http.cancel(handle)` placed later in Mesh cannot clean up after the continuation was forcibly unwound.

**Required bounded correction:** associate scoped prepared requests with their original lifetime and dispose them when that ownership actually ends, with an atomic ownership transfer to the admitted HTTP worker on send. Preserve requests legitimately used by a still-live same-original descendant, and preserve ordinary standalone ownership behavior. Do not cancel an already admitted finite effect or claim its worker has stopped; this finding concerns requests never activated.

**Required regression, not run here:** barrier after real request creation/body copying and before send, suspend, cancel the scheduled handle, wait for terminal cleanup, then verify the pending entry is gone. Repeat past the handle cap and require a fresh actual loopback request to succeed. Include cancel-versus-activate and a live same-original child control so cleanup cannot erase another surviving owner or an accepted attempt. No test of only `send_cooperatively`'s existing refusal branch will catch this case.

## Prior blockers and accepted fixes

| Item | Round-two assessment |
|---|---|
| Implementation R1 F1, Repo transaction bypass/lease leak | **Addressed.** `db/pool.rs:216–219` refuses at the shared production `with_connection` boundary before pool dereference, checkout, connection or health query. Base Repo.transaction reaches that boundary. The new test calls the public Repo route for empty/idle pools and live/cancelled contexts, checks no frames/capacity change, then succeeds with an ordinary query. The retained red records one health query versus zero expected; green and workspace receipts pass. The test's variable named `retired` calls `context.cancel()`, so it covers cancelled invocation state rather than separately retiring a generation; the guard itself is unconditional on any scoped context. |
| Implementation R1 F2, JIT registration | **Addressed.** `library.rs:7` publicly reexports the actual implementation; `mesh-repl/src/jit.rs:1290–1293` maps that function. The invariant passes in `jit-submit-green2.log`; the workspace receipt includes 70 REPL tests passing. No invariant was weakened. |
| Implementation R1 G2, Ready before root cleanup | **Addressed for the inspected unborrowed-root path.** Worker and resume Process Arcs are explicitly dropped, finished coroutine storage is released, and process-table removal/drop precedes `owned.finish`. The real barrier red is take0 while root remains registered; green expects Pending12 and weak unborrowed root gone at terminal take. This does not resolve F1's separate loan cycle. |
| Source-audit F1, WS rooms | Preserved: scoped join/leave/broadcast refusal precedes membership mutation and both local/cluster dispatch. Local socket positive/refusal coverage remains; no live-cluster qualification inferred. |
| Source-audit F2, remote controls | Preserved: MONITOR/DEMONITOR/MONITOR_EXIT/LINK/EXIT refuse invocation-owned targets under the target lock. TestPeer exercises those real dispatch branches, not a complete authenticated network-session integration. |
| Source-audit F3, HTTP completion buffers | Preserved: completed body/header collections and queued errors have Zeroizing owners; failed send/drop follows those destructors. Direct allocator-wipe observation remains absent; provider-internal partial buffers are not claimed covered. |
| Source-audit F4, router captures | Preserved: router creation/route/middleware refuse before permanent closure loans. This does not cover F1's permitted message loans. |
| Source-audit F5, delayed sends | Preserved: scoped delayed send refuses before capture/thread creation. Timer.apply_after uses a scheduler-owned actor. No cancellable native sleeping-thread support is claimed. |
| Accepted R1 semantics | Exact-base SQLite ownership/open/prepare/step/commit admission retained; no arbitrary transaction callback admission. Host callback failure provenance and original nine-callback table remain. Timer-dispatch and 4,100 refusal controls remain enumerated in the retained generated suite. No R1 target/family was modified. |

B1/B2/N1/N2 remain accepted at contract level. Source implements output clearing, null-before-reentry precedence, Pending/Ready/Taken, once-only terminal transfer, cancel/take completion-mutex ordering and the separate V1 callback compatibility rule. A successful take transfers private native bytes only. It does not authorize a native/JS response.

## G1–G8 and the evidence boundary

| Gate | Evidence supported here | Remaining limitation |
|---|---|---|
| G1: admission/startup | Input/context/completion/process owned before enqueue; one-worker bounds enforced; lifecycle check serialized against Stopping. Injected preparation and stack failure tests pass in retained output. | The early preparation hook is not injection into every allocation/queue stage; a complete init/submit/shutdown race and worker-start-failure receipt are not established. Process-wide allocator abort remains excluded. |
| G2: completion/unwind/GC | Original context installed on resume and unwind; cleanup before Ready corrected; unborrowed-root barrier and acyclic child-loan controls pass. | **F1 remains.** Do not substitute process-table removal or active-count zero for reclaiming a cyclic heap. |
| G3: cancellation/finite work | Completion-mutex arbitration, safe-point stop, accepted blocked Host write settles before cleanup, queued cancelled entry does not execute; new cancel-after-Taken child control passes. | Complete simultaneous take/cancel, spawn/cancel and accepted SQL-commit cancellation races remain unqualified. **F2** covers preparation lost on unwind. |
| G4: waits/resources | Nine modes now exercise HTTP, sleep, actor receive, job await, service call, cooperative receive/timeout, reduction and a resource scope. They require unwind, zero transaction count, resource mutex release and a fresh invocation succeeding. | Successful same-original SQLite use after a permitted HTTP wait remains an explicit missing positive control. The resource-mode test uses a test mutex/scope; it is not native-provider mutex qualification. |
| G5: Host/HTTP/foreign release | All nine callbacks and retain/release are exercised on actual native stacks with original identity. Held success/error HTTP retains Host through root cancellation and scheduler shutdown until worker exit. ActiveRequestGuard unregisters activated requests. | **F2 remains.** No direct allocator-erasure probe, exhaustive last-owner process/completion lock race proof or real JNI attach/global-ref/JNIEnv execution. Tests probe lifecycle/table locks, not every possible last-reference interleaving. |
| G6: transfers/unsupported routes | Exact InvocationContext equality for local application work; same-original service scalar call/reply positive and five cross-original negatives, including same authority with distinct invocation. Supervisors/registries/channels/remote work/PG/pools/Repo/WS/server/stream/continuity routes are refused at inspected boundaries. | **F1 bounds the allowed shared-loan representation.** Same-original work is not full shared-service capability16. Native post-wait original-caller observations are absent; a tombstone scan is not ProcessOwner revalidation. |
| G7: actual progress | Fresh retained baseline success/error receipts show fresh load/status BUSY3 while old HTTP is held. Candidate generated suite's finished log is green; its inspected driver runs actual held HTTP success/error and demands load/status finish before release, then no later old SQL/HTTP/private output. | Final coherent family association is pending. Actual unchanged Core cannot reach the selected held RPC because accepted schema PRAGMAs refuse; no bypass was accepted. Native serializers remain and actual product progress is unproven. |
| G8: public ABI/regression | Separate one-way companion header, fixed private pointer-result thunk and compiler limits, same logical manifest rows, unchanged legacy metadata; real symbol/JIT entries and collision tests. Bounds/state/post-shutdown-byte and unsupported-bit checks are in the passing generated suite. | Required final source/tool/artifact pins remain pending; no native-platform test is inferred. Findings require new red/green evidence and another independent frozen-source review. Mask4 stays off. |

Source inspection found no additional concrete final Host release under the lifecycle/completion/process-table locks in the paths traced. That negative observation is not exhaustive lock-safety proof. The caller's prohibition on releasing a handle concurrently with use, and on mutating release reentry from lifetime callbacks, remains a valid native ABI obligation. Blocking scheduler shutdown and process-lifetime code pinning are accepted restrictions, not defects merely because detached admitted HTTP can outlive shutdown.

The unsupported-route inspection includes local send/service/link/monitor/exit, actor/job/timer capture, channel entrypoints, supervisor and registry entrypoints, remote ordinary/control ingress and outbound work, Repo's internal leasing edge, PG public operations, WS room/client/server paths, HTTP serving/router captures and continuity entrypoints. It does not promote unchanged standalone internals, generic file/stdin/environment effects, or all reachable application exports to complete-effects coverage.

## Retained results observed during this pass

These are reads of parent-produced receipts, not executions by this reviewer. Copies and SHA-256 values are retained under `evidence/` and `evidence-pins.json`.

- `repo-refusal-red.log`: real health-query assertion failed, actual1 versus expected0. `repo-refusal-green.log`: 15 passed, 4 ignored. Expanded public-Repo matrix is also green in the workspace/final runtime receipts.
- `root-completion-red.log`: take0 before registered-root removal versus required12. `root-completion-green.log`: barrier test passes.
- `jit-submit-green2.log`: existing runtime-registration invariant passes.
- `owned-lifetime-gates3.log`: **9 top-level tests passed**, including the service child, all-nine callbacks and the nine wait submodes. Subprocess repetitions are not additional top-level tests.
- `workspace-lib-bins4.log`: successful library/binary summaries, including **70 REPL** and **1,672 runtime / 20 ignored**. As STATUS says, this predates the new service test but not the production Repo/Ready fixes.
- `runtime-source-final-r2.log`: now complete, **1,673 passed, 0 failed, 20 ignored**; includes the new service test. This log was pending in the original brief and is no longer pending at this read.
- `runtime-integrations-final-r2.log`: six named test binaries end successfully: cooperative HTTP accept1, HTTP response persistence4, library1, boundaries1, GC1 and memory1.
- `generated-library-final-r2.log`: now complete, **3 e2e_library** tests and **2 e2e_native_invocations** tests passed. The frozen driver enumerates all31 R1 cases in each of legacy/scheduled mode, plus actual held HTTP, bounds/state and C/C++ include-order checks. This is not 62 distinct scheduled behaviors: V1 and refusal/activation controls retain their original scope. Individual final-family case logs/artifacts still need the parent's exact family manifest; I have not relabeled an earlier family as final.
- `app-compatibility.json` reports byte-identical h/ts/swift/kt/jni.c and 176 unchanged logical export rows/limits/audiences; `app-consumer.json` reports the actual parser's identical 175 public/164 scoped result. These are retained compatibility receipts. Per STATUS, their app binaries **predate the Repo/Ready fixes** and are not final runtime qualification. No app parser or binary was rerun here.

No allocator-wipe instrument, real JNI, SDK/JSI/WK publication, original platform caller revalidation, or same-Core held-RPC/concurrent-load success receipt was supplied or manufactured. Existing ignored tests remain ignored; no claim of a fully executed platform or whole-workspace integration suite follows from the library/binary gate.

## Commands, artifacts and handback

Review actions were bounded file reads, SHA-256 comparisons and exact Git-object reads. The source-dependency command was:

```text
git show 9b844dbab34e61f6de575af2b25b0ed0f321059e:<owning-file>
```

Python was used only for text/diff/hash processing and writing these review artifacts; `rg`/`sed`/`nl` selected source ranges. There was no runtime load, reproduction execution, build, network, child, Git mutation, push, PR, merge, deployment or app edit. All new files are confined to `/Users/andreyyurlov/Documents/dev/mesh-lang/.tmp/native-owned-invocations/implementation-review-r2/`.

The frozen reproduction entrypoint documented by the owner is `cargo test --locked -p meshc --test e2e_native_invocations`; its source explains the actual fixture commands and artifact layout. It was **not** run by this review. Parent supplies the exact final commands/tool/artifact pins; earlier family identity is not inferred.

Hand back F1/F2 for narrowly scoped TDD fixes or an explicit runtime refusal of the unsupported ownership routes, then freeze a new candidate for independent review. Keeping numeric1 is necessary but does not itself repair these lifetime failures. Cap4 promotion and full Core/native/product completion remain separate decisions with the gates above.
