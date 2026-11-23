"""Read the sealed native arms and reconstruct the observed terminal batches."""

import csv
import hashlib
import io
import json
import statistics
import sys
import tarfile
from collections import Counter, defaultdict
from pathlib import Path


HERE = Path(__file__).resolve().parent
SERIES = (
    ("ordinary-trusted-v1", "ordinary-trusted-v1-dense", 1),
    ("ordinary-trusted-v1", "ordinary-trusted-v1-short", 5),
    ("ordinary-trusted-v1", "ordinary-trusted-v1-slow", 5),
    ("ordinary-trusted-v1-large", "ordinary-trusted-large-assembly-matched", 5),
    ("ordinary-trusted-v2", "ordinary-trusted-v2-short", 5),
)


def peak_before_terminal(jobs, start):
    # Count [start, terminal) using the net delta at each timestamp.
    changes = Counter()
    for j in jobs:
        changes[j[start]] += 1
        changes[j["settled_us"]] -= 1
    pending = peak = 0
    for tick in sorted(changes):
        pending += changes[tick]
        peak = max(pending, peak)
    assert pending == 0
    return peak


def timed_batches(arrivals, hold_us, width):
    """Flush on the deadline of the oldest job or when the batch fills."""
    sizes = []
    for stream in arrivals.values():
        deadline = None
        size = 0
        for at in sorted(stream):
            if size and at > deadline:
                sizes.append(size)
                size = 0
            if not size:
                deadline = at + hold_us
            size += 1
            if size == width:
                sizes.append(size)
                size = 0
        if size:
            sizes.append(size)
    return sizes


def arm(path, expected_hash, score):
    sha = hashlib.sha256(path.read_bytes()).hexdigest()
    assert sha == expected_hash, path
    with tarfile.open(path) as archive:
        ledger = json.load(archive.extractfile("adapter/ledger.json"))
        assert json.load(archive.extractfile("status.json"))["exit_code"] == 0
    jobs = ledger["jobs"]
    assert len(jobs) == score["completed"] == score["ordered_independent_writes"]
    assert score["failed"] == 0 and score["affinity"] == [0, 1]
    assert len({j["job_key"] for j in jobs}) == len(jobs)
    groups = Counter()
    times = {}
    app_xids = set()
    arrivals = defaultdict(list)
    for j in jobs:
        assert j["state"] == "completed" and j["attempts"] == j["effects"] == j["attempt"] == 1
        assert j["entered_us"] <= j["attempt_committed_us"] <= j["effect_committed_us"] <= j["settled_us"]
        app_xids.update((j["attempt_xid"], j["effect_xid"]))
        xid = j["settled_xid"]
        assert times.setdefault(xid, j["settled_us"]) == j["settled_us"]
        groups[xid] += 1
        if j.get("worker_pid") is not None:
            arrivals[j["worker_pid"]].append(j["effect_committed_us"])
    assert len(app_xids) == 2 * len(jobs) and app_xids.isdisjoint(groups)
    assert all(int(xid) > 0 for xid in app_xids | set(groups))
    assert len(groups) == score.get("terminal_xids", len(groups))
    assert max(groups.values()) == score.get("largest_terminal_cohort", max(groups.values()))
    result = {
        "jobs": len(jobs), "slots": ledger["slots"], "terminal_xids": len(groups),
        "one_job_xids": sum(n == 1 for n in groups.values()),
        "largest_xid_jobs": max(groups.values()),
        "peak_entered_before_terminal": peak_before_terminal(jobs, "entered_us"),
        "peak_both_writes_before_terminal": peak_before_terminal(jobs, "effect_committed_us"),
        "effect_to_terminal_median_us": statistics.median(j["settled_us"] - j["effect_committed_us"] for j in jobs),
        "last_effect_to_last_terminal_us": max(times.values()) - max(j["effect_committed_us"] for j in jobs),
        "raw_sha256": sha,
    }
    ordinary = path.parents[1].name.startswith("measured-ordinary")
    if ordinary:
        assert len(arrivals) <= ledger["runners"] and sum(map(len, arrivals.values())) == len(jobs)
    for hold in (0, 250, 1000, 2000):
        key = f"modeled_xids_{hold}us"
        result[key] = ""
        if ordinary:
            sizes = timed_batches(arrivals, hold, min(ledger["slots"] // ledger["runners"], 8))
            assert sum(sizes) == len(jobs)
            result[key] = len(sizes)
    return result


def collect():
    rows = []
    for base, series, count in SERIES:
        source = HERE.parent / f"2026-11-23-{base}" / "evidence" / series / "blocks"
        for group in range(1, count + 1):
            receipt, = source.glob(f"{group:02}-*/reconciled.json")
            sealed = json.loads(receipt.read_text())
            for lane, score in sorted(sealed["scores"].items()):
                path, = receipt.parent.glob(f"measured-{lane}/*/raw.tar.gz")
                raw = str(path.parent.relative_to(receipt.parent))
                rows.append({"series": series, "group": group, "lane": lane,
                             **arm(path, sealed["raw_sha256"][raw], score)})
    out = io.StringIO()
    writer = csv.DictWriter(out, fieldnames=list(rows[0]), lineterminator="\n")
    writer.writeheader()
    writer.writerows(rows)
    return out.getvalue()


if __name__ == "__main__":
    output = collect()
    if sys.argv[1:] == ["--check"]:
        if output != (HERE / "batches.csv").read_text():
            raise SystemExit("saved batch evidence differs from replay")
        print("Verified 110 measured arm archives.")
    elif sys.argv[1:]:
        raise SystemExit("usage: replay.py [--check]")
    else:
        sys.stdout.write(output)
