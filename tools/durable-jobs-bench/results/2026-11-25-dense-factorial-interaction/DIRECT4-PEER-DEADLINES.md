# How early Direct4 needs more grants

The two private Direct4 arms finished in 909.286 and 929.240 ms from submit. Six separately released warm River arms finished in 770.658–787.280 ms. At the current Direct4 enqueue times, **at least 137 forward and 153 reverse jobs had yet to receive their database claim stamp** by the time Mesh would have had to finish. Giving every already-entered handler an immediate success at those deadlines would leave those jobs outstanding. A change that saves only idle CPU can help the pilot's resource gate. A change aimed at matching warm River must also advance productive claims or the earlier work that frees their seats.

Each row compares one original Direct4 arm to all six historical warm River arms. The cutoff is the latest time after the Mesh enqueue commit that would tie or beat the peer on both submit and drain. In the matched-enqueue sensitivity, Mesh gets the peer's actual enqueue duration and keeps its own subsequent schedule. The four right columns count jobs that have yet to reach each event. Columns overlap.

| Direct4 arm | Enqueue assumption | Cutoff from commit (ms) | Drain advance required (ms) | Certainly unstamped | Awaiting handler entry | Awaiting second write | Awaiting settlement |
|---|---|---:|---:|---:|---:|---:|---:|
| Forward | Actual 166.596 ms | 604.062–620.684 | 122.006–138.628 | 137–171 | 155–186 | 161–190 | 177–193 |
| Reverse | Actual 173.063 ms | 597.595–614.217 | 141.960–158.582 | 153–165 | 176–196 | 182–202 | 204 |
| Forward | Matched to River | 684.688–715.131 | 27.559–58.002 | 6–35 | 20–65 | 25–70 | 51–88 |
| Reverse | Matched to River | 684.688–715.131 | 41.046–71.489 | 12–48 | 32–67 | 35–71 | 45–95 |

The stock peer comparison with River and Oban used a different readiness policy. Warm River had 67 ready sessions and Direct4 had 41. The deadlines compare workloads from separate releases and preserve each release's measured session supply. A peer qualification requires fresh matched arms. [Warm River release](../2026-11-25-dense-fixed8-river-readiness/NATIVE-RESULTS.md)

## Capacity at the original claim stamp

The original Direct4 runs had 107 and 102 productive grants. The PostgreSQL row counts independently reconcile the number of grant groups, and all four runner logs have zero contention hints. Only 23 and 24 grants were full at 16 rows. For each partial grant the replay counts same-owner terminal commits in the four milliseconds before its stamp through the end of its one-millisecond stamp bin. It caps any potential increase by remaining local and global seats, preserves the other owner's rows at the same stamp, and counts only future rows still available in the recorded workload.

| Direct4 arm | Partial grants | Partials with room to grow | Partials with room to reach 16 |
|---|---:|---:|---:|
| Forward | 84 | 35 | 17 |
| Reverse | 78 | 48 | 23 |

The earlier public two-runner reference had 12 and 8 grants with room to grow and zero that could reach 16. Direct4's faster handlers changed the opportunity. Checking remaining workload supply removes one forward growth opportunity from the initial 36-grant estimate. A durable commit can precede the original reservation or follow the claimant's SELECT. Even a timely commit needs a checked receipt applied to the right reservation before it can release a seat. These counts cannot be totaled into transactions saved or elapsed time.

Direct4 already waits for a checked BEGIN before sending its ordinary bounded SELECT. That boundary gives a candidate one opportunity to consume a confirmed release without adding a queue prelock or another exchange. The decisive measurement is how often the effective limit actually changes at that point and how many additional rows reach the workers. Runner CPU, later durable ranks and the full-submit clock determine whether that pays.

## Where the durable effects fall behind

Direct4 reached its 64th durable effect earlier than all six warm River arms. By the 256th effect both Direct4 arms were behind every River arm. Times are milliseconds, measured from each arm's own enqueue commit for the first two rows.

| Durable milestone or interval | Direct4 forward | Direct4 reverse | Warm River range |
|---|---:|---:|---:|
| Effect 64 | 60.785 | 62.757 | 65.037–84.634 |
| Effect 256 | 185.982 | 183.812 | 152.593–175.441 |
| Effects 256 to 768 | 337.887 | 345.255 | 205.990–227.280 |
| Effects 768 to 1,024 | 209.995 | 216.654 | 121.942–144.773 |
| Final effect to final settlement | 8.826 | 10.456 | 178.013–209.890 |

