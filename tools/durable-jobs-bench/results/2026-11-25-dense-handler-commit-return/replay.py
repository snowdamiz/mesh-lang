"""Join dense application commit records to the corresponding compiled handler returns."""
import argparse
from collections import defaultdict
from hashlib import sha256
import json
from pathlib import Path
from statistics import median
import tarfile

HERE = Path(__file__).resolve().parent
SOURCE = HERE.parent / "2026-11-24-trusted-handler-app-pool/timing-clean-identity-retry/evidence/timing"
ARMS = ("service-observed", "direct-observed")


def overlap(intervals, start, end, runners):
    # Clip first so every span lies inside the same deployment-wide middle window.
    events = defaultdict(list)
    for pid, kind, left, right in intervals:
        left, right = max(start, left), min(end, right)
        if left < right:
            events[left].append((pid, kind, 1))
            events[right].append((pid, kind, -1))
    counts = {pid: {"before": 0, "after": 0, "lookup": 0} for pid in runners}
    last = start
    either = both = lookup = runner_time = 0
    for when in sorted(set(events) | {end}):
        dt = when - last
        qualified = [c for c in counts.values() if c["after"] >= 2 and c["before"] == 0]
        runner_time += dt * len(qualified)
        either += dt * bool(qualified)
        both += dt * (len(qualified) == len(runners))
        lookup += dt * sum(c["lookup"] > 0 for c in qualified)
        for pid, kind, change in events[when]:
            counts[pid][kind] += change
            assert counts[pid][kind] >= 0
        last = when
    assert all(not any(c.values()) for c in counts.values())
    assert both <= either <= end - start and lookup <= runner_time
    assert runner_time == either + both  # Two runners, with concurrent time counted twice.
    return dict(middle_us=end-start, either_runner_us=either, both_runners_us=both,
                combined_runner_us=runner_time, combined_with_lookup_us=lookup)


def clocks(block, mode, score):
    path = block / ("measured-" + mode) / "measured-mesh/raw.tar.gz"
    digest = sha256(path.read_bytes()).hexdigest()
    assert digest == score["raw_sha256"]
    with tarfile.open(path) as archive:
        ledger = json.load(archive.extractfile("adapter/ledger.json"))
    tx, = ledger["transactions"]
    jobs = ledger["jobs"]
    last = max(j["settled_us"] for j in jobs)
    assert len(jobs) == len({j["job_key"] for j in jobs}) == 1024
    assert all(j["state"] == "completed" for j in jobs)
    assert len({j[k] for j in jobs for k in ("attempt_xid", "effect_xid", "settled_xid")}) == 3072
    result = dict(submit_ms=(last-tx["submitted_us"])/1000, drain_ms=(last-tx["committed_us"])/1000,
                  first_commit_ms=median((j["attempt_committed_us"]-j["entered_us"])/1000 for j in jobs))
    assert result["submit_ms"] == score["submit_ms"] and result["drain_ms"] == score["drain_ms"]
    assert abs(result["first_commit_ms"] - score["entry_to_attempt_commit_p50_ms"]) < 1e-6
    return {**result, "raw_sha256": digest}


def read_arm(block, mode, score):
    path = block / ("measured-" + mode) / "measured-mesh/raw.tar.gz"
    digest = sha256(path.read_bytes()).hexdigest()
    assert digest == score["raw_sha256"]
    with tarfile.open(path) as archive:
        def read(name):
            return json.load(archive.extractfile("adapter/" + name + ".json"))
        ledger = read("ledger")
        identity = read("runner-identity")
        pids = [r["pid"] for r in identity["runners"]]
        assert len(set(pids)) == 2 and identity["scheduler_min"] == identity["scheduler_max"] == 2
        assert identity["mode"] == mode and identity["slots_per_runner"] == 32
        assert ledger["phase"] == "measured" and ledger["profile"] == "noop"
        db = ledger["database"]
        assert db["fsync"] and db["synchronous_commit"] and db["tls"] and db["version"] == "17.11"
        by_id = {j["id"]: j for j in ledger["jobs"]}
        seen, xids, writes, intervals = set(), set(), [], []
        for index, pid in enumerate(pids):
            for line in archive.extractfile(f"adapter/runner-{index}.log"):
                if not line.startswith(b"APP_POOL_JOB "):
                    continue
                w = json.loads(line.removeprefix(b"APP_POOL_JOB "))
                j = by_id[w["job_id"]]
                assert j["id"] not in seen and j["worker_pid"] == pid and j["job_key"] == w["job_key"]
                assert j["attempt"] == w["attempt"] == j["effects"] == j["attempts"] == 1
                assert j["state"] == "completed" and j["entered_us"] == w["entered_us"]
                seen.add(j["id"])
                xids.update(j[k] for k in ("attempt_xid", "effect_xid", "settled_xid"))
                begin, ready = w["lookup_begin_us"], w["lookup_end_us"]
                assert j["entered_us"] <= begin <= ready and w["second_return_us"] <= j["settled_us"]
                intervals.append((pid, "lookup", begin, ready))
                for first, stamp, last in ((ready, j["attempt_committed_us"], w["first_return_us"]),
                                           (w["first_return_us"], j["effect_committed_us"], w["second_return_us"])):
                    assert first <= stamp <= last
                    writes.append((stamp - first, last - stamp))
                    intervals.extend(((pid, "before", first, stamp), (pid, "after", stamp, last)))
    assert seen == by_id.keys() and len(seen) == 1024 and len(xids) == 3072 and len(writes) == 2048
    assert score["terminal_transactions"] == 1024 and score["app_sql_calls"] == 4096
    effects = sorted(j["effect_committed_us"] for j in by_id.values())
    middle = overlap(intervals, effects[102], effects[921], pids)
    pre, post = zip(*writes)
    return dict(raw_sha256=digest, completed=len(seen), writes=len(writes),
                write_window_us=sum(pre)+sum(post), record_to_return_us=sum(post),
                record_to_return_share=round(sum(post)/(sum(pre)+sum(post)), 6),
                before_record_p50_us=median(pre), record_to_return_p50_us=median(post), **middle)


def replay(source, groups):
    out = {}
    for group in groups:
        block = source / f"{group:02}-noop"
        scored = json.loads((block / "reconciled.json").read_text())
        assert scored["group"] == group and scored["profile"] == "noop"
        arms = scored["measured"]
        rows = {}
        for mode in ARMS:
            result = read_arm(block, mode, arms[mode])
            controls = ("direct",) if mode == "direct-observed" else ("service-a", "service-b")
            result["observer_drain_deltas_ms"] = [round(arms[mode]["drain_ms"] - arms[c]["drain_ms"], 3)
                                                   for c in controls]
            rows[mode] = result
        rows["unobserved"] = {name: clocks(block, name, arms[name])
                              for name in ("direct", "service-a", "service-b")}
        out[group] = rows
    return out


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, default=SOURCE)
    parser.add_argument("groups", nargs="*", type=int, default=list(range(1, 7)))
    args = parser.parse_args()
    print(json.dumps(replay(args.source, args.groups), indent=2, sort_keys=True))
