# Dense logical-key refill attribution

Archive the refill follow-up. Reserving later has little room to remove grants on the released timeline. Coalescing small requests would require a source of confirmed exits that the timing arms do not establish. The logical SQL gained 155–163 ms across the two clocks and two rotations. Runner lifetime CPU still exceeded the higher stock duplicate in both rotations, by 18.580 and 2.878 CPU-ms. The original decision keeps the peer gate closed. [Released pilot](../2026-11-25-dense-logical-grant-key-native/NATIVE-RESULTS.md) · [Raw replay](replay.json)

## Which transactions increased

The three-arm blocks used opposing orders, A/key/B and B/key/A. All eight arms, including the excluded plan pair, completed 1,024 jobs with a separate durable enqueue transaction and 2,048 distinct application-write XIDs. The two blocks used the same grouped-hot ELF as the original stock bridge, two 32-slot runners, 32 warm application TLS sessions and two pinned CPUs. [Pinned packet](../2026-11-25-dense-logical-grant-key-native/controller-packet.json)

| Rotation | Arm | Workload grants | Positive | Empty | Width 1–4 | Width 16 | Mean positive width | Outside workload grants | Priming idle runner SQL calls |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|
| 1 | Stock A | 93 | 83 | 10 | 4 | 34 | 12.34 | 26 | 136 |
| 1 | Logical key | 143 | 131 | 12 | 44 | 21 | 7.82 | 26 | 136 |
| 1 | Stock B | 88 | 82 | 6 | 3 | 36 | 12.49 | 26 | 136 |
| 2 | Stock B | 87 | 78 | 9 | 0 | 33 | 13.13 | 25 | 131 |
| 2 | Logical key | 154 | 141 | 13 | 65 | 24 | 7.26 | 26 | 136 |
| 2 | Stock A | 87 | 79 | 8 | 2 | 38 | 12.96 | 26 | 136 |

