# How closely do jobs on one dense runner finish?

In the two native 8×8 bounded-claim arms, the recorded job times guarantee at least **177 and 155 disjoint pairs of actors** whose exits on the same runner fell within eight milliseconds. Each job had an individual terminal transaction. This locates an opportunity for the public grouped writer at eight slots per runner. It does not establish how many exits the writer would receive together or how much time it could save.

| Checked application checkout and individual settlement | Jobs with a guaranteed same-runner neighbor, out of 1,024 | Disjoint guaranteed pairs found | Jobs with no possible neighbor |
|---|---:|---:|---:|
| Bounded claim, rotation 1 | 392 | 177 | 50 |
| Bounded claim, rotation 2 | 349 | 155 | 40 |
| Stock claim, rotation 1 duplicates | 678 / 645 | 305 / 282 | 33 / 59 |
| Stock claim, rotation 2 duplicates | 568 / 566 | 248 / 253 | 60 / 76 |

The exit-confirmed handler returns after the second independent application commit. The runner records terminal success after actor teardown. Both database commit timestamps therefore bracket the exit on the same database clock. For exit intervals `[a,b]` and `[c,d]`, their largest possible separation is `max(b-c,d-a)`. Their smallest possible separation is `max(0,a-d,c-b)`. The replay compares only jobs with the same runner PID and counts eight milliseconds inclusively. Greedily taking disjoint guaranteed pairs in endpoint order gives a reproducible lower bound.

At eight runners, 392 and 349 jobs had a guaranteed neighbor after the faster claim, fewer than in either corresponding stock arm. Grouping can delay release of a local slot. The actual cohort histogram, last effect, submit and drain clocks must therefore come from the new configuration. These observations preserve the original two-block decision. [Native bounded-claim pilot](../2026-11-25-public-dense-stock-logical-key/continuation/evidence/native/replay.json)

Run `python3 -B eight_runner_exits.py | cmp - eight-runner-exits.json` in this directory. The replay checks the release summary, six raw archive hashes, all 48 runner job counts, and the individual terminal XIDs.
