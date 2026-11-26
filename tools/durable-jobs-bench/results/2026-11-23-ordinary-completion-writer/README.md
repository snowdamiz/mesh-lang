# Ordinary trusted Mesh Jobs completion writer

The zero-hold writer led the local short screen. In five rotations of 128 jobs, its median paired enqueue-commit-to-settlement rate was 35.5% above writer off and 32.5% above the duplicate writer-off control. It won four and five of those pairs, respectively. The 1 ms hold cut terminal transactions further and drained more slowly than zero hold in four of five rotations. The controller packet lists zero hold as the primary native candidate and 1 ms as the second mode. [Matched runs](evidence/rotations-r1-matched/summary.json) · [Controller packet](controller-plan.json)

## Frozen short results

Each row gives the median across five runs and the observed range. Each block used the frozen short workload, one compiled Mesh runner, 16 slots, eight application sessions, and two scheduler workers. PostgreSQL 17.11 ran with verified TLS, synchronous commit, fsync, and a 20-connection cap on the macOS host. The timing arms used the stock, uninstrumented PostgreSQL schema and the same compiled handler logic. [Run order and per-arm reconciliation](evidence/rotations-r1-matched/summary.json)

| Arm | Enqueue commit to settlement, jobs/s | Submit to settlement, jobs/s | Terminal XIDs per 128 jobs | Runner SQL calls/job |
| --- | ---: | ---: | ---: | ---: |
| Original V2 binary | 1,695 (680 to 2,129) | 1,103 (448 to 1,237) | 126 (119 to 128) | 1.64 |
| Same image, writer off | 2,095 (1,439 to 2,847) | 1,335 (981 to 1,864) | 127 (116 to 128) | 1.57 |
| Writer, zero hold | 3,114 (2,011 to 3,742) | 1,783 (1,195 to 2,036) | 67 (58 to 83) | 1.59 |
| Writer, 1 ms hold | 2,843 (1,950 to 3,162) | 1,520 (693 to 1,955) | 27 (24 to 29) | 0.96 |
| Same image, writer off again | 1,808 (1,520 to 2,825) | 1,101 (996 to 1,781) | 124 (112 to 128) | 1.55 |

The duplicate off arms differed by as much as 58% in the commit window. Zero hold's paired gain over writer off ranged from −0.2% to +101.7%. Its full-submit median paired gain was 22.9% over writer off and 32.3% over duplicate off, with three and five wins, respectively. Full-submit includes producer startup and enqueue. The earlier pinned native medians were 773 jobs/s for River and 335 for Oban in the commit window, and 715 and 298 jobs/s in the full-submit window. The candidate has macOS results and awaits a matched native peer result. [Frozen native peer data](../2026-11-23-ordinary-trusted-v2/evidence/ordinary-trusted-v2-short/summary.json)

## Completion and connection evidence

V2 checks the dispatcher attempt list for already reaped successes. It runs completion SQL inline before it consumes a batch claim reply. The private writer moves the existing lifecycle session to one writer thread for one SQL request. The dispatcher can consume confirmed reservations during that request. The local attempt and PostgreSQL execution cap continue to account for slots. A returned receipt restores exclusive session ownership to the dispatcher. `JobSession::with` discards a broken or non-idle connection before reuse. [Source patch](replay/completion-writer.patch)

The writer submits up to eight returned successes. A singleton, retries, and skipped cohort members use scalar SQL. The collection deadline starts at handler return and respects the earliest relevant renewal and lease deadline. Runner or queue stop ends collection early, as does filling the group or having no worker left that can complete. A due renewal takes the available lifecycle session before a new cohort. A receipt already in flight must return or fail before the dispatcher can reuse that session. Uncertain completions keep their original tokens and retry scalar settlement. If PostgreSQL stays unreachable, `Jobs.stop` returns its grace report and `Jobs.join` waits for the token to settle or become verifiably stale. [Source patch](replay/completion-writer.patch)

All 3,200 measured jobs had two independent application XIDs per attempt, separate from the enqueue and settlement XIDs. Each arm's successful terminal statement rows summed to 128. The committed-transition diagnostic verified exactly one `executing` to `completed` update per member, proved that each writer group followed its actual handler returns, and measured a peak of 16 committed executing jobs. Each arm observed 13 TLS backends, including four queue-role, eight application-role, and one producer backend. The held-receipt proof also measured identical backend IDs before and during the handoff in its five-connection fixture. [Diagnostic runs](evidence/diagnostic-r1/summary.json) · [Held receipt](evidence/writer-faults-r3/held-middle/result.json)

In the instrumented run, zero hold formed 51 terminal transactions. The 1 ms mode formed 21, including eleven eight-member groups. Median age from the oldest handler return to handoff was 329 µs for zero hold and 1,027 µs with the 1 ms hold. Median handoff-to-writer time was 6 µs and 7 µs. The largest return-to-handoff ages were 3.7 ms and 2.6 ms, respectively. The deadline bounds deliberate collection. Dispatcher scheduling and outstanding SQL can make observed age longer. The zero-hold screen roughly halved terminal transactions while total queue SQL calls stayed close to the control. The 1 ms mode held slots longer after handler return and reduced total queue SQL. [Cohort distributions and timing](evidence/diagnostic-r1/summary.json) · [Scalar comparator](evidence/scalar-diagnostic/summary.json)

## Fault evidence

- A TLS relay held `ReadyForQuery` after a durable scalar completion. The dispatcher launched three confirmed peers while it held the first completion's slot. A fifth job stayed unlaunched until confirmation. Two peers settled together while the middle job's row was locked. [Held middle result](evidence/writer-faults-r3/held-middle/result.json)
- The runner stopped admission during a withheld receipt and exited after confirmation. A held member retained its local slot past lease expiry and the original token could no longer complete the expired row. [Stop](evidence/writer-faults-r3/stop/result.json) · [Expired middle](evidence/writer-faults-r3/expired-middle/result.json)
- Dropping the idle receipt for a committed success cohort produced three `ack_uncertain` events and three scalar `ack_stale` confirmations. All three completed handlers ran once. Dropping the idle receipt for a committed claim launched zero handlers for those grants. Recovery then ran each at attempt two. [Ambiguous completion](evidence/writer-faults-r3/ambiguous-completion/result.json) · [Ambiguous claim](evidence/writer-faults-r3/ambiguous-claim/result.json)
- A due renewal committed before the ready cohort. An operator-cancelled member settled through scalar SQL while its peers shared a success XID. The compiled lifecycle proof also covered retry, cancellation during an application write, timeout, crash before and after the effect, and stale-token replay. [Renewal](evidence/writer-faults-r4/renew-priority/result.json) · [Cancellation](evidence/writer-faults-r4/cancelled-member/result.json) · [Lifecycle](evidence/lifecycle-faults-r1/cases.json)

## Repository checks

The private snapshot passed the workspace build, 2,018 core workspace tests, 1,600 runtime tests, and the PostgreSQL Jobs gate. The PostgreSQL gate passed 38 protocol tests, 71 runtime Jobs tests, and 3 compiled end-to-end tests. [Gate receipt](gate-receipt.json)

[Replay and raw evidence](REPLAY.md)
