"""Price earlier grants against durable seat releases in the quiet R5 pilot."""
import argparse
import json
import tarfile
from collections import defaultdict
from io import BytesIO
from pathlib import Path

from direct4_peer_deadlines import warm_river
from direct4_quiet_cost import RELEASE, REPLAY, read


def arm(directory, name, score, hashes):
    path = f"{name}/measured-mesh/raw.tar.gz"
    with tarfile.open(fileobj=BytesIO(read(directory / path, hashes[path]))) as archive:
        ledger = json.load(archive.extractfile("adapter/ledger.json"))
        for index in range(2):
            log = archive.extractfile(f"adapter/runner-{index}.log").read().splitlines()
            state = json.loads(log[-1])["queues"]["default"]
            assert state["limit"] == 32 and state["drained"]
    jobs = ledger["jobs"]
    assert len(jobs) == len({job["job_key"] for job in jobs}) == 1024
    assert len({job[k] for k in ("attempt_xid", "effect_xid") for job in jobs}) == 2048
    assert all(job["effects"] == job["attempts"] == 1 for job in jobs)
    tx, = ledger["transactions"]
    origin = tx["committed_us"]
    settled = max(job["settled_us"] for job in jobs)
    assert settled - origin == score["drain_us"]
    assert settled - tx["submitted_us"] == score["submit_us"]
    return jobs, origin, origin - tx["submitted_us"]


def seats(lanes, seats_per_lane, origin, deadline, field):
    earliest = 0
    shortfall = 0
    for lane in lanes:
        times = sorted(job[field] - origin for job in lane)
        needed = max(0, len(times) - seats_per_lane)
        if needed:
            earliest = max(earliest, times[needed - 1])
            shortfall += max(0, needed - sum(t <= deadline for t in times))
    return {"additional_jobs_needed": shortfall, "earliest_last_claim_us": earliest,
            "lag_us": earliest - deadline}


def replay(directory):
    release = json.loads(read(directory / "controller-release.json", RELEASE))
    report = json.loads(read(directory / "controller/replay.json", REPLAY))
    assert report["complete"] and not report["errors"] and release["exit_status"] == 0
    assert report["request_sha256"] == release["request_sha256"]
    peers = warm_river()
    result = {}
    for block, phases in report["arms"].items():
        measured = phases["measured"]
        arms = {name: arm(directory, f"{block}/measured-{name}", score, report["evidence_sha256s"])
                for name, score in measured.items()}
        faster = min(("off-a", "off-b"), key=lambda name: measured[name]["drain_us"])
        control, baseline_origin, _ = arms[faster]
        control_deadline = max(job["claimed_us"] for job in control) + 999 - baseline_origin
        jobs, origin, enqueue = arms["on"]
        owners = defaultdict(list)
        for job in jobs:
            assert job["effect_committed_us"] <= job["settled_us"]
            owners[job["worker_pid"]].append(job)
        assert len(owners) == 2
        last_claim_end = max(job["claimed_us"] for job in jobs) + 999 - origin
        deadlines = {f"{faster}_last_stamp_upper": control_deadline}
        for group, peer in peers.items():
            deadlines[f"warm_river_{group}"] = min(peer["drain_us"], peer["submit_us"] - enqueue)
        rows = {}
        for label, deadline in deadlines.items():
            cases = {}
            for field in ("effect_committed_us", "settled_us"):
                fixed = seats(owners.values(), 32, origin, deadline, field)
                pooled = seats([jobs], 64, origin, deadline, field)
                assert pooled["earliest_last_claim_us"] <= fixed["earliest_last_claim_us"] <= last_claim_end
                assert pooled["additional_jobs_needed"] <= fixed["additional_jobs_needed"]
                cases[field] = {"fixed_owners": fixed, "pooled": pooled}
            rows[label] = {"deadline_us": deadline, **cases}
        result[block] = rows
    return result


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("pilot_directory", type=Path)
    print(json.dumps(replay(parser.parse_args().pilot_directory), sort_keys=True, indent=2))
