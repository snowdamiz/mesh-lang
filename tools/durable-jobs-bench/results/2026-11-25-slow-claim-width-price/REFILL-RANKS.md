# Progress across 64-job ranks on SLOW

At the previous five public Mesh runs, 296–379 ms elapsed in total between
each 64th scalar settlement and the entry 64 ranks later. The seven gaps
use settlement ranks 64 through 448. Once 64 slots have filled, reaching
entry rank 128 requires at least 64 terminal receipts. The same argument
applies at each later rank. The ledger records the database commit, which
precedes the client receipt.

| Block | Seven settlement-to-next-entry gaps | Next entry to first commit | Submit gap vs River |
|---|---:|---:|---:|
| 1 | 296 ms | 21 ms | +95 ms |
| 2 | 375 ms | 39 ms | +183 ms |
| 3 | 379 ms | 16 ms | −12 ms |
| 4 | 360 ms | 20 ms | −72 ms |
| 5 | 314 ms | 21 ms | +97 ms |

With every other term held fixed, covering the three submit losses would
require reducing the seven gaps by 32%, 49% and 31%. Each gap includes
the time until a later entry rank is reached and can include useful work.
Its size does not establish removable waiting time. The two ranks need not
describe the same job or a specific seat handoff.

For each arm the replay sums enqueue, time to first-commit rank 64, eight
mandatory 100 ms holds, excess first-to-second-commit time at those eight
ranks, effect-to-settlement at those ranks, the seven refill gaps, and
seven next-entry-to-first-commit intervals. The sum exactly equals first
submit through last settlement. River can admit later jobs while earlier
effects await terminal commit, so its settlement-to-next-entry term is
signed. Both engines retain their original receipts and contracts.

Run `python3 -B refill_ranks.py | cmp - refill-ranks.json` in this directory.
For a fresh Mesh timing capture, pass `--raw PATH --sha256 DIGEST` from its
sealed release. This is a descriptive comparison for the cap68 timing pilot.
Its advancement criteria come from the pilot packet.
