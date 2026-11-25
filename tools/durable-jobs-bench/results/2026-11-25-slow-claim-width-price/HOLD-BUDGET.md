# Where the canonical SLOW slots spent time

In the three submit losses to River, Mesh spent **211, 307 and 244 ms more** outside the interval from handler entry to the second application commit, after dividing accumulated slot time by 64. Inside that interval, Mesh made up 154–176 ms on entry to the first commit. The workload's 100 ms hold accounts for exactly 800 ms of normalized time in every arm. The time between the two commits above that minimum differed by at most 10 ms in those three losses.

| Rotation | Enqueue | Entry to first commit | Between commits above hold | Outside entry through second commit | Drain |
|---|---:|---:|---:|---:|---:|
| 1 | +58 | −175 | +1 | +211 | +37 |
| 2 | +33 | −154 | −3 | +307 | +150 |
| 3 | +53 | −207 | −27 | +169 | −65 |
| 4 | +66 | −164 | −5 | +31 | −139 |
| 5 | +38 | −176 | −9 | +244 | +59 |

Values are Mesh minus stock River in milliseconds. A positive value means Mesh took longer. Each row's three slot-time differences sum to the measured drain difference before rounding. Adding enqueue gives the first-submit through last-settlement difference. The entry timestamp includes the handler's pool lookup or acquisition in both engines. Slot time outside the handler interval includes startup after enqueue commit, time before handler entry, and terminal work. The comparisons describe elapsed workload phases and carry no CPU attribution.

The Mesh ledger further separates that outside time. Across five blocks, the claim stamp through handler entry accounted for 151–171 normalized ms and the second commit through terminal commit for 49–60 ms. The remaining 258–368 ms accrued outside the stamped claim-to-terminal intervals. Claim stamps have millisecond resolution and precede the claim acknowledgement. At 512 jobs and 64 slots this contributes up to 8 ms of uncertainty to the two terms containing the stamp. Time outside these intervals can include a runner waiting for a terminal acknowledgement before reusing a slot.

The wider-grant experiment can act on the claim and launch terms. A sleep or second-write-only experiment has limited support here. Mesh's entire between-commits excess was 28–32 normalized ms per block. The effect of earlier slot reuse on subsequent waves still requires a native comparison.

From this directory run `python3 -B hold_budget.py | cmp - hold-budget.json`. The replay binds all 20 measured archives to the published scorecard. It verifies 512 completed jobs, 1,024 independent writes, the 100 ms hold, and the 64-slot limit for each observed handler interval. [Source series](../2026-11-24-trusted-jobs-product-composition/evidence/native/timing-closure-retry/slow/summary.json)
