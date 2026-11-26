"""Bracket entered handler coverage while a checked Direct4 batch awaits entry."""
import argparse
import hashlib
import io
import json
import subprocess
import tarfile
from collections import Counter, defaultdict
from pathlib import Path

from direct4_idle_budget import blob
from direct4_peer_deadlines import ledger, warm_river
from warm_river_occupancy import SERIES
from xid_bounds import brackets


def coverage(jobs):
    received = {}
    transactions = {}
    for job in jobs:
        key = job["worker_pid"], job["claimed_us"]
        entry = job["entered_us"]
        received[key] = min(received.get(key, entry), entry)
        first, effect = job["attempt_committed_us"], job["effect_committed_us"]
        for xid, lower, commit in ((job["attempt_xid"], entry, first),
                                   (job["effect_xid"], first, effect),
                                   (job["settled_xid"], effect, job["settled_us"])):
            xid = int(xid)
            old_lower, old_commit = transactions.get(xid, (lower, commit))
            assert old_commit == commit
            transactions[xid] = max(lower, old_lower), commit
    latest = {xid: upper for xid, _, _, _, _, upper in brackets(
        [(xid, None, lo, hi) for xid, (lo, hi) in transactions.items()])}
    effects = sorted(j["effect_committed_us"] for j in jobs)
    begin, end = effects[255], effects[767]
    result = {}
    for pid in sorted({j["worker_pid"] for j in jobs}):
        changes = defaultdict(Counter)
        for job in (j for j in jobs if j["worker_pid"] == pid):
            entry = job["entered_us"]
            record = job["effect_committed_us"]
            upper = latest[int(job["settled_xid"])]
            assert entry <= record <= upper <= job["settled_us"]
            spans = (("unentered", received[pid, job["claimed_us"]], entry),
                     ("effect", entry, record), ("plus_1ms", entry, min(record + 1000, upper)),
                     ("plus_2ms", entry, min(record + 2000, upper)), ("exit_upper", entry, upper))
            for name, start, stop in spans:
                start, stop = max(start, begin), min(stop, end)
                if start < stop:
                    changes[start][name] += 1
                    changes[stop][name] -= 1
        live = Counter()
        worker_us = Counter()
        previous = begin
        for stamp in sorted({begin, end, *changes}):
            for name in ("effect", "plus_1ms", "plus_2ms", "exit_upper"):
                uncovered = max(0, 4 - live[name])
                worker_us[name] += (stamp - previous) * min(uncovered, live["unentered"])
            live.update(changes[stamp])
            assert all(n >= 0 for n in live.values()) and live["effect"] <= 4
            previous = stamp
        assert not any(live.values())
        assert worker_us["effect"] >= worker_us["plus_1ms"] >= worker_us["plus_2ms"] >= worker_us["exit_upper"]
        result[pid] = dict(worker_us)
    assert len(result) == 2
    return result


def owner_write_overlap(jobs):
    """Time with at least five unfinished effects on one runner, using the quiet ledger."""
    effects = sorted(j["effect_committed_us"] for j in jobs)
    windows = {"full": (min(j["entered_us"] for j in jobs), effects[-1]),
               "middle": (effects[255], effects[767])}
    result = {}
    for pid in sorted({j["worker_pid"] for j in jobs}):
        own = [j for j in jobs if j["worker_pid"] == pid]
        result[pid] = {}
        for label, (begin, end) in windows.items():
            spans = {}
            for field in ("entered_us", "attempt_committed_us"):
                edges = Counter()
                for job in own:
                    lo = max(begin, job[field])
                    hi = min(end, job["effect_committed_us"])
                    if lo < hi:
                        edges[lo] += 1
                        edges[hi] -= 1
                active = peak = five_us = excess_job_us = 0
                previous = begin
                for stamp, change in sorted(edges.items()):
                    if active >= 5:
                        five_us += stamp - previous
                        excess_job_us += (active - 5) * (stamp - previous)
                    active += change
                    assert active >= 0
                    peak = max(peak, active)
                    previous = stamp
                assert active == 0
                spans[field] = dict(peak=peak, five_or_more_us=five_us, beyond_five_job_us=excess_job_us)
            assert spans["entered_us"]["five_or_more_us"] >= spans["attempt_committed_us"]["five_or_more_us"]
            result[pid][label] = spans
    assert len(result) == 2
    return result


