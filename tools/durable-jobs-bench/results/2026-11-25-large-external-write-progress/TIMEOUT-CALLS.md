# Prepared timeout setup during the LARGE submit clock

At most 67 and 74 timeout-setup statements could have completed between first submit and final settlement in the two measured combined EXTERNAL arms. At least 87 and 88 completed outside that clock. PostgreSQL reported 139.621 and 125.093 ms of executor elapsed for these statements across the full trials. Assigning their whole-trial mean to every possible in-clock completion gives 60.744 and 57.141 ms of aggregate executor elapsed. The actual cost of those calls and their contribution to wall time remain unknown.

| Rotation and prepared lane | Runner statements in surrounding bracket | Lifetime timeout calls | Maximum timeout completions in submit clock | Whole-trial mean × maximum (ms) |
|---|---:|---:|---:|---:|
| 1 consumer | 472 | 158 | 74 | 56.049 |
| 1 both | 428 | 154 | 67 | 60.744 |
| 2 consumer | 494 | 165 | 78 | 71.234 |
| 2 both | 470 | 162 | 74 | 57.141 |

Each listener-enabled claim session checks the due time, starts a transaction, sets its timeouts, and calls PREPARE. It then either checks contention or activates a grant before issuing COMMIT. The recorded counts match those six steps on every capture. On one serial session, a continuous interval containing k completed timeout calls contains at least 6k − 5 completed statements from the sequence. Eight sessions allow a total edge correction of at most 40 statements.

The harness reads the runner SQL counters after idle and before submit, and again after observing durable settlement. The difference includes all 64 scalar settlement calls. Subtracting those 64 gives an upper bound on claim statements during the scored interval. For example, the forward combined bound is floor((428 − 64 + 40) / 6) = 67. Other runner SQL and work outside the clock can only widen the bound. A timeout query still running at the end of the clock can overlap the window without completing inside it.

Only eight or nine turns activated work per measured arm. The rest of the full-trial calls include polling. A cheaper timeout command could reduce work on the same two CPUs. Inferring a drain gain requires phase-specific costs and a matched run. The scenario in the table uses a mean across phases and concurrent sessions, so it supplies neither an elapsed-time ceiling nor a speedup estimate.

Run `python3 -B timeout_calls.py | cmp - timeout-calls.json` here to reproduce all eight prepared warmup and measured captures. It reads the sealed EXTERNAL archive from commit `39f7fae80`, checks the original score and raw hashes, and reconciles each prepared turn and all terminal transactions.
