"""Compare confirmed work awaiting entry with ongoing handlers in the same runner."""
from native_report import replay as replay_scores
from refill_evidence import replay as replay_backlog


def report():
    by_count = [replay_backlog(count) for count in range(3)]
    scores, summary = replay_scores()
    lines = [
        "# Handler progress while confirmed dense jobs wait",
        "",
        "The six grouped-settlement hot-pool rotations used two scheduler workers per runner. "
        "A handler counts here from its recorded entry through its second durable application write. "
        "For each runner the replay divides its middle interval, from second write 103 to 922 "
        "across the deployment, into periods with zero, one, or at least two such handlers.",
        "",
        "| Group | Runner time with 2+ handlers | Confirmed work waiting with 2+ handlers, lower bound "
        "| Possible work waiting with 0–1 handlers, upper bound (ms summed over runners) "
        "| Middle gap to River after removing that entire upper bound (ms) |",
        "| ---: | ---: | ---: | ---: | ---: |",
    ]
    for group, scores_by_lane in zip(summary["blocks"], scores, strict=True):
        hot = [category[group]["hot"] for category in by_count]
        bounds = [category["bounds"]["1"] for category in hot]
        duration = bounds[0]["runner_us"]
        assert len({category["raw_sha256"] for category in hot}) == 1
        assert all(bound["runner_us"] == duration for bound in bounds)
        assert all(bound["possible_upper_us"] <= category["bucket_us"]
                   for category, bound in zip(hot, bounds, strict=True))
        assert sum(category["bucket_us"] for category in hot) == duration
        possible_ms = sum(bound["possible_upper_us"] for bound in bounds[:2]) / 1000
        mesh = scores_by_lane["hot"]["writes"]
        river = scores_by_lane["river-2x32"]["writes"]
        residual = (mesh[921] - mesh[102]) - (river[921] - river[102]) - possible_ms
        lines.append(f"| {group} | {100 * hot[2]['bucket_us'] / duration:.1f}% "
                     f"| {bounds[2]['lower_percent']:.1f}% | {possible_ms:.1f} | {residual:+.1f} |")
    lines.extend([
        "",
        "The percentages use the combined time on both runners. The lower bound requires a "
        "confirmed grant and at least one job from that runner still awaiting entry. The upper "
        "bound counts from the database claim stamp, before the client receives confirmation. "
        "Adding the two runners' durations can credit the same wall time twice.",
        "",
        "Even deleting every observed millisecond in that upper bound leaves Mesh 245–317 ms "
        "behind River over the middle writes if all other intervals stay fixed. A change to "
        "dispatch or database concurrency could also change subsequent intervals. Handler "
        "presence includes SQL and scheduler waits, and this ledger does not measure worker "
        "utilization or progress between the second write and actor teardown.",
        "",
        "Run `python3 -B refill_overlap.py` in this directory to replay the native archives. "
        "[Grant confirmation bounds](CONFIRMED-REFILL.md) · [Four-engine scorecard](NATIVE-RESULTS.md)",
        "",
    ])
    return "\n".join(lines)


if __name__ == "__main__":
    print(report(), end="")
