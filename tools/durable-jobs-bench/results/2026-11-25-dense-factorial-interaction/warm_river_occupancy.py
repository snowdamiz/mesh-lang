"""Measure overlapping handler stages during the released dense middle 512 writes."""
from collections import Counter, defaultdict
from hashlib import sha256
import json
from pathlib import Path
import tarfile


SERIES = (Path(__file__).resolve().parent.parent
          / "2026-11-25-dense-fixed8-river-readiness/evidence/native")
STAGES = ("claimed_us", "entered_us", "attempt_committed_us", "effect_committed_us", "settled_us")


def occupancy(jobs, begin, end):
    width = end - begin
    area = {}
    for left, right in zip(STAGES, STAGES[1:]):
        area[f"{left[:-3]}_to_{right[:-3]}"] = sum(
            max(0, min(end, job[right]) - max(begin, job[left])) for job in jobs)
    events = defaultdict(Counter)
    for job in jobs:
        a, b = max(begin, job["entered_us"]), min(end, job["effect_committed_us"])
        if a < b:
            key = job.get("worker_pid", "all")
            events[a][key] += 1
            events[b][key] -= 1
    active = Counter()
    last = begin
    by_active_runners = Counter()
    for stamp in sorted(set(events) | {end}):
        if stamp > last:
            by_active_runners[sum(n > 0 for n in active.values())] += stamp - last
        active.update(events[stamp])
        assert all(n >= 0 for n in active.values())
        last = stamp
    assert not any(active.values()) and sum(by_active_runners.values()) == width
    result = dict(middle_512_us=width, stage_job_us=area,
                  mean_jobs={k: round(v / width, 4) for k, v in area.items()})
    if all("worker_pid" in job for job in jobs):
        assert len({job["worker_pid"] for job in jobs}) == 2
        result["us_with_active_runners_entry_to_second_commit"] = {
            n: by_active_runners[n] for n in range(3)}
    return result


def replay():
    release = json.loads((SERIES / "controller/rest-release.json").read_bytes())
    assert release["exit_status"] == 0 and release["groups"] == [3, 4, 5, 6]
    data = json.loads((SERIES / "replay-rest.json").read_bytes())
    result = {}
    for group in range(1, 7):
        rows = {}
        arms = data["groups"][str(group)]["measured"]
        assert set(arms) == {"combined", "river-2x32", "river-cold", "river-warm"}
        for lane, row in arms.items():
            files = SERIES.glob(f"blocks/{group:02}-*/measured-{lane}/*/raw.tar.gz")
            digest = row["raw_sha256"]
            path, = (p for p in files if sha256(p.read_bytes()).hexdigest() == digest)
            with tarfile.open(path) as tar:
                assert json.load(tar.extractfile("status.json"))["exit_code"] == 0
                ledger_bytes = tar.extractfile("adapter/ledger.json").read()
            assert sha256(ledger_bytes).hexdigest() == row["ledger_sha256"]
            ledger = json.loads(ledger_bytes)
            jobs = ledger["jobs"]
            assert len(jobs) == len({j["job_key"] for j in jobs}) == 1024
            assert len({j[k] for j in jobs for k in ("attempt_xid", "effect_xid")}) == 2048
            assert all(j["attempt"] == j["attempts"] == j["effects"] == 1 and
                       [j[k] for k in STAGES] == sorted(j[k] for k in STAGES) for j in jobs)
            times = sorted(job["effect_committed_us"] for job in jobs)
            observed = occupancy(jobs, times[255], times[767])
            assert observed["middle_512_us"] == row["progress"]["second_write"]["middle_512_us"]
            observed["raw_sha256"] = digest
            rows[lane] = observed
        result[group] = rows
    return result


if __name__ == "__main__":
    print(json.dumps(replay(), indent=2, sort_keys=True))