Replacing only Direct4's middle 512 interval with each River arm's measured interval would still leave 0.664–22.211 ms forward and 13.250–34.797 ms reverse to recover to match both submit and drain. This calculation keeps each Direct4 arm's enqueue time, early progress, last 256 effects and final settlement tail fixed.

The first handler entry in a grant witnesses that the runner had received the checked grant for all its members by that time. During the middle 512 effects, both runners simultaneously held at least one such job whose handler had yet to enter for 191.678 ms forward and 184.656 ms reverse. Each held at least four unentered jobs for 67.376 and 72.846 ms. During the last 256 effects, both held an unentered job for 101.450 and 140.301 ms. These durations are lower bounds because the receipt can precede the first handler entry. Publication to the scheduler and the availability of an idle worker are unobserved. A refill change must demonstrate that additional granted jobs reach their durable writes sooner even while earlier grants are still being dispatched.

Run `python3 -B direct4_peer_deadlines.py` in this directory for all 24 deadline comparisons and the individual arm hashes. It checks 1,024 jobs and 2,048 independent application transactions in each of the eight source archives. Jobs with a claim stamp in the cutoff's ambiguous millisecond are reported separately. The [Direct4 idle CPU analysis](DIRECT4-IDLE-BUDGET.md) covers the distinct runner cost question.

## Durable write cadence while both owners have work

For each second-write commit from rank 257 through 768, the replay counts how many of the two runners had at least four jobs from checked batches still awaiting their second write. It assigns simultaneous commits to the state immediately before their timestamp. Wall time in the same state divided by its commit count gives the aggregate cadence. Each column partitions all 512 writes and the entire interval.

| Runners with at least four such jobs | Forward time / writes | Forward µs per write | Reverse time / writes | Reverse µs per write |
|---|---:|---:|---:|---:|
| Neither | 25.767 ms / 29 | 889 | 31.595 ms / 32 | 987 |
| One | 149.084 ms / 198 | 753 | 158.367 ms / 215 | 737 |
| Both | 163.036 ms / 285 | 572 | 155.293 ms / 265 | 586 |

Applying the fastest observed cadence in each arm to all 512 writes gives **292.893 ms forward and 300.038 ms reverse**. Even the slowest of the six historical warm River intervals finished in **227.280 ms**. A stricter count using four checked jobs per runner whose handlers had yet to enter gives 111 commits in 67.376 ms forward and 122 in 72.846 ms reverse.

These rates describe the released schedule. Supply and completions influence each other, and the ledger cannot show which checked jobs a worker could run immediately. Closing the whole historical middle gap would require a change that also improves on the observed cadence when both owners have work. A fresh comparison can measure whether faster settlement or earlier refill changes that cadence.

### How many jobs had Mesh claimed by River's 768th durable effect?

The slowest of the six warm River arms committed its 768th effect **396.911 ms after enqueue commit**. At that elapsed time the six silent Direct4 controls still needed **160–207 additional stamped jobs** to make 768 effects possible, even if every job already stamped had completed immediately. The count gives Mesh credit for every claim whose millisecond timestamp overlaps the cutoff. Receipt, publication and dispatch can occur later.

| Direct4 control | Jobs possibly stamped at cutoff | Effects at cutoff | Additional claims needed | Earliest observed 768th stamp minus cutoff (ms) | 768th first write minus cutoff (ms) | 768th effect minus 768th first write (ms) |
|---|---:|---:|---:|---:|---:|---:|
| Pool supply, forward | 608 | 588 | 160 | 115.563 | 125.031 | 1.927 |
| Pool supply, reverse | 593 | 577 | 175 | 105.849 | 129.553 | 2.603 |
| Idle spin, forward | 580 | 563 | 188 | 123.693 | 141.365 | 1.942 |
| Idle spin, reverse | 561 | 527 | 207 | 129.776 | 162.398 | 0.898 |
| Event park, forward | 571 | 556 | 197 | 121.694 | 131.110 | 2.821 |
| Event park, reverse | 597 | 559 | 171 | 105.353 | 125.395 | 2.058 |

