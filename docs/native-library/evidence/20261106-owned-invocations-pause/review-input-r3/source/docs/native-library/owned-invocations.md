# Native scheduler-owned invocations

This companion API is implemented for bounded runtime qualification. The capability
mask remains **1** pending the full scheduler gate; applications requiring scheduler
bit4 must reject activation until that gate is accepted. Bits2,8,16,32 remain
unsupported. Native serializers and the legacy CALL_LOCK remain. An invocation
handle owns lifetime and a result; it is never effect or publication permission.

The existing R1 authority header and all legacy binding bytes remain unchanged.

## Direct generated companion and routing

Each existing logical export `name` gets one native-only companion:
`int32_t name_submit_v2(const MeshNativeAuthority *, const uint8_t *, uint64_t, MeshOwnedInvocationV2 **);`
The companion supplies the private pointer-result entry thunk and compiler-declared limits directly to runtime bounded-submit infrastructure. It never calls or parks the legacy synchronous wrapper, and does not expose a raw aggregate-return thunk as an actor entry.

The compiler emits companion declarations and the opaque handle/functions below ONLY in a separate `<artifact>.invocations.h`, which includes the unchanged legacy header. The legacy header does NOT include or declare this extension. Existing native consumers may identify exports by literal declarations in the legacy header and resolve the three-argument ABI. Native adapters opt into the separate header. Existing logical exports/legacy symbols/limits/audiences/wires remain unchanged; the existing logical Mesh operations are NOT the total physical symbol count after companions. Enumerate runtime support, legacy and companion physical symbols and reject every collision, including an existing `x_submit_v2` versus the companion for `x`.

Manifest adds `scheduledSymbol` and companion ownership/capability metadata on the SAME logical rows; legacy top-level borrowed-request/synchronous metadata stays true. Companion metadata says native-only, copied bounded input, opaque once-owned completion, capability4 required for qualified use. Compiler JS bindings and app scoped-bindings consume legacy `row.symbol` and original audience; no additional JS operations. Swift/Kotlin/JNI legacy outputs unchanged. Trusted native validates/narrows module/export descriptor, audience, operation and corresponding companion before passing its function pointer; request bytes cannot choose pointer, limits, authority or lifetime handle.

## Functions and caller ownership

`int32_t mesh_library_invocation_take_v2(MeshOwnedInvocationV2 *, MeshLibraryBytes *);`
`int32_t mesh_library_invocation_cancel_v2(MeshOwnedInvocationV2 *);`
`void mesh_library_invocation_release_v2(MeshOwnedInvocationV2 *);`

Submit status0 transfers exactly one caller-owned handle reference. Pointer copies do not retain. Caller may concurrently take/cancel while that reference remains alive; release must be externally excluded from all concurrent uses and consumes it exactly once. Null release is a no-op. Forged handles, double release, and use-after-release are outside the valid native ABI. Internal process/worker/descendant owners retain independent references. Release neither cancels nor joins execution.

A valid take destination must be empty on entry (caller frees any prior allocation first). The runtime writes `{NULL,0}` before every return. It never frees an unrelated prior allocation. Null output returns1 without consuming state. Returned successful bytes are a separate runtime allocation, owned independently of the handle and valid after handle release/shutdown; use only existing `mesh_library_free_returned_bytes`, which wipes using private allocation length. Private output is not publication permission.

## Normative status/state table

| Operation and condition | Return | State/output |
|---|---:|---|
| Submit: valid output pointer on entry | — | Set handle to NULL before validation |
| Submit: null authority/output; null input with nonzero length; invalid limit outside1..64MiB | 1 | No caller handle, no queued process |
| Submit: input above compiler max (default1MiB; ceiling64MiB) | 6 | No caller handle, no queued process |
| Submit: before init, shutdown begun, or after shutdown | 2 | No caller handle, no queued process |
| Submit: nested Mesh/operation/lifetime callback entry | 3 | No caller handle, no queued process |
| Submit: unsupported scheduler configuration (this unit requires bounds1..1) | 11 | No caller handle, no queued process |
| Submit: original authority retired or invocation-ID exhausted | 10 | No caller handle, no queued process |
| Submit: recoverable owned-buffer/queue preparation allocation failure | 6 | No caller handle, no executable orphan |
| Submit: input/context/completion/process owned before successful enqueue | 0 | Exactly one caller reference; root Pending |
| Accepted root: coroutine allocation/start failure or panic | eventual4 | Root cleanup finishes, then Ready4 with no bytes |
| Take: null handle/output | 1 | Valid output cleared; no state consumption |
| Take: valid handle/output, nested Mesh or operation/lifetime callback entry | 3 | Empty output; no state consumption or mutation |
| Take: Pending (including cancel requested but root cleanup unfinished) | 12 | Empty output; remains Pending |
| Take: Ready success, original still live | 0 | Transfer bounded native bytes exactly once; Taken |
| Take: Ready application error / panic / invalid result / oversized output | 9 / 4 / 1 / 6 | Empty output, including private errors; Taken |
| Take: Ready after cancellation or retirement wins original scan | 10 | Wipe/drop staged bytes; empty output; Taken |
| Take: output allocation fails on Ready success | 6 | Empty output; staged bytes wiped; Taken |
| Take: Taken | 13 | Empty output; remains Taken |
| Cancel: null handle | 1 | No change |
| Cancel: valid handle, nested Mesh or operation/lifetime callback entry | 3 | No refusal/termination mutation; state unchanged |
| Cancel: any valid handle in permitted call context (Pending/Ready/Taken, repeated) | 0 | Monotone refusal; request safe-point stop of original local processes; no join |
| Cancel: Pending | 0 | Remains Pending until actual root unwind/resource cleanup, then Ready10 |
| Cancel: Ready | 0 | Wipe/replace output with Ready10 |
| Cancel: Taken | 0 | Remains Taken; surviving descendants still refused/stopped |
| Release: null | void | No-op |
| Release: valid caller reference | void | Drop caller ownership outside runtime locks; no cancel/join |

