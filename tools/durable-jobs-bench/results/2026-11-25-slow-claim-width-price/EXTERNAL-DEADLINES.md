# First writes required for a 20% SLOW throughput lead

In the five matched historical EXTERNAL blocks, 63–89 Mesh jobs still needed their first durable write at the deadline for a 20% drain throughput lead over River. The last first write would have needed to move **133–188 ms earlier**, even with free settlement. Against Oban, the corresponding count was 30–63 jobs and the last first write needed to move 76–142 ms earlier.

For 512 jobs, a 20% throughput advantage requires settlement by five sixths of the peer's elapsed time. Each job needs 100 ms between its first durable write and its effect. This places a deadline on its first write 100 ms before settlement. The replay reads each job's committed timestamps and applies the deadline separately on submit and drain. It checks the older Mesh, River, and Oban archives in each matched EXTERNAL block.

## Implications for the public full-refill runner

Using the same historical deadlines as design targets, the public DEFAULT forward refill arm finished drain 3–53 ms inside all five River targets. Its reverse arm finished 25–75 ms outside them. In reverse, **15–52 jobs** still needed their first writes at the cutoff. Even a free terminal commit would require bringing the last first write forward **21–70 ms**. Against the five Oban drain targets the forward arm finished inside each. The reverse arm ranged from 47 ms inside to 28 ms outside.

The submit target also constrains the producer. If public drain remained exactly as observed, the greatest enqueue time compatible with a 20% submit lead over River would be **275–314 ms forward** and **197–236 ms reverse**. The old matched EXTERNAL Mesh producer took **355–398 ms**. Consequently a transfer that merely reproduced the old producer and the public DEFAULT drain would miss every 20% River submit target. This combines observations from separate series to size an experiment. No public full-refill versus EXTERNAL peer race has been measured here.

The [receipt](external-deadlines.json) has each comparison in microseconds. To reproduce it, supply the timing evidence directory from `a3070ff56` to `python3 -B external_deadlines.py "$timing_evidence" | cmp - external-deadlines.json`. The two public inputs are pinned to the cap68 controller release. The historical inputs use the [matched peer reader](../2026-11-23-iteration-comparison/slow-peer-cutoffs.py).