The last column compares two independently sorted commit ranks. Different jobs can occupy the two ranks. A change to reply handling could still help substantially if it brings forward subsequent settlements, seat reuse and grants. The product measurement for that mechanism is whether the later grant stamps and first writes advance alongside the effects. These cross-release clocks locate the deficit and leave its cause open.

All six Mesh ledgers record 64 slots and a successful effect for every job. To start job 768, at least 704 earlier jobs must have freed a seat. At the cutoff, only **2–22** completed effects were awaiting a terminal commit record. Even granting immediate seat reuse for every recorded effect across both runners, **116–177 more effects** were needed to make the 768th claim possible. The six actual 704th effects came **82.967–117.029 ms after the cutoff**. This is a lower bound with the observed application-write schedule held fixed. Faster settlement can still help by bringing forward *new* application writes through earlier refill. A change that closes the gap must demonstrate that feedback in the ledger.

Run `python3 -B direct4_throughput_frontier.py EXPOSURE_DIR --rank-768` with the released event-park continuation directory. The output includes the selected River and six Mesh raw hashes.

### How much worker capacity could an unentered handler use?

The commit timestamp on a job's second application write can precede the caller's return from PostgreSQL. Across six silent Direct4 controls on ELF `e836b79a`, using that timestamp as the end of handler occupancy leaves 704–819 apparent worker-ms for checked jobs awaiting handler entry. Using the latest defensible handler return leaves just 2–12 worker-ms in the same calculation. The difference determines whether to target dispatch or the returning database call.

| Release and order | Through second write | Allow 1 ms to return | Allow 2 ms to return | Through terminal XID assignment upper bound |
|---|---:|---:|---:|---:|
| Pool supply, forward | 755.248 | 394.868 | 171.832 | 2.140 |
| Pool supply, reverse | 749.294 | 392.848 | 157.298 | 11.908 |
| Idle spin, forward | 724.232 | 368.874 | 147.870 | 10.652 |
| Idle spin, reverse | 818.871 | 459.820 | 215.286 | 4.080 |
| Event park, forward | 751.629 | 409.291 | 199.453 | 9.919 |
| Event park, reverse | 704.225 | 366.527 | 165.842 | 12.306 |

Each entry sums worker-ms over both runners from effect rank 256 to 768. For each runner the replay starts counting waiting members at the first handler entry from their checked batch. It subtracts the number of entered handlers covered by the assumed return times from four, floors at zero and caps the result at the number of witnessed waiting members. The last column extends every handler to the earliest recorded commit of an equal or higher XID than its terminal writer. That writer could only select handlers with confirmed exits. The two fixed-lag columns cap each return at the same upper bound.

These intervals do not identify an idle thread. A checked member may still await individual publication, and a worker may be doing other runtime work. The older two-worker direct-handler trace measured a median 0.793–1.280 ms from commit record to application return across its six instrumented arms. The missing measurements on these four-worker controls are the return from the second write and publication of the next eligible attempt. [Two-worker trace](../2026-11-25-dense-handler-commit-return/README.md)

Run `python3 -B direct4_handler_coverage.py EXPOSURE_DIR` here with the released event-park continuation directory. The reducer verifies the release and its replay, reads the other four controls from their pinned Git objects and checks all six native ledgers and process identities.

## Enqueue cost at River's later write cadence

Even if Direct4 matched warm River from the 256th through the 1,024th effect, its current enqueue time would leave it **44.255–75.293 ms short** of 20% higher throughput from submission. Under the same assumption it would already clear the 20% drain-throughput target by 13.538–59.858 ms. The difference matters when choosing between further runner work and accelerating the producer.

This sensitivity crosses six silent Direct4 controls with six historical warm River ledgers. Each calculation keeps Direct4's measured time to the 256th effect after enqueue commit and its final effect-to-settlement interval. It inserts one River arm's elapsed time for all 768 intervening effects. At 1,024 jobs each, a 20% throughput lead permits five-sixths of the peer's elapsed time. Subtracting modeled drain from that submit deadline gives the time available for enqueue.

| Direct4 control | Actual enqueue (ms) | Enqueue saving required across six River arms (ms) |
|---|---:|---:|
| Pool supply, forward | 166.596 | 47.867–65.462 |
| Pool supply, reverse | 173.063 | 53.794–71.389 |
| Idle spin, forward | 170.314 | 44.255–61.850 |
| Idle spin, reverse | 160.222 | 57.698–75.293 |
| Event park, forward | 164.450 | 51.402–68.997 |
| Event park, reverse | 156.078 | 46.087–63.682 |

