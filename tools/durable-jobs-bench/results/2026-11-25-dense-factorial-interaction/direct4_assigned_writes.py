"""Count application transactions witnessed between XID assignment and their commit record."""
import argparse
from collections import Counter, defaultdict
import json
from pathlib import Path

from direct4_device_flushes import measured_archives
from direct4_peer_deadlines import clocks, ledger


def witnessed_intervals(jobs):
    applications = {}
    terminals = {}
    for job in jobs:
        owner = job.get("worker_pid")
        for phase in ("attempt", "effect"):
            xid = int(job[f"{phase}_xid"])
            assert xid not in applications
            applications[xid] = (owner, job[f"{phase}_committed_us"])
        xid = int(job["settled_xid"])
        assert terminals.setdefault(xid, job["settled_us"]) == job["settled_us"]
    assert len(applications) == 2048 and applications.keys().isdisjoint(terminals)
    commits = {**terminals, **{xid: stamp for xid, (_, stamp) in applications.items()}}
    assert 3 <= min(commits) <= max(commits) < 2**31
    bound = max(commits.values())
    for xid in sorted(commits, reverse=True):
        bound = min(bound, commits[xid])
        if xid in applications:
            owner, stamp = applications[xid]
            yield owner, bound, stamp


def concurrency(intervals, owners, left, right, inset):
    events = defaultdict(Counter)
    for owner, begin, end in intervals:
        start, finish = max(left, begin + inset), min(right, end - inset)
        if start < finish:
            events[start][owner] += 1
            events[finish][owner] -= 1
    active = dict.fromkeys(owners, 0)
    peak = dict.fromkeys(owners, 0)
    total_peak = 0
    multi_us = large_us = both_five_us = 0
    previous = left
    for stamp in sorted({left, right, *events}):
        duration = stamp - previous
        if duration:
            total = sum(active.values())
            total_peak = max(total_peak, total)
            multi_us += duration * (total >= 2)
            large_us += duration * (total >= 9)
            if len(owners) == 2 and min(active.values()) >= 5:
                both_five_us += duration
            for owner in owners:
                peak[owner] = max(peak[owner], active[owner])
        for owner, change in events[stamp].items():
            active[owner] += change
            assert active[owner] >= 0
        previous = stamp
    assert not any(active.values()) and 0 <= both_five_us <= large_us <= multi_us <= right - left
    return dict(peak=total_peak, owner_peaks=peak if len(owners) == 2 else None, two_or_more_us=multi_us,
                nine_or_more_us=large_us, both_five_us=both_five_us if len(owners) == 2 else None)


def replay(cold, fixed, protected):
    rows = {}
    for name, path, digest, drain, _ in measured_archives(cold, fixed, protected):
        jobs, txn = ledger(path.read_bytes(), digest)
        assert clocks(jobs, txn)["drain_us"] == drain
        owners = {j.get("worker_pid") for j in jobs}
        if "measured-river-warm" in name:
            assert owners == {None}
        else:
            assert None not in owners and len(owners) == 2
        effects = sorted(j["effect_committed_us"] for j in jobs)
        left, right = effects[255], effects[767]
        intervals = list(witnessed_intervals(jobs))
        rows[name] = dict(raw_sha256=digest, middle_us=right - left,
                          nominal=concurrency(intervals, owners, left, right, 0),
                          inset_2ms=concurrency(intervals, owners, left, right, 2000))
    assert len(rows) == 30
    return rows


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for arg in ("cold", "fixed", "protected"):
        parser.add_argument(arg, type=Path)
    args = parser.parse_args()
    print(json.dumps(replay(args.cold, args.fixed, args.protected), indent=2, sort_keys=True))
