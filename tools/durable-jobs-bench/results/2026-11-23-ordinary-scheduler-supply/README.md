# Ordinary V1 scheduler supply

The local screen kept two scheduler workers per runner. Four and eight workers increased runner CPU per job in every rotation. Dense at eight workers lost submit-to-settlement throughput in all five rotations. The experiment leaves the native two-CPU effect unresolved and did not advance to a Linode diagnostic or a runtime compensation prototype. [Recorded rollup](evidence/original-rollup-stdout.txt)

| Profile | Workers per runner | Submit throughput | Durable throughput | Runner CPU/job |
|---|---:|---:|---:|---:|
| Short | 4 | −21.1% | −21.6% | +14.1% |
| Short | 8 | −12.2% | −10.6% | +30.0% |
| Dense | 4 | −2.0% | −4.2% | +22.8% |
| Dense | 8 | −25.1% | −11.3% | +70.3% |

Each value is the median paired change against the mean of two fixed-two controls in the same rotation. Submit throughput begins at the producer call. Durable throughput begins at enqueue commit. Both end at the final durable settlement. CPU includes runner startup, idle, work, and shutdown. [Receipt with ranges and win counts](evidence/performance-receipt.json)

The five short rotations preserved one runner, 16 slots, eight application connections, and a PostgreSQL cap of 20. The five dense rotations preserved eight runners, eight slots and four application connections per runner, and a cap of 96. Each rotation used the same pinned ordinary executable on Apple Silicon. PostgreSQL 17.11 used verified TLS and synchronous durability. The local executable linked the Cargo dev runtime, and one-minute host load ranged from 12.4 to 65.2 on 15 logical CPUs. Peak runner threads across each profile were 9/11/15 and 64/80/112 for two/four/eight workers. The submit throughput of the duplicate stock controls varied by −31% to +83% in short and −53% to +17% in dense. [Input pins](replay/pins.json) · [Recorded rollup](evidence/original-rollup-stdout.txt)

## Recovery after scratch deletion

The November 23 run reported 40 reconciled arms, 23,040 jobs, and 46,080 application commits. The transcript retains all 40 exit records and the final rollup as printed. It retains individual rates for the first short group and first two dense groups, plus per-group paired changes for the first three short and first four dense groups. The remaining individual values and additional metrics in the original `local-v1-summary.json` are unavailable. The compact JSON receipt preserves the printed precision and omits unavailable values. [Dispatch log recovered from overlapping excerpts](evidence/local-v1-dispatch.txt) · [Compact receipt](evidence/performance-receipt.json)

The original 1,359-file seal was verified before deletion. Its `SHA256SUMS` identity was `5e8a3f3638dca7b221ce26f9ed4bbaad4d799e2420960784bcf6ae921411dd78`. The original manifest, full job and transaction ledgers, full rollup JSON, SQL-counter exports, and complete trace files remain unavailable. The archive's separate checksum manifest covers the recovered files. [Original seal console receipt](original-seal.txt) · [Recovery checksums](SHA256SUMS)

The retained short-arm excerpt shows the first readiness check's rejection. The runner printed `READY` while two queue connections were still opening. Both idle samples recorded all 13 connections before submission, and the runner log shows 128 completed jobs and a notification-triggered scan. The reducer was corrected to check the warmed topology, and the original arm passed replay. [Initial rejection](evidence/early-replay-error.txt) · [Arm samples](evidence/short04-workers4-records.stdout.txt) · [Runner log](evidence/short04-workers4-runner-0.log)

Two PostgreSQL activity excerpts each show three backends with an active attempt `INSERT`. In the eight-worker snapshot all three were waiting on `WALWrite`. Stack excerpts from two later captures show `mesh_pool_execute` above blocking receive calls. The original trace reconciliation records both initial sample failures and both successful retries. These selected excerpts support those observations. Checking job identities, transaction order, and the full measurement window again requires the deleted ledgers. [PostgreSQL excerpts](evidence/pg-activity-excerpts.txt) · [Stock stacks](evidence/stock-runner-stack-excerpts.txt) · [Eight-worker stacks](evidence/eight-runner-stack-excerpts.txt) · [Trace reconciliation stdout](evidence/profile-reconciliation.json)

## Transcript provenance

Recovery used Codex session `01a20a42-4d4a-7830-96ac-5983826aff29`. Line numbers refer to its original JSONL rollout dated November 23, 2026.

| Rollout lines | Recovered material |
|---|---|
| 90, 96, 103, 129, 153, 321, 384 | Pinned input digests and replay source writes and edits. The source archive and executable survive and the pins recheck independently. |
| 162, 275, 303, 324, 352 | Dispatch records and completion marker. Lines 162, 268, and 303 also preserve early group statistics. |
| 303, 310 | First replay rejection, short-arm row and resource observations, and the runner log. |
| 357 | Full console projection of the five-group aggregate receipt. |
| 380, 393, 408, 415, 423, 430, 437 | Initial profiler failures, selected live database and stack records, offline test tail, and trace reconciliation. |
| 437 | Successful original checksum verification and original manifest digest. |

Archive verification and instructions for a fresh local run are in the replay guide. [Replay guide](REPLAY.md)
