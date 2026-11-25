"""Reconcile matched public 8×8 effect progress on both complete peer rotations."""
from hashlib import sha256
import io
import json
from pathlib import Path
import subprocess
import tarfile

ROOT = "tools/durable-jobs-bench/results/2026-11-25-public-dense-8x8/path-recovery/peer-stage-recovery"
COMMIT = "893710708c7828e50200dbbca6022320f8916dd1"


def blob(relative):
    return subprocess.check_output(["git", "show", f"{COMMIT}:{ROOT}/{relative}"],
                                   cwd=Path(__file__).resolve().parent)


def progress(data, score):
    assert sha256(data).hexdigest() == score["raw_sha256"]
    with tarfile.open(fileobj=io.BytesIO(data)) as raw:
        assert json.load(raw.extractfile("status.json"))["exit_code"] == 0
        ledger = json.load(raw.extractfile("adapter/ledger.json"))
    jobs = ledger["jobs"]
    tx, = ledger["transactions"]
    assert ledger["slots"] == 64 and ledger["runners"] == 8 and len(jobs) == 1024
    assert len({j["job_key"] for j in jobs}) == 1024
    assert len({j[k] for j in jobs for k in ("attempt_xid", "effect_xid")}) == 2048
    for j in jobs:
        assert j["state"] == "completed" and j["attempt"] == j["effects"] == j["attempts"] == 1
        assert tx["committed_us"] <= j["attempt_committed_us"] <= j["effect_committed_us"] <= j["settled_us"]
    writes = sorted(j["effect_committed_us"] for j in jobs)
    last = max(j["settled_us"] for j in jobs)
    times = [tx["submitted_us"], tx["committed_us"], writes[63], writes[767], writes[-1], last]
    names = ["enqueue", "first_64", "next_704", "last_256", "terminal_tail"]
    phases = {k: b - a for k, a, b in zip(names, times, times[1:])}
    assert sum(phases.values()) == score["submit_us"] and last - times[1] == score["drain_us"]
    return {"raw_sha256": score["raw_sha256"], "phases_us": phases}


def replay():
    data = []
    for n, prefix in enumerate(("", "reverse-continuation/"), 1):
        base = f"{prefix}evidence/peers"
        release = json.loads(blob(f"{base}/controller-release.json"))
        assert release["exit_status"] == (1 if n == 1 else 0)
        block = f"{base}/blocks/{n:02}-noop"
        scores = json.loads(blob(f"{block}/reconciled.json"))
        results = {}
        for lane, engine in (("B", "mesh"), ("river", "go"), ("oban", "oban"), ("apalis", "rust")):
            cell = f"measured-{lane}"
            results[lane] = progress(blob(f"{block}/{cell}/measured-{engine}/raw.tar.gz"), scores[cell])
        data.append(results)
    return data


if __name__ == "__main__":
    print(json.dumps(replay(), indent=2, sort_keys=True))
