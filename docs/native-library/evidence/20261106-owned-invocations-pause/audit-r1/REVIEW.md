# R2 runtime audit, round 1

**Disposition: changes requested from bounded source inspection.** Five open findings below. This is not a PASS, an exhaustive inventory, execution evidence, or qualification of G2/G4/G5/G6. No builds, tests, workers, network operations, or public API edits were performed. All audit writes are inside this directory.

The initial snapshot was taken at **2026-11-07 01:42:31 UTC**, on `fix/native-owned-invocations`, HEAD `9b844dbab34e61f6de575af2b25b0ed0f321059e`. `pin.json` pins 142 individual files; `runtime.diff` has SHA256 `9b984ba82a3199bbbba7103773b47873c5b4eca1daeca95e0312675d9915733e`. The snapshot is a bounded sequence of reads, not an atomic checkout. Individual file hashes are the reference when comparing with the actively edited tree.

Initial `library/invocation.rs`: `1ac7a32719b59adfaa08266e96e04f74f46f1cb5c458cc1d2eb5db0df811122a`. Inventory: `d6dc9bdc738dc0701e39322996a70217e9af8143898f0aece1e5d14c142f5391`. Amendment 3 matches the supplied `544325a5eb2c656741e40da7524edc1ed645a1b7aeaea7219814e2a7bfbe79dc`. Legacy authority header matches `5bbaf429dbd3749924b041847aba40ce40ee33c5a13a5c97f454601d79f87843`.

