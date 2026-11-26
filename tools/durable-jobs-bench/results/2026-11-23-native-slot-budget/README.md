# Native slot occupancy and time to catch the peers

In the five dense Linode groups, 821–846 of 1,024 Mesh jobs were still waiting for their first claim when the matched River run had finished. Faster admission can bring those jobs forward. To finish by the same deadline, Mesh would also need to cut the average time each attempt occupies a slot by at least 54–70 ms, even if it filled all 64 slots immediately and kept them full. The equivalent minimum for Oban was 33–46 ms/job.

| Native profile | Jobs / slots | Mean occupied slots over full drain / middle drain¹ | Jobs still unclaimed at Oban's finish | Jobs still unclaimed at River's finish | Minimum mean residence cut to match Oban | Minimum mean residence cut to match River |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Dense no-op | 1,024 / 64 | 24.68 / 25.24 | 747–780 | 821–846 | 32.66–45.78 ms | 53.59–70.38 ms |
| Slow | 512 / 64 | 36.37 / 38.24 | 144–208 | 172–256 | 0 ms | 0–22.44 ms |
| Short | 128 / 16 | 12.90 / 13.35 | 17–48 | 65–96 | 0–15.93 ms | 25.44–37.55 ms |
| Large | 64 / 64 | 8.45 / 9.25 | 22–26 | 0 | 0 ms | 0 ms |

¹ The occupancy columns give medians of the five groups' conservative time-weighted means. The other columns give the range across matched groups. Each peer deadline is its elapsed time from enqueue commit to final durable settlement, aligned to the Mesh arm's own enqueue commit. The replay also outputs the Rust Apalis comparisons.

For each Mesh job, the replay counts the interval from the latest possible recorded claim instant to durable terminal commit. The claim stamp has millisecond precision. The calculation allows up to 999 µs after its recorded value and uses handler entry as an upper bound. It clips each interval to the scored window before summing occupied slot time. The middle window uses the exact completion ranks in the published stage receipt. Sparse uses separately paced submissions and is outside this backlog model.

With `N` jobs, `S` slots, a peer duration of `T` and total Mesh slot residence of at least `R`, the minimum required average residence cut is `max(0, R − S × T) / N`. Zero means slot capacity alone leaves enough room for the observed residence times. Actual execution still pays for admission, the handler, PostgreSQL and settlement. The ratios apply to the frozen native stock arms. Changes to queue contention or scheduling could shorten several of these intervals together.

The full stage replay verifies the host, all four engines and their durable ledgers. This calculation checks its input index digest, each Mesh raw archive digest, the single producer transaction, all 20 residence sums and the exact scoring boundaries. Run it from any directory with Python 3. [Source ledgers and stage analysis](../2026-11-23-baseline-stage-analysis/report.md) · [Replay](replay.py)

```sh
python3 -B tools/durable-jobs-bench/results/2026-11-23-native-slot-budget/replay.py
```
