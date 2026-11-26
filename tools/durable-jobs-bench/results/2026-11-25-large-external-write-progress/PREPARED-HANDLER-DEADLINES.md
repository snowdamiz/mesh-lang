# Checkout cost on the prepared FILE handlers

Each of the four measured span arms made 128 application-pool health queries, 64 first writes and 64 effect writes. The released adapter ran the set-based policy with hot checkout disabled. The same runtime supports skipping a health query for a connection checked back in less than 500 ms ago. The archives establish the query count and the cost inside PostgreSQL. They do not establish which checkouts would qualify, or how much client wall time each saved exchange would remove.

The sealed historical Oban drain screen compares the slower span duplicate plus the absolute duplicate spread with a 446.855 ms deadline. The forward and reverse arms missed by 9.944 and 15.666 ms. The calculation below asks what the same screen would report if the two writes completed earlier on the observed schedule. Each job retains its observed handler entry, 1 ms hold, and effect-to-settlement interval. The model subtracts the same allowance from the selected phases of each job, down to zero, then recomputes both duplicates' final settlements and spread. A phase includes time on the scheduler, in the application, and at PostgreSQL. The model assigns the entire discount to elapsed time.

| Where the discount applies | Forward | Reverse |
|---|---:|---:|
| Entry through first commit only | Best margin −3.211 ms | Best margin −7.552 ms |
| End of required hold through effect commit only | Clears at 5.366 ms per job | Best margin −12.474 ms |
| Both phases, equal discount on each | Clears at 3.577 ms per phase | Clears only from 16.990 through 20.832 ms per phase |

The reverse margin peaks at **1.921 ms** with an 18.911 ms discount in each phase. At larger discounts the faster duplicate gains enough to widen the spread again. This is why an extrapolation that simply subtracts total saved query time from the slower arm gives a misleading answer. The combined server execution time for all 128 health queries ranged from **0.389 to 0.465 ms per arm**. Their client round-trip cost and any effect on other owners' grant times are unmeasured. The separate DENSE hot-checkout series priced a cumulative benefit over 1,024 jobs, with the first durable write ranging from 4.030 ms later to 5.082 ms earlier. Those numbers do not establish a 17 ms saving on each LARGE write phase.

Keep the hot-checkout option out of native intake on this evidence. A treatment that advances the actual late grant owners could move handler entry as well as the two writes. The sealed product gates for grant, effects, full submit, CPU and control stability would still apply.

Run `python3 -B prepared_handler_deadlines.py` from this directory. It reads the committed pilot at `ae33e9e6ca`, checks the four raw hashes against the scored arms, matches every job ID to the enqueue receipt, verifies 192 distinct application and terminal transaction IDs per arm, and evaluates every integer-microsecond discount through saturation. The checked source is `f3b3612d9:compiler/mesh-rt/src/db/pool.rs`, and the cross-workload comparison comes from `67c8b9238`.
