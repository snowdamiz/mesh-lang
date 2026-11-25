"""Price parent-seat-only replacement at the released eight-runner cohort sizes."""
from collections import Counter, defaultdict
import hashlib
import io
import json
from pathlib import Path
import subprocess
import tarfile


COMMIT = "dad83049a5d35f03dd22e8d59c14b9b79db1679b"
PILOT = "tools/durable-jobs-bench/results/2026-11-25-public-dense-8x8/path-recovery/evidence/pilot"


def member(path):
    return subprocess.check_output(["git", "show", f"{COMMIT}:{PILOT}/{path}"],
                                  cwd=Path(__file__).resolve().parent)


def measure(group, score):
    data = member(f"blocks/{group:02}-noop/measured-B/measured-mesh/raw.tar.gz")
    assert hashlib.sha256(data).hexdigest() == score["raw_sha256"]
    with tarfile.open(fileobj=io.BytesIO(data)) as archive:
        ledger = json.load(archive.extractfile("adapter/ledger.json"))
        policy = json.load(archive.extractfile("adapter/queue-policy.json"))
    jobs = ledger["jobs"]
    assert ledger["runners"] == 8 and ledger["slots"] == 64 and len(jobs) == 1024
    assert len({j[k] for j in jobs for k in ("attempt_xid", "effect_xid")}) == 2048
    claims = Counter((job["worker_pid"], job["claimed_us"]) for job in jobs)
    by_runner = defaultdict(Counter)
    for job in jobs:
        assert job["state"] == "completed" and job["attempts"] == job["effects"] == 1
        by_runner[job["worker_pid"]][job["settled_xid"]] += 1
    assert len(by_runner) == 8
    sizes = [width for cohorts in by_runner.values() for width in cohorts.values()]
    assert len(sizes) == score["terminal_xids"] == len({j["settled_xid"] for j in jobs})
    assert Counter(sizes) == {int(k): v for k, v in score["terminal_widths"].items()}
    query, = [q for q in policy["queue_statements"] if "FROM mesh_jobs.claim_batch" in q["query"]]
    assert query["calls"] - (query["rows"] - len(jobs)) == len(claims)
    replacement_calls = 0
    replacement_seats = 0
    for cohorts in by_runner.values():
        # Allow eight free initial seats on every runner. Ignore time, lease
        # expiry, queue ordering and the actual availability of further work.
        demand = sum(cohorts.values()) - 8
        assert demand > 0
        replacement_seats += demand
        for width in sorted(cohorts.values(), reverse=True):
            replacement_calls += 1
            demand -= width
            if demand <= 0:
                break
    assert replacement_seats == 960
    return dict(raw_sha256=score["raw_sha256"], terminal_xids=len(sizes),
                terminal_widths=dict(sorted(Counter(sizes).items())), full_eight_cohorts=sizes.count(8),
                observed_positive_stamp_groups=len(claims), optimistic_replacement_grants=replacement_calls,
                minimum_grant_bearing_turns=8 + replacement_calls,
                normalized_effect_to_terminal_ms=sum(j["settled_us"] - j["effect_committed_us"] for j in jobs)/64000)


def replay():
    assert json.loads(member("controller-release.json"))["exit_status"] == 0
    score = json.loads(member("replay.json"))
    assert score["advance_to_peers"]
    return {group: measure(group, score["groups"][str(group)]["measured-B"]) for group in (1, 2)}


if __name__ == "__main__":
    print(json.dumps(replay(), indent=2, sort_keys=True))
