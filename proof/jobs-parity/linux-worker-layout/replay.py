"""Replay the frozen Linux slow64 score with only the Python standard library."""

import argparse
from collections import Counter, defaultdict
import csv
from hashlib import sha256
import importlib.util
import json
from math import isclose, isfinite
from pathlib import Path
from statistics import median

HERE = Path(__file__).resolve().parent
ARMS = ("mesh-stock", "mesh-candidate", "oban")
SCORED = ("mesh-candidate", "oban")
INPUT_SHA256 = "42bb9e2be94c17b36110081430faf2647a04f3a3e0f2decf00e7c216fed462fd"
FIELDS = ("jobs_per_second", "cpu_seconds_per_job", "peak_rss_bytes", "pickup_p99_ms")
RESOURCES = ("peak_connections", "peak_processes", "peak_threads")
SHUTDOWN_FIELDS = ("rss_bytes", "processes", "threads")


def hash_file(path):
    return sha256(path.read_bytes()).hexdigest()


def read(name):
    return json.loads((HERE / name).read_text())


def load_intervals(expected_hash):
    path = HERE / "hold-intervals.csv"
    assert hash_file(path) == expected_hash
    intervals = defaultdict(list)
    with path.open(newline="") as stream:
        reader = csv.DictReader(stream)
        assert reader.fieldnames == ["group", "arm", "first_commit_us", "second_commit_us"]
        for row in reader:
            intervals[row["group"], row["arm"]].append(
                (int(row["first_commit_us"]), int(row["second_commit_us"])))
    return intervals


def hold_summary(intervals):
    events = Counter()
    total = 0
    for start, end in intervals:
        assert 0 <= start < end
        events[start] += 1
        events[end] -= 1
        total += end - start
    active = peak = integral = previous = 0
    for moment in sorted(events):
        integral += (moment - previous) * active
        active += events[moment]
        assert active >= 0
        peak = max(peak, active)
        previous = moment
    assert active == 0 and integral == total
    return {"peak": peak, "integral_job_microseconds": integral,
            "minimum_hold_us": min(end - start for start, end in intervals)}


def verify_metrics(metrics, jobs, slots):
    positive = ("completed", "jobs_per_second", "sql_calls_per_job", "runner_sql_calls_per_job",
                "enqueue_sql_calls_per_job", "claim_calls_per_job", "estimated_peak_executing",
                "peak_connections", "peak_rss_bytes", "cpu_seconds_per_job", "peak_processes",
                "peak_threads")
    nonnegative = ("pickup_p99_ms", "claim_p99_ms", "settlement_p99_ms", "idle_queries_per_minute",
                   "peak_executing")
    for field in positive + nonnegative:
        value = metrics[field]
        assert type(value) in (int, float) and isfinite(value)
        assert value > 0 if field in positive else value >= 0
    assert type(metrics["peak_executing"]) is int and metrics["peak_executing"] <= slots
    assert metrics["peak_threads"] >= metrics["peak_processes"]
    assert metrics["completed"] == jobs and metrics["input_sha256"] == INPUT_SHA256