def control_blocks(exposure):
    digest = lambda data: hashlib.sha256(data).hexdigest()
    release = (exposure / "controller/release.json").read_bytes()
    assert digest(release) == "139ebf36ffe58bb0b30aa88b63f0e8cb7017cf3dec27487359f82e76d90e11f1"
    proof = (exposure / "verified.json").read_bytes()
    assert digest(proof) == "bbfde59ae0e1850c2b1f73c9b7903dd11c2da06c1ad1fad56f3cafd3a3521bf8"
    verified = json.loads(proof)
    assert verified["release_sha256"] == digest(release)
    root = "tools/durable-jobs-bench/results/2026-11-25-direct4-hot-idle/evidence/native"
    for group in (1, 2):
        block = f"blocks/{group:02}-noop"
        for series, read in (("pool_supply", blob),
                             ("idle_spin", lambda p: subprocess.check_output(["git", "show", f"cb4f825c10:{root}/{p}"])),
                             ("event_park", lambda p: (exposure / p).read_bytes())):
            scores = json.loads(read(f"{block}/reconciled.json"))["measured"]
            pins = verified["raw_sha256"] if series == "event_park" else {}
            yield f"{series}-{group}", block, read, scores, pins


def controls(exposure):
    for name, block, read, scores, pins in control_blocks(exposure):
        path = f"{block}/measured-direct4/measured-mesh/raw.tar.gz"
        score = scores["direct4"]
        raw = read(path)
        jobs, txn = ledger(raw, score["raw_sha256"], topology=(2, 64))
        with tarfile.open(fileobj=io.BytesIO(raw)) as archive:
            processes = json.load(archive.extractfile("adapter/process-identity.json"))
            assert len(processes) == 3 and all(p["executable_sha256"] ==
                "e836b79a2dbd5ee0e10897de8d3d07fbf789ea038e122d2403799b9879bbbaf4" for p in processes)
            supply = json.load(archive.extractfile("adapter/pool-supply.json"))
            assert supply["binding"] == "direct" and supply["scheduler_workers"] == 4
        if pins:
            assert pins[path] == score["raw_sha256"]
        yield name, score, jobs, txn


def replay(exposure):
    return {name: dict(raw_sha256=score["raw_sha256"], worker_us=coverage(jobs),
                       write_overlap=owner_write_overlap(jobs))
            for name, score, jobs, _ in controls(exposure)}


def river_overlap():
    result = {}
    for group, score in warm_river().items():
        path, = SERIES.glob(f"blocks/{group:02}-*/measured-river-warm/*/raw.tar.gz")
        raw = path.read_bytes()
        jobs, _ = ledger(raw, score["raw_sha256"])
        with tarfile.open(fileobj=io.BytesIO(raw)) as archive:
            attempts = json.load(archive.extractfile("adapter/river/measured/ledger.json"))["attempts"]
        owners = {(row["job_key"], str(row["xid"])): row["worker_pid"] for row in attempts}
        assert len(owners) == len(attempts) == len(jobs)
        for job in jobs:
            job["worker_pid"] = owners.pop((job["job_key"], str(job["attempt_xid"])))
        assert not owners
        result[group] = dict(raw_sha256=score["raw_sha256"], write_overlap=owner_write_overlap(jobs))
    return result


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("exposure", type=Path)
    parser.add_argument("--warm-river", action="store_true", help="also join the six warm River owner ledgers")
    args = parser.parse_args()
    results = replay(args.exposure)
    if args.warm_river:
        results["warm_river"] = river_overlap()
    print(json.dumps(results, indent=2))
