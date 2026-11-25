# How often eight dense runners had application work

During the middle 512 effects, **2.70 and 2.58 of the eight public grouped hot runners** had an entered handler awaiting its second durable write, averaged over elapsed time. The matched individual controls averaged 1.34–1.47. All eight processes took part in both rotations, and at least 240 jobs in each grouped hot arm received their claim stamps after the measured middle interval. In the separate public 2×32 grouped hot pilot, both runners had entered work throughout both middle intervals. The topology series ran separately on the same public binary.

| Public grouped hot, 8×8 | Rotation 1 | Rotation 2 |
|---|---:|---:|
| 256th to 768th durable effect | 591 ms | 631 ms |
| No runner had entered work awaiting its effect | 4.8 ms | 8.9 ms |
| Exactly one runner had such work | 63.2 ms | 97.3 ms |
| Mean runners with at least one such job | 2.70 | 2.58 |
| Mean runners with no stamped job awaiting terminal commit | 2.43–2.62 | 2.77–2.95 |

A runner can use CPU, wait for PostgreSQL, or receive a terminal acknowledgement while it has no outstanding application write. These measurements locate potential refill opportunities. They do not establish how many slots were confirmed reusable or convert time without entered work into recoverable wall time. The row about stamped jobs allows each SQL claim timestamp to fall anywhere in its recorded millisecond before handler entry.

Run `python3 -B runner_work.py | cmp - runner-work.json` in this directory. The replay verifies ten measured archives from the successful pilot releases at `dad83049a` and `f0ae21054`, checks 1,024 jobs and 2,048 distinct application XIDs per arm, and accounts for every runner-microsecond in the middle window. The process concurrency histogram independently reconciles the total runner time with entered work. [Full counts](runner-work.json) · [Public 2×32 bridge](PUBLIC-BRIDGE.md)
