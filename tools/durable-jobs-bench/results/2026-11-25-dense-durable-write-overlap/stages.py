"""Integrate native job-stage occupancy in the middle of each dense drain."""
from collections import Counter
from hashlib import sha256
import json
from statistics import median
import tarfile

from replay import HERE, SOURCE, member, minimum_events, windows_from_ledger

PILOT = HERE.parent / "2026-11-24-dense-hot-readiness/evidence/pilot"


def occupancy(intervals, start, stop, slots):
    boundaries = Counter()
    for a, b in intervals:
        if b < a:
            raise ValueError("reversed stage clock")
        left, right = max(start, a), min(stop, b)
        if left < right:
            boundaries[left] += 1
            boundaries[right] -= 1
    assert start < stop and slots > 0
    active = area = full = peak = 0
    previous = start
    for timestamp, change in sorted(boundaries.items()):
        duration = timestamp - previous
        area += active * duration
        if active >= slots:
            full += duration
        active += change
        peak = max(active, peak)
        previous = timestamp
    assert active == 0
    return dict(job_us=area, mean_jobs=round(area / (stop - start), 3), peak_jobs=peak,
                at_or_above_slots_us=full, at_or_above_slots_pct=round(100 * full / (stop - start), 3))


def reduce_arm(path, digest):
    assert sha256(path.read_bytes()).hexdigest() == digest
    with tarfile.open(path) as raw:
        ledger = member(raw, "ledger")
    all_writes = windows_from_ledger(ledger)
    jobs = ledger["jobs"]
    effects = sorted(j["effect_committed_us"] for j in jobs)
    start, stop = effects[102], effects[921]
    keys = ["entered_us", "attempt_committed_us", "effect_committed_us", "settled_us"]
    stages = list(zip(keys, keys[1:]))
    if all(j.get("claimed_us") is not None for j in jobs):
        stages.extend([("claimed_us", "entered_us"), ("claimed_us", "settled_us")])
    result = {
        a + "_to_" + b: occupancy(((j[a], j[b]) for j in jobs), start, stop, ledger["slots"])
        for a, b in stages
    }
    first = all_writes[::2]
    return dict(raw_sha256=digest, first_write_only_floor=minimum_events(first),
                median_first_to_second_us=median(b-a for a, b, _ in first),
                middle_us=stop-start, stages=result)


def reduce_all():
    wal = json.loads((SOURCE / "final-replay.json").read_text())
    pilot = json.loads((PILOT / "summary.json").read_text())
    assert pilot["missing"] == [1] and set(pilot["blocks"]) == {"2"} and not pilot["advances"]
    rows = []
    for experiment, source, groups in [("wal", SOURCE, wal["groups"]), ("readiness", PILOT, pilot["blocks"])]:
        for group, block in sorted(groups.items()):
            scores = block["arms"] if experiment == "wal" else block["scores"]
            for lane, score in sorted(scores.items()):
                if lane.startswith("measured-"):
                    path, = (source / "blocks" / block["attempt"] / lane).glob("*/raw.tar.gz")
                    rows.append(dict(series=experiment, group=int(group), lane=lane, **reduce_arm(path, score["raw_sha256"])))
    assert len(rows) == 11
    return rows


if __name__ == "__main__":
    print(json.dumps(reduce_all(), indent=2, sort_keys=True))
