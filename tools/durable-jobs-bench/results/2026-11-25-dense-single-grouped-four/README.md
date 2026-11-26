# Dense 1×64 grouped settlement with four workers

Keep the native slot closed for this comparison. A 64-slot runner with four
synchronous workers and 32 warm application TLS sessions is configurable on the
released public ELF. Reducing serial settlement work could improve scalar
single-four. The archives price that improvement below the amount required to
justify a 1×64 versus grouped 2×32 pilot on this image.

The exact target is ELF `4924b5880247059032786f43f490cf99bde8a0a69adc790994ee01a51ab91058`
from public source `3c4032d06`, fixed8 grouped success, the public bounded
claim with d388 body, and hot application checkout. A topology-only comparison
would use this same executable and opt-in on both sides, with four synchronous
workers total, 32 warm application sessions total, two CPUs, durable PostgreSQL
17.11 and 1,024 jobs with two independent app commits each. The compiled
fixture reads the slot and pool counts from its launch environment and requires
pool lanes to fit the slot count. One dispatcher has one pending claim lane,
with up to 16 seats per request, and one completion writer, with up to eight
exit-confirmed successes per transaction. The writer holds those seats until
its receipt. Claims can proceed during a held write using separately
available seats.

The local controller journal, request files, Git history and Acid project
thread titles showed no separate owner or grant for this exact configuration.
The matching active thread was this task. The scalar single-four booking
released exit 1. The public 2×32 and 8×8 grouped pilots have separate releases.
The public ELF's existing correctness replay passed 11 cases, including the
held terminal reply, renewal, and concurrent progress case. That held-reply
case exercised five jobs and three available seats. An exact compiled
single-four witness would still need a 32-connection app roster bound to that
PID through readiness and submit, queue-role limits, one observed writer, a
held grouped receipt with seats charged, and replacement writes using only
independently available capacity. This decision closes before a SETUP request.

## Where scalar single-four spent the drain

The six measured scalar primary archives come from the released raw
`1e70f18d` pilot. The table compares single-four to the faster 2×32 control on
drain in each block. The middle interval spans ascending write ranks 103–922.
[Released scalar replay](../2026-11-25-dense-single-dispatch/RELEASED-REPLAY.md)

| Block | Lane | Commit → 1st first write | Commit → 64th first write | Middle 820 first / second writes | Commit → final second write | Final second write → final terminal |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| 1 | Scalar 2×32 | 14.1 ms | 89.6 ms | 910.0 / 911.1 ms | 1,188.4 ms | 2.2 ms |
| 1 | Scalar 1×64 | 21.5 ms | 128.9 ms | 1,012.3 / 1,010.9 ms | 1,378.5 ms | 4.0 ms |
| 2 | Scalar 2×32 | 18.7 ms | 100.6 ms | 893.6 / 891.2 ms | 1,186.5 ms | 1.8 ms |
| 2 | Scalar 1×64 | 15.3 ms | 100.3 ms | 997.3 / 997.4 ms | 1,300.5 ms | 6.5 ms |

The single runner lost 192.0 and 118.8 ms on drain. Only 4.0 and 6.5 ms of
its drain came after the final second write. It reached 43 claimed and
unsettled jobs at peak in both runs, compared with 64 and 62 for the faster
controls. In the middle second-write interval, single-four averaged 27.2 and
27.0 charged jobs, versus 43.0 and 42.6. The single runner had 15.6 jobs on
average between claim and entry, versus 26.7 and 25.1, and 5.0–5.1 between
entry and the first durable write, versus 7.7–8.6. This accounts for most of
the charged-occupancy gap before the first write.

After the second write, these single-four runs still averaged 5.6 and 5.4
outstanding jobs and peaked at 18 and 17 in that interval. The faster controls
averaged 7.2 and 7.5 and peaked at 19 in both. The ledger lacks actor exit
timestamps and receipt arrival. This compartment includes jobs that have yet
to exit and jobs awaiting settlement. Its occupancy bounds the count of
exit-confirmed jobs still awaiting terminal commit from above. It cannot be
used as an observed ready cohort. The public grouped 2×32 arms averaged
9.0–10.8 in that interval and reached 22. Their claimed occupancy averaged
59.2–59.9. Grouping and bounded refill can change progress during the drain,
which is why the SQL budget below is a decision screen and carries no
end-to-end timing prediction.

## What one writer could save

The scalar single writer processed 1,024 XIDs in each run. Its terminal
statement execution totaled 367.4 and 337.2 ms, or 0.359 and 0.329 ms per XID.
Those are measured PostgreSQL statement times across serial SQL calls. They
include statement waits and omit time outside the statements, including the
dispatch and receipt path. They are distinct from PostgreSQL CPU.

