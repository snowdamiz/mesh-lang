# Bulk V2 with the exit-confirmed completion writer

The controller packet prices the writer in a single six-arm native comparison. It carries forward the exact previous bulk V2 lane and the same River, Oban, and Apalis inventories. The two candidate arms use one runner ELF and change only the writer switch. Both keep the existing bulk V2 broker and migrator at manifest `7f228c19`. [Controller packet](controller-packet.json) · [Offline plan verification](evidence/offline-plan-check.json) · [Prior matched bulk series](../../2026-11-23-bulk-enqueue-v2-short/README.md)

The frozen Python driver assigns independent subprocess entry points to the runner and broker. The split adapter changes only `Subject.run`, where it substitutes a checked executable on a copy of the subject. The original subject still calls `migrate` and owns the warm `session` broker. The native replay checks both the original broker SHA and the new runner SHA, including the latter in the inventory's verified file map. Ledger artifact identity still identifies the broker and migrator. [Split adapter](replay/split_arm.py) · [Benchmark evidence checks](replay/proof.py)

## Actor teardown

V2 sends the result from the actor entry closure. The public runtime at `db5937e7` sends it from the scheduler's callback after stack unwind and exit cleanup. The combined runner takes the exact public scheduler and stack source and preserves its exit-confirmed publication and deadline checks. The writer's timing metadata still records when the handler returned. The PostgreSQL queue path is the V2 benchmark path. This comparison qualifies the teardown order shared with the public runtime. [Release-exit source patch](replay/release-exit.patch)

The probe registers a real native terminate callback from the compiled handler. PostgreSQL has already committed both independent application writes when the callback enters. A local latch holds the callback until the observer records the job's state. V2 and the original r1 zero-hold writer reached durable completion while the callback was held. The public product and both modes of the combined runner kept the job executing until the callback returned. Every case reached the completed state with the original pair of application XIDs. [V2](evidence/teardown/v2/teardown.json) · [Frozen r1](evidence/teardown/r1/teardown.json) · [Public product](evidence/teardown/product/teardown.json) · [Combined off](evidence/teardown/off/teardown.json) · [Combined zero](evidence/teardown/zero/teardown.json)

## Local proof and native gates

Both local split smokes reconciled 128 jobs, one bulk enqueue, 256 independent application XIDs, eight warmed application sessions, and a peak of 13 physical connections under the 20-connection cap. They recorded 125 terminal XIDs with the writer off and 79 at zero hold. These are compatibility smokes. The ready-time query can see two, three, or four queue-role connections while the existing claim and listener connections finish opening. It checks all eight application sessions and the broker at readiness, and the run checks the 13-session peak. [Smoke and fault receipts](evidence/qualification.json)

The split setup passed a locked middle member, lease expiry under lock, renewal priority, cancellation, stop, and ambiguous claim and completion receipts. These proofs use the same candidate runner as timing and the rebuilt local bulk V2 broker. The controller's single native SETUP checks the existing Linux bulk ELF and manifest, rebuilds only the candidate runtime, and runs the same fault proofs and both off/zero smokes. A witness binary links against that same runtime for the native teardown check. [Split faults](evidence/split-faults.log) · [Native SETUP](replay/native-setup.sh)

The controller can reuse an unstarted r1 reservation. The startup and timing commands are pinned in the packet. Timing begins with two opposite orders in one series, then uses the same inventory for the remaining three groups. The exact bulk V2 lane measures the aggregate change from the prior result. The same-ELF off and zero arms isolate the writer. [Timing plan and reducer](replay/timing.py)

The workspace build passed. The PostgreSQL Jobs gate passed 38 protocol tests, 72 runtime Jobs tests, and three compiled end-to-end tests. The separate scheduler Drop test verified stack unwind and registration cleanup before exit notification on forced termination. [Gate receipt](evidence/gate-receipt.json)

## Replay the teardown witness

Run `replay/teardown-fixture.py SOURCE OUT` to copy the benchmark fixture and add the gated callback. Link that copy using its `replay/link.sh`, a pinned Mesh compiler, and the release runtime you want to check. Set `COMPLETION_WRITER_SOURCE_SHA256` to the release-exit patch digest in the controller packet before linking fixtures from the combined source. Set `COMBINED_BULK_SOURCE` to the archived bulk V2 source so the proof loads the original PostgreSQL adapter. Run `replay/teardown.py EVIDENCE BROKER RUNNER MODE` with the compiled bulk V2 broker, the witness runner, and one of `v2`, `r1`, `product`, `off`, or `zero`. The controller packet contains the exact native invocation and file pins. [Witness fixture](replay/teardown-fixture.py) · [Witness assertions](replay/teardown.py)

[Witness source and build digests](evidence/teardown-source.json)

[Local raw evidence archive](local-evidence.tar.gz)
