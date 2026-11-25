"""Lower-bound application transaction overlap from same-cluster XID and commit order."""
from collections import Counter
from itertools import accumulate
import json

from settlement_scans import FIXED8, RESULTS, archive, drain


def overlap(transactions, left, right, margin_us):
    # A higher XID's commit supplies a latest possible assignment time for every earlier XID.
    ordered = sorted(transactions)
    assert len(ordered) == len({xid for xid, _ in ordered})
    assert 3 <= ordered[0][0] <= ordered[-1][0] < 2**31
    latest = reversed(list(accumulate((commit for _, commit in reversed(ordered)), min)))
    events = Counter({left: 0, right: 0})
    for (_, commit), assigned_by in zip(ordered, latest, strict=True):
        start = max(left, assigned_by + margin_us)
        end = min(right, commit - margin_us)
        if start < end:
            events[start] += 1
            events[end] -= 1
    active = 0
    duration = Counter()
    previous = left
    for stamp, change in sorted(events.items()):
        if previous < stamp:
            duration[active] += stamp - previous
        previous = stamp
        active += change
        assert active >= 0
    assert active == 0 and sum(duration.values()) == right - left
    return dict(peak_floor=max(duration),
                at_least_5_us=sum(dt for n, dt in duration.items() if n >= 5),
                at_least_9_us=sum(dt for n, dt in duration.items() if n >= 9),
                assigned_awaiting_commit_job_us=sum(n * dt for n, dt in duration.items()))


def measure(root, group, lane, digest):
    with archive(root, group, lane, digest) as raw:
        begin, end, jobs = drain(raw)
    transactions = []
    for j in jobs:
        first, second = int(j["attempt_xid"]), int(j["effect_xid"])
        assert first < second and begin <= j["attempt_committed_us"] <= j["effect_committed_us"] <= end
        transactions.extend(((first, j["attempt_committed_us"]), (second, j["effect_committed_us"])))
    commits = sorted(j["effect_committed_us"] for j in jobs)
    windows = {"drain": (begin, end), "middle_512": (commits[255], commits[767])}
    return dict(raw_sha256=digest, windows={
        name: dict(duration_us=b-a, exact=overlap(transactions, a, b, 0),
                   inset_2ms=overlap(transactions, a, b, 2000)) for name, (a, b) in windows.items()})


def replay():
    measured = json.loads((FIXED8 / "replay-rest.json").read_bytes())["groups"]
    output = {}
    for i in range(1, 7):
        lanes = {}
        for lane in ("combined", "river-warm", "river-cold"):
            score = measured[str(i)]["measured"][lane]
            lanes[lane] = measure(FIXED8, i, lane, score["raw_sha256"])
        output[i] = lanes
    previous = RESULTS / "2026-11-24-dense-hot-readiness/evidence/pilot"
    summary = json.loads((previous / "summary.json").read_bytes())
    assert set(summary["blocks"]) == {"2"} and not summary["advances"]
    separate = {}
    for lane in ("hot-sync-a", "hot-ready", "hot-sync-b"):
        path, = previous.glob(f"blocks/02-*/measured-{lane}/*/raw.tar.gz")
        digest = summary["raw"][str(path.relative_to(previous))]
        separate[lane] = measure(previous, 2, lane, digest)
    return dict(warm_river=output, previous_individual_group_2=separate)


if __name__ == "__main__":
    print(json.dumps(replay(), indent=2, sort_keys=True))
