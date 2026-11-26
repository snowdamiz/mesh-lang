# Handler progress while confirmed dense jobs wait

The six grouped-settlement hot-pool rotations used two scheduler workers per runner. A handler counts here from its recorded entry through its second durable application write. For each runner the replay divides its middle interval, from second write 103 to 922 across the deployment, into periods with zero, one, or at least two such handlers.

| Group | Runner time with 2+ handlers | Confirmed work waiting with 2+ handlers, lower bound | Possible work waiting with 0–1 handlers, upper bound (ms summed over runners) | Middle gap to River after removing that entire upper bound (ms) |
| ---: | ---: | ---: | ---: | ---: |
| 1 | 84.4% | 47.3% | 287.7 | +317.0 |
| 2 | 82.8% | 45.0% | 314.3 | +263.4 |
| 3 | 82.6% | 40.3% | 318.5 | +286.9 |
| 4 | 83.8% | 46.2% | 293.4 | +245.5 |
| 5 | 82.6% | 42.2% | 322.0 | +248.5 |
| 6 | 82.1% | 44.7% | 333.6 | +247.0 |

The percentages use the combined time on both runners. The lower bound requires a confirmed grant and at least one job from that runner still awaiting entry. The upper bound counts from the database claim stamp, before the client receives confirmation. Adding the two runners' durations can credit the same wall time twice.

Even deleting every observed millisecond in that upper bound leaves Mesh 245–317 ms behind River over the middle writes if all other intervals stay fixed. A change to dispatch or database concurrency could also change subsequent intervals. Handler presence includes SQL and scheduler waits, and this ledger does not measure worker utilization or progress between the second write and actor teardown.

Run `python3 -B refill_overlap.py` in this directory to replay the native archives. [Grant confirmation bounds](CONFIRMED-REFILL.md) · [Four-engine scorecard](NATIVE-RESULTS.md)