def reconcile(data):
    jobs, slots = data["jobs_per_arm"], data["slots"]
    intervals = load_intervals(data["hold_intervals_sha256"])
    holds, canonical, shutdown = {}, [], []
    for i, group in enumerate(data["groups"], start=1):
        name = group["name"]
        assert name == f"slow-{i:02d}"
        assert tuple(group["order"]) == (ARMS if i % 2 else tuple(reversed(ARMS)))
        assert set(group["arms"]) == set(ARMS)
        holds[name] = {}
        timed_metrics, shutdown_metrics = {}, {}
        for arm in ARMS:
            source = group["arms"][arm]
            metrics = source["metrics"]
            verify_metrics(metrics, jobs, slots)
            assert metrics["engine"] == ("oban" if arm == "oban" else "mesh")
            assert source["warmup_completed"] == source["warmup_ordered_pairs"] == jobs
            writes = source["ordered_writes"]
            assert writes["jobs"] == writes["ordered"] == jobs
            assert writes["worker_pids"] == (1 if arm == "oban" else jobs)
            spans = intervals.pop((name, arm))
            assert len(spans) == jobs
            hold = hold_summary(spans)
            assert hold["peak"] == source["observed_committed_hold_peak"] <= slots
            assert hold["minimum_hold_us"] >= 99000
            assert abs(hold["minimum_hold_us"] - writes["minimum_hold_ms"] * 1000) < 1.0
            holds[name][arm] = hold
            drain = source["drained_mesh_status"]
            assert len(drain) == (0 if arm == "oban" else 1)
            for status in drain:
                assert status["drained"] and status["claimed"] == status["finished"] == jobs
                assert status["pending_claims"] == status["undrained"] == status["stale_acks"] == 0
                assert status["last_error"] is None

            closure = source["shutdown"]
            assert closure[-1] == {"drain_verified": True}
            reaped, = [row for row in closure if "pid" in row]
            assert reaped["exit_code"] == 0
            assert isclose(reaped["user_s"] + reaped["system_s"],
                           metrics["cpu_seconds_per_job"] * jobs, abs_tol=1e-9, rel_tol=0)
            samples = [row["sample"] for row in closure if "sample" in row]
            assert len(samples) >= 2
            timed_metrics[arm] = metrics
            shutdown_metrics[arm] = {**metrics, **{
                f"peak_{field}": max(metrics[f"peak_{field}"], *(s[field] for s in samples))
                for field in SHUTDOWN_FIELDS
            }}
        canonical.append(timed_metrics)
        shutdown.append(shutdown_metrics)
    assert not intervals
    return canonical, shutdown, holds


def describe(values):
    return {"min": min(values), "median": median(values), "max": max(values)}


def score(rows, settings, runners, slots):
    statistics = {arm: {key: describe([pair[arm][key] for pair in rows])
                        for key in FIELDS + RESOURCES}
                  for arm in ARMS}
    mesh, oban = (statistics[arm] for arm in SCORED)
    gates = {
        "budgets.cpu_seconds_per_job":
            mesh["cpu_seconds_per_job"]["median"] <= oban["cpu_seconds_per_job"]["median"]
            * settings["relative_cpu_seconds_per_job"],
        "budgets.peak_rss_bytes":
            mesh["peak_rss_bytes"]["median"] <= oban["peak_rss_bytes"]["median"] * settings["relative_peak_rss"],
        "comparison.jobs_per_second":
            mesh["jobs_per_second"]["median"] > oban["jobs_per_second"]["median"]
            * settings["core_throughput_ratio_min"],
        "comparison.pickup_p99_ms":
            mesh["pickup_p99_ms"]["median"] <= oban["pickup_p99_ms"]["median"]
            * settings["core_pickup_p99_ratio_max"],
    }
    caps = {
        "peak_connections": slots + runners * settings["connection_slots_per_runner"],
        "peak_processes": slots * settings["processes_per_slot"],
        "peak_threads": slots * settings["threads_per_slot"] + runners * settings["threads_per_runner"]
                        + settings["threads_per_host"],
    }
    assert "pickup_p99_budget_ms" not in settings["workloads"]["slow"]
    for arm, engine in zip(SCORED, ("mesh", "oban"), strict=True):
        for key, cap in caps.items():
            gates[f"{engine}.{key}"] = statistics[arm][key]["max"] <= cap
    queued = [pair["mesh-candidate"]["peak_queued_context_bytes"] for pair in rows]
    gates["mesh.peak_queued_context_bytes"] = (
        None not in queued and 0 < max(queued) <= settings["batch_queued_context_bytes_max"])
    wins = sum(pair["mesh-candidate"]["jobs_per_second"] > pair["oban"]["jobs_per_second"] for pair in rows)
    return {"passed": all(gates.values()), "gates": gates, "throughput_pair_wins": wins,
            "resource_caps": caps, "arms": statistics}


