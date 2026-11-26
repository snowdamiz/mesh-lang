# Where the two LARGE FILE pilots saved time

On default storage, preparing the next producer input early moved the last durable application write forward in both rotations. Preparing claims before taking the queue lock delayed the first wave of durable writes, then recovered enough time to finish the last wave earlier. The sole controller reserved a separate experiment at 10:07 UTC to test both changes together.

Each column uses the same control throughout one rotation, selected by the fastest submit-to-final-settlement time among the two same-image duplicates. Positive values are milliseconds saved. The original advancement decisions used their own per-clock controls.

| Milestone or interval | Producer forward | Producer reverse | Claim forward | Claim reverse |
|---|---:|---:|---:|---:|
| Submit to durable enqueue | 130.887 | 146.872 | −104.551 | −71.412 |
| Enqueue commit to 8th durable effect | −17.297 | 28.857 | −176.956 | −160.693 |
| Enqueue commit to 64th durable effect | −42.351 | 24.876 | 146.687 | 115.880 |
| Submit to 64th durable effect | 88.536 | 171.748 | 42.136 | 44.468 |
| Last durable effect to final settlement | −1.443 | −0.227 | −8.023 | 0.705 |
| Submit to final settlement | 87.093 | 171.521 | 34.113 | 45.173 |

For the claim treatment, 39 and 51 effects had already committed by the time the last runner confirmed its grant. Another 67.235 and 49.007 ms elapsed before the last durable effect. The original arms cannot establish why producer time varied across claim modes or how the two changes interact.

## Time inside the serialized grant updates

The two measured prepared arms each made eight activation calls returning 72 rows. Eight rows were counts. Eight runners each reported eight confirmed grants, and every job completed on its first attempt. Together those checks identify one productive activation per runner. The first and last grant stamps inside each activation were separated by a total of **113 ms forward and 210 ms reverse**. The eight intervals in each arm were disjoint. Each timestamp is floored to milliseconds, giving less than 8 ms of rounding uncertainty in the sum.

| Inside the eight grant intervals | Forward | Reverse |
|---|---:|---:|
| Other runners’ first application commits | 25 | 45 |
| Other runners’ second application commits | 19 | 27 |
| Other runners’ terminal commits | 12 | 30 |

The counts use the conservative interior of each interval. PostgreSQL recorded 584 and 633 ms of aggregate activation statement elapsed, which includes concurrent callers waiting for the queue lock. The observed grant intervals include scheduling and other runners’ progress. To establish a saving, a changed activation would have to advance the last effect and settlement on fresh matched runs. The first warmup had ten activations and cannot be partitioned using the measured arms’ one-activation-per-runner argument.

## Replay

From the repository root, with both original archive commits available in local Git, run

```sh
d=tools/durable-jobs-bench/results/2026-11-25-public-large-claim-gap
python3 -B "$d/crossed_file_phases.py" | cmp - "$d/crossed-file-phases.json"
```

The replay verifies both release seals, the original advancement decisions, all 16 measured archive hashes, stock default storage, and all three scorer clocks. Each arm ran eight runners with 64 slots, submitted the same 64 payloads in 64 scalar admissions, and recorded 128 application XIDs and 64 individual terminal XIDs. Both original peer stages stayed closed. The prospective experiment has its own stock FILE bridge, duplicate controls and comparison against each individual treatment.

## What a 20% throughput lead would take on default storage

Across the five released stock-peer rotations, FILE already exceeded River's submit throughput by 20% in all five. It cleared that margin on both clocks in four. Against Oban, FILE won all five from submit, but reached a 20% submit lead in one. Oban finished the post-commit drain sooner in every rotation.

The table shows the further advance needed from the original FILE arm in each paired block to reach 1.20× Oban's throughput. The last row gives the minimum by which the final application effect must advance even if final settlement costs zero. Units are milliseconds.

| FILE advance needed | 1 | 2 | 3 | 4 | 5 |
|---|---:|---:|---:|---:|---:|
| Submit to final settlement | 105.7 | 198.1 | 123.7 | 56.8 | 0.0 |
| Enqueue commit to final settlement | 229.8 | 247.7 | 249.1 | 236.2 | 215.6 |
| Last application effect from commit with an instantaneous final settlement | 225.5 | 243.4 | 245.1 | 234.2 | 211.9 |

The separate consumer pilots advanced the final effect from commit by 146.7 and 115.9 ms against their own controls. Earlier durable writes therefore matter even if the combined producer captures all the intended enqueue savings. The fresh peer runs, if qualified, provide the matched comparison.

At the same Oban drain deadline, 21–26 of the 64 Mesh jobs still lacked a database claim stamp. Even if each job could finish at the instant PostgreSQL stamped its claim, the last stamp would have to advance by at least 184–212 ms. The claim timestamps precede COMMIT and have millisecond resolution. The count ranges include stamps whose millisecond straddled the deadline.

| FILE progress at the paired Oban deadline | 1 | 2 | 3 | 4 | 5 |
|---|---:|---:|---:|---:|---:|
| Minimum advance needed at last database claim stamp (ms) | 184.4 | 212.4 | 207.1 | 201.3 | 195.7 |
| Jobs with claim stamps after deadline | 25 | 24 | 25–26 | 24 | 21–22 |
| Jobs still awaiting their first durable write | 32 | 32 | 32 | 32 | 24 |
| Jobs still awaiting their second durable write | 32 | 32 | 32 | 32 | 24 |

In every rotation, every job that had committed its first write by the deadline had also committed its second. A change that only accelerates settlement after the last grant cannot cover this gap. Earlier handler or settlement changes could still advance subsequent grants.

For equal batches of 64 jobs, a 20% throughput gain requires FILE to finish by five sixths of the peer's time. The replay verifies the raw hashes, ledgers, boot identity and three clock boundaries for 20 measured peer and FILE archives. It includes Apalis's delayed settlement in rotation four. This descriptive margin does not affect either original gate.

```sh
d=tools/durable-jobs-bench/results/2026-11-25-public-large-claim-gap
python3 -B "$d/peer_deadlines.py" | cmp - "$d/peer-deadlines.json"
```

## When the combined path overtook producer lookahead alone

The released experiment used the same producer for the combined and producer-only cells. Starting at first submit, the combined path reached each of the first **34** durable effects later in rotation 1. It reached effects **35–64** earlier. In reverse, the first **38** were later and **39–64** were earlier. These ranks count completed jobs across all eight runners. The two arms can finish different job IDs at the same rank.

Against the faster full-submit OFF duplicate, the combined path reached all 64 effect ranks earlier in rotation 1. In reverse, only ranks 1–8 arrived later. Producer lookahead paid for most of the early delay in that comparison. The same-producer comparison above exposes the delay a smaller initial prepared request would need to reduce while preserving the later gains.

Both combined cells finished 244–259 ms earlier than the faster OFF duplicate and passed the drain, last-confirmed-grant and resource gates. Only the forward rotation qualified overall. In reverse the OFF duplicates differed by 134.232 ms to enqueue and 190.362 ms to settlement. The 83.466 ms combined enqueue saving and the 182.603 ms full-submit saving over producer-only missed those thresholds. The peer stage closed.

Run `factorial_progress.py` with the pilot evidence directory exported from owner commit `34b4403224f4880dad4710934801183189346e49`. It verifies the release and score binding, all 12 measured raw hashes, and the 8th and 64th effect timestamps on both clocks, then prints the paired difference at every rank. [Pilot release and score](../../../../../t3-09dc428a/tools/durable-jobs-bench/results/2026-11-25-public-large-file-factorial/evidence/pilot)