Giving Mesh the same River arm's enqueue time as well would put 35 of 36 combinations inside both deadlines. The remaining one misses submit by 0.740 ms. The calculation uses twelve measured arms from separate runs. Each arm appears in six comparisons. Actual joint performance requires a matched native run. Run `python3 -B direct4_throughput_frontier.py EXPOSURE_DIR` with the same exposure directory as the handler coverage replay to reproduce the individual margins and source hashes.

## Would a 32-row claim keep four workers supplied?

A larger ceiling can change a full grant even when the owner has no new capacity to add after BEGIN. The two Direct4 runs reached the 16-row ceiling 23 and 24 times. For each full grant the replay keeps all preceding grants, credits every durable settlement through the end of the stamp's millisecond and respects both seat limits, same-stamp peer grants and remaining jobs. This gives an optimistic bound on room at the decision. The archives omit the client's checked credit at that instant.

| On the original schedule | Forward | Reverse |
|---|---:|---:|
| Full grants with room for any extra jobs | 16 | 14 |
| Full grants with room for the next entire same-owner batch | 13 | 10 |
| Such pairs with no intervening or same-stamp peer grant | 4 | 3 |
| Such uninterrupted pairs starting in the middle 512 | 2 | 0 |

For the four and three uninterrupted pairs, the first entry of the earlier batch proves the owner already had its checked receipt. Up to the first entry of the later batch, the owner always had at least one previously received job whose second write was outstanding. It had fewer than four such jobs for at most **8.551 ms out of 69.902 summed owner-ms** forward and **3.333 ms out of 49.555** reverse. The count uses only grants already proved received at the start. A job awaiting its second write may be running, waiting or awaiting dispatch. The other pairs would require reasoning about how an earlier, wider claim redistributes the ordered prefix between the two owners.

Widening might reduce fixed transaction overhead or change the later schedule. It would also put more row work before the first checked receipt. The original ledgers supply little evidence that contiguous pairs delayed a worker for lack of received jobs. The [earlier width-32 experiment](../2026-11-23-wide-trusted-grant/README.md) used two-worker runners and scalar settlement. Its noisy local rotations supply no measured Direct4 CPU saving. Keep the 16-row ceiling for the current native comparison. Run `python3 -B direct4_wide_claim.py` here to reproduce the per-grant bounds from the two original native archives.

## What would one extra worker have to recover?

The first Direct4 checked-return series stopped during warmup. Its ON arm nevertheless recorded a complete application trace for all 1,024 jobs. During the middle 512 second-write commits, preserving the recorded handler residence and owner assignment would require **5.34 and 5.53 continuously occupied workers** on the two runners to reach the slowest historical warm River interval of 227.280 ms. Improving pickup alone with one extra worker per runner would still leave at least **24.239 ms** to recover in this comparison.

| Instrumented warmup, middle 350.921 ms | Runner 1 | Runner 2 |
|---|---:|---:|
| Durable effects in the window | 258 | 254 |
| Handler dispatch through release (worker-ms) | 1,213.397 | 1,257.595 |
| First SQL send through ReadyForQuery (worker-ms) | 642.585 | 627.086 |
| Second SQL send through ReadyForQuery (worker-ms) | 539.261 | 597.260 |
| Both sends, portion certainly after their commit records (worker-ms) | 959.186 | 1,008.746 |
| Handler residence above five workers × 227.280 ms (worker-ms) | 76.997 | 121.195 |

The replay clips each actual handler turn at the two rank boundaries, including turns already in flight at rank 256. It sums residence separately on the two runners. Dividing the larger sum by five gives **251.519 ms** even with instantaneous pickup and perfect packing of that work. The arithmetic holds recorded service durations fixed. Extra concurrency could change those durations, in either direction. The first and second SQL intervals are disjoint and measure elapsed time on a worker. The after-record sums exclude the clock alignment allowance of 1.055 µs at each commit. PostgreSQL can timestamp the commit before flushing WAL. The trace also includes server scheduling and reply delivery. It cannot attribute the interval to any one of those costs. [Earlier post-flush screen](../2026-11-25-dense-postflush-boundary/screen-loader-retry/evidence/README.md)

