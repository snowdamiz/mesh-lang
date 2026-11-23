#!/usr/bin/env python3
"""Reduce admission CPU and matched protected guest runs with work witnesses."""
import argparse
from collections import defaultdict
import json
from pathlib import Path
import statistics

MODES = ("stock", "strict", "cohort", "pipeline")
PRICE_GROUPS = dict(
    admission=("admit_begin", "admit_query", "admit_commit", "admit_commit_and_query"),
    probe=("probe",),
    renew=("renew_commit_and_query",),
    settlement=("settle_commit_and_query",),
    application_first=("app_first_begin", "app_first_write", "app_first_commit", "app_first_reset"),
    application_second=("app_second_begin", "app_second_write", "app_second_commit", "app_second_reset"),
    resets=("app_first_reset", "app_second_reset"),
)


def load(path):
    return json.loads(path.read_text())


def spread(data):
    return dict(n=len(data), median=statistics.median(data), minimum=min(data), maximum=max(data))


def price(path):
    source = load(path)
    arms = defaultdict(list)
    for row in source["rounds"]:
        arms[row["arm"]].append(row)
    grouped = {}
    for arm, rows in arms.items():
        assert len(rows) >= 5
        samples = defaultdict(list)
        for row in rows:
            witness, ops = row["witness"], row["operations"]
            assert witness["rows"] == 1024 and witness["distinct_app_xids"] == 2048
            assert ops["app_first_reset"]["calls"] == ops["app_second_reset"]["calls"] == 1024
            assert ops["renew_commit_and_query"]["calls"] == 64
            for group, names in PRICE_GROUPS.items():
                samples[group].append(sum(
                    ops.get(name, {}).get("backend_cpu_ns", 0) for name in names) / 1_000_000 / 1024)
            samples["jobs_admission_probe_settlement"].append(sum(
                samples[key][-1] for key in ("admission", "probe", "settlement")))
        grouped[arm] = dict(per_call_ms=source["median_backend_ms_per_call"][arm],
                            per_job_ms={group: spread(ms) for group, ms in samples.items()})
    return grouped


def guest(path, modes):
    assert load(path / "exit.json")["status"] == 0
    config = load(path / "evidence/config.json")
    hashes = {name: digest for name, digest in config["hashes"].items()
              if name in ("proof/mesh-v3", "proof/mesh-pipeline", "proof/mesh-current", "proof/v3-wait.so")}
    load_recs = [json.loads(line) for line in (path / "host-load.jsonl").read_text().splitlines()]
    host_load = {item["event"].split()[-1]: item["load"][0]
                 for item in load_recs if " measured " in item["event"]}
    rows = {}
    for mode in modes:
        arm = path / "evidence" / mode
        metrics = load(arm / "metrics.json")
        proof = load(arm / "arm-audit.json")
        dispatch = load(arm / "admission-ledger.json")
        assert metrics["input_sha256"] == "9afb598e8e80b56fc96689cfbe7a7061d82f8e5e3c33a2c534d680ce3db2528a"
        assert metrics["completed"] == dispatch["terminal"]["jobs"] == 1024
        assert metrics["peak_executing"] <= 64 and metrics["peak_connections"] <= 96
        assert proof["xid_counts"]["distinct"] == 2048 and proof["resets_total"] == 2080
        assert (dispatch["admitted"] > 0) == (mode in ("strict", "cohort"))
        assert (dispatch["cohorts"] > 0) == (mode == "cohort")
        if mode != "cohort":
            assert dispatch["terminal"]["xids"] == 1024
        rows[mode] = dict(jobs_per_second=metrics["jobs_per_second"],
                          peak_executing=metrics["peak_executing"],
                          peak_connections=metrics["peak_connections"],
                          runner_cpu_ms_per_job=metrics["cpu_seconds_per_job"] * 1000,
                          runner_sql_per_job=metrics["runner_sql_calls_per_job"],
                          terminal_xids=dispatch["terminal"]["xids"],
                          v3_claim_calls=dispatch["admitted"], cohorts=dispatch["cohorts"],
                          host_load_1m=host_load[mode])
    return dict(hashes=hashes, order=[item[0] for item in config["plan"]], modes=rows)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--price", type=Path, default=Path("target/jobs-admission-v3/explicit-prices.json"))
    parser.add_argument("--runs", nargs=5, required=True, type=Path)
    args = parser.parse_args()
    runs = [guest(path, MODES) for path in args.runs]
    for one in runs[1:]:
        assert one["hashes"] == runs[0]["hashes"]
    comparisons = {}
    for mode in MODES:
        samples = [run["modes"][mode] for run in runs]
        comparisons[mode] = {key: spread([row[key] for row in samples]) for key in samples[0]}
        comparisons[mode]["paired_rate_vs_stock"] = spread([
            row["jobs_per_second"] / run["modes"]["stock"]["jobs_per_second"]
            for row, run in zip(samples, runs, strict=True)])
    print(json.dumps(dict(price=price(args.price), guest_runs=runs, guest_summary=comparisons), indent=2))


if __name__ == "__main__":
    main()
