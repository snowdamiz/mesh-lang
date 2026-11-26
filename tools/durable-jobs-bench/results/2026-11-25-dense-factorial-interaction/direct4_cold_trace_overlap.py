"""Read the R3 intact client traces from the screened native release."""
import argparse
from collections import Counter, defaultdict
import hashlib
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


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("raw", type=Path)
    raw = parser.parse_args().raw
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