The ON warmup is diagnostic. Native release `65115b81` exited 1 on a settlement counter comparison before either measured block ran. The trace distinguishes a receipt delivered by the grouped writer from the SQL that writer uses for a one-member group. The fresh mirrored controls must qualify before using observed reply or dispatch exposure to advance a candidate. A readiness-based suspension would need to show an actual socket wait with another published actor available, while preserving the first write's checked outcome before sending the second.

Run `python3 -B direct4_checked_worker_budget.py EXPOSURE_ARGV_V2 REDUCE_PY` in this directory. The replay pins the failed release, ON warmup archive and original trace decoder from `23df94494`. It verifies the six historical warm River ledgers through the existing peer reducer.

### Fresh mirrored series

The corrected counter allowed native release `ace2f2cb` to complete all 16 arms. Strict replay matched `verified.json` byte for byte. Both measured orders failed the original observer parity checks, including submit, middle span and host CPU comparisons. The forward OFF duplicates alone differed by 39.976 ms in the middle. The reverse submit spread was 53.708 ms against a 50.656 ms allowance. The recorder's estimated CPU charges passed at 1.059 and 0.961 CPU-ms. All measured arms had complete PostgreSQL PID attribution.

| Measured ON, middle 512 effects | Forward | Reverse |
|---|---:|---:|
| Observed span (ms) | 359.442 | 375.106 |
| Handler residence by runner (worker-ms) | 1,077.905 / 1,206.336 | 1,216.891 / 1,317.343 |
| Minimum span at that residence with five workers on each runner (ms) | 241.267 | 263.469 |
| Excess over the 227.280 ms historical River interval (ms) | 13.987 | 36.189 |
| Reply with definitely queued work, clipped at two cores (slot-ms) | 648.445 | 722.345 |
| Upper bound on between-turn dispatch gaps with queued work, same clipping (slot-ms) | 44.636 | 54.844 |

Even with perfect packing, one owner in each arm would have to reduce total handler residence by at least 69.936 or 180.943 worker-ms for five workers to hit that reference. In those two ON arms the reply intervals overlapped actual queued work, but the failed controls leave the amount on the uninstrumented path undetermined. Neither interval is recoverable wall time. This result supports examining whether an already-admitted coroutine can release its worker during an actual socket wait. It supplies no qualified speedup for doing so and earns no peer comparison.

Use `--measured` with the preceding command and the released `exposure-settlement-v3` directory. The reducer checks the sealed replay digest, both ON archive hashes and all 2,048 job identities across the two arms.

### Could a second spare find another published job?

Both measured ON arms of `ace2f2cb` had stretches where all four workers on the same runner were between sending an application write and accepting ReadyForQuery, while **two distinct fresh attempts** had been published and had yet to be picked. The independent trace join counts the two writes separately. A window starts only after the later publication bound and ends at the earliest possible pickup. This counts eligible jobs in the recorded flow; the trace does not identify when an underlying socket actually blocked or how many healthy database leases were available to a spare.

| Measured ON runner | All four in replies, ≥1 fresh (ms) | All four in replies, ≥2 fresh (ms) | Episodes with ≥2 fresh | Longest such episode (ms) | Time still eligible for the second spare after 0.5 / 1.0 ms per episode (ms) |
|---|---:|---:|---:|---:|---:|
| Forward 1 | 197.831 | 180.933 | 344 | 4.091 | 68.864 / 31.644 |
| Forward 2 | 227.053 | 209.604 | 395 | 4.189 | 80.497 / 35.491 |
| Reverse 1 | 223.449 | 207.261 | 315 | 7.029 | 100.400 / 57.477 |
| Reverse 2 | 245.053 | 230.264 | 394 | 4.126 | 95.507 / 49.982 |

The second column includes the third. Subtracting a hypothetical activation delay once per continuous episode shows why the wake cost matters: a 1 ms delay leaves 17–28% of the recorded time with two fresh attempts. An admitted handler can keep running after an original worker gets its reply, so a spare can also impose CPU cost outside these windows. The duration calculation gives neither the number of additional completions nor a predicted gain. It supplies two concrete requirements for the source proof: measure time to actual pickup after a raw socket wait, and account for how long the borrowed worker stays occupied after the original reply arrives. The official parity verdict remains the one above.

### How far could two fresh jobs get before the first base worker received its reply?

