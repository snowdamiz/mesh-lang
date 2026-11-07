# Synchronous native effect authority (unit 1)

This extension implements synchronous admission for a bounded native embedding.
It does not qualify a protected application. `CAP_COMPLETE_EFFECTS`, scheduler
invocations, native policy, service context and private publication are **not
supported**. An application requiring any of those bits must reject activation;
there is no fallback from a missing scope to standalone execution.

The existing nine-callback V1 table, export symbols, input/output ownership and
standalone invocation behavior remain. V2 uses a separate callback struct and
an explicit pre-initialization, one-way protection setting. Generated headers
and ABI manifests describe the extension. `mesh_library_call_scoped_v2` takes an
existing generated function pointer from a trusted native adapter: the adapter
must validate its generated descriptor, audience and allowed operation before
calling it. Never derive that pointer or an authority handle from request bytes.
The synchronous wrapper and `CALL_LOCK` remain in place.

## Native lifetime

Create a domain, create immutable generation nodes, then bind an authority to
1–32 original nodes in that domain and an optional retained V2 Host registration.
The bound set cannot change. There is no domain-wide transition quota: each
node is separately reference counted and reclaimed when native references and
old authorities release it. A domain can create further nodes while old nodes
and authorities remain alive. A retired node never becomes live again. A new
allocation is not a rebind of any live handle. Native handles follow ordinary C
ownership: do not forge, reuse a released reference, or release a reference
concurrently with a call using it. Authority bindings hold their own references.

`mesh_authority_retire_v2(node)` stores a monotone atomic tombstone and returns;
it never takes Core, scheduler, SQLite, Host or I/O locks, and never drains work.
Native lifecycle code must retire the actual predicate's node synchronously
before acknowledging loss or exposing replacement. This mechanism does not
establish that a platform has those hooks. The synthetic tests do not represent
iOS, Android, React, Tauri or OS path ownership.

V2 retains callback context once at binding and releases it after the last
invocation/descendant/worker owner. V1 global callback replacement cannot retarget
a scoped invocation. Retain/release functions must not unwind across C. Callback
code/context must remain callable and thread-safe until release. A registration
resource/policy change must retire a node included in its original bindings;
retention itself is not permission or a live-policy snapshot.

## Acceptance and settlement

Only the sealed runtime sink types implement `FixedEffect`: a fixed HTTP attempt,
a fixed SQL preparation/recovery, a bound SQL statement, a fixed SQLite open, a specific Host callback request or
secure-store mutation. `perform` consumes that operation and returns only its
completed outcome. It never returns a permission Boolean, ticket, mutable job,
Mesh closure or transaction callback. All user computation is outside acceptance.

Node state is immutable except for `live -> retired`. A successful bounded scan
linearizes at its first node load: every node later observed live was also live
at that first load. Thus the entire original set was live at one point during
acceptance. A retirement preceding that point refuses the effect; a retirement
after it may lose to that exact operation. Failure of any load executes nothing.
The shared invocation sequence uses one CAS attempt; contention/exhaustion fails
closed. There is no retry loop, lock, callback or I/O in admission or retirement.
This proof relies on monotonicity; adding node reset would invalidate it.

For local effects, increment retained node settlement counters before **any**
liveness load and decrement after completion/refusal. After retiring a node,
`mesh_authority_local_inflight_v2(node) == 0` establishes that its previously
accepted finite local operations have settled. A concurrent new attempt may
briefly increment the counter, but it cannot be accepted after retirement.
HTTP and read-only Host callbacks are excluded. This is a primitive for native
resource fencing, not a complete account replacement protocol. Never treat a
timeout as a stopped OS write. Finalizers run outside admission and table locks.

## Implemented sinks and limits

- **HTTP client:** context is captured before worker creation and retained through
  worker completion, including cancellation/timeout. Admission consumes one
  immutable method, URL, headers, body and TLS selection at the dispatch executor.
  Scoped requests set redirects to zero; application retries admit separately.
  No response grants another effect. Protected streams refuse because their
  off-actor compiled callbacks are not qualified.
