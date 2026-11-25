# Applying the dense 8 ms hold to a 16-slot short runner

All five public short grouped controls settled 128 jobs in 111–125 terminal transactions. Each job committed two independent application writes. The runner had 16 slots. The dense timing winner had 32 slots per runner. [Original short selection](../2026-11-24-trusted-jobs-product-composition/evidence/native/timing-closure-retry/short-smaller/selection-evidence.json)

| Rotation | Observed grouped terminal transactions | Modeled groups at 8 ms | New grants exceeding the cap on the original timeline | Groups if flushed immediately at slot pressure |
|---|---:|---:|---:|---:|
| 1 | 121 | 22 | 35 | 32 |
| 2 | 111 | 20 | 51 | 29 |
| 3 | 116 | 20 | 46 | 33 |
| 4 | 125 | 20 | 32 | 32 |
| 5 | 124 | 19 | 28 | 26 |

The model takes a success as available at its original terminal commit, a point by which actor teardown had completed. It flushes when eight successes accumulate or 8 ms elapse from the oldest. For each historical claim it uses the latest timestamp consistent with the millisecond database mark and handler entry. It counts only newly claimed slots above 16 at that time. Extra seats required peaked at six or seven in every rotation. Under the same rules the five individual-terminal controls also exceeded capacity, on 40–51 new grants per rotation.

An ideal flush at each claim that would exceed the cap permitted every original grant and still grouped the 128 successes into 26–33 transactions in the grouped controls. A real flush requires database execution, COMMIT and client confirmation. A real runner must hold each slot until confirmation and can shift the subsequent claims. The counts establish a reason to measure slot availability during a short transfer. They supply no elapsed-time prediction and do not select a new production policy.

Run `python3 -B short_transfer.py | cmp - short-transfer.json` from this directory. Replay verifies ten SHA-256 digests against the selected native scores, 1,280 completed jobs and 2,560 application transactions within their respective arms. All inputs come from the original Linode runs.
