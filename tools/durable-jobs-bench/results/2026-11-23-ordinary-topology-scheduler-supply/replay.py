"""Verify scheduler supply, reuse the sealed V1 ledger gate, and compute paired effects."""

import argparse
import json
from pathlib import Path
import runpy
from statistics import median
import sys
from types import ModuleType

from arm import ACTIVITY, CASES, ORDERS, PACKET, PRIOR, PRIMARY, ROOT


def prior_verifier():
    original = sys.modules["arm"]
    prior = ModuleType("arm")
    prior.__dict__.update(runpy.run_path(str(PRIOR / "arm.py")))
    sys.modules["arm"] = prior
    try:
        return runpy.run_path(str(PRIOR / "replay.py"))
    finally:
        sys.modules["arm"] = original


prior = prior_verifier()
read, sha, require, distribution = (prior[k] for k in ("read", "sha", "require", "distribution"))
verify_v1 = prior["verify_arm"]
verify_v1.__globals__["LAYOUTS"] = {label: case[:3] for label, case in CASES.items()}

CONTRASTS = {
    "duplicate-controls": ("stock-b", ("stock-a",)),
    "supply-two-primary": (PRIMARY, ("two-w2",)),
    "supply-one": ("one-w16", ("one-w2",)),
    "supply-two-low": ("two-w2", ("two-w1",)),
    "fixed-16-two-vs-stock": (PRIMARY, ("stock-a", "stock-b")),
    "fixed-16-one-vs-stock": ("one-w16", ("stock-a", "stock-b")),
    "fixed-16-one-vs-two": ("one-w16", (PRIMARY,)),
    "fixed-2-two-vs-one": ("two-w1", ("one-w2",)),
}
METRICS = ("submit_jobs_per_s", "commit_jobs_per_s", "first_submit_to_commit_ms",
           "runner_cpu_ms_per_job", "runner_bracket_cpu_ms_per_job", "runner_end_observer_lag_ms",
           "claim_calls", "admission_hints", "queue_calls", "app_calls",
           "used_application_backends", "active_application_backends_observed",
           "peak_active_app_inserts", "peak_active_claim_queries", "samples_with_claim_and_app_active",
           "warm_application_sessions", "warm_queue_sessions",
           "sampled_peak_slots", "mean_occupied_slots", "peak_grants_per_runner", "min_grants_per_runner",
           "entry_to_attempt_commit_p50_ms", "attempt_to_effect_commit_p50_ms",
           "effect_commit_to_settlement_p50_ms", "host_load_1m")


def validate_cpu(path, pids, score, calibration):
    host = read(path / "host-before.json")
    require(all(host[key] == calibration[key] for key in ("system", "kernel", "architecture")),
            "CPU calibration host differs")
    points = read(path / "runner-cpu-window.json")
    begin, end = (points[key] for key in ("before_submit", "terminal_observed"))
    require(begin["begin_us"] <= begin["end_us"] <= score["first_submit_us"]
            <= score["first_commit_us"] < score["last_settlement_us"]
            <= end["begin_us"] <= end["end_us"], "CPU samples do not bracket durable work")
    require(set(begin["pids"]) == set(end["pids"]) == pids, "CPU samples missed a runner")
    total = 0
    for pid in pids:
        start, stop = begin["pids"][pid], end["pids"][pid]
        require(start["start_abstime"] == stop["start_abstime"] > 0, "runner PID changed")
        for key in ("user_ns", "system_ns"):
            require(0 <= start[key] <= stop[key], "runner CPU went backwards")
            total += stop[key] - start[key]
    # The recorded field names describe the ABI slots. Live calibration
    # established they contain Mach absolute ticks on this arm64 host.
    ms_per_job = total * calibration["numer"] / calibration["denom"] / 1_000_000 / score["completed"]
    require(0 < ms_per_job <= score["runner_cpu_ms_per_job"] + 0.02,
            "Darwin CPU bracket exceeds lifetime runner-tree CPU")
    return ms_per_job, (end["end_us"] - score["last_settlement_us"]) / 1000


def validate_activity(path, mode, score):
    raw = read(path / "raw.json")
    samples = raw["samples"]
    for sample in samples + raw["idle_samples"]:
        require(all(isinstance(sample[name], list) for name in
                    ("app_active", "app_insert_active", "claim_active")), "missing activity sample")
    sessions = read(path / "sessions-before.json")
    queue = {row["backend"] for row in sessions if row["role"] == "parity_subject"}
    apps = set(score["used_application_backend_ids"])
    for sample in samples:
        require(set(sample["claim_active"]) <= queue
                and set(sample["app_insert_active"]) <= set(sample["app_active"]) <= apps,
                "query sample reflects an unexpected physical backend")
    claim = max(len(row["claim_active"]) for row in samples)
    require(claim <= score["layout"][0], "more than one active claim lane per runner")
    require(mode != "activity" or len(samples) >= 2, "activity observer did not repeat")
    return {
        "peak_active_claim_queries": claim,
        "peak_active_app_inserts": max(len(row["app_insert_active"]) for row in samples),
        "samples_with_claim_and_app_active": sum(bool(row["claim_active"] and row["app_active"]) for row in samples),
        "samples_with_claim_and_insert_active": sum(bool(row["claim_active"] and row["app_insert_active"])
                                                    for row in samples),
        "activity_sample_count": len(samples),
    }