| Released lane | Terminal widths 1–8 | XIDs per writer | Terminal SQL across writers | Mean SQL per terminal XID |
| --- | --- | --- | ---: | ---: |
| Public 2×32, forward | 1 / 4 / 3 / 19 / 19 / 30 / 41 / 46 | 81, 82 | 267.6 ms | 1.642 ms |
| Public 2×32, reverse | 2 / 7 / 17 / 22 / 50 / 32 / 37 / 21 | 93, 95 | 293.7 ms | 1.562 ms |
| Private 2×32 bridge, forward | 0 / 1 / 6 / 17 / 24 / 35 / 42 / 39 | 81, 83 | 271.2 ms | 1.653 ms |
| Private 2×32 bridge, reverse | 1 / 4 / 6 / 15 / 16 / 33 / 37 / 50 | 80, 82 | 255.2 ms | 1.575 ms |
| Public 8×8, forward | 85 / 130 / 112 / 39 / 30 / 5 / 1 / 0 | 46–54 | 674.6 ms | 1.678 ms |
| Public 8×8, reverse | 91 / 122 / 133 / 32 / 25 / 5 / 1 / 0 | 46–57 | 733.0 ms | 1.792 ms |

The replay retains each writer's exact width histogram. SQL execution time is
aggregated by statement in the archives. For public 2×32 the busier writer
necessarily spent at least 133.8 and 146.8 ms in terminal statements. Exact
per-writer statement durations cannot be split from the aggregate. Sending
the same transactions through one writer at the observed costs prices
267.6 and 293.7 ms of serial SQL. The private bridge used its own
`d56d0e61` ELF and drained in 832.3 and 825.5 ms. It is a bridge reference.
Public 8×8 used 16 synchronous workers across eight dispatchers and smaller
cohorts. Its histograms show how topology changes width and aggregate SQL
service. Neither series supplies a single-four gain.
The 2×32 release and private bridge archives are in local commit `f0ae21054`.

Holding the public 2×32 cohort counts and statement prices fixed gives **99.8
and 43.5 ms** less terminal SQL on one runner relative to scalar single-four.
That is enough to pursue an incremental single-runner improvement if the
target is its own baseline. Under a packing scenario with all 128
transactions at width eight and the same mean price, the difference reaches
157.2 and 137.2 ms. This scenario credits no added collection delay,
receipt waits or width-dependent SQL cost. The old scalar gate itself
required 251.5 and 186.9 ms less drain after including the larger of its
5% or duplicate-spread thresholds. To cover that from terminal SQL alone at
128 XIDs, the retained per-XID statement cost would have to fall below
0.91 and 1.17 ms, respectively.

The public grouped 2×32 arms drained in **837.6 and 1,033.8 ms**, with
615.5–769.6 ms for middle first writes, 616.1–770.1 ms for middle second
writes, and 829.1–1,026.8 ms for final second writes. Against these historical
controls, single-four would need **273.3–545.0 ms** less drain and
**244.3–579.8 ms** less submit time to draw even. These releases ran at
different times. Comparing their corresponding order positions prices scale
and carries no timing-equivalence claim. In the faster block, granting back
the entire measured 367.4 ms of scalar terminal SQL still leaves 177.6 ms
on drain. The matched public transfer itself passed one of two gates, with
runner CPU missing the second. The single-runner bottleneck for this decision
is delivering work to four writers and recycling seats quickly enough
through one dispatcher and one claim lane. The archived phases localize the
deficit before the last write and cannot isolate a particular scheduler or
SQL wait as its cause.

Warm River and Apalis are stretch targets. Warm River led the separate
matched six-block fixed8+d388 series by paired medians of 272.6 ms on submit
and 158.9 ms on drain. Apalis led a separate five-block series by 266.0 and
131.6 ms. Those series establish the size of the remaining durable-progress
gap for their own controls. This single-runner screen does not advance a
peer phase. [Warm River](../2026-11-25-dense-fixed8-river-readiness/NATIVE-RESULTS.md) · [Stock peers](../2026-11-25-dense-fixed8-logical-factorial/PEER-RESULTS.md)

## Reproduce

`replay.py` checks 16 measured raw archives against the released scorer
digests in Git, source-binds the public d388 function and live ACL,
reconciles the 32 TLS app sessions, exact runner identity, all app and
terminal XIDs, and reconstructs the clocks and seat counts. It keeps the
scalar, public grouped and private bridge observations separate. Run from
the checkout root with Python and Git.

```sh
d=tools/durable-jobs-bench/results/2026-11-25-dense-single-grouped-four
set -o pipefail
python3 -B "$d/replay.py" | cmp - "$d/result.json"
```

The replay ran offline. No new benchmark times, controller grant, SETUP,
native staging, cap71 action, or prior-pilot changes were made.