The workload delta starts at the observer sample after idle priming, before producer submission. It ends at the sample after durable settlement. The raw idle counter covers all runner SQL during priming. The lifetime grant count minus the workload delta includes startup and priming and could include calls after the final sample. The timing archives lack the individual idle grant snapshots. The separate diagnostic pair has 5 grant calls before priming and 26 after priming in each arm. Its workload counts reconcile to 84 positive and 9 empty for stock and 108 positive and 12 empty for the key. Those arms qualify the stock scan and replacement point-key plans. Their clocks are excluded. [Observer counters](../../../../scripts/jobs-parity.py#L964) · [Original plan and gate replay](../2026-11-25-dense-logical-grant-key-native/evidence/native/final-replay.json)

Each successful set grant stamps all of its jobs with one millisecond value. Each distinct runner and claim stamp therefore requires at least one positive call. The calls return one row per granted job or one row for an empty or fallback marker. In each arm, the positive bucket count plus the excess rows over 1,024 exactly equals total grant calls. This accounts for every call, including same-runner timestamp collisions, which would have required an additional call. Each marker has a corresponding next-due probe and the SQL census contains no cursor fallback or rollback. Every recorded grant has a matching BEGIN and COMMIT. There are no observed failed claims or zero-row claim calls, and both runners report a clean drain. The width is the number of jobs returned. The original reservation argument and the reason a positive grant was smaller than 16 are absent from these captures. [Pinned SQL](../2026-11-25-dense-logical-grant-key-native/candidate.sql) · [Reconciliation and full width histograms](replay.json)

Compared with the two stocks, rotation 1 added 48–49 positive grants and 2–6 empty grants. Rotation 2 added 62–63 positive grants and 4–5 empty grants. Every additional claim transaction adds three top-level runner statements. Terminal traffic also shifted toward individual writes.

| Rotation | Arm | Scalar terminal transactions | Grouped terminal transactions | Terminal XIDs | Workload runner SQL calls |
|---|---|---:|---:|---:|---:|
| 1 | Stock A | 759 | 105 | 864 | 1,173 |
| 1 | Logical key | 817 | 75 | 892 | 1,353 |
| 1 | Stock B | 765 | 97 | 862 | 1,156 |
| 2 | Stock B | 758 | 106 | 864 | 1,159 |
| 2 | Logical key | 801 | 79 | 880 | 1,375 |
| 2 | Stock A | 789 | 88 | 877 | 1,168 |

In rotation 1 the extra grant triplets account for 150–165 of the 180–197 extra runner statements. Terminal transactions account for another 28–30, with two more statements outside those categories. In rotation 2 the grant triplets account for 201 of the 207–216 extra statements. Terminal transactions add 3–16 and the remainder ranges from −1 to +3. This attributes the extra transactions and statements. It gives no per-transaction runner CPU attribution. Runner CPU includes startup, idle, and teardown. PostgreSQL statement elapsed time includes waits, and the separate PostgreSQL CPU bracket has endpoint slack. The same key arms were at least 270 and 310 CPU-ms lower on that PostgreSQL bracket. [Counts and CPU readings](replay.json)

## What a later reservation could buy

At public `b3557273d`, the dispatcher polls and settles attempts, then `BatchLane::request` reserves against current local state. The split lane receives a fixed slot count and submits that count in the checked transaction. A later refill could grow the existing reservation under the same state lock if the dispatcher had removed more attempts while the request was in flight. The pending reservation would still have to fence a second request and remain charged until grant adoption or resolution. A local pause, scale, or stop can race that handoff. The checked COMMIT receipt still governs launch. An actor's exit or a terminal commit timestamp alone cannot supply confirmed local capacity. The pinned source paths are `compiler/mesh-rt/src/jobs/runner.rs` and `compiler/mesh-rt/src/jobs/store.rs` at that revision.

The replay keeps each job on its original runner. It can advance jobs into earlier observed grant turns and can skip a turn once those jobs are already assigned. It searches for the fewest positive turns at 16 jobs per grant and 32 occupied slots per runner, with no job granted later than its observed bucket. Within a tied bucket it credits earlier releases first. The later-reservation case credits terminal records up to the *end* of each grant millisecond and removes receipt and submission latency entirely. That gives late sizing room even for commits that could only have occurred after submission. The model keeps the recorded terminal timeline and grants demand in hindsight. It prices that fixed timeline and cannot predict a new one.

| Rotation | Arm | Actual positive calls | Later sizing, modeled calls | Coalescing within 4 ms, modeled calls | Coalescing with unrestricted advance, modeled calls |
|---|---|---:|---:|---:|---:|
| 1 | Stock A | 83 | 83 | 83 | 83 |
| 1 | Logical key | 131 | 125 | 117 | 107 |
| 1 | Stock B | 82 | 82 | 81 | 81 |
| 2 | Stock B | 78 | 78 | 78 | 77 |
| 2 | Logical key | 141 | 137 | 123 | 108 |
| 2 | Stock A | 79 | 79 | 79 | 77 |

With unrestricted advance and credit through the end of the millisecond, later sizing removes six and four positive grants, worth 18 and 12 top-level statements. Using the start of the millisecond removes only one grant in each rotation. Even the larger count would need net savings of 3.097 and 0.719 runner CPU-ms per removed grant to meet the two stock ceilings. The captures have no reservation or receipt timestamps to establish a lower bound above zero.

The coalescing columns give immediate release at the second application commit record and allow any grant width to grow. That assumes free confirmation ahead of the actual second-write receipt, actor exit, terminal statement and terminal reply. The 4 ms window refers to advancing later claims into earlier grant stamps, with no added pickup delay in the model. Even this favors the candidate by deleting 14 and 18 calls at 4 ms, or 24 and 33 with unrestricted advance. At 4 ms, meeting the runner gate would require about 1.33 and 0.16 net CPU-ms per removed grant, before any extra confirmation work. The ledgers contain no exit-ready timestamps to show that the dispatcher could actually settle those jobs earlier. The existing loop already groups eligible exit-confirmed successes and tries scalar settlement before requesting another grant. Another sweep would need to find exits missed by that pass and pay their terminal I/O before claiming. Reserving slots on speculation would change the capacity and lost-reply contract. [Model output and both clock edges](replay.json)

## Prior experiments and disposition

The short gated refill added a bounded wait to collect more local slots. It reduced claims and missed the elapsed-time gate. The dense grouped-refill overlay changes when a separate terminal writer flushes, including at refill demand. The early-launch experiment moves adoption of a confirmed grant before dispatcher receipt. The claim-ahead advance experiment in `28b36ad4a` overlaps one claim with serial launch, with `0efa5c59b` conditioning commit on completed launches. The single-dispatch experiment changes runner topology. A timer, extra terminal writer or adoption change would repeat one of those mechanisms. Later reservation is distinct, but the available fixed-timeline gain is too small to justify an experiment against the measured CPU deficit. The coalescing model depends on confirmation the archives cannot establish. [Short gated refill](../2026-11-23-exit-confirmed-short-refill/README.md) · [Grouped refill](../2026-11-25-dense-grouped-refill/README.md) · [Confirmed early launch](../2026-11-25-dense-claim-lane-1e70f18d/README.md) · [Single dispatcher](../2026-11-25-dense-single-dispatch/README.md)

This archive adds no runner overlay or new controller request. The sealed `cc5eff4a` release and the `d388baa3` SQL remain the references, including the original stock bridge, proof-first opposing rotations, and failed conditional peer gate.

Run the offline replays from the checkout root.

```sh
	d=tools/durable-jobs-bench/results/2026-11-25-dense-logical-grant-key-native
	python3 -B "$d/controller.py" replay | cmp - "$d/evidence/native/final-replay.json"
	a=tools/durable-jobs-bench/results/2026-11-25-dense-logical-refill-attribution
	python3 -B "$a/replay.py" | cmp - "$a/replay.json"
```