def arm(path, label, pins, mode, calibration):
    status = read(path.with_suffix(".status.json"))
    require(status["exit_code"] == 0, f"arm exit {status['exit_code']}; see raw log")
    topology = read(path / "topology.json")
    runners, slots, app, workers = CASES[label]
    require((topology["runners"], topology["slots_per_runner"], topology["app_sessions_per_runner"],
             topology["scheduler_min"], topology["scheduler_max"])
            == (runners, slots, app, workers, workers), "measured scheduler configuration differs")
    pids = {str(pid) for pid in topology["runner_pids_by_index"]}
    expected = dict.fromkeys(pids, workers + 6)
    for point in ("ready", "terminal"):
        require(read(path / f"scheduler-{point}.json") == expected,
                f"live process thread inventory differs at {point}")

    def files(name):
        value = read(path / name)
        # The prior gate hard-codes scheduler=2. The actual values have been
        # checked against this experiment's launch and both live inventories.
        return dict(value, scheduler_min=2, scheduler_max=2) if name == "topology.json" else value

    score = verify_v1(files, lambda name: (path / name).read_text(), label, pins,
                      local=True, sample_activity=True)
    cpu, lag = validate_cpu(path, pids, score, calibration)
    return {**score, **validate_activity(path, mode, score),
            "scheduler_per_runner": workers, "live_runner_threads": sum(expected.values()),
            "total_scheduler_workers": workers * runners,
            "runner_bracket_cpu_ms_per_job": cpu, "runner_end_observer_lag_ms": lag,
            "peak_grants_per_runner": score["max_grants_per_runner"],
            "host_load_1m": status["load_before"][0]}


def paired(groups):
    result = {}
    for label, (treatment, controls) in CONTRASTS.items():
        pairs = [block["arms"] for block in groups
                 if all(name in block["arms"] for name in (treatment, *controls))]
        if not pairs:
            continue
        result[label] = {}
        for key in ("submit_jobs_per_s", "commit_jobs_per_s", "runner_cpu_ms_per_job",
                    "runner_bracket_cpu_ms_per_job"):
            values = [100 * (arms[treatment][key] / median(arms[c][key] for c in controls) - 1)
                      for arms in pairs]
            result[label][key] = dict(distribution(values), by_group=values,
                                      positive=sum(value > 0 for value in values))
    return result


def replay(path, calibration):
    manifest = read(path / "series.json")
    mode = manifest["mode"]
    expected_orders = ORDERS if mode == "timing" else (ACTIVITY,)
    orders = [list(order) for order in expected_orders]
    require(mode in {"timing", "activity"} and manifest["primary"] == PRIMARY
            and manifest["cases"] == {k: list(v) for k, v in CASES.items()}
            and manifest["orders"] == orders, "unexpected experiment design")
    require(read(path / "finished.json")["inputs_unchanged"]
            and all(sha(ROOT / name) == digest for name, digest in manifest["code"].items()),
            "measured source changed")
    pins = manifest["pins"]
    require(pins == read(PRIOR / "local-pins.json")
            and sha(prior["VERIFY"]) == pins["verifier_sha256"], "frozen V1 pins changed")
    pins = dict(pins, version="3828f20d2d6beb683868cce33e6dd1958365914b")
    require(calibration["numer"] > 0 and calibration["denom"] > 0 and len(calibration["checks"]) == 3
            and all(abs(check["raw_ticks"] * calibration["numer"] / calibration["denom"]
                        / check["clock_ns"] - 1) <= 0.02 for check in calibration["checks"]),
            "unverified CPU tick conversion")
    groups, failures = [], []
    for number, order in enumerate(orders, 1):
        arms = {}
        for label in order:
            try:
                arms[label] = arm(path / f"{number:02}" / label, label, pins, mode, calibration)
            except (OSError, ValueError, TypeError, KeyError, IndexError) as exc:
                failures.append(dict(group=number, label=label, error=str(exc)))
        groups.append(dict(number=number, order=order, arms=arms))
    rollup = {}
    for label in CASES:
        rows = [group["arms"][label] for group in groups if label in group["arms"]]
        if rows:
            rollup[label] = {key: distribution([arm[key] for arm in rows]) for key in METRICS}
    return dict(primary=PRIMARY, mode=mode, groups=groups, rollup=rollup,
                paired_percent=paired(groups) if mode == "timing" else {}, failures=failures)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("path", type=Path)
    parser.add_argument("--calibration", type=Path, default=PACKET / "evidence/cpu-calibration.json")
    args = parser.parse_args()
    result = replay(args.path, read(args.calibration))
    print(json.dumps(result, indent=2, sort_keys=True, allow_nan=False))
    sys.exit(bool(result["failures"]))