def compare_checkout(data, settings, rows, result):
    """Cross-check local formulas with the pinned repository harness when present."""
    root = HERE.parents[2]
    path = root / "scripts/jobs-parity.py"
    assert hash_file(path) == data["frozen_source"]["jobs-parity.py"]
    spec = importlib.util.spec_from_file_location("frozen_harness", path)
    h = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(h)
    trials = [dict(pair[label], pair=i, engine=engine)
              for i, pair in enumerate(rows)
              for label, engine in zip(SCORED, ("mesh", "oban"), strict=True)]
    h.validate_pairs(trials, settings["trial_pairs"], data["slots"])
    original = h.profile_summary(trials, "slow", settings["workloads"]["slow"], settings,
                                 data["runners"], data["slots"], ("mesh", "oban"), True, True, None)
    assert result["gates"] == {f"{scope}.{metric}": value["within_limit"]
        for scope, metrics in original.items() for metric, value in metrics.items()
        if isinstance(value, dict) and "within_limit" in value}
    for label, engine in zip(SCORED, ("mesh", "oban"), strict=True):
        assert all(result["arms"][label][metric] == original[engine][metric] for metric in FIELDS)


def run(compare_original):
    data, settings = read("qualification.json"), read("frozen-profiles.json")
    assert hash_file(HERE / "frozen-profiles.json") == data["frozen_source"]["profiles"]
    assert data["schema"] == 1 and data["profile"] == "slow"
    assert (data["runners"], data["slots"], data["jobs_per_arm"]) == (1, 64, 512)
    assert len(data["groups"]) == settings["trial_pairs"] == 5
    assert data["runtime"]["oban_erl_flags"] == settings["oban_erl_flags"]
    components = data["shared_component_sha256"]
    assert components["proof/mesh-current"] == data["candidate_sha256"]
    assert components["proof/mesh-stock-original"] == data["stock_sha256"]
    assert components["proof/oban.inetrc"] == data["runtime"]["oban_erl_inetrc_sha256"]
    assert components["proof/jobs-parity.py"] == data["frozen_source"]["jobs-parity.py"]
    assert components["proof/jobs-parity-profiles.json"] == data["frozen_source"]["profiles"]
    exact = data["held_exact_reap"]
    assert exact["candidate_sha256"] == data["candidate_sha256"]
    pids = set(exact["held_pids"])
    for field in ("held_pids", "committed_pids", "successfully_execed", "exited_zero",
                  "exact_successful_consuming_waits"):
        assert len(exact[field]) == len(pids) == data["slots"] and set(exact[field]) == pids

    canonical, shutdown, holds = reconcile(data)
    frozen = score(canonical, settings, data["runners"], data["slots"])
    included = score(shutdown, settings, data["runners"], data["slots"])
    if compare_original:
        compare_checkout(data, settings, canonical, frozen)
        compare_checkout(data, settings, shutdown, included)
    partial = {arm: [name for name, record in holds.items() if record[arm]["peak"] < data["slots"]]
               for arm in ARMS}
    matched = not any(partial[arm] for arm in SCORED)
    assert data["recorded_result"] == {"frozen": frozen["passed"], "shutdown_inclusive": included["passed"],
                                       "matched_full_occupancy": matched,
                                       "throughput_pair_wins": frozen["throughput_pair_wins"]}
    return {
        "candidate_sha256": data["candidate_sha256"], "stock_sha256": data["stock_sha256"],
        "pairs": len(canonical), "scored_jobs": sum(pair[arm]["completed"] for pair in canonical for arm in SCORED),
        "frozen": frozen, "shutdown_inclusive": included,
        "matched_full_occupancy": {"passed": matched, "expected": data["slots"], "below": partial},
        "committed_holds": holds, "separate_held_exact_reaps": len(pids),
    }


if __name__ == "__main__":
    if not __debug__:
        raise RuntimeError("Run without Python optimization to enable verification")
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("--compare-checkout", action="store_true", help="cross-check the pinned repository scorer")
    opts = cli.parse_args()
    print(json.dumps(run(opts.compare_checkout), indent=2, sort_keys=True))
