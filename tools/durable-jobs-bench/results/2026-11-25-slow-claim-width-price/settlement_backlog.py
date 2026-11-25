"""Replay the durable second-write to scalar-settlement backlog in SLOW group 1/2."""
from collections import Counter
import hashlib
import io
import json
import subprocess
import tarfile

REV = "7df5b961c"
ROOT = ("tools/durable-jobs-bench/results/2026-11-25-public-slow-width16-logical-key-"
        "writer-recovery/continuation/evidence/native")


def blob(path):
    return subprocess.check_output(["git", "show", f"{REV}:{path}"])


def measure(jobs, start, end):
    changes = Counter({start: 0, end: 0})
    for job in jobs:
        left = max(start, job["effect_committed_us"])
        right = min(end, job["settled_us"])
        if left < right:
            changes[left] += 1
            changes[right] -= 1
    depth = area = total = longest = run = 0
    previous = start
    for stamp, delta in sorted(changes.items()):
        duration = stamp - previous
        area += depth * duration
        if depth >= 8:
            total += duration
            run += duration
            longest = max(longest, run)
        else:
            run = 0
        depth += delta
        assert 0 <= depth <= 64
        previous = stamp
    assert depth == 0
    return dict(span_us=end-start, pending_job_us=area,
                eight_or_more_us=total, longest_eight_or_more_us=longest)


def replay():
    groups = json.loads(blob(ROOT + "/replay.json"))["groups"]
    paths = subprocess.check_output(["git", "ls-tree", "-r", "--name-only", REV,
                                     ROOT + "/blocks"], text=True).splitlines()
    output = {}
    for group in ("1", "2"):
        output[group] = {}
        for lane in ("stock-a", "key", "stock-b"):
            suffix = f"/measured-{lane}/measured-mesh/raw.tar.gz"
            path, = [p for p in paths if f"/blocks/0{group}-" in p and p.endswith(suffix)]
            data = blob(path)
            score = groups[group]["measured"][lane]
            assert hashlib.sha256(data).hexdigest() == score["raw_sha256"]
            with tarfile.open(fileobj=io.BytesIO(data)) as raw:
                assert json.load(raw.extractfile("status.json"))["exit_code"] == 0
                ledger = json.load(raw.extractfile("adapter/ledger.json"))
            jobs = ledger["jobs"]
            assert ledger["slots"] == 64 and ledger["runners"] == 1
            assert len(jobs) == len({j["id"] for j in jobs}) == 512
            assert all(j["state"] == "completed" and j["effect_committed_us"] <= j["settled_us"]
                       for j in jobs)
            effects = sorted(j["effect_committed_us"] for j in jobs)
            full = measure(jobs, score["first_commit_us"], score["last_settlement_us"])
            assert full["span_us"] == score["drain_us"]
            assert full["pending_job_us"] == sum(j["settled_us"] - j["effect_committed_us"] for j in jobs)
            output[group][lane] = dict(raw_sha256=score["raw_sha256"], drain=full,
                                       middle=measure(jobs, effects[63], effects[447]))
    return output


if __name__ == "__main__":
    print(json.dumps(replay(), indent=2, sort_keys=True))