The replay selects by publication order, using the job ID to break ties. Each hypothetical spare can start one job per four-worker reply window. It must wait for the fourth worker to enter its reply interval and for the job's publication, then pay the modeled pickup delay while the job is still known to be queued. The calculation shifts that job's own recorded pickup-to-send and pickup-to-checked-reply durations to this earlier pickup. The middle-window boundaries also clip the calculation. Every window is evaluated independently, so an actor can appear in multiple windows. The counts describe opportunities in the observed schedules.

| Runner | Windows with two pickups at 0.5 ms | Both send first write before boundary | Both check first reply | Both check second reply |
|---|---:|---:|---:|---:|
| Forward 1 | 129 | 93 | 6 | 0 |
| Forward 2 | 151 | 120 | 4 | 1 |
| Reverse 1 | 127 | 107 | 11 | 2 |
| Reverse 2 | 151 | 113 | 7 | 1 |

At this delay, both jobs could send a first write in 433 of 558 windows, but both could check its reply in only 28. The model assumes two usable connections and preserves each job's original service cost with extra requests in flight. Neither assumption has been verified. The qualification needs to follow work past the first base reply and count checked writes, outstanding connections and worker occupancy. Crediting two completed writes at pickup would overstate what the traces support. The reducer reports each runner at 0, 0.5 and 1 ms and deduplicates job IDs for its separate distinct-job counts.

### What changes when a borrowed worker stays occupied?

A second replay takes each job only once. Each spare becomes available again at pickup plus the job's recorded duration through scheduler release. At most one job per spare can start in each window. If a previously moved job appears among the four original waiters later, that entire later window is ineligible. This avoids borrowing from a base wait the moved job would no longer generate. The replay keeps the other three waits, original arrivals and service durations fixed and assumes a usable connection. It omits any new work a base worker might take in a vacated turn. The two configurations each start with their spares free at the middle-window boundary.

| Runner, modeled 0.5 ms activation | Distinct jobs picked by one spare | Distinct jobs picked by two | Two-spare jobs with both replies checked before window end | Two-spare residence beyond window end |
|---|---:|---:|---:|---:|
| Forward 1 | 39 | 60 | 3 | 85.9% |
| Forward 2 | 41 | 65 | 1 | 83.1% |
| Reverse 1 | 40 | 65 | 6 | 78.8% |
| Reverse 2 | 43 | 70 | 1 | 87.9% |

The second spare increased distinct pickups by 21–27 per runner in this replay. Most borrowed-worker residence extended beyond the original wait window. The percentages charge the entire borrowed turn, including the tail of a turn still active at rank 768. They measure elapsed occupancy, which includes waiting. Only a native comparison can determine whether these extra jobs improve durable progress enough to pay the CPU cost.

### How much four-worker overlap can the sparse recorder see?

The raw-receive observer selects an application operation after connection checkout if its start falls in a common 2 ms window every 8 ms. All four operations must have been selected to observe a joint wait. The older checked-return traces in release `ace2f2cb` bracket that selection point between the caller's pool-entry and pre-send timestamps. Applying the mask to those traces gives lower and upper bounds on **recorder coverage of send-to-ReadyForQuery overlap**. A certain selection requires every possible start inside the window. A possible selection requires at least one such start. Both use the original monotonic clock. Only the old trace supplies the timings in this table.

| Selected runner in each measured order | Total all-four overlap | Covered at the sealed offset | Whole-arm operations selected | Covered across eight 2-in-8 ms phases | Covered across eight 8-in-32 ms phases |
|---|---:|---:|---:|---:|---:|
| Forward, runner 0 | 208.981 ms | 5.603 ms (2.7%) | 227 | 4.9–18.2 ms | 34.7–44.5 ms |
| Reverse, runner 1 | 257.357 ms | 15.202 ms (5.9%) | 273–277 | 4.6–15.2 ms | 36.4–55.6 ms |

The overlap columns use the middle 512 durable effects. The eight phases are equally spaced within each period. Both masks select starts during one quarter of elapsed time. A longer continuous window covers more four-way overlap on these schedules because the four writes often start at different times. With the 8-in-32 ms mask, the whole-arm counts ranged from 243–292 operations forward and 251–287 reverse across the eight phases. Those counts do not establish equal recorder CPU. Genuine socket EAGAIN, available fresh work and a recently returned connection impose further conditions. A small positive or zero in the sealed native run cannot be scaled into a full-run wait estimate using the nominal duty cycle.

### Can a fifth connection enter the hot pool?

