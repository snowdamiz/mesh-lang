# Late capacity for the public dense eight-runner Jobs lane

The guarded test on public source `3c4032d06` and PostgreSQL 17.11 found one two-seat request waiting on the actual d388 queue row lock. The grouped writer applied three more confirmed completions while the same PostgreSQL backend remained blocked. Stock granted two replacements. The opt-in candidate granted five under the same conditions. The local controls use deterministic exit signals and the production dispatcher, acknowledgement writer, transaction and adoption paths. They exercise ordering and capacity only.

The candidate is local commit `456749c4c675043a3b800e9674a4de4bca430806` on the exact public source. The compressed patch reproduces its source tree. The Linode SETUP released exit 0 at 06:39:29 UTC. Independent replay verified
all eleven compiled cases and seven guarded widths on candidate ELF
`839c4f32eb4bff7559a3d8f76d959f8ae4e0469b86a2db69fca4cc1723c26c1b`.
The timing request binds that ELF and the original grouped-hot bridge.
[Native proof](evidence/native-proof/verified.json) ·
[Bound pilot](evidence/bound-pilot/controller-packet.json)

## Canonical evidence

Both archived measured B arms are on ELF `4924b5880247059032786f43f490cf99bde8a0a69adc790994ee01a51ab91058`, the d388 bounded SQL body, eight 8-slot runners, two scheduler workers each and 32 warmed application connections. Both reconcile 1,024 completions, 2,048 distinct application-write XIDs and one producer transaction. The two measured arms issued 385 bounded claim calls each, of which 255 and 254 yielded work.

| Assumed receipt lag after durable settlement | First rotation, disjoint pairs | Second rotation, disjoint pairs |
| --- | ---: | ---: |
| 0 ms | 106 | 112 |
| 2 ms | 102 | 105 |
| 4 ms | 94 | 101 |
| 8 ms | 82 | 91 |
| 16 ms | 44 | 47 |

The cutoff is four milliseconds before each first grant stamp. The replay groups grants by runner and database stamp, keeps preceding grants charged and credits a terminal commit at the specified lag. It counts disjoint adjacent pairs that fit all jobs in eight seats. At zero lag, the earliest maximum-count selection weights 229 and 250 normalized slot-ms from first grant stamp to second grant stamp. Selecting weight among maximum-count solutions yields 234 and 254. The weight-first selection yields 263 and 277 at 95 and 102 pairs. The normalization divides total repeated seat-time by 64. These are modeled opportunity sizes. The logs retain neither actual receipt times nor request widths. The calculation also assumes sufficient eligible work if the grant order changes. The marginal SQL work and time holding the row across the additional client handoff require native measurement.

## Transaction and local admission

- For a partial grouped request, the candidate pipelines a parameterized queue lock with the existing READ COMMITTED BEGIN exchange. It validates BEGIN, SET LOCAL, row-lock result and transaction status. The same bounded d388 SELECT, grant decoder, COMMIT receipt and adoption path follow. The original SQL retains its namespace try-lock, queue state, global cap and fresh snapshot checks. The pilot retains the original fixed 8 ms completion collection policy.
- The grouped writer credits the pending request only when it applies a completed receipt and removes the tracked attempt under the state lock. The claimant takes a nonblocking state snapshot after the PostgreSQL lock receipt and checks current local phase, local limit and process cap. The reservation can grow by at most its confirmed credits. A contended local mutex keeps the original reservation. The dispatcher can continue polling receipts and renewals while the claim waits. The claimant holds no local mutex across PostgreSQL I/O.
- The prelock uses the same queue row and 100 ms lock bound. A missing row or acquired lock with zero safe seats rolls back. A wire failure retires suspect connection state. The checked grant and commit contracts continue to fence all adoption. The exact-prefix cursor fallback uses the width already reserved. Late local control still goes through the final phase and cap check before launch. The stock worker role already has the privileges used by the existing invoker grant.
- Two stages with four request exchanges would require at least 97 removed bounded calls per arm just for exchange-count parity. Pipelining preserves three exchanges. The serialized lock still spans one client handoff and a second lock lookup inside d388. The candidate can lose on this cost even when some requests grow.

A test failure caused by PostgreSQL's transaction-local `pg_stat_activity` snapshot was kept. The successful rerun clears the snapshot at each observation. A subsequent stock
rerun exposed two separate claim transactions stamped in the same millisecond.
The successor fixture identifies the first replacement transaction by its
PostgreSQL `xmin` while the replacement rows are still executing. The first
native request remains held and its bundle is retained under `native-ready`.
The first xmin bundle under `native-xid` is also held. The fixture-complete
successor adds the five tracked crypto vectors and the scoped Jobs SQL fixture.
All 692 existing files in the xmin bundle are byte-identical. The extracted
capsule verifies all 16 literal include sites against 11 tracked files and
builds the runtime library test harness on Darwin. The final guard covers baseline, growth, local and PostgreSQL cap shrink, local and namespace pause, and stop. The native proof additionally requires the existing wire cut, acknowledgement loss, cancellation, mixed SQL and worker teardown cases.

