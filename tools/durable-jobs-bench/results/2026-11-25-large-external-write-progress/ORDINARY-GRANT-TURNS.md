# Where the ordinary cursor gained time

Pilot `25282314` measured eight public runners, 64 jobs, the original FILE broker and separate DEFAULT and EXTERNAL storage rotations. The slower span duplicate drained faster than the faster stock duplicate in all four measured blocks. DEFAULT cleared the product gate only in block 1 and EXTERNAL only in block 2. Each policy required both. Neither earned fresh peers.

The 48 archives identify **421 productive cursor transactions**. The sum of the first-to-last job stamp intervals inside transactions fell by **79–108 ms** in all four measured comparisons. The intervals between successive productive transactions' job stamps grew in three of the four. In DEFAULT block 2, a 108 ms gain inside turns coincided with 110 ms more between turns.

| Policy / group | Faster stock / slower span on drain | Enqueue commit to first stamp | Inside turns | Between turns | Final stamp to settlement | Drain gain |
|---|---|---:|---:|---:|---:|---:|
| DEFAULT 1 | B / A | +52.307 | +81 | −65 | +20.266 | +88.573 |
| DEFAULT 2 | A / B | +44.886 | +108 | −110 | +22.000 | +64.886 |
| EXTERNAL 1 | B / A | −10.524 | +94 | −37 | −14.245 | +32.231 |
| EXTERNAL 2 | A / A | +28.484 | +79 | +31 | −32.643 | +105.841 |

Values are stock minus span in milliseconds. Each row uses the same two arms for all four intervals so their sum gives the drain gain. Job stamps have millisecond resolution. The first context in each grant has already rendered by the first stamp. The between-turn interval includes the previous grant's last update, returned rows and COMMIT, admission of the next grant, and the work through its first stamp.

After a handler from the previous grant has entered, its queue lock has already been released. Across those windows, from earliest such entry up to the next first stamp, slower span accumulated **254.952 / 271.235 ms** in DEFAULT and **233.572 / 212.665 ms** in EXTERNAL. Concurrent jobs committed **37 / 45** durable effects in the DEFAULT windows and **36 / 39** in EXTERNAL. A change that avoids a redundant set attempt must be priced against the next successful grant's arrival and the real durable effect schedule. The whole window cannot be charged to an idle queue or recovered by making COMMIT faster.

## Reconstruction

The checked cursor has one runner per productive transaction and at most eight rows. Each job is stamped under the queue lock and enters its handler after the checked COMMIT. The only extra cursor rows in this small-enough-context fixture are single-row contention hints. In every arm, extra cursor rows plus set lock timeouts match the runner contention counters. Subtracting extra rows from successful cursor calls bounds the number of productive calls from above, allowing zero-row calls.

Enumerating all contiguous partitions under that bound, the eight-row cap, the PID, and the earliest handler entry gives a single partition in each arm. For tied millisecond stamps, the search permits the latest entry among the tied jobs, and verifies the ties belong to one PID and never cross an inferred boundary. Each resulting count reaches the SQL upper bound. The four intervals then telescope exactly to official drain for all 3,072 completed jobs. The archives lack per-turn queue-lock release timestamps. Durations after previous handler entry give conservative observed windows and include competing useful work.

Run `python3 -B ordinary_grant_turns.py EVIDENCE CANONICAL_RELEASE` against the pilot's evidence directory and compare stdout to the saved JSON. The reducer verifies the exact release, official score file, block scores and all 48 raw hashes.

[Per-arm reconstruction](ordinary-grant-turns.json) · [Reducer](ordinary_grant_turns.py) · [Historical Oban and River deadlines](ORDINARY-CURSOR-DEADLINES.md)