- **SQLite:** handles retain a serialized connection. Protected connections are
  owned by the opening original invocation; they cannot be adopted by another
  invocation. Preparation/schema loading can recover a hot journal and therefore
  admits as a separate local operation before any SQLite prepare call. Each bound statement (including read routes, `RETURNING`, raw
  `COMMIT` and outer `RELEASE`) admits independently. The callback transaction
  remains arbitrary Mesh code; no transaction callback is a fixed effect.
  Prepare-time PRAGMAs, attach/detach, virtual-table creation and non-allowlisted
  functions refuse via SQLite's authorizer. Use the bundled SQLite without
  externally installed UDFs/extensions/VFS side effects. Progress/busy budgets
  bound SQLite VM work/waits; OS I/O itself has no false deadline guarantee.
  Refusal rolls back provisional transactions; process exit closes owned
  protected connections. HTTP suspension while this invocation owns a SQLite
  transaction refuses and later cleanup rolls it back. General service/message
  transfer and long-lived cross-invocation connections are not qualified.
- **Host:** scoped calls use the captured nine-callback table. Each exact
  request is admitted; secure-store put/delete also register local settlement.
  Synchronous coroutine Host callbacks execute on their worker's native stack
  with the same original invocation, borrowed-buffer fence and retained context.
  The callback must implement exactly that request and complete before returning;
  it cannot treat return success as permission for arbitrary deferred work.
  Platform JNI dispatchers and their own resource generations still require
  separate integration proof; no foreign `JNIEnv`, PID or yielder is copied.
- **Platform nonce:** the existing per-key lock fixes the next record; its actual
  secure-store put is admitted separately. A successful reservation remains
  burned if any later operation refuses. Direct file-backed and custom-provisioned
  storage keys explicitly refuse in protected scopes pending sink/lifetime work.

Root calls, spawned actors/jobs, timer callbacks and ordinary HTTP workers carry
the original retained invocation. Scheduler resume installs/restores that context
beside the PID; heap loans remain separate. Terminate callbacks retain original
context and get no special cleanup grant; protected SQLite open refuses without
a live process/resource frame. Runtime-only rollback/close is limited
to already owned connections. Shared service messages, supervisor restart and
remote actor work require their own metadata/ownership unit. No fresh-owner
progress is claimed while the synchronous export holds `CALL_LOCK`.

Protected calls return fixed refusal statuses, never raw application error
payloads. Returning owned successful bytes is still **not** an actual IPC/UI
publication gate. Native receiving adapters must qualify final delivery, private
errors, SDK effects and their original recipients before integration.

## Required integration rejection

A native application must derive required capabilities from its actual source
and generated export inventory, including every reachable runtime effect. At
minimum a real protected app needs complete effects, native policy, service
context and actual publication; its activation therefore currently returns
`MESH_LIBRARY_ERR_UNSUPPORTED_CAPABILITY`. Do not request only the unit1 bit to
bypass this requirement. That bit is for the bounded synthetic embedding tests.

Still unqualified: File/stdin/environment policy; Postgres/pools/repos; WebSocket
client/server/automatic protocol traffic; HTTP serving; distribution/remote actor
and service messages; supervisors; process/consensus/continuity effects; generic
stdout/stderr/panic release; file/custom nonce sources; native SDK/policy/final
publication; real-platform lifecycle and serializer removal. Those runtime paths
retain standalone behavior; they are not covered by the unit1 bit. The generated
manifest explicitly reports `completeEffectInventory: false`.

## Reproduction

`cargo test --locked -p meshc --test e2e_native_authority` builds the real generated
ABI and runs a loopback HTTP server, real SQLite and retained storage callbacks.
It checks retirement before a later send/write/commit; acceptance before retire
inside a real SQLite commit hook and secure-store mutation; nonce burn; original
actor/timer identity and native stack; callback lifetime after root return;
missing scope; unsupported activation; and over 1,024 generation transitions
while the original authority remains live. No external network is used.

The fixture's `check.py <library> write --baseline` intentionally fails against
8256d5d: after the blocked HTTP response is released, a retired synthetic outer
owner still commits one row. `control --baseline` succeeds. Parent's separate
ObjC baseline also demonstrated a descendant commit/send after export return;
its source/artifact/log receipts are retained outside this repository's source.
