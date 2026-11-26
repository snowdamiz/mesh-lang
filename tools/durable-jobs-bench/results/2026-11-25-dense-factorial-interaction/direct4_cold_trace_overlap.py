"""Read the R3 intact client traces from the screened native release."""
import argparse
from collections import Counter, defaultdict
import hashlib
import json
from pathlib import Path
import re
import statistics
import tarfile

RAW = "9291b708bb815f50fdc8f2afbb350008f0df7564bdb2b186ac7ac5f8f82adbf7"
TRACE = re.compile(r"./semantic-root/cold/([01])-intact/runner-([01])\.receive-(\d+)-(\d+)\.log")


def words(line, width):
    assert re.fullmatch(r"[0-9a-f]+", line) and len(line) == 16 * width
    return tuple(int(line[i:i+16], 16) for i in range(0, len(line), 16))


def spans_at_five(spans):
    changes = Counter()
    for begin, end in spans:
        assert begin < end
        changes[begin] += 1
        changes[end] -= 1
    live = peak = total = previous = 0
    for tick, change in sorted(changes.items()):
        if live >= 5:
            total += tick - previous
        live += change
        peak = max(live, peak)
        previous = tick
    assert live == 0
    return peak, round(total / 1e6, 3)


def measure(lanes):
    jobs = defaultdict(dict)
    outstanding = []
    for events in lanes.values():
        sent = {r[5]: r[1] for r in events if r[0] == 12}
        for r in events:
            if r[0] != 2:
                continue
            _, start, checked, _, operation, actor, phase = r[:7]
            send = sent.pop(operation)
            assert r[11] == r[13] == 1 and start <= send <= checked
            assert phase not in jobs[actor]
            jobs[actor][phase] = start, checked
            outstanding.append((send, checked))
        assert not sent
    assert all(set(j) == {1, 2} for j in jobs.values())
    lower = [(j[1][1], j[2][0]) for j in jobs.values()]
    upper = [(j[1][0], j[2][1]) for j in jobs.values()]
    print(len(jobs), spans_at_five(outstanding), spans_at_five(lower), spans_at_five(upper))
    if 4 not in lanes:
        return
    spare = lanes[4]
    waits = [[r for r in lanes[i] if r[0] == 1 and r[10] == 0] for i in range(4)]
    observations = []
    for pick in (r for r in spare if r[0] == 3):
        by_lane = [[r for r in rows if r[1] <= pick[1] < r[3]] for rows in waits]
        assert all(len(rows) == 1 for rows in by_lane)
        first_return = min(rows[0][3] for rows in by_lane)
        writes = sorted((r[4], r[1]) for r in spare if r[0] == 12 and r[2] == pick[2])
        assert [phase for phase, _ in writes] == [1, 2]
        j = jobs[pick[2]]
        ticks = [writes[0][1], j[1][1], writes[1][1], j[2][1]]
        observations.append([(first_return - pick[1]) / 1e6, *(int(t < first_return) for t in ticks)])
    first, *later = observations
    milestone_counts = [sum(row[i] for row in later) for i in range(1, 5)]
    median_wait = statistics.median(row[0] for row in later)
    print("initial pick", first, "later picks", len(later), "median ms until first read returned",
          round(median_wait, 3), "later sends and checks", milestone_counts)


def relay_bounds(raw):
    """Bound actual first-to-second commit intervals by server round trips."""
    assert hashlib.sha256(raw.read_bytes()).hexdigest() == "07233af4fcd3cd09c6398f2f4f75813ff734d92dfd708edd8835728ab3cc7b1a"
    with tarfile.open(raw) as archive:
        for mode in (0, 1):
            prefix = f"./semantic-root/cold/{mode}-intact/"
            wire = json.load(archive.extractfile(prefix + "wire.json"))
            releases = []
            for owner in range(2):
                for line in archive.extractfile(prefix + f"gates-{owner}.log"):
                    edge = json.loads(line)
                    if edge["edge"] == "resolved":
                        releases.append(edge["ns"])
            jobs = {}
            starts = []
            ends = []
            for owner, (calls, completions) in enumerate(zip(wire["operations"], wire["completions"], strict=True)):
                receipts = {tuple(r[:2]): r[2:] for r in completions}
                assert len(receipts) == len(completions)
                for epoch, sequence, backend, key, sql, arrived in calls:
                    ready, tag, rows, status = receipts[epoch, sequence]
                    if sql == "SELECT 1":
                        continue
                    assert tag == "INSERT 0 1" and rows == 0 and status == "I" and arrived <= ready
                    job = jobs.setdefault(key, {"owner": owner})
                    assert job["owner"] == owner and backend in wire["rosters"][owner]
                    if sql.startswith("INSERT INTO public.mesh_jobs_parity_attempts"):
                        assert "first" not in job
                        job["first"] = arrived, ready
                    else:
                        assert sql.startswith("INSERT INTO public.mesh_jobs_parity_effects") and "effect" not in job
                        job["effect"] = arrived, ready
                        starts.append(arrived)
                        ends.append(ready)
            assert len(jobs) == 1024 and jobs.keys() == wire["jobs"].keys()
            assert len(starts) == len(ends) == 1024
            starts.sort()
            ends.sort()
            for owner in range(2):
                lower, upper = [], []
                for key, job in jobs.items():
                    begin, first_ready = job["first"]
                    effect_arrived, end = job["effect"]
                    assert first_ready <= effect_arrived
                    assert wire["jobs"][key]["state"] == "completed"
                    if job["owner"] == owner:
                        lower.append((first_ready, effect_arrived))
                        upper.append((begin, end))
                windows = [("full", 0, ends[-1], 0, ends[-1]),
                           ("after holds", max(releases), ends[-1], max(releases), ends[-1]),
                           ("ranks 256-768", ends[255], starts[767], starts[255], ends[767])]
                for label, inner_start, inner_end, outer_start, outer_end in windows:
                    assert outer_start <= inner_start < inner_end <= outer_end
                    bounds = []
                    for intervals, left, right in ((lower, inner_start, inner_end), (upper, outer_start, outer_end)):
                        clipped = [(max(lo, left), min(hi, right)) for lo, hi in intervals if max(lo, left) < min(hi, right)]
                        bounds.append(spans_at_five(clipped))
                    print("R4", mode, owner, label, "peak and five-way ms, lower/upper", bounds)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("raw", type=Path)
    parser.add_argument("--r4-relay", type=Path)
    args = parser.parse_args()
    raw = args.raw
    assert hashlib.sha256(raw.read_bytes()).hexdigest() == RAW
    cases = defaultdict(dict)
    with tarfile.open(raw) as archive:
        for member in archive:
            match = TRACE.fullmatch(member.name)
            if match is None:
                continue
            mode, owner, pid, lane = map(int, match.groups())
            version, hdr, before, after, *payload = archive.extractfile(member).read().decode("ascii").splitlines()
            assert version == "D4RECV01"
            header = words(hdr, 11)
            assert header[:2] == (pid, lane) and header[3:5] == (len(payload), 0)
            assert lane not in cases[mode, owner]
            cases[mode, owner][lane] = [words(row, 20) for row in payload]
    assert set(cases) == {(0, 0), (0, 1), (1, 0), (1, 1)}
    for (mode, owner), lanes in sorted(cases.items()):
        assert set(lanes) == set(range(4 + (mode == 1 and owner == 0)))
        print("mode", mode, "runner", owner, end=": ")
        measure(lanes)
    if args.r4_relay:
        relay_bounds(args.r4_relay)
