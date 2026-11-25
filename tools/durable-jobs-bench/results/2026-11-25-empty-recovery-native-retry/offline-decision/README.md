# Recovered native decision for the empty recovery UPDATE

**Hold the predicate.** On PostgreSQL 17.11 at a history depth of 1,024 committed rows, recovering a full 16-row page took 0.656 ms per call with the predicate versus 0.551 ms with stock. The 0.104 ms increase (18.9%) exceeded both original regression limits, 0.075 ms and 15%. Each definition processed 192 rows across 12 calls and visited 282.3 shared blocks per call. This is the one regression signal among the 12 history-and-eligibility combinations. The retained-history benefit also missed its two-part threshold.

The two canonical dense arms each completed 1,024 jobs with 2,048 independent application writes on the same grouped-hot runner. Stock spent a total of 13.673 ms on 16 empty recovery UPDATEs during the submit-to-observer bracket and visited 16,817 shared blocks. The candidate spent 0.078 ms on 12 such calls and visited 24 blocks. Both affected zero rows. The SQL times include waits. Candidate submit-to-settlement took 1,317.281 ms versus 1,872.588 ms for stock. One ordered pair cannot attribute the 555.307 ms difference to the predicate. The original comparison function returned `advance=false`.

## What was recovered

All three native invocations exported their archives with exit zero. The original controller subsequently stopped on a missing `hot` key and recorded exit 1. The key names the verifier's expected checkout policy. The retry inventory named each lane by SQL treatment. This offline replay gives the verifier a second key referencing the exact lane it is already checking. The verifier still compares the full host file inventory, ELF, workload and PostgreSQL settings to the export. The shakedown stays outside the timed pair. Both original scorer exceptions reproduce before the corrected calls, and the original threshold code grades the result.

From this directory, run `python3 -B replay.py | cmp - replayed.json`. The three raw archive hashes and sealed series hash are checked before scoring. [Original controller release](../evidence/native/controller-release.json) · [Replayed decision, plans and counters](replayed.json)
