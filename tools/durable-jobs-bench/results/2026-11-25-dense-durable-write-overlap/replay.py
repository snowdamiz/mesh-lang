"""Bound application-write durability events in the released two-CPU WAL trials."""
from collections import Counter
from hashlib import sha256
import json
from pathlib import Path
import tarfile

HERE = Path(__file__).resolve().parent
SOURCE = HERE.parent / "2026-11-24-dense-wal-cost/evidence/native"
MARKERS = HERE.parent / "2026-11-25-dense-handler-commit-return/native.json"
CALLERS = HERE.parent / "2026-11-24-trusted-handler-app-pool/timing-clean-identity-retry/evidence/timing"


def minimum_events(windows, cap=None):
    """One event can cover overlapping closed intervals up to cap per runner."""
    if cap is not None and cap <= 0:
        raise ValueError("event capacity must be positive")
    pending = sorted((end, start, pid) for start, end, pid in windows)
    if any(start > end for end, start, _ in pending):
        raise ValueError("inverted durability window")
    events = 0
    while pending:
        at = pending[0][0]
        used = Counter()
        later = []
        for end, start, pid in pending:
            if start <= at and (cap is None or used[pid] < cap):
                used[pid] += 1
            else:
                later.append((end, start, pid))
        events += 1
        pending = later
    return events


def member(raw, name):
    return json.load(raw.extractfile("adapter/" + name + ".json"))


def windows_from_ledger(ledger):
    jobs = ledger["jobs"]
    assert ledger["profile"] == "noop"
    assert (ledger["runners"], ledger["slots"]) == (2, 64)
    db = ledger["database"]
    assert db["version"] == "17.11" and db["tls"] and db["synchronous_commit"] and db["fsync"]
    assert len(jobs) == len({j["job_key"] for j in jobs}) == 1024
    app = {j[k] for j in jobs for k in ("attempt_xid", "effect_xid")}
    terminal = {j["settled_xid"] for j in jobs}
    assert len(app) == 2048 and app.isdisjoint(terminal)
    intervals = []
    for j in jobs:
        assert j["attempts"] == j["effects"] == 1 and j["state"] == "completed"
        pid = j.get("worker_pid", 0)
        a, b, c = (j[k] for k in ("attempt_committed_us", "effect_committed_us", "settled_us"))
        assert a < b <= c
        intervals.extend(((a, b, pid), (b, c, pid)))
    return intervals


def wal_arms():
    original = json.loads((SOURCE / "final-replay.json").read_text())
    assert not original["missing"] and not original["failures"] and len(original["groups"]) == 2
    result = []
    for group, block in sorted(original["groups"].items()):
        for lane in ("d2-on", "river-2x32", "oban-2x32", "apalis-2x32"):
            name = "measured-" + lane
            score = block["arms"][name]
            path, = (SOURCE / "blocks" / block["attempt"] / name).glob("*/raw.tar.gz")
            digest = sha256(path.read_bytes()).hexdigest()
            assert digest == score["raw_sha256"]
            with tarfile.open(path) as raw:
                ledger = member(raw, "ledger")
                call = json.load(raw.extractfile("invocation.json"))
                assert (call["phase"], call["engine"], call["profile"]) == ("measured", ledger["engine"], "noop")
                windows = windows_from_ledger(ledger)
                effects = sorted(j["effect_committed_us"] for j in ledger["jobs"])
                low, high = effects[102], effects[921]
                middle = [w for w in windows if low <= w[0] and w[1] <= high]
                before = member(raw, "wal-before")["frame"]
                after = member(raw, "wal-restored")["restored"]["frame"]
                assert before["system_id"] == after["system_id"]
                for f in (before, after):
                    assert f["settings"]["wal_sync_method"] == "fdatasync"
                assert before["wal"]["stats_reset"] == after["wal"]["stats_reset"]
                syncs = after["wal"]["wal_sync"] - before["wal"]["wal_sync"]
                assert syncs == score["postgres_wal"]["whole_workload_delta"]["wal_sync"]
                row = dict(group=int(group), engine=ledger["engine"], raw_sha256=digest,
                           unrestricted_floor=minimum_events(windows), cluster_wal_sync=syncs,
                           middle_ms=(high-low)/1000, middle_contained_writes=len(middle),
                           middle_unrestricted_floor=minimum_events(middle),
                           terminal_xids=len({j["settled_xid"] for j in ledger["jobs"]}))
                if lane == "d2-on":
                    runners = member(raw, "dense-launches")["runners"]
                    assert len(runners) == 2 and {r["pid"] for r in runners} == {p for _, _, p in windows}
                    for r in runners:
                        options = r["options"]
                        assert options["MESH_SCHEDULER_MIN_WORKERS"] == options["MESH_SCHEDULER_MAX_WORKERS"] == "2"
                        assert options["MESH_PG_HOT_CHECKOUT"] == "1"
                    row["two_writers_per_runner_floor"] = minimum_events(windows, cap=2)
                    row["middle_two_writers_per_runner_floor"] = minimum_events(middle, cap=2)
            result.append(row)
    return result


def observed_arms():
    result = []
    for group, rows in sorted(json.loads(MARKERS.read_text()).items()):
        path = CALLERS / f"{int(group):02}-noop/measured-direct-observed/measured-mesh/raw.tar.gz"
        digest = sha256(path.read_bytes()).hexdigest()
        assert digest == rows["direct-observed"]["raw_sha256"]
        with tarfile.open(path) as raw:
            ledger = member(raw, "ledger")
            assert ledger["phase"] == "measured"
            loose = windows_from_ledger(ledger)
            identity = member(raw, "runner-identity")
            assert identity["scheduler_min"] == identity["scheduler_max"] == 2
            assert identity["mode"] == "direct-observed" and len(identity["runners"]) == 2
            jobs = {j["id"]: j for j in ledger["jobs"]}
            tight = []
            for number, runner in enumerate(identity["runners"]):
                for line in raw.extractfile(f"adapter/runner-{number}.log"):
                    if not line.startswith(b"APP_POOL_JOB "):
                        continue
                    w = json.loads(line.removeprefix(b"APP_POOL_JOB "))
                    j = jobs.pop(w["job_id"])
                    pid = runner["pid"]
                    assert pid == j["worker_pid"] and j["job_key"] == w["job_key"]
                    a, b = j["attempt_committed_us"], j["effect_committed_us"]
                    assert a <= w["first_return_us"] <= b <= w["second_return_us"] <= j["settled_us"]
                    tight.extend(((a, w["first_return_us"], pid), (b, w["second_return_us"], pid)))
            assert not jobs and len(tight) == 2048
        ledger_bound = minimum_events(loose, cap=2)
        caller_bound = minimum_events(tight, cap=2)
        assert ledger_bound <= caller_bound
        result.append(dict(group=int(group), raw_sha256=digest, ledger_floor=ledger_bound,
                           caller_return_floor=caller_bound))
    return result


if __name__ == "__main__":
    print(json.dumps(dict(wal_measurements=wal_arms(), observed_direct_handlers=observed_arms()),
                     indent=2, sort_keys=True))
