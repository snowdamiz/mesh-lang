"""Reconstruct bounds on native V1 ordinary claim batches from committed evidence."""

import hashlib
import json
import sys
import tarfile
from collections import Counter, defaultdict
from pathlib import Path


HERE = Path(__file__).resolve().parent
SOURCE = HERE.parent / "2026-11-23-ordinary-trusted-v1" / "evidence"
SERIES = {"dense": (1, 1024, 8, 8), "short": (5, 128, 1, 16), "slow": (5, 512, 1, 64)}


def minimum_turns(windows, width):
    """Each turn needs a common instant in at most width closed intervals."""
    # Sort by deadline. Place a turn at the first deadline and fill it with
    # eligible jobs in deadline order. See the exchange argument in README.
    pending = sorted(windows, key=lambda w: w[1])
    turns = 0
    while pending:
        deadline = pending[0][1]
        remaining = []
        count = 0
        for start, end in pending:
            if start > end:
                raise ValueError("claim timestamp exceeds handler entry")
            if start <= deadline and count < width:
                count += 1
            else:
                remaining.append((start, end))
        pending = remaining
        turns += 1
    return turns


def residence(jobs, start, end, stamp_delay_us):
    """Integrate per-PID claim-stamp to terminal-commit intervals on a fixed window."""
    changes = defaultdict(Counter)
    occupancy = {job["worker_pid"]: 0 for job in jobs}
    for job in jobs:
        pid = job["worker_pid"]
        stamp = min(job["claimed_us"] + stamp_delay_us, job["entered_us"])
        changes[stamp][pid] += 1
        changes[job["settled_us"]][pid] -= 1
    changes[end]  # Close the final integration interval.
    occupied_us = empty_runner_us = peak = 0
    previous = start
    for time, deltas in sorted(changes.items()):
        if time > end:
            break
        duration = max(0, time - max(start, previous))
        if duration:
            occupied = sum(occupancy.values())
            occupied_us += occupied * duration
            empty_runner_us += sum(value == 0 for value in occupancy.values()) * duration
            peak = max(peak, occupied)
        for pid, delta in deltas.items():
            occupancy[pid] += delta
        previous = time
    return {"occupied_job_us": occupied_us, "empty_runner_us": empty_runner_us,
            "peak_resident_jobs": peak}


def peer_budgets(mesh_path, ledger):
    jobs = ledger["jobs"]
    expected = {j["job_key"]: (j["payload_sha256"], j["hold_ms"]) for j in jobs}
    mesh_tx = ledger["transactions"][0]
    mesh_producer_us = mesh_tx["committed_us"] - mesh_tx["submitted_us"]
    job_us = [
        sum(j["settled_us"] - min(j["claimed_us"] + delay, j["entered_us"]) for j in jobs)
        for delay in (999, 0)
    ]
    result = {}
    for peer, engine in (("oban", "oban"), ("river", "go"), ("apalis", "rust")):
        path = mesh_path.parents[2] / f"measured-{peer}" / f"measured-{engine}" / "raw.tar.gz"
        with tarfile.open(path) as archive:
            assert json.load(archive.extractfile("status.json"))["exit_code"] == 0
            other = json.load(archive.extractfile("adapter/ledger.json"))
        request = json.loads((path.parent / "request.json").read_text())
        assert request["phase"] == "measured" and request["engine"] == other["engine"] == engine
        assert request["profile"] == other["profile"] == ledger["profile"]
        assert other["input_sha256"] == ledger["input_sha256"]
        assert len(other["jobs"]) == len(expected) and len(other["transactions"]) == 1
        assert all(j["state"] == "completed" for j in other["jobs"])
        assert {j["job_key"]: (j["payload_sha256"], j["hold_ms"]) for j in other["jobs"]} == expected
        tx = other["transactions"][0]
        terminal = max(j["settled_us"] for j in other["jobs"])
        horizons = {
            "drain": terminal - tx["committed_us"],
            "full_with_mesh_producer": terminal - tx["submitted_us"] - mesh_producer_us,
        }
        budgets = {}
        for label, horizon in horizons.items():
            budgets[label] = {
                "available_after_commit_us": horizon,
                "required_mean_jobs": [round(s / horizon, 4) for s in job_us] if horizon > 0 else None,
                "exceeds_configured_slots_at_lower_bound": horizon * ledger["slots"] < job_us[0],
            }
        result[peer] = {"sha256": hashlib.sha256(path.read_bytes()).hexdigest(), "windows": budgets}
    return {"mesh_producer_us": mesh_producer_us, "summed_job_residence_us": job_us, "peers": result}