For take/cancel, null-argument validation has priority over reentry: a valid take destination is cleared first; null handle/output returns1; otherwise reentry returns3 before any completion/process lock or state change. Lifetime callbacks must not release any runtime handle (including another invocation/authority), even though null release remains a no-op; this is a native caller obligation, not implicit authority to mutate.

Nonblocking take never waits for execution, I/O, a callback or an executor; it may briefly serialize on its completion mutex and is not lock-free. Concurrent takes have one terminal winner. Cancellation writes refusal and chooses cancel-versus-take ordering in that same mutex. Generation retirement is independent: take scans the immutable original nodes under the completion mutex, with R1's monotone first-load linearization. Retirement-first refuses; transfer-first can return only its selected original bytes. Later cancellation/retirement cannot recall those bytes or authorize publication. Callback buffers retain their independent1MiB limit. No raw protected error payload leaves any terminal path.

Ordinary allocator failure is handled before enqueue with fallible reservation; process-wide allocator abort/panic-abort/OS termination cannot return any ABI result and is not claimed recoverable. Enqueue is publish-once only after every fallible preparation. The single-lane owned queue uses the existing non-disconnecting injector; worker/thread/stack construction failures after acceptance must finalize the owned root with4. Tests inject recoverable preparation and coroutine-start failures; no false successful queued status is proof of progress.

## Lifetime callbacks and code lifetime

This bounded V2 API supports STATIC/process-lifetime code only. From first scheduled use, the embedding must keep the generated module, Mesh runtime, and all registered operation/lifetime callback code loaded until process exit. For dlopen embeddings retain their library handles until process exit; do not dlclose them after shutdown or after a callback reports context release. Dynamic runtime/module/callback-code unloading is explicitly unsupported by this unit. There is no purported quiescence or unload permission. This is the concrete supported lifetime rule, not a future unspecified worker fence.

Host CONTEXT memory has a shorter, usable lifetime: V2 binding calls retain once; the final authority owner calls release exactly once after every original invocation/descendant/HTTP-worker owner ends. The release callback may run on a native caller thread (including handle/authority release after shutdown), scheduler worker's native stack, delayed local timer native thread, or detached HTTP worker at actual exit. It must be thread-safe, bounded, nonjoining and not unwind. It may free pure native storage/references immediately or enqueue owned disposal to an embedding executor guaranteed to remain available until that disposal finishes; that queue must own what it needs. It may not block/wait for Main/JS/JNI/provider/runtime work or reenter mutating/invocation runtime APIs. Diagnostic current_invocation remains allowed for R1 compatibility. JSI/runtime-bound objects and borrowed JNIEnv/stack pointers cannot be stored in this retained context. JNI adapters retain JavaVM/global refs and attach at the actual callback thread; no real JNI qualification is inferred here.

Nine operation callbacks remain synchronous ready-operation calls on the actual worker's native stack, with original context and borrowed-buffer lifetime until return. Their effect admission and local-settlement distinction remain R1's. They do not share the lifetime callback's permission to schedule native context disposal, and their success is not permission for arbitrary future work.

All ownership capable of foreign release is detached under lifecycle/completion/process-table/process locks and disposed after unlock. NativeAuthority final release uses the normal-stack trampoline when necessary, with a lifetime-disposal reentrancy marker. Submit/invoke/take/cancel reject nested Mesh and operation/lifetime callback reentry before acquiring their locks; diagnostic identity is safe. The new init/enable/register rejection applies to V2-owned operation callbacks and all V2 lifetime callbacks, not standalone V1 operation callbacks. In standalone V1 callbacks, idempotent init and valid callback-table registration preserve their existing status/behavior (registration affects subsequent calls; the current callback uses its copied table). Nested shutdown independently returns3 from any operation/lifetime callback before locks, including V1, to avoid the old self-deadlock; this defensive refusal does not change normal V1 shutdown. Callback guards do not broaden authority or modify standalone V1 registration outside this explicit boundary. Process destructors/Mesh termination/resource rollback run outside lifecycle/completion/process-table locks, in explicit original invocation cleanup context. Root completion becomes observable only afterward. This retains the R1 no-resource-frame refusal for terminate callbacks; it does not mint a new cleanup grant.