All source locations below are relative to `compiler/mesh-rt/src/` in **snapshot/** unless explicitly identified as drift. These files remain available here even if the parent changes the live lines. The four-file **drift-snapshot/** and its own pin record capture relevant concurrent fixes; the later final drift receipt records changes relative to the initial snapshot.

## Open findings

### F1 — P1 — WebSocket room APIs bypass the new protected refusals (G6)

**Locations:** `ws/rooms.rs:270-278`, `:233-247`, `:160-173`, `:200-217`; compare guarded `ws/server.rs:536-537`.

`mesh_ws_send` refuses a scoped caller, but `mesh_ws_broadcast` forwards to `mesh_ws_broadcast_except`, which directly calls both `deliver_locally` and `broadcast_room_to_cluster`. Neither path checks original authority. Local delivery calls `conn.io.send`; cluster delivery queues an application frame on every session. Existing connections/rooms created outside the invocation suffice: the protected caller only needs a room name, not a connection pointer. A retired invocation still executing before its next scheduler check can therefore send private bytes through this route. `mesh_ws_join` and `mesh_ws_leave` also mutate shared membership without refusal.

This is a missed R2 boundary on an existing route, not a claim that the baseline qualified WebSockets. Inventory item 11's broad WebSocket discussion does not enumerate these entry points, and the new server/client guards do not dominate them.

**Narrow fix:** refuse scoped room join/leave/broadcast/broadcast-except before membership mutation or local/cluster queueing. Preserve standalone behavior. Do not grant a fresh authority at the reactor or remote receiver.

**Behavior test:** establish a real standalone room connection and a peer session; invoke a scoped direct entry that broadcasts, once with live and once with retired original authority. Expect refusal and no local frame or cluster application frame. Keep an unscoped positive control. Check membership remains unchanged on refused join/leave.

### F2 — P1 — Remote control ingress bypasses protected process ownership (G6, G5)

**Locations:** `dist/node.rs:2197-2298`, especially monitor insertion at `:2216`, link insertion at `:2261`, control-message enqueue at `:2294`, and forced exit at `:2297`; resulting egress at `actor/scheduler.rs:1136-1159`.

Ordinary `DIST_SEND` correctly reaches the new guarded `deliver_local`, but `DIST_MONITOR`, `DIST_DEMONITOR`, `DIST_MONITOR_EXIT`, `DIST_LINK`, and `DIST_EXIT` access the target process directly from the distribution reader. They neither propagate native provenance nor refuse a target with an invocation. Outbound protected link/monitor refusals do not protect this ingress.

A connected peer can install a monitor/link on a live protected PID. Finalization then sends the process's actual exit reason to that remote owner; a panic-derived reason can contain application text. A remote exit can also enqueue a control message or kill a protected process, causing its terminate path to run under the original native context. The peer session need not forge native authority: these branches bypass the common target check entirely. This matters when an already-running standalone distribution service coexists with owned invocations; submit checks worker count, not that distribution is absent.

**Narrow fix:** reject these inbound controls for invocation-owned targets before any mutation/notification, with any protocol response limited to a fixed public refusal. Keep target lookup and the ownership decision coherent with process finalization. Do not apply the public sender-TLS helper mechanically inside a process lock. Review disconnect cleanup separately from admission of a new remote relationship.

**Behavior test:** feed actual peer-session MONITOR/LINK/EXIT/DEMONITOR frames targeting a scheduled protected process. Assert no registration, mailbox insertion, termination callback, or private exit-reason frame. Include an ordinary unscoped PID control. The existing ordinary-message refusal is not this test.

### F3 — P2 — Late HTTP success/error storage is dropped without erasure (G5)

**Locations:** `http/client.rs:90-99`, `:782-805`, `:871-877`; initial `OwnedInvocation` zeroizing output at `library/invocation.rs:12-15` does not cover these allocations. The drift HTTP edit changes the suspend refusal only.

`WorkerResponse.body` is a plain `Vec<u8>`, response headers contain plain `String`s, and `WorkerEvent::Complete` carries plain error `String`s. Cancellation detected after body reading returns a fixed error while dropping the private body/header allocations normally. If completion races receiver teardown, the ignored `sender.send(...)` error drops the event normally; dropping an already queued event has the same issue. There is no wiping destructor on these types. Suppressing delivery is implemented separately from the requested erasure of runtime-owned late private success/error buffers.

**Narrow fix:** give the owned worker response and private error buffers erasing ownership before they can enter any channel or cancellation branch. Cover both queued-event and send-failure destruction, including headers if private response data is in scope. Audit request-owned buffers separately; do not claim erasure of all TLS/OS/library-internal copies.

**Behavior test:** hold a real response body, cancel/drop the root receiver, then release success and error variants. Use a bounded instrumented disposal boundary to verify owned payloads are wiped on cancel-after-read, queued-event drop, and send failure. Separately verify exactly-once Host release after actual worker exit; zero delivery alone does not prove erasure.

### F4 — P2 — Router closure registration can permanently pin a completed root heap (G2, G6)

**Locations:** `http/router.rs:183-209`, `:285-300`; `actor/mod.rs:572-587`; `actor/scheduler.rs:848-869`; `actor/process.rs:366-371`.

Protected HTTP serving refuses only when `mesh_http_serve*` is called. Before that, all route variants reach `route_with_method`, which calls `pin_closure_env`; middleware does likewise. `pin_closure_env` deliberately forgets the `HeapBorrow` vector. For a closure capturing the invocation's managed input, that vector owns a strong `Arc<Process>` and the heap loan. Even if serve then refuses, root completion removes the process from the table but cannot reclaim the heap: the forgotten loan has no release path. Repeated rejected invocations can retain their request heaps indefinitely.

This finding is heap/resource retention, not a claim that the original Host authority remains in the finalized process: finalization takes its invocation field. Inventory items 11/14 discuss serving and loans, but not the earlier permanent closure registration.

**Narrow fix:** for the bounded unsupported-server unit, refuse scoped route/middleware capture before pinning. Alternatively, make a router an explicitly owned resource with owned capture loans and a real disposal path; do not silently allow a captured router to execute later under an unrelated service's context.

**Behavior test:** a generated owned entry captures request data in a route/middleware closure and attempts unsupported serving. After terminal refusal and handle release, prove the original process/heap is reclaimable. Repeat enough calls to detect retained heaps, with a normal same-invocation child-loan control that releases only after the child ends.

### F5 — P2 — Cancel does not reclaim delayed-send owners until an arbitrary timer expires (G2, G5)

**Locations:** `actor/mod.rs:1041-1067`; `actor/scheduler.rs:302-311`; `library/invocation.rs:50-60`.

`Timer.send_after` now captures the correct invocation and rechecks target compatibility at delivery, which prevents authority substitution. However, it starts a detached native thread that enters that invocation and calls `thread::sleep(delay)`. The delay comes from an unrestricted positive i64 millisecond value. Cancellation marks the context and visits scheduler processes only; this thread has no cancellation wake or owned timer registration. Its captured message can retain heap loans and its TLS scope retains the original Host context until the entire delay expires, even after root completion, cancel, handle release, and scheduler shutdown. The eventual send will refuse, but no effect has been accepted that requires keeping this dormant timer owner alive for years.

The amendment explicitly allows a delayed timer thread as a release executor, so the objection is the missing cancellation/settlement path, not which thread performs release. Inventory item 6 suggested early refusal or an owned timer process; the implemented native sleep adds provenance but not cancellation ownership.

**Narrow fix:** refuse scoped delayed sends in this unit, or use an owned cancellable timer registration/process whose cancellation removes the payload and releases loans/context outside timer/process locks. Cancellation must not join a sleeping native thread.

**Behavior test:** schedule a far-future same-invocation send carrying a borrowed heap value, finish/take the root, cancel and release native ownership, and assert exactly-once Host/context and heap reclamation without advancing to that deadline. Keep an ordinary delayed delivery positive control and a cancel-versus-fire race.

## Concurrent fixes observed; not open findings

- The initial blanket callback marker made standalone V1 callback `init`/valid `register_host_callbacks` return BUSY. The drift `library.rs` now uses `in_owned_callback` for those operations, while operation callbacks classify scoped versus V1 execution. This addresses the identified source mismatch; the added compatibility test was not run by this audit.
- Initial `authority.rs:298` called retain directly, without native-stack trampoline or callback marker. The drift file wraps retain in `on_native_stack` and `callback_scope(true)`, and release uses the owned marker. Source-addressed, not executor/reentry qualification.
- Initial `http/client.rs:854-856` returned an error when a writer transaction prevented suspension, allowing the continuation to keep running with the transaction until a later stopping point. The drift file drops the pending request, preserves a non-unwinding callback refusal, and otherwise calls `suspend_or_stop`. The parent also added `writer_transaction_aborts_http_before_dispatch`. This fixes the identified direct scheduled path in source; rollback/real dispatch evidence and the full wait matrix remain unexecuted here.

## Reviewed call-site classification

“Safe” below means the inspected source has the required structural ownership boundary for that route. It is not a tested PASS or closure of a gate. “Needs propagation” includes missing cancellation/resource ownership, even where original effect identity is already retained.

| Route and actual call sites | Classification | Source evidence / remaining limit |
|---|---|---|
| Owned root: `library/invocation.rs:85-141`; `scheduler.rs:296-299,329-334,952-1009` | Safe, structurally | Owned copied input, direct entry, process context installed on resume; cancellation unwinds on the owning worker before resource finalization. No runtime/panic/GC proof executed. |
| Root finish: `scheduler.rs:1092-1116,1175-1179`; `invocation.rs:36-47` | Safe, structurally | Extract context/owned root under process lock, run cleanup outside it, stage terminal completion after SQLite/secret cleanup. Root result is distinct from descendants and detached workers. |
| Accepted finite effects vs cancel: `process.rs:442-456`; `authority.rs:191-220` | Safe, structurally | Scheduled processes skip remote-thread secret destruction; exact accepted effect executes before owner-worker unwind. No accepted-write race was run. |
| Actor/shaped spawn and jobs: `scheduler.rs:329,345-358,896-929`; `job.rs:94-144` | Safe, structurally | Context captured; heap copies/loans retained. Job result target comes from captured spawner, not a fresh-authority lookup. |
| Local messages and remote ordinary ingress: `actor/mod.rs:652-675,744-751`; `dist/node.rs:2185-2187` | Explicit refusal at incompatible boundary | Exact InvocationContext identity; None/Some and different contexts refuse. No queue-mutation race test performed. |
| Service call/cast/reply: `service.rs:75-101,159-171,186-191` | Explicit refusal at incompatible boundary | Call checks before monitor/enqueue; casts/replies check original target. Same-context path remains to be behavior-tested; shared services are not qualified. |
| Channels: `channel.rs:178,236,277` | Explicit refusal | Scoped create/send/receive refuse. Metrics do not carry work. |
| Timer.apply_after: `actor/mod.rs:982-1025`; scheduler spawn/resume | Safe, structurally | Owned actor context and sleep guard preserve origin. Cancellation/readiness and root-return GC tests remain. |
| Timer.send_after: `actor/mod.rs:1041-1067` | Needs propagation/ownership | Original context and delivery refusal added; cancellation does not settle dormant thread owner (F5). |
| Public link/monitor/exit/terminate registration: `actor/mod.rs:1131,1159,1507,1531` | Explicit refusal at incompatible boundary | New target checks; not evidence for inbound distribution control (F2). |
| Supervisors: `actor/mod.rs:1356,1399,1412,1442`; restarts in `actor/supervisor.rs` | Explicit refusal at protected entry | Start/management refuses before reaching unqualified restart/shutdown mechanisms. No bit16 grant. |
| Process/global registry and node monitor: `actor/mod.rs:1217,1237,1629,1670,1714,1730` | Explicit refusal | Scoped public entry refuses. Startup function registrations remain separate initialization machinery. |
| Outbound remote actors / node lifecycle: `actor/mod.rs:691`; `dist/node.rs:492,6198,6241,6405,7924` | Explicit refusal | Refuses before scoped dispatch/start/connect/spawn. Existing unscoped node state may still exist. |
| Inbound remote controls: `dist/node.rs:2197-2298` | Missing | F2. Ordinary remote message guard does not dominate these branches. |
| PostgreSQL: `db/pg.rs:1342,1499,1629,1650,1662,1674,1709,1725,1740,1782,1877` | Explicit refusal | Connect, existing-handle operations, transactions, decode/query callbacks refuse before work. |
| Pools / Repo: `db/pool.rs:259,313,331,371,388,399,410,434,453`; sampled `db/repo.rs:276,324,363,389,748,1402,1516,1528` | Explicit refusal through pool boundary | Pool open/checkout/checkin/operations/close guarded. Sampled Repo sinks delegate to those guards; not an exhaustive Repo audit. |
| WS client: `ws/client.rs:351,639,650,658,714`; server: `ws/server.rs:264,290,326,506,537` | Explicit refusal | Connect/serve/send/receive/close entry guards. Room APIs remain missing (F1). |
| HTTP serving: `http/server.rs:1176,1184,1205,1240` | Explicit refusal | Refuses bind/serve; earlier router captures are not covered (F4). |
| Ordinary HTTP worker: `http/client.rs:858-878` | Safe provenance; missing erasure | Context captured before thread start and retained until worker return; active registry RAII guard cancels/removes on root unwind. F3 remains. |
| HTTP streaming: `http/client.rs:1043-1046` (initial snapshot) | Explicit refusal | Protected stream path refuses before callback worker; stream callback qualification is not inferred. |
| Continuity: `dist/continuity.rs:3066,3096,3130,3146,3157,3168,3184,3199` | Explicit refusal at public entry | Scoped submit/status/complete/replica paths refuse. Internal consensus/store/background recovery are not independently qualified by this pass. |
| Central waits: `stack.rs:99-100`; `actor/mod.rs:154,184,820,945`; `service.rs:76-78` | Safe guard structure, untested matrix | Final yield plus cooperative receive, actor/job receive, sleep and service boundary guards. HTTP direct path source-fixed in drift. |
| SQLite legal wait: `db/sqlite.rs:68-83,113-143,621-662`; `authority.rs:130-131` | Safe root retention, bounded | Root-owned connection remains registered across a legal no-transaction wait; exact original context required. Open transactions conservatively refuse suspension. Root exit still closes creator handles; descendant use after root cleanup is not qualified. |
| Platform storage mutex: `storage_wrapping.rs:1089-1090,1118-1119` | Safe guard structure, untested | Scoped resource count surrounds the platform lock. This is not proof that every possible resource mutex is tracked. |
| Host operation/lifetime callbacks | Safe structure after drift; tests pending | Operation callbacks use original table and native stack. Retain/release marker fixes noted above. No concrete final NativeAuthority release under the named lifecycle/completion/process locks was established by this bounded trace. This negative finding is not lock-safety proof. |

## Limits that remain explicit

The inventory is a baseline research memo, not a completion ledger: its opening pins 9b844dba, and several described gaps now have partial source fixes. It should not be used as evidence that every named family is covered. In particular it needs explicit room APIs, inbound control frames, router capture loans, and delayed-send cancellation ownership.

The common coroutine guard does not establish coverage for unrelated synchronous blocking APIs. For example `io.rs:93-99` still locks stdin and calls `read_line` without a scoped boundary. File/stdin/environment and other effects are already excluded in the accepted unit1 document; if a selected R2 descriptor can reach them, they need a concrete refusal/qualification decision. This audit did not expand into a complete-effects review or assert descriptor reachability.

Shutdown is explicitly blocking in amendment 3. `signal_shutdown` does not itself cancel every owned root, and cooperative HTTP waits can remain until their response/deadline unless cancelled. I have not misclassified that declared blocking behavior as a cancellation-API defect. The existing test cancels before shutdown, so it does not establish arbitrary-shutdown behavior.

No actual generated ABI or Core composition, all-nine-callback exercise, final-release lock probe, GC stress run, retained R1/runtime/workspace suite, JNI/adapter run, public API approval, or capability promotion was performed. The initial capability function still returns mask1. Existing `CALL_LOCK` remains. No conclusion here authorizes serializer removal, bit4, merging, native integration, publication, or whole-goal completion.
