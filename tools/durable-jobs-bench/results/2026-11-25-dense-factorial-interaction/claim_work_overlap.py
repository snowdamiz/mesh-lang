"""Bound the overlap of pending prelocks and confirmed application work in UTC."""
import hashlib
import json
import sys
import tarfile
from collections import Counter, defaultdict
from pathlib import Path

from claim_phase_budget import RAW, VERIFIED, checked


def sweep(arm, jobs, lower):
    lo, hi = arm["clock_offsets_ns"]["utc"]
    start, end = [t * 1000 for t in arm["write_edges_us"]]
    events = defaultdict(Counter)
    events[start]
    events[end]

    def span(pid, state, a, b):
        a, b = max(a, start), min(b, end)
        if a < b:
            events[a][pid, state] += 1
            events[b][pid, state] -= 1

    cohorts = defaultdict(list)
    for job in jobs:
        cohorts[int(job["worker_pid"]), job["claimed_us"]].append(job)
    for turn in arm["turns"]:
        pid = turn["pid"]
        if turn["prelock_receipt"] is not None:
            span(pid, "wait", turn["acquisition_ns"] + (hi if lower else lo),
                 turn["prelock_receipt"]["ns"] + (lo if lower else hi))
        if not turn["rows"]:
            continue
        cohort = cohorts.pop((pid, turn["grant_stamp_us"]))
        assert len(cohort) == turn["rows"]
        for job in cohort:
            # Database and handler clocks are recorded at microsecond precision.
            first = job["attempt_committed_us"] * 1000
            span(pid, "pending", turn["confirmed_ns"] + (hi if lower else lo),
                 first + (0 if lower else 1000))
            span(pid, "entered", job["entered_us"] * 1000 + (1000 if lower else 0),
                 first + (0 if lower else 1000))
    assert not cohorts
    state = Counter()
    duration = Counter()
    previous = start
    for ns in sorted(events):
        dt = ns - previous
        waiting = {pid for (pid,k),n in state.items() if k == "wait" and n}
        for k in ("pending", "entered"):
            working = {pid for (pid,kind),n in state.items() if kind == k and n}
            for threshold in (1, 2, 4, 6, 8):
                if len(working) >= threshold:
                    duration[f"at_least_{threshold}_runners_{k}"] += dt
            duration[f"accumulated_{k}_runner_time"] += dt * len(working)
            if any(working - {pid} for pid in waiting):
                duration["other_" + k + "_while_waiting"] += dt
        for threshold in (1, 2, 4, 6, 8):
            if len(waiting) >= threshold:
                duration[f"at_least_{threshold}_runners_waiting"] += dt
        state.update(events[ns])
        assert min(state.values(), default=0) >= 0
        previous = ns
    assert not any(state.values())
    return duration


def main(evidence):
    verified = json.loads(checked((evidence / "verified.json").read_bytes(), VERIFIED))
    raw = evidence / "raw.tar.gz"
    with raw.open("rb") as f:
        assert hashlib.file_digest(f, "sha256").hexdigest() == verified["raw_sha256"] == RAW
    results = []
    with tarfile.open(raw) as tar:
        for idx, group in enumerate(verified["result"]["groups"], 1):
            arm = group["arms"]["observe/measured"]
            data = tar.extractfile(f"./g{idx}-observe-measured/ledger.json").read()
            jobs = json.loads(checked(data, arm["hashes"]["ledger.json"]))["jobs"]
            lower, upper = (sweep(arm, jobs, low) for low in (True, False))
            start, end = arm["write_edges_us"]
            for k in lower.keys() | upper.keys():
                limit = 8 if k.startswith("accumulated_") else 1
                assert lower[k] <= upper[k] <= limit * 1000 * (end - start)
            results.append({"middle_ms": (end-start)/1000,
                            "duration_ms_bounds": {k: [lower[k]/1e6, upper[k]/1e6]
                                                   for k in sorted(lower.keys() | upper.keys())}})
    print(json.dumps({"raw": RAW, "groups": results}, indent=2))


if __name__ == "__main__":
    main(Path(sys.argv[1]))