Release `eb12dc74` stopped on the forward measured OFF-b TLS census. Its forward ON archive contains 244 sampled writes on runner 0, all of which encountered EAGAIN before CommandComplete. ReadyForQuery was already buffered when the client reached it in every write. The warmup recorded another 242 pre-CommandComplete EAGAINs. Four distinct application backends accounted for each arm's waits. Counting the receive records by outstanding leases gives the following whole-arm distribution.

| Outstanding leases when the read blocked | Recently checked-in idle connections | Measured reads | Warmup reads |
|---:|---:|---:|---:|
| 1 | 3 | 5 | 4 |
| 2 | 2 | 9 | 9 |
| 3 | 1 | 28 | 21 |
| 4 | 0 | 202 | 208 |

The pool takes the most recently returned connection first. After the initial 500 ms health window expires, four synchronous workers returning healthy connections can maintain at most four hot handles if no other caller uses the pool. At four concurrent leases, the remaining twelve connections require the ordinary checkout health check. A spare that requires an already-hot fifth connection cannot start in this state. The held fixture established that a spare *could* have a hot connection under its own startup and fault schedule. The ordinary measured arm had 20 sampled middle-window intersections of four pre-response waits with fresh jobs published, covering 14.770 ms in total. Each had zero hot idle connections. That duration includes scheduling between kernel readiness and userspace read completion. Neither this partial release nor the historical traces establish a throughput gain.

The older checked-return trace also prices the first ordinary checkouts. On each runner, the first four pool entries belonged to four distinct workers, and all four writes had been sent before any of their checked replies returned. Pool entry to the start of the send ranged from 0.509–6.363 ms in the forward block and 0.357–6.709 ms in reverse. The median for later writes was 0.003 ms on each of the four runner/block pairs. These startup intervals include checkout, caller preparation and scheduling. A spare using a fifth or sixth connection would need its own checked acquisition, which may finish after the first base worker returns. The archives measure neither the extra lease's first-use cost during sustained work nor how often it would stay warm. [Checked-return reducer](direct4_checked_worker_budget.py) · [Pool checkout](../../../../compiler/mesh-rt/src/db/pool.rs)

### How much can an extra worker change with the same published jobs?

In the measured checked-return traces, a fifth worker on each runner could advance the last durable effect by at most **7.462 ms forward and 14.545 ms reverse** if publication times, owner assignment and each job's pickup-to-effect duration stayed fixed. This bound allows the fifth worker to run continuously, gives every checkout zero added cost and allows all five workers to share work perfectly. The reverse limit comes from the publication and service time of the last job itself.

| Always available workers per runner | Maximum final-effect advance, forward | Maximum final-effect advance, reverse |
|---|---:|---:|
| 4 | 6.124 ms | 14.545 ms |
| 5 | 7.462 ms | 14.545 ms |
| 6 | 8.353 ms | 14.545 ms |
| Unlimited for this workload | 8.553 ms | 14.545 ms |

For each runner, the replay takes the earliest recorded publication bound of each job in turn. All jobs published at or after that cut still need their pickup-to-second-write time. The final effect cannot occur earlier than the cut plus the sum of those durations divided by the worker count. It must also allow each individual job to finish. The forward five-worker bound comes from twelve jobs on the owner of `job_001012`. The reverse bound depends on `job_001023`. These limits concern the final application write. A real change can advance later publications by returning checked credits earlier or change service durations by increasing backend concurrency.

The pickup replay separately models one spare per runner with a 0.5 ms activation delay, applies it over the entire trial and recalculates all 1,024 durable second-write ranks. It moves each selected job's effect by its pickup advance less the extra checkout charge. The spare stays occupied until scheduler release, including that charge. Other jobs keep their recorded write times. With a single 6.709 ms cold charge per runner and free later reuse, 156 forward and 164 reverse jobs moved. Rank 256 advanced 0.348 ms forward, and rank 960 advanced 0.510 ms reverse. Ranks 64, 512, 768 and 1,024 stayed at their original times in both orders. Charging 6.709 ms at every pickup instead left the final effect **3.160 and 2.427 ms later**. The charge is a sensitivity based on the startup brackets above. The two measured traces failed observer parity. A native comparison has to measure actual checkout costs and whether gains reach the later checked grants and durable write ranks. [Reproducible rank and capacity calculations](direct4_checked_worker_budget.py)
