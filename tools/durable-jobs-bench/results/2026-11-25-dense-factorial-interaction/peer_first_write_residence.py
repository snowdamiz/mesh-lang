"""Summarize in-flight first application writes over each lane's own middle first writes."""
import json
from hashlib import sha256
from io import BytesIO
import tarfile
from collections import Counter, defaultdict
from statistics import median

from peer_grant_progress import EVIDENCE, SEAL, load


def measure(jobs):
    arrivals = sorted(j["attempt_committed_us"] for j in jobs)
    start, end = arrivals[255], arrivals[767]
    duration = end - start
    changes = defaultdict(Counter)
    changes[start]
    changes[end]
    residence = []
    for job in jobs:
        entered, first = job["entered_us"], job["attempt_committed_us"]
        if start < first <= end:
            residence.append(first - entered)
        left, right = max(entered, start), min(first, end)
        if left < right:
            pid = job["worker_pid"]
            changes[left][pid] += 1
            changes[right][pid] -= 1
    active = Counter()
    histogram = Counter()
    job_us = 0
    previous = start
    for when, delta in sorted(changes.items()):
        elapsed = when - previous
        histogram[sum(n > 0 for n in active.values())] += elapsed
        job_us += elapsed * sum(active.values())
        active.update(delta)
        assert all(n >= 0 for n in active.values())
        previous = when
    assert not any(active.values()) and sum(histogram.values()) == duration
    return {"middle_ms": duration / 1000, "mean_jobs": round(job_us / duration, 3),
            "mean_pids": round(sum(k*n for k,n in histogram.items()) / duration, 3),
            "no_entered_job_ms": histogram[0] / 1000, "writes_in_half_open_window": len(residence),
            "clipped_residence_per_512_ms": job_us / 512000,
            "median_completed_residence_ms": median(residence) / 1000,
            "pid_duration_us": dict(sorted(histogram.items()))}


def main():
    release = json.loads((EVIDENCE / "controller-release.json").read_bytes())
    assert release["request_sha256"] == SEAL and release["exit_status"] == 0
    summary = json.loads((EVIDENCE / "scored-before-release.json").read_bytes())["groups"]
    groups = []
    for i in range(1, 6):
        block = EVIDENCE / "blocks" / f"{i:02}-noop"
        scores = json.loads((block / "reconciled.json").read_bytes())
        assert scores == summary[str(i)]
        group = {}
        for lane in ("B", "A1", "A2", "river", "oban"):
            score = scores["measured-" + lane]
            jobs, _, _ = load(block, lane, score)
            if lane == "river":
                raw, = block.glob("measured-river/measured-go/raw.tar.gz")
                data = raw.read_bytes()
                assert sha256(data).hexdigest() == score["raw_sha256"]
                with tarfile.open(fileobj=BytesIO(data)) as tar:
                    physical = json.load(tar.extractfile("adapter/river/measured/ledger.json"))
                attempts = {(r["job_key"], str(r["xid"])): r for r in physical["attempts"]}
                assert len(attempts) == len(jobs) == len(physical["attempts"])
                for job in jobs:
                    entry = attempts.pop((job["job_key"], str(job["attempt_xid"])))
                    job["worker_pid"] = entry["worker_pid"]
                assert not attempts
            assert len({j["worker_pid"] for j in jobs}) == 8
            group[lane] = dict(raw_sha256=score["raw_sha256"], **measure(jobs))
        raw, = block.glob("measured-B/measured-mesh/raw.tar.gz")
        data = raw.read_bytes()
        assert sha256(data).hexdigest() == group["B"]["raw_sha256"]
        with tarfile.open(fileobj=BytesIO(data)) as tar:
            stmts = json.load(tar.extractfile("adapter/trusted-application-statements.json"))
        attempt, = [s for s in stmts if s["query"].startswith("INSERT INTO public.mesh_jobs_parity_attempts")]
        assert "ON CONFLICT (namespace, job_id, job_generation, attempt) DO NOTHING" in attempt["query"]
        assert attempt["calls"] == attempt["rows"] == 1024
        group["B"]["whole_attempt_executor_ms"] = attempt["execution_ms"]
        groups.append(group)
    print(json.dumps(groups, sort_keys=True, indent=2))


if __name__ == "__main__":
    main()