## Shutdown and scheduling

V1 `mesh_library_shutdown` remains an explicitly blocking scheduler shutdown, not a V2 cancellation/retirement API and not an HTTP/code-quiescence fence. CALL_LOCK still serializes it with legacy synchronous exports; after acquiring that legacy guard it transitions lifecycle Running->Stopping under LIFECYCLE and releases LIFECYCLE BEFORE finalization/join/foreign disposal. Nested callback shutdown refuses3 before locks. Concurrent shutdown callers retain legacy serialization. Submission observes Stopping and refuses2. Scheduler-owned roots finish/unwind/close before shutdown completes; caller handles remain usable for terminal take/release/free afterward. HTTP workers already executing admitted work may outlive shutdown and retain their original Host context until real completion; lifetime callbacks can consequently run after shutdown. Process-lifetime code pinning above makes that safe. Shutdown cannot be used to authorize native replacement or claim effects settled.

A SchedulerOwnedProcess owns its actual PID, coroutine/yielder/GC stack, copied input, original immutable context and completion BEFORE enqueue. One existing scheduler lane; reject other configured bounds before enqueue. No CallContext parking. Legacy CALL_LOCK remains; only direct scheduled companions bypass it after the red/green proof. Existing native serializers untouched.

Root completion is distinct from descendant/worker settlement. Descendants retain original authority/Host and borrowed heaps. Cancel refuses new effects immediately, requests stopping at a safe point, and never forcibly destroys an executing admitted finite SQL/Host effect or its secrets from another thread. The owner worker unwinds under original PID/context, then closes resources and completes the root. HTTP cancellation/drop unregisters the request but does not claim the worker/OS attempt stopped. Root-owned SQLite handles survive permitted external waits only with no open transaction/resource lock; any suspend-capable call while such ownership is held refuses and unwinds. Callback SQL transactions and separate prepare/step/commit admission remain.

## Qualification boundary

The fixture exercises actual generated companions, SQLite and held loopback HTTP.
The source inventory deliberately refuses shared/cross-invocation services, raw
channels, supervisors, delayed sends, remote work/control, PostgreSQL, WebSocket
rooms/connections and HTTP serving/router capture. Same-original actor/job/timer
work retains its originating context. Full service capability remains off.

Scoped message transfers must copy every reachable value described by their
compiler shape. Opaque values requiring a heap loan (including SHARED replies)
refuse before installing the loan. Spawn-time root heap ownership remains for
live descendants; a root's completion does not invalidate those borrows.

Prepared HTTP requests retain their original invocation identity. Only that
invocation's processes may modify, send or cancel them. Cancellation disposes
its never-sent requests; natural completion disposes them after its last process
exits, preserving a live child's builders after root completion or handle release.
Process counts serve disposal only and are never effect permissions. Activation
atomically transfers a builder to the active request registry; already admitted
workers keep their separate original context until actual completion.

`Timer.apply_after` uses an owned actor; `Timer.send_after` remains unsupported in
scoped execution because its native sleeping thread has no cancellable ownership.
A root-owned SQLite connection survives a legal wait without an open transaction.
Root teardown closes its connections; descendants cannot adopt those handles.

Host callbacks execute synchronously. A blocked native callback occupies its
scheduler lane. An HTTP worker may outlive cancellation and scheduler shutdown,
retaining original context until actual completion. Runtime-owned completed HTTP
bodies, headers and queued error strings erase on destruction; this is not an
erasure claim for TLS/OS/provider-internal copies.

The scratch MobileCore composition cannot currently reach its held RPC under the
bounded policy: Storage.Blobs.ensure_schema uses PRAGMA busy_timeout and
secure_delete, which accepted R1 deliberately refuses. No app source is changed
to bypass that boundary. Shared Core service and Job.map compatibility, platform
JNI/global-reference adoption, SDK effects, and actual private publication remain
separate gates. Code pinning and context retention must be implemented by native
adapters before adoption; legacy shutdown is not safe dynamic-unload permission.

Reproduction: `cargo test --locked -p meshc --test e2e_native_invocations` compiles
the real ABI, checks companion collisions and C/C++ header inclusion, runs actual
held HTTP progress in success/error branches and reuses all31 R1 cases through
legacy and scheduled entries. `cargo test --locked -p mesh-rt` includes native
lifetime, cancellation, startup failure, transaction unwind and provenance tests.
No test here establishes a native-platform/product qualification.
