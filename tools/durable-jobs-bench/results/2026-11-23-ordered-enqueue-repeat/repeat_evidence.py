"""Replay a native four-engine repeat against the sealed baseline."""

import argparse
import json
from pathlib import Path
from statistics import median
import sys
import tarfile

ROOT = next(path for path in Path(__file__).resolve().parents
            if (path / "tools/durable-jobs-bench/run.py").is_file())
sys.path.insert(0, str(ROOT / "tools/durable-jobs-bench"))
import run


BASELINE = "2026-11-23-baseline"
METRICS = (
    "submit_jobs_per_s", "commit_jobs_per_s", "producer_commit_p50_ms",
    "producer_ack_p50_ms", "submit_to_pickup_p99_ms", "commit_to_pickup_p99_ms",
    "runner_cpu_ms_per_job",
)


def span(values):
    values = list(values)
    return {"min": min(values), "median": median(values), "max": max(values)}


def paired(rows):
    result = {}
    for metric in METRICS:
        baseline = [row["baseline"][metric] for row in rows]
        candidate = [row["candidate"][metric] for row in rows]
        changes = [100 * (new / old - 1) for old, new in zip(baseline, candidate, strict=True)]
        result[metric] = {
            "baseline": span(baseline), "candidate": span(candidate),
            "paired_change_percent": span(changes),
            "pairs_higher": sum(change > 0 for change in changes),
            "pairs_lower": sum(change < 0 for change in changes),
        }
    return result


def metrics(folder, score):
    with tarfile.open(folder / "raw.tar.gz", "r:gz") as archive:
        transactions = run.file_json(archive, "adapter/ledger.json")["transactions"]
    run.require(all(tx["submitted_us"] <= tx["committed_us"] <= tx["acknowledged_us"]
                    for tx in transactions), "Mesh producer submit, commit and acknowledgement are out of order")
    return dict(score, producer_commit_p50_ms=median(
        (tx["committed_us"] - tx["submitted_us"]) / 1000 for tx in transactions),
        producer_ack_p50_ms=median((tx["acknowledged_us"] - tx["submitted_us"]) / 1000
                                  for tx in transactions))


def matrix(directory):
    series = run.read_json(directory / "series.json")
    selected, issues = run.archive_index(directory, series["plan"], series["inventory"])
    run.require(len(selected) == len(series["plan"]), f"incomplete matrix {directory.name}: {issues}")
    return series, selected, issues


def host(score):
    return [score["host_boot_id"], score["host_kernel"], score["pg_mount"]["maj:min"]]


def report(directory):
    original = run.RESULTS / BASELINE
    baseline, previous, _ = matrix(original)
    candidate, current, issues = matrix(directory)
    reference = run.verify_reference(BASELINE, candidate["inventory"])
    run.require(candidate["kind"] == "repeat" and reference == candidate["reference"],
                "comparison needs the original controlled baseline")
    profiles = {}
    for block in candidate["plan"]:
        key = run.block_prefix(block)
        profile = block["profile"]
        old_path, old = previous[key]
        new_path, new = current[key]
        run.require(all(host(score) == reference["host"] for score in new.values()),
                    f"{key}: measured host identity changed")
        row = {"group": block["group"], "order": block["order"],
               "baseline_block": old_path, "candidate_block": new_path,
               "baseline": metrics(original / "blocks" / old_path / "measured-mesh", old["mesh"]),
               "candidate": metrics(directory / "blocks" / new_path / "measured-mesh", new["mesh"]),
               "peer_change_percent": {
                   peer: {metric: 100 * (new[peer][metric] / old[peer][metric] - 1)
                          for metric in METRICS[:2]} for peer in run.ENGINES if peer != "mesh"},
               "candidate_host_max_steal_percent": max(score["cpu_steal_percent"] for score in new.values()),
               "candidate_host_max_iowait_percent": max(score["cpu_iowait_percent"] for score in new.values()),
               "candidate_measured_jobs": sum(score["completed"] for score in new.values())}
        profiles.setdefault(profile, []).append(row)
    return {
        "reference": reference,
        "candidate_series": directory.name,
        "baseline_mesh": baseline["inventory"]["engines"]["mesh"],
        "candidate_mesh": candidate["inventory"]["engines"]["mesh"],
        "invalid_or_incomplete_attempts": issues,
        "profiles": {profile: {"matched_groups": rows, "distribution": paired(rows)}
                     for profile, rows in profiles.items()},
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("series", help="Name of the completed repeat in results/")
    args = parser.parse_args()
    directory = (run.RESULTS / args.series).resolve()
    run.require(directory.parent == run.RESULTS, "use an archived series name")
    print(json.dumps(report(directory), indent=2, sort_keys=True, allow_nan=False))


if __name__ == "__main__":
    main()
