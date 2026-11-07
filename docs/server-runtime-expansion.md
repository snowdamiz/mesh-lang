# Server runtime expansion

This is the implementation and verification plan for the server capabilities
requested after the Mesh and Elixir comparison. The baseline is
`ab92750ea970cd6db71d5a0c884a5030905b0d4a`.

## Product contract

Durable jobs are part of Mesh's language and runtime. An application does not
install a broker, provision a queue database, or supply database credentials.
Mesh owns persistent storage, scheduling, recovery, worker execution, and queue
operations. Existing application PostgreSQL support remains independent.

The implementation must distinguish accepted durable work from transient actor
work. An acknowledgement follows durable acceptance. A crash after an external
effect can cause another attempt, so delivery is at least once. Runtime queue
transactions do not imply atomicity with an unrelated application database.

## Completion criteria

Each row needs implementation, public documentation, a compiled usage example
where applicable, focused tests, and independent review. A completed API must
perform its advertised operation. A parser-only feature or an unsupported stub
does not satisfy the row.

| Unit | Required observable behavior | State |
| --- | --- | --- |
| Runtime-owned durable jobs | Accepted jobs and arguments survive process restart without an external database | Reviewed state/store integrated; local Raft artifact checks pass; source review and lifecycle proof follow-up running |
| Queue execution | Named workers execute on actors with bounded per-queue concurrency and durable results | Actor runner implementation running; public API and durable result payloads pending |
| Queue lifecycle | Scheduling, recurring jobs, retries, uniqueness, cancellation, pause, resume, retry, discard, and inspection operate on persisted jobs | Pending |
| Queue ownership | Competing claims, stale completion, crash recovery, and cluster owner loss preserve attempt authority | Pending |
| Queue atomicity | Batch enqueue commits all jobs or none; documentation states the boundary with application databases | Pending |
| Selective receive | Unmatched application messages remain queued in order until a later matching receive | Reviewed and integrated; focused combined-branch checks pass; workspace gates pending |
| Actor stack exhaustion | An actor exhausts its stack without aborting unrelated actors or the native process | Feasibility evidence integrated; containment pending |
| Mutual tail calls | Mutually recursive functions and actor state transitions execute in bounded stack space | Lexical-scope correction has a reproduced runtime-symbol collision; integration blocked pending final correction |
| Map patterns | Partial map destructuring matches keys and typed values correctly | Implementation running in isolated worktree |
| Pinned patterns | A pinned expression compares with an existing binding rather than rebinding it | Implementation running in isolated worktree |
| Bitstring patterns | Binary construction and destructuring honor segment widths, bounds, and byte order | Pending |
| Test selection | Test names, source locations, and tags select actual test cases | Reviewed and integrated; focused integrated tests pass; workspace gates pending |
| Concurrent tests | Explicitly asynchronous cases run with bounded concurrency and isolated fixtures | Reviewed and integrated; focused integrated tests pass; workspace gates pending |
| Documentation tests | Executable documentation examples run with expected result checks | Reviewed and integrated; focused integrated tests pass; workspace gates pending |
| Coverage | Mesh source execution produces an accurate coverage report with unexecuted code visible | Pending |
| Editor references and rename | Scope-aware references and rename work through the actual LSP transport | Invalid-pattern recovery gap reproduced; third focused correction running; integration pending |
| Persistent REPL values | Reading a previous value does not rerun its initializer or effects | Implementation running in isolated worktree |
| Typed query results | Queries decode directly to declared row types and report incompatible rows | Pending |
| Transaction composition | Named dependent database operations use one connection and roll back together | Pending |
| Server-driven live UI | A compiled Mesh application renders a page, handles browser events, and sends state updates with reconnect behavior | Pending |
| User macros | User code defines and expands hygienic syntax transformations with useful diagnostics | Pending |
| Hot code replacement | Validated code versions replace callable modules while existing calls retain safe code and state ownership | Pending |

## Delivery order

1. Capture the local build baseline and choose the runtime ownership design.
2. Implement persistence and the queue state machine with restart and contention
   tests before exposing worker execution.
3. Expose durable jobs through compiled Mesh and operator commands. Prove local
   restart and cluster ownership behavior before claiming queue completion.
4. Implement actor semantics and language patterns in separate verified units.
5. Implement tooling and data APIs in isolated worktrees. Integrate changes to
   shared parser, typechecker, and code-generation registries sequentially.
6. Implement the live UI, macros, and hot replacement against explicit public
   contracts and real application tests.
7. Run independent adversarial review and the applicable authoritative workspace,
   documentation, editor, and runtime proof gates on the integrated commit.

The Whatsdown project may concurrently produce Mesh fixes in another session.
Keep this effort on its isolated branch. Before each integration stage, inspect
new shared Mesh commits and compare overlapping changes. Preserve external
fixes, reconcile behavior deliberately, and rerun the affected checks after
integration. Never rewrite another session's branch or discard its changes.

## Verification boundary

Focused tests guide each implementation. The authoritative commands are owned
by `.github/workflows/authoritative-verification.yml` and the applicable proof
workflows. Local evidence does not establish hosted CI, release, or deployment
readiness. No merge or deployment is authorized by this task.

Baseline and per-unit logs live under `.tmp/server-runtime/evidence/`. The
append-only decision record is `docs/server-runtime-decisions.tsv`.
