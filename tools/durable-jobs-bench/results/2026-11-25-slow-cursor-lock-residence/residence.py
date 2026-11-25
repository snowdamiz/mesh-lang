"""Lower-bound time between row stamps while the V1 cursor owns the queue lock."""

import argparse
import hashlib
import json
import tarfile
from bisect import bisect_right
from collections import Counter, defaultdict
from pathlib import Path


def middle_headroom(jobs, start, end):
    # Integrate time when at most 24 jobs are stamped and still unsettled.
    changes = Counter({start: 0, end: 0})
    for job in jobs:
        left, right = max(start, job["claimed_us"]), min(end, job["settled_us"])
        if left < right:
            changes[left] += 1
            changes[right] -= 1
    times = sorted(changes)
    area = [0]
    counts = []
    count = 0
    for i, t in enumerate(times):
        if i:
            area.append(area[-1] + (t - times[i - 1] if count <= 24 else 0))
        count += changes[t]
        counts.append(count)

    def clock(t):
        t = min(end, max(start, t))
        i = bisect_right(times, t) - 1
        return area[i] + (t - times[i] if counts[i] <= 24 else 0)

    return clock


def partition_bounds(rows, other, max_calls, clock, upper):
    # At tied stamps, assigning the earliest first write to the earliest
    # group preserves feasibility because that grant finishes first.
    stamps = [row[0] for row in rows]
    costs = [{} for _ in range(len(stamps) + 1)]
    costs[0][0] = 0
    for i, first in enumerate(stamps):
        next_other = bisect_right(other, first)
        earliest_write = rows[i][1]
        for end in range(i + 1, min(i + 16, len(stamps)) + 1):
            last = stamps[end - 1]
            earliest_write = min(earliest_write, rows[end - 1][1])
            if earliest_write < last or (next_other < len(other) and other[next_other] < last):
                break
            if upper:
                charge = clock(min(last + 999, earliest_write)) - clock(first) if end > i + 1 else 0
            else:
                charge = max(0, clock(last) - clock(first + 999))
            dest = costs[end]
            for n, previous in costs[i].items():
                if n < max_calls:
                    candidate = previous + charge
                    if n + 1 not in dest or (candidate > dest[n + 1] if upper else candidate < dest[n + 1]):
                        dest[n + 1] = candidate
    return costs[-1]


def counters(rows):
    matches = [row for row in rows if row["role"] == "parity_subject"
               and "FROM mesh_jobs.claim_batch_v1(" in row["query"]]
    if len(matches) != 1:
        raise ValueError("Expected exactly one cursor wrapper in the snapshot")
    return matches[0]


def read_arm(path, digest):
    with path.open("rb") as data:
        if hashlib.file_digest(data, "sha256").hexdigest() != digest:
            raise ValueError(f"Release digest mismatch: {path}")
    with tarfile.open(path) as archive:
        def read(name):
            return json.load(archive.extractfile("adapter/" + name + ".json"))

        jobs = read("ledger")["jobs"]
        before, end = [counters(read("statements-" + cut)) for cut in ("before", "end")]
        statuses = []
        for runner in (0, 1):
            log = archive.extractfile(f"adapter/runner-{runner}.log")
            rows = [json.loads(line) for line in log if line.startswith(b"{")]
            stopped, = [row for row in rows if row.get("state") == "stopped"]
            statuses.append(stopped)
    calls = end["calls"] - before["calls"]
    rows = end["rows"] - before["rows"]
    hints = sum(s["batch_contention_hints"] for s in statuses)
    if (len(jobs) != 512 or rows != 512 + hints or sum(s["claimed"] for s in statuses) != 512
            or any(s["last_error"] or s["finished"] != s["claimed"] for s in statuses)):
        raise ValueError("Cursor and lifecycle counters disagree")
    max_productive = calls - hints
    by_runner = defaultdict(list)
    for job in jobs:
        stamp = job["claimed_us"]
        if stamp % 1000:
            raise ValueError("Expected millisecond claim timestamps")
        by_runner[job["worker_pid"]].append(job)
    first, second = by_runner.values()
    left, right = [sorted((job["claimed_us"], job["attempt_committed_us"]) for job in group)
                   for group in (first, second)]
    writes = sorted(job["attempt_committed_us"] for job in jobs)
    start, end_time = writes[63], writes[447]

    def bound(left_clock, right_clock, upper=False):
        a = partition_bounds(left, [stamp for stamp, _ in right], max_productive, left_clock, upper)
        b = partition_bounds(right, [stamp for stamp, _ in left], max_productive, right_clock, upper)
        extreme = max if upper else min
        return extreme(x + y for n, x in a.items() for m, y in b.items() if n + m <= max_productive)

    def elapsed(t):
        return t

    def middle(t):
        return min(end_time, max(start, t))

    left_clock = middle_headroom(second, start, end_time)
    right_clock = middle_headroom(first, start, end_time)
    return dict(raw_sha256=digest, cursor_calls=calls, cursor_rows=rows, contention_hints=hints,
                max_productive_calls=max_productive, minimum_time_between_locked_row_stamps_us=bound(elapsed, elapsed),
                minimum_between_locked_stamps_in_middle_us=bound(middle, middle),
                minimum_in_middle_with_eight_peer_slots_outside_stamps_us=bound(left_clock, right_clock),
                maximum_in_middle_with_eight_peer_slots_outside_stamps_us=bound(left_clock, right_clock, upper=True),
                summed_cursor_executor_ms=end["total_exec_time"] - before["total_exec_time"])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("evidence", type=Path, nargs="+")
    result = {}
    for directory in parser.parse_args().evidence:
        release = json.loads((directory / "controller-release.json").read_bytes())
        if release["exit_status"] != 0:
            raise ValueError("Requires an exit-zero phase release")
        scores = json.loads((directory / "verified-replay.json").read_bytes())
        for group, block in scores.items():
            if not block["eligible"] or group in result:
                raise ValueError(f"Ineligible or duplicate group {group}")
            result[group] = {}
            for codec in ("pglz", "lz4"):
                lane = "measured-two-bulk-" + codec
                relative = f"{int(group):02d}/{lane}/measured-mesh/raw.tar.gz"
                digest = release["raw_sha256"][relative]
                if digest != block["scores"][lane]["raw_sha256"]:
                    raise ValueError("Scored digest differs from release")
                result[group][codec] = read_arm(directory / relative, digest)
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
