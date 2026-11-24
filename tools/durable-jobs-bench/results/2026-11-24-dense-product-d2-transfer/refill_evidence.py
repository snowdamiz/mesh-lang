"""Bound confirmed pre-entry backlog from the measured grant and handler clocks."""
from collections import Counter, defaultdict
from hashlib import sha256
import json
from pathlib import Path
import tarfile

HERE = Path(__file__).resolve().parent
NATIVE = HERE / "evidence/native-closure-retry"
THRESHOLDS = (1, 2, 4)


def load(archive, name):
    return json.load(archive.extractfile(f"adapter/{name}.json"))


def coverage(jobs, calls):
    """All durations are integer microseconds summed independently per runner."""
    by_runner = defaultdict(list)
    for job in jobs:
        by_runner[job["worker_pid"]].append(job)
    assert len(by_runner) == 2
    effects = sorted(job["effect_committed_us"] for job in jobs)
    lo, hi = effects[102], effects[921]
    duration = 2 * (hi - lo)
    possible = Counter()
    witnessed = Counter()
    single_turn = Counter()
    exposure = {k: Counter() for k in THRESHOLDS}
    extra_seats = []
    cohort_count = 0
    for pid, stream in by_runner.items():
        groups = defaultdict(list)
        for job in stream:
            stamp = job["claimed_us"]
            assert stamp % 1000 == 0 and stamp <= job["entered_us"] <= job["settled_us"]
            groups[stamp].append(job)
        stamps = sorted(groups)
        cohort_count += len(stamps)
        assert max(map(len, groups.values())) <= 16
        events = defaultdict(list)
        events[lo] = []
        events[hi] = []
        for idx, stamp in enumerate(stamps):
            group = groups[stamp]
            if len(group) == 16:
                # Give every prior terminal commit in this millisecond credit.
                live = sum(j["claimed_us"] < stamp and j["settled_us"] >= stamp + 1000 for j in stream)
                extra_seats.append(32 - live - 16)
            next_turn = stamps[idx+1] + 1000 if idx+1 < len(stamps) else hi
            first_entry = min(j["entered_us"] for j in group)
            for j in group:
                end = min(hi, j["entered_us"])
                for kind, start in (("possible", stamp), ("witnessed", next_turn),
                                    ("same", min(first_entry, next_turn))):
                    start = max(start, lo)
                    if start < end:
                        events[start].append((kind, stamp, 1))
                        events[end].append((kind, stamp, -1))
        live = {kind: Counter() for kind in ("possible", "witnessed", "same")}
        times = sorted(events)
        for start, end in zip(times, times[1:]):
            for kind, stamp, change in events[start]:
                live[kind][stamp] += change
                if not live[kind][stamp]:
                    del live[kind][stamp]
            counts = {key: counter.total() for key, counter in live.items()}
            assert 0 <= counts["witnessed"] <= counts["same"] <= counts["possible"]
            dt = end - start
            for k in THRESHOLDS:
                if counts["possible"] >= k:
                    possible[k] += dt
                if counts["witnessed"] >= k:
                    witnessed[k] += dt
                if counts["same"] >= k:
                    single_turn[k] += dt
                    if counts["witnessed"] < k:
                        for stamp, count in live["same"].items():
                            if count > live["witnessed"][stamp]:
                                exposure[k][pid, stamp] += dt

    # Each stamp contains at least one successful grant. A split stamp costs
    # at least one extra call. Idle or empty calls make this bound looser.
    splits = calls - cohort_count
    assert splits >= 0
    bounds = {}
    for k in THRESHOLDS:
        losses = sorted(exposure[k].values(), reverse=True)
        low = max(witnessed[k], single_turn[k] - sum(losses[:splits]))
        assert 0 <= low <= possible[k] <= duration
        bounds[str(k)] = dict(confirmed_lower_us=low, possible_upper_us=possible[k],
                              later_grant_only_us=witnessed[k], runner_us=duration,
                              lower_percent=100*low/duration, upper_percent=100*possible[k]/duration)
    assert extra_seats and min(extra_seats) >= 0
    return dict(stamp_groups=cohort_count, max_split_groups=splits, calls=calls,
                groups_at_16=len(extra_seats), possible_extra_seats=sum(extra_seats),
                groups_at_16_with_room=sum(x > 0 for x in extra_seats), bounds=bounds)


def replay():
    sealed = json.loads((NATIVE / "final-replay.json").read_bytes())
    result = {}
    for group, block in sealed["blocks"].items():
        result[group] = {}
        for lane in ("hot", "checked-a", "checked-b"):
            name = "measured-" + lane
            relative = f"blocks/{block['attempt']}/{name}/measured-mesh/raw.tar.gz"
            path = NATIVE / relative
            digest = sha256(path.read_bytes()).hexdigest()
            assert digest == sealed["raw"][relative] == block["scores"][name]["raw_sha256"]
            with tarfile.open(path) as archive:
                ledger = load(archive, "ledger")
                jobs = ledger["jobs"]
                runners = {p["pid"] for p in load(archive,"process-identity") if p["argv"][-1] == "run"}
                assert runners == {j["worker_pid"] for j in jobs}
                assert len(jobs) == len({j["id"] for j in jobs}) == 1024
                assert len({j[k] for j in jobs for k in ("attempt_xid", "effect_xid")}) == 2048
                assert all(j["state"] == "completed" for j in jobs)
                result[group][lane] = dict(raw_sha256=digest, **coverage(jobs, load(archive,"raw")["raw_claim_calls"]))
    return result


if __name__ == "__main__":
    print(json.dumps(replay(), sort_keys=True, indent=2))