## Local PostgreSQL closeout

The original authoritative PostgreSQL 17.11 invocation passed the scoped roles,
Rust Jobs, and compiled Jobs suites and then exited 1 on line 242 of the
inherited claim-admission proof script. The separate verification worktree
used the sealed runtime at `456749c4c675` with the exact three-line script fix
from `4f50dbcef`. That invocation exited 0, including all 30 checked claim
fault cases and all seven set-based enqueue fault cases. The raw logs remain
in `target/late-capacity`. The original and corrected receipts pin each log
and script. The fixture-complete native SETUP request retains SHA-256
`831c20d8b823e0be3f28f87bb266046bb5c5fb30d0518f6b74f31a2afb4fd939`.
[Original receipt](local-postgres-original.json) · [Corrected receipt](local-postgres-closeout.json)

## Released Linode pilot

Both opposed rotations passed the seal at 06:46:03 UTC. All 16 arms
reconciled. Against the faster same-image off control, B saved 150 and
188 ms on submit and 146 and 195 ms on drain. Against the fresh exact
public bridge it saved 229 and 130 ms on submit, and 205 and 135 ms on
drain. It finished with 139 and 137 positive grants, with 133 and 129
requests receiving at least one additional confirmed seat. All Mesh
arms peaked at 65 PostgreSQL connections. The original CPU, topology,
XID and wire evidence is in the released pilot archive. A distinct
[matched peer packet](peer-comparison/controller-packet.json) uses five
fresh rotations and the original duplicate-spread decision.
[Released pilot replay](evidence/pilot/independent-replay.json)

## Current bridge and matched peers

The released matched peer rotations at `893710708` set the current price. All 12 measured raw hashes were independently checked against the replay.

| Peer | Rotation 1 Mesh minus peer submit / drain | Rotation 2 Mesh minus peer submit / drain |
| --- | ---: | ---: |
| River | +225 / +140 ms | +222 / +124 ms |
| Oban | −82 / −117 ms | −208 / −234 ms |
| Apalis | +638 / +495 ms | +518 / +378 ms |

The sealed peer scorer ruled River and Oban inconclusive and Apalis ahead. Matching River's clocks requires both gaps in each rotation. Exceeding the historical comparison margins requires about 289/199 ms and 494/392 ms submit/drain savings, including the wide second-rotation duplicate spread. The candidate pilot prices against fresh same-image duplicates and the original grouped-hot reference. This packet supports a native viability test for the current bridge. It establishes no peer win.

## Replay and controller handoff

Run these from the worktree root.

```sh
d=tools/durable-jobs-bench/results/2026-11-25-public-dense-late-capacity
python3 -B "$d/replay.py" | cmp - "$d/replay.json"
python3 -B "$d/peer_price.py" | cmp - "$d/peer-price.json"
python3 -B "$d/offline_check.py"
python3 -B "$d/pilot.py" check-plan
python3 -B "$d/qualification.py" check target/late-capacity/source target/late-capacity/native-xid-fixtures
python3 -B "$d/qualification.py" extract target/late-capacity/source target/late-capacity/native-xid-fixtures \
  --extracted target/late-capacity/reextract | cmp - "$d/capsule-dependencies.json"
```

The native bundle is under `target/late-capacity/native-xid-fixtures`. `setup-seal.json` pins its source archive, native script and registration bytes. The native controller registers the byte-exact `jobs.json` and runs the setup behind existing bookings. After a successful release, `pilot.py bind` records the exact manifest and produces the separate timing registration. The proof archive, controller release and staging pins are mandatory. The current grouped-hot reference remains on the qualified public ELF.

```sh
python3 -B "$d/controller.py" setup CONTROLLER_STATE target/late-capacity/source target/late-capacity/native-xid-fixtures
python3 -B "$d/pilot.py" bind target/late-capacity/source target/late-capacity/native-xid-fixtures target/late-capacity/bound
python3 -B "$d/controller.py" timing CONTROLLER_STATE target/late-capacity/source target/late-capacity/native-xid-fixtures target/late-capacity/bound
python3 -B "$d/pilot.py" replay target/late-capacity/bound target/late-capacity/bound/evidence
```

The timing plan has two mirrored, fresh groups with a warmup per arm. A1 and A2 share one candidate image with the option off. B uses the same image with the option on. The fourth arm is the exact stock grouped-hot reference. Gate both clocks and positive grants beyond 5% and duplicate spread, advance last effect, reduce bounded calls, keep resource usage within the recorded envelope and witness real confirmed top-ups. Score both groups and retain complete exports on failure. Any performance result is conditional on a released native run through the sole controller.
