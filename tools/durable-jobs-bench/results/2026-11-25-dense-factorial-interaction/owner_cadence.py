"""Compare rank-matched durable work with the number of owners carrying that work."""
from collections import defaultdict
import json
import tarfile

from peer_grant_progress import EVIDENCE, SEAL, load


def owner_times(jobs, start_field, begin, end):
    owners = defaultdict(lambda: defaultdict(int))
    for job in jobs:
        left = max(begin, job[start_field])
        right = min(end, job["effect_committed_us"])
        if left < right:
            events = owners[job["worker_pid"]]
            events[left] += 1
            events[right] -= 1

    events = defaultdict(int)
    owner_time_by_jobs = [0] * 9
    for changes in owners.values():
        jobs_in_stage, previous = 0, begin
        for timestamp, delta in sorted(changes.items()):
            duration = timestamp - previous
            owner_time_by_jobs[jobs_in_stage] += duration
            before = jobs_in_stage > 0
            jobs_in_stage += delta
            assert 0 <= jobs_in_stage <= 8
            events[timestamp] += int(jobs_in_stage > 0) - int(before)
            previous = timestamp
        assert jobs_in_stage == 0
        owner_time_by_jobs[0] += end - previous
    owner_time_by_jobs[0] += (8 - len(owners)) * (end - begin)
    assert sum(owner_time_by_jobs) == 8 * (end - begin)
    total = sum(owner_time_by_jobs[1:])
    durations = [0] * 9
    active, previous = 0, begin
    for timestamp, delta in sorted(events.items()):
        durations[active] += timestamp - previous
        active += delta
        assert 0 <= active <= 8
        previous = timestamp
    durations[active] += end - previous
    assert active == 0 and sum(durations) == end - begin
    assert sum(n * duration for n, duration in enumerate(durations)) == total
    return dict(owner_us=total, wall_us_by_owners=durations,
                owner_us_by_jobs=owner_time_by_jobs)


def measure(block, lane, score):
    jobs, _, _ = load(block, lane, score)
    if lane == "river":
        path, = block.glob("measured-river/measured-*/raw.tar.gz")
        with tarfile.open(path) as archive:
            rows = json.load(archive.extractfile("adapter/river/measured/ledger.json"))["attempts"]
        by_key = {r["job_key"]: r for r in rows}
        assert len(by_key) == len(rows) == len(jobs)
        for job in jobs:
            row = by_key[job["job_key"]]
            assert int(row["xid"]) == int(job["attempt_xid"]) and row["attempt"] == 1
            job["worker_pid"] = row["worker_pid"]
    assert len({j["worker_pid"] for j in jobs}) == 8
    effects = sorted(j["effect_committed_us"] for j in jobs)
    ranks = effects[255:768:32]
    slices = [b-a for a, b in zip(ranks, ranks[1:])]
    assert len(slices) == 16 and sum(slices) == effects[767] - effects[255]
    return dict(raw_sha256=score["raw_sha256"], span_us=sum(slices), slices_us=slices,
                entered=owner_times(jobs, "entered_us", ranks[0], ranks[-1]),
                between_commits=owner_times(jobs, "attempt_committed_us", ranks[0], ranks[-1]))


def replay():
    release = json.loads((EVIDENCE / "controller-release.json").read_bytes())
    assert release["exit_status"] == 0 and release["request_sha256"] == SEAL
    groups = json.loads((EVIDENCE / "scored-before-release.json").read_bytes())["groups"]
    result = {}
    for index, scores in sorted(groups.items()):
        block = EVIDENCE / "blocks" / f"{int(index):02}-noop"
        assert scores == json.loads((block / "reconciled.json").read_bytes())
        result[index] = {lane: measure(block, lane, scores[f"measured-{lane}"])
                         for lane in ("B", "A1", "A2", "river", "oban")}
    assert set(result) == {"1", "2", "3", "4", "5"}
    return result


if __name__ == "__main__":
    print(json.dumps(replay(), sort_keys=True, indent=2))
