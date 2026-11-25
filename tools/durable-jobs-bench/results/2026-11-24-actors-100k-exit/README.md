# 100K actor process exit

The macOS ARM64 process printed `100000 actors done` and stayed alive until
the test killed it. The archived log has no child stderr. The cause remains
open. The log lacks the time of the print, actor progress counters, and
thread stacks. It cannot distinguish unfinished actor work from a stall in
shutdown. [Workspace log](../2026-11-24-qualified-hot-pool/evidence/local/workspace-correctness.log.gz)
· [Local receipt](../2026-11-24-qualified-hot-pool/evidence/local-receipt.json)

## What the test observes

The fixture spawns each `tiny` actor, sends it one message, and continues.
`send` queues the message and can wake the recipient. The fixture never
waits for a reply or exit notification and never checks the result of a
send. The final line records that the spawn and send loops returned.
[Fixture](../../../../tests/e2e/actors_100k.mpl#L11)
· [Local send](../../../../compiler/mesh-rt/src/actor/mod.rs#L712)

`actors_100k` builds the fixture in a temporary project with the test
compiler and Cargo's debug runtime archive. The child wait starts after
spawn. The helper drains both pipes and polls for process exit every 20 ms.
The deadline is 30 seconds plus an unconditional 180-second launch allowance.
The helper kills and reaps a child still alive at that deadline. The test
checks stdout only after it sees a successful exit. In the failed run,
`status: None` means the killed child had no numeric exit code. Cargo
returned 101 for the failed suite. [Actor test](../../../../compiler/meshc/tests/e2e_actors.rs#L348)
· [Runtime build and child wait](../../../../compiler/meshc/tests/support/test_artifacts.rs#L28)

## What must finish after the print

The generated C entry point returns zero after `mesh_rt_run_scheduler`
returns. The runtime calls Jobs shutdown, marks the main actor exited,
signals the scheduler, and joins its worker threads. After the shutdown
signal, workers continue processing actor work. Finalization delivers
exit notifications, removes the actor, and decrements the active count.
Workers can exit once the active count reaches zero. The archived output
does not locate the main thread or workers along that path.
[Generated entry point](../../../../compiler/mesh-codegen/src/codegen/mod.rs#L1337)
· [Runtime shutdown](../../../../compiler/mesh-rt/src/actor/mod.rs#L1386)
· [Worker shutdown](../../../../compiler/mesh-rt/src/actor/scheduler.rs#L1836)
· [Finalization and active count](../../../../compiler/mesh-rt/src/actor/scheduler.rs#L2224)

The diff from pre-pool `6d3c3dad4ed5e0a57735e56703d8c38507f53cb1` to
`1e70f18d624a841740d3bbb3d0fd09b269257f68` changes PostgreSQL pool checkout
and wire handling in `pool.rs` and `pg.rs`, plus the database docs. The
fixture uses no PostgreSQL APIs. The actor runtime, code generation, fixture,
and test helper match byte for byte. This rules out a direct change to
the shutdown path in the pool patch. It leaves the observed timeout
unattributed.

## Small shutdown checks already passed

The same workspace run passed
`shutdown_receive_waits_for_a_linked_exit_before_its_worker_claims_it`.
That test keeps the receiver waiting through shutdown, then delivers the
pending linked exit or monitor notification and checks that the active
count reaches zero. The same run passed
`shutdown_waits_when_it_cannot_check_a_process_state`, which checks that a
locked actor prevents a quiescence decision. The workspace continuation
passed `e2e_service_call_pending_at_exit_stops_the_caller`. These are
correctness results for small cases. The spawning benchmark was not rerun.
[Scheduler tests](../../../../compiler/mesh-rt/src/actor/scheduler.rs#L2908)
· [Pending service call test](../../../../compiler/meshc/tests/e2e_concurrency_stdlib.rs#L261)
· [Continuation log](../2026-11-24-qualified-hot-pool/evidence/local/workspace-remainder.log.gz)

## Linode booking status

The matrix implementation operator is the sole native controller. The local
queue ledger at 00:05 UTC on 2026-11-25 showed earlier bookings and ready
requests, with no availability posted for this proof. No request was
registered.

The released standalone proof copied the exact-source Linux debug compiler
out of its build tree. It recorded SHA-256
`6b3e0920079aaff7120054a61d5c37e6e6a6c1427caa8fc2f69dfe3ce42a7e0a`.
The launcher cleaned the Cargo build tree, including its runtime archive.
The Jobs proof retained linked fixture executables and the SHA-256 of its
release runtime. It also cleaned the tree holding that archive. The
already queued logger-off relink rebuilds the release archive if absent and
cleans its own Cargo build tree on exit. The retained records offer no
linkable, exact debug runtime for the 100K fixture. A source-matched run
would require a new runtime build. A matched pre-pool comparison would
require a second runtime identity. The build work and existing bookings
make a separate Linux run disproportionate for the failure observed on macOS.
[Standalone evidence](../2026-11-24-qualified-hot-pool/evidence/standalone/README.md)
· [Jobs evidence](../2026-11-24-qualified-hot-pool/evidence/jobs/README.md)
· [Proof launcher](../2026-11-24-e4156740-trusted-jobs-native/native.sh#L30)
· [Queued relink launcher](../2026-11-24-dense-individual-hot-pool/logger-off-retry/setup.sh#L30)

If an exact runtime becomes available, an isolated Linode run could answer
whether the current source also times out on Linux. It would retain the
unmodified fixture, the original 210-second child deadline, the actual
wait status, both output streams, and the build and binary hashes. On
timeout, the original decision and kill would remain the test result. A
separate attempt could latch that same deadline decision before collecting
bounded thread stacks off the clock. Worker progress across samples would
support a backlog diagnosis. Repeated stacks at the same shutdown point
without progress would support a stall diagnosis. A Linux result alone
would leave the original macOS failure unexplained.
