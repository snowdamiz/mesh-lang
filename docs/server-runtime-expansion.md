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
| Runtime-owned durable jobs | Accepted jobs and arguments survive process restart without an external database | Both reviews of 99d5b8df are clear, and the fresh Jobs suite passed 81 tests, consensus 18, and rustfmt. It rejects a stale applied membership, rebuilds an intact snapshot behind the purge watermark, and checks snapshot rename inodes. A rewritten consistent state and log remains unauthenticated, including a job table after the applied entry is purged. A quarantine rename mismatch can remove the committed snapshot name without committing a new pointer. Not combined with the runner. Not integrated |
| Queue execution | Named workers execute on actors with bounded per-queue concurrency and durable results | Cancellation correction b4911b7f is committed on t3-jobs-runner-cancel. Expiry revocation shares the attempt mutex with renewal adoption, and a worker return of 2 proposes Cancel before the next clock tick. Author tests are not the gate. OpenAI review mesh-jobs-runner-cancel-review-openai-r1, Grok review mesh-jobs-runner-cancel-review-grok-r1, and artifact mesh-jobs-runner-cancel-artifact-r1 are running. Not combined with the storage correction. Public API pending. Not integrated |
| Queue lifecycle | Scheduling, recurring jobs, retries, uniqueness, cancellation, pause, resume, retry, discard, and inspection operate on persisted jobs | Pending |
| Queue ownership | Competing claims, stale completion, crash recovery, and cluster owner loss preserve attempt authority | Pending |
| Queue atomicity | Batch enqueue commits all jobs or none; documentation states the boundary with application databases | Pending |
| Selective receive | Unmatched application messages remain queued in order until a later matching receive | Reviewed and integrated; focused combined-branch checks pass; workspace gates pending |
| Actor stack exhaustion | An actor exhausts its stack without aborting unrelated actors or the native process | Feasibility evidence integrated; containment pending |
| Mutual tail calls | Mutually recursive functions and actor state transitions execute in bounded stack space | OpenAI review of 85b4c242 is clear. Grouped local calls, closure and for bindings, and branch lets match the required rules. The Grok review and a fresh execution are still running. A let inside while stays uninitialized after the loop. The baseline loop-result cleanup panic stays outside this correction. Not integrated |
| Map patterns | Partial map destructuring matches keys and typed values correctly | Both reviews of f12bdab9 are clear on loop order, nil keys, and shared continuations, and that execution passed. Both reviews of ba17fe5b are clear, and the fresh typeck file passed 9 tests and 14 meshc probes. A rejected numeric token has no key identity. The syntax_kind count is unchanged. Not integrated |
| Pinned patterns | A pinned expression compares with an existing binding rather than rebinding it | Both reviews of f12bdab9 are clear on value-only pins, and that execution passed. Both reviews of map-key identity ba17fe5b are clear, and that fresh execution passed. Not integrated |
| Bitstring patterns | Binary construction and destructuring honor segment widths, bounds, and byte order | Pending |
| Test selection | Test names, source locations, and tags select actual test cases | Reviewed and integrated; focused integrated tests pass; workspace gates pending |
| Concurrent tests | Explicitly asynchronous cases run with bounded concurrency and isolated fixtures | Reviewed and integrated; focused integrated tests pass; workspace gates pending |
| Documentation tests | Executable documentation examples run with expected result checks | Reviewed and integrated; focused integrated tests pass; workspace gates pending |
| Coverage | Mesh source execution produces an accurate coverage report with unexecuted code visible | Pending |
| Editor references and rename | Scope-aware references and rename work through the actual LSP transport | Recovery commit c454584f passed both source reviews and independent stdio checks. The semantic stack is not on this branch. Integration waits for the current typechecker writers |
| Persistent REPL values | Reading a previous value does not rerun its initializer or effects | Both reviews of c54ac14a are clear, and the seven documented commands passed: meshc build, mesh-repl 48, e2e_repl 13, repl tests 3, actor message shapes 20, closures 4, typeck repl 1, and codegen shapes 2. It refuses nested database and channel handles before effects, and generated slot names stay outside user source. Cross-evaluation resource ownership remains unimplemented. Not integrated |
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