def reduce_arm(path, expected_jobs, runners, limit):
    def read_json(name):
        return json.load(archive.extractfile(name))

    with tarfile.open(path) as archive:
        ledger = read_json("adapter/ledger.json")
        raw = read_json("adapter/raw.json")
        summary = read_json("adapter/row.json")
        affinity = read_json("adapter/process-affinity.json")
        assert read_json("status.json")["exit_code"] == 0
        reports = []
        for ix in range(runners):
            lines = archive.extractfile(f"adapter/runner-{ix}.log").read().splitlines()
            reports.append(json.loads(lines[-1]))

    assert ledger["execution_contract"] == "ordinary-trusted-shared-process"
    assert ledger["phase"] == "measured" and not ledger["local_smoke"]
    assert ledger["engine"] == "mesh" and ledger["database"]["version"] == "17.11"
    assert ledger["database"]["tls"] and ledger["database"]["synchronous_commit"]
    assert ledger["runners"] == runners and ledger["slots"] == runners * limit
    jobs = ledger["jobs"]
    assert len(jobs) == expected_jobs == summary["completed"]
    assert len(ledger["transactions"]) == 1
    txn = ledger["transactions"][0]
    assert txn["enqueued_ids"] == {job["job_key"]: job["id"] for job in jobs}
    assert len(set(txn["enqueued_ids"].values())) == expected_jobs
    windows = defaultdict(list)
    xids = {txn["xid"]}
    for job in jobs:
        assert job["state"] == "completed" and job["attempts"] == job["effects"] == job["attempt"] == 1
        assert job["claimed_us"] % 1000 == 0 and job["claimed_us"] <= job["entered_us"]
        assert job["claimed_us"] + 999 >= txn["committed_us"]
        assert job["entered_us"] <= job["attempt_committed_us"] <= job["effect_committed_us"] <= job["settled_us"]
        windows[job["worker_pid"]].append((job["claimed_us"], job["entered_us"]))
        xids.update(job[key] for key in ("attempt_xid", "effect_xid", "settled_xid"))
    assert len(xids) == 1 + 3 * expected_jobs
    assert len(windows) == runners and all(affinity[str(pid)] == [0, 1] for pid in windows)
    assert sorted(map(len, windows.values())) == sorted(r["claimed"] for r in reports)
    for report in reports:
        assert report["queues"]["default"]["limit"] == limit
        assert report["claimed"] == report["finished"] == report["outcomes"]["completed"]
        assert report["drained"] and report["last_error"] is None
        assert not any(report[k] for k in ("undrained", "recovered", "stale_acks", "pending_claims"))
    calls = raw["raw_claim_calls"]
    hints = sum(r["batch_contention_hints"] for r in reports)
    # A hint query which saw the final ready rows may finish after the closing
    # counter snapshot. There is at most one such query per private claim lane.
    edge_allowance = sum(r["batch_contention_hints"] > 0 for r in reports)
    width = min(limit, 16)
    lower = sum(minimum_turns(w, width) for w in windows.values())
    upper = calls - hints + edge_allowance
    assert lower <= upper <= calls
    start = txn["committed_us"]
    last_stamp = max(job["claimed_us"] for job in jobs)
    assert start < last_stamp
    return {
        "archive": str(path.relative_to(SOURCE)), "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
        "jobs": expected_jobs, "width": width, "per_pid_jobs": sorted(map(len, windows.values())),
        "claim_statements": calls, "lifetime_hints": hints, "max_hints_outside_closing_snapshot": edge_allowance,
        "granting_statements_lower": lower, "granting_statements_upper": upper,
        "backlog_window_us": last_stamp - start,
        "residence_from_earliest_stamps": residence(jobs, start, last_stamp, 0),
        "residence_from_latest_stamps": residence(jobs, start, last_stamp, 999),
        "fixed_residence_budgets": peer_budgets(path, ledger),
    }


def main():
    result = {}
    for series, (groups, n, runners, limit) in SERIES.items():
        arms = []
        for group in range(1, groups + 1):
            paths = list((SOURCE / f"ordinary-trusted-v1-{series}" / "blocks").glob(
                f"{group:02}-*/measured-ordinary/measured-mesh/raw.tar.gz"))
            if len(paths) != 1:
                raise ValueError(f"expected one measured arm for {series} group {group}: {paths}")
            arms.append(reduce_arm(paths[0], n, runners, limit))
        result[series] = arms
    output = json.dumps(result, indent=2, sort_keys=True) + "\n"
    if sys.argv[1:] == ["--check"]:
        if output != (HERE / "result.json").read_text():
            raise SystemExit("replay differs from result.json")
        print("Verified all 11 measured arms.")
    elif not sys.argv[1:]:
        sys.stdout.write(output)
    else:
        raise SystemExit("usage: replay.py [--check]")


if __name__ == "__main__":
    main()
