"""Reconcile the eight checked grants against the historical FILE drain cutoff."""
from hashlib import sha256
import json
from pathlib import Path
import subprocess
import sys
import tarfile

from activation_arrival import arrival_bounds

CUSTODY = "b5b444052e2729d6038aa552a55ddfa69c33c1a4"
PACKET = "tools/durable-jobs-bench/results/2026-11-25-public-large-external-prepared-4m-16m-stage-recovery"


def packed_activation(owners, cutoff):
    """Keep the observed activation order, interior durations and post-receipt tails."""
    ordered = sorted(owners, key=lambda owner: owner["first_activation_stamp_us"])
    occupied = 0
    finish = 0
    previous_last = -1
    for owner in ordered:
        first, last = owner["first_activation_stamp_us"], owner["last_activation_stamp_us"]
        assert previous_last <= first <= last <= owner["confirmed_us"]
        previous_last = last
        # The database floors each stamp to a millisecond. Round the span down.
        occupied += max(0, last - first - 1000)
        tail = owner["last_settlement_us"] - owner["confirmed_us"]
        finish = max(finish, occupied + tail)
    return dict(interior_total_us=occupied,
                first_observed_stamp_us=ordered[0]["first_activation_stamp_us"],
                packed_finish_from_first_stamp_us=finish,
                latest_first_stamp_for_cutoff_us=cutoff - finish)


def two_activation_lanes(turns):
    first_stamp = turns[0]["first_activation_stamp_us"]
    estimates = {}
    for endpoint in ("last_activation_stamp_us", "confirmed_us"):
        available = [first_stamp, first_stamp]
        last_finish = 0
        for turn in turns:
            lane = available.index(min(available))
            duration = turn[endpoint] - turn["first_activation_stamp_us"]
            if endpoint == "last_activation_stamp_us":
                duration = max(0, duration - 1000)
            available[lane] += duration
            tail = turn["last_settlement_us"] - turn["confirmed_us"]
            last_finish = max(last_finish, available[lane] + tail)
        estimates[endpoint] = last_finish
    return estimates


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
        notification_scans = []
        for i, runner in enumerate(runners):
            ready, result = archive.extractfile(f"adapter/runner-{i}.log").read().splitlines()
            report = json.loads(result)
            assert ready == b"READY" and report["drained"] and not report["last_error"]
            assert report["batch_protocol"] == "locked_prepare_pg17_v1"
            notification_scans.append(report["wakeups"]["notification_scans"])
            owned = [job for job in jobs if job["worker_pid"] == runner["pid"]]
            assert len(owned) == report["claimed"] == report["finished"] == 8
            receipt = report["last_batch_confirmed_us"]
            for job in owned:
                assert zero <= job["claimed_us"] <= receipt <= job["entered_us"]
                assert job["entered_us"] <= job["attempt_committed_us"] <= job["effect_committed_us"] <= job["settled_us"]
            owners.append(dict(pid=runner["pid"], confirmed_us=receipt-zero,
                               first_activation_stamp_us=min(job["claimed_us"] for job in owned)-zero,
                               last_activation_stamp_us=max(job["claimed_us"] for job in owned)-zero,
                               last_effect_us=max(job["effect_committed_us"] for job in owned)-zero,
                               last_settlement_us=max(job["settled_us"] for job in owned)-zero))
        first = min(owner["first_activation_stamp_us"] for owner in owners)
        pg = read("workmem-observation")["postgres"]
        interior_samples = [p for p in pg["points"] if zero <= p["begin_us"] <= p["end_us"] <= zero + first]
        crossing, = [p for p in pg["points"] if zero < p["begin_us"] <= zero + first
                     and zero + first + 1000 <= p["end_us"]]
        first_activation_observation = dict(
            notification_scans_per_runner=notification_scans,
            completed_pg_samples_before_first_stamp=len(interior_samples),
            crossing_sample_begin_us=crossing["begin_us"] - zero,
            crossing_sample_end_us=crossing["end_us"] - zero,
            crossing_sample_processes=crossing["processes"])
    owners.sort(key=lambda o: o["confirmed_us"])
    activation_turns = sorted((o["first_activation_stamp_us"], o["last_activation_stamp_us"],
                               o["confirmed_us"]) for o in owners)
    confirmed_late = sum(8 for owner in owners if owner["confirmed_us"] > cutoff)
    effect_late = sum(job["effect_committed_us"] - zero > cutoff for job in jobs)
    terminal_late = sum(job["settled_us"] - zero > cutoff for job in jobs)
    assert confirmed_late <= effect_late <= terminal_late
    return dict(raw_sha256=arm["raw_sha256"], owners=owners, unconfirmed_jobs=confirmed_late,
                activation_arrival=arrival_bounds(activation["execution_ms"], activation_turns),
                two_lanes_all_ready_at_first_stamp_finish_us=two_activation_lanes(
                    sorted(owners, key=lambda o: o["first_activation_stamp_us"])),
                packed_activation=packed_activation(owners, cutoff),
                first_activation_observation=first_activation_observation,
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
