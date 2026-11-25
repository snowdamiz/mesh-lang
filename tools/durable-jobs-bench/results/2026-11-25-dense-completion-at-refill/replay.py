"""Replay the recorded dense clocks; no database or host access."""
from bisect import bisect_right
from collections import Counter, defaultdict
import hashlib
import json
from pathlib import Path
import tarfile

HERE = Path(__file__).resolve().parent
NATIVE = HERE.parent / "2026-11-24-dense-individual-hot-pool/logger-off-retry/evidence/native"
LIMIT = 32
WIDTH = 8


def merge_timeline(jobs, hold_us, flush_on_claim):
    # Settlement stamps witness that teardown was already complete. Holding a
    # receipt in this model never makes it available before that stamp.
    claims = Counter(job["claimed_us"] for job in jobs)
    ready = Counter(job["settled_us"] for job in jobs)
    assert sum(ready.values()) == sum(claims.values())
    arrivals = sorted(x for x in ready.elements())
    events = sorted(set(claims) | set(ready))
    pending = []
    releases = []
    cumulative_claims = 0
    pressure = missed_seats = 0
    for stamp in events:
        if pending and pending[0] + hold_us <= stamp:
            releases.append((pending[0] + hold_us, len(pending)))
            pending.clear()
        if claims[stamp]:
            if flush_on_claim and pending:
                releases.append((stamp, len(pending)))
                pending.clear()
            cumulative_claims += claims[stamp]
            # Include stock dispositions stamped at the same time as the
            # grant only in the stock feasibility check.
            assert cumulative_claims - bisect_right(arrivals, stamp) <= LIMIT
            short = max(0, cumulative_claims - sum(n for _, n in releases) - LIMIT)
            pressure += bool(short)
            missed_seats += min(claims[stamp], short)
        for _ in range(ready[stamp]):
            pending.append(stamp)
            if len(pending) == WIDTH or hold_us == 0:
                releases.append((stamp, len(pending)))
                pending.clear()
    if pending:
        releases.append((pending[0] + hold_us, len(pending)))
    assert sum(n for _, n in releases) == len(jobs)
    return dict(groups=len(releases), grant_stamps=len(claims), grant_stamps_over_cap=pressure,
                seats_unavailable_at_original_grants=missed_seats)


def individual_series():
    series = json.loads((NATIVE / "summary-1795570643531732000.json").read_text())
    assert series["qualified_rotations"] == 6 and not series["missing"] and not series["failures"]
    results = {}
    for rotation, block in sorted(series["blocks"].items()):
        arms = {}
        for key, score in sorted(block["scores"].items()):
            if not key.startswith("measured-"):
                continue
            path, = (NATIVE / "blocks" / block["attempt"] / key).glob("*/raw.tar.gz")
            assert hashlib.sha256(path.read_bytes()).hexdigest() == score["raw_sha256"]
            with tarfile.open(path) as raw:
                ledger = json.load(raw.extractfile("adapter/ledger.json"))
            jobs = ledger["jobs"]
            assert len(jobs) == score["completed"] == 1024 and score["failed"] == 0
            assert len({j[k] for j in jobs for k in ("attempt_xid", "effect_xid")}) == 2048
            effects = sorted(j["effect_committed_us"] for j in jobs)
            q = effects[255]
            terminal = max(j["settled_us"] for j in jobs)
            assert score["last_settlement_us"] == terminal
            row = dict(raw_sha256=score["raw_sha256"], submit_to_q256_us=q-score["first_submit_us"],
                       enqueue_commit_to_q256_us=q-score["first_commit_us"],
                       q256_to_last_effect_us=effects[-1]-q, last_effect_to_last_settlement_us=terminal-effects[-1],
                       settled_at_q256=sum(j["settled_us"] <= q for j in jobs))
            if key in ("measured-hot", "measured-checked-a", "measured-checked-b"):
                assert len({j["settled_xid"] for j in jobs}) == 1024 and ledger["slots"] == 64
                by_pid = defaultdict(list)
                for j in jobs:
                    assert j["claimed_us"] <= j["entered_us"] <= j["attempt_committed_us"] < j["effect_committed_us"] <= j["settled_us"]
                    by_pid[j["worker_pid"]].append(j)
                assert len(by_pid) == ledger["runners"] == 2
                models = {}
                for hold in (0, 1000, 4000, 8000):
                    for flush in (False, True):
                        name = f"hold_{hold}_us" + ("_flush_on_claim" if flush else "")
                        counters = Counter()
                        for own in by_pid.values():
                            counters.update(merge_timeline(own, hold, flush))
                        models[name] = dict(counters)
                assert models["hold_0_us"]["groups"] == 1024
                row["models"] = models
            arms[key.removeprefix("measured-")] = row
        results[rotation] = arms
    return results


def grouped_series():
    native = HERE.parent / "2026-11-24-dense-product-d2-transfer/evidence/native-closure-retry"
    series = json.loads((native / "summary-1795560178774805000.json").read_text())
    assert series["qualified_rotations"] == 6 and not series["missing"] and not series["failures"]
    groups = {}
    for g, block in sorted(series["blocks"].items()):
        score = block["scores"]["measured-hot"]
        archive = native / "blocks" / block["attempt"] / "measured-hot/measured-mesh/raw.tar.gz"
        assert hashlib.sha256(archive.read_bytes()).hexdigest() == score["raw_sha256"]
        with tarfile.open(archive) as raw:
            ledger = json.load(raw.extractfile("adapter/ledger.json"))
        jobs = ledger["jobs"]
        assert len(jobs) == score["completed"] == 1024 and score["failed"] == 0
        by_pid = defaultdict(list)
        xid_pids = defaultdict(set)
        for job in jobs:
            by_pid[job["worker_pid"]].append(job)
            xid_pids[job["settled_xid"]].add(job["worker_pid"])
        assert len(by_pid) == 2 and all(len(pids) == 1 for pids in xid_pids.values())
        assert len({job[k] for job in jobs for k in ("attempt_xid", "effect_xid")}) == 2048
        assert len(xid_pids) < len(jobs)
        models = {}
        for flush in (False, True):
            result = Counter()
            for own in by_pid.values():
                result.update(merge_timeline(own, 8000, flush))
            models["flush_on_claim" if flush else "fixed_hold"] = dict(result)
        groups[g] = dict(actual_terminal_xids=len(xid_pids), raw_sha256=score["raw_sha256"], models=models)
    return groups


def replay():
    return dict(individual=individual_series(), grouped=grouped_series())


if __name__ == "__main__":
    print(json.dumps(replay(), indent=2, sort_keys=True))
