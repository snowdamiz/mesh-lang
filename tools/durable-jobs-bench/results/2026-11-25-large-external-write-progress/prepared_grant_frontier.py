"""Reconcile the eight checked grants against the historical FILE drain cutoff."""
from hashlib import sha256
import json
from pathlib import Path
import subprocess
import sys
import tarfile


CUSTODY = "b5b444052e2729d6038aa552a55ddfa69c33c1a4"
PACKET = "tools/durable-jobs-bench/results/2026-11-25-public-large-external-prepared-4m-16m-stage-recovery"


def measure(raw, arm, cutoff):
    assert sha256(raw.read_bytes()).hexdigest() == arm["raw_sha256"]
    with tarfile.open(raw) as archive:
        def read(name):
            return json.load(archive.extractfile("adapter/" + name + ".json"))

        ledger = read("ledger")
        transaction, = ledger["transactions"]
        zero = transaction["committed_us"]
        jobs = ledger["jobs"]
        assert transaction["enqueued_ids"] == {job["job_key"]: job["id"] for job in jobs}
        assert len(jobs) == len({job["id"] for job in jobs}) == 64
        _, *runners = read("process-identity")
        assert len(runners) == len({runner["pid"] for runner in runners}) == 8
        activation, = [q for q in read("queue-policy")["queue_statements"] if "activate_batch_v1(" in q["query"]]
        assert activation["rows"] == 72 and activation["calls"] == 8
        owners = []
        for i, runner in enumerate(runners):
            ready, result = archive.extractfile(f"adapter/runner-{i}.log").read().splitlines()
            report = json.loads(result)
            assert ready == b"READY" and report["drained"] and not report["last_error"]
            assert report["batch_protocol"] == "locked_prepare_pg17_v1"
            owned = [job for job in jobs if job["worker_pid"] == runner["pid"]]
            assert len(owned) == report["claimed"] == report["finished"] == 8
            receipt = report["last_batch_confirmed_us"]
            for job in owned:
                assert zero <= job["claimed_us"] <= receipt <= job["entered_us"]
                assert job["entered_us"] <= job["attempt_committed_us"] <= job["effect_committed_us"] <= job["settled_us"]
            owners.append(dict(pid=runner["pid"], confirmed_us=receipt-zero,
                               last_effect_us=max(job["effect_committed_us"] for job in owned)-zero,
                               last_settlement_us=max(job["settled_us"] for job in owned)-zero))
    owners.sort(key=lambda o: o["confirmed_us"])
    confirmed_late = sum(8 for owner in owners if owner["confirmed_us"] > cutoff)
    effect_late = sum(job["effect_committed_us"] - zero > cutoff for job in jobs)
    terminal_late = sum(job["settled_us"] - zero > cutoff for job in jobs)
    assert confirmed_late <= effect_late <= terminal_late
    return dict(raw_sha256=arm["raw_sha256"], owners=owners, unconfirmed_jobs=confirmed_late,
                confirmed_without_effect=effect_late-confirmed_late,
                effect_without_settlement=terminal_late-effect_late, settled=64-terminal_late)


def replay(evidence):
    observed = json.loads(subprocess.check_output(
        ["git", "show", f"{CUSTODY}:{PACKET}/failed-setup-observations.json"]))
    assert observed["released_exit"] == 1
    cutoff = round(observed["best_historical_oban_20pct_drain_deadline_ms"] * 1000)
    results = {}
    for lane in ("prepared-4a", "prepared-16", "prepared-4b"):
        raw, = evidence.glob(f"blocks/*/diagnostic-{lane}/diagnostic-mesh/raw.tar.gz")
        results[lane] = measure(raw, observed["arms"][lane], cutoff)
    return dict(release_sha256=observed["release_sha256"], historical_cutoff_us=cutoff, arms=results)


if __name__ == "__main__":
    print(json.dumps(replay(Path(sys.argv[1])), indent=2))
