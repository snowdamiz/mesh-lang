"""Reduce paired V2 short metrics from the selected score and raw CPU samples."""

import argparse
import json
from pathlib import Path
from statistics import median
import tarfile


def idle_and_steal(raw, score, cpus):
    with tarfile.open(raw, "r:gz") as archive:
        with archive.extractfile("host-samples.jsonl") as stream:
            samples = [json.loads(line) for line in stream]
    before = next(point for point in reversed(samples) if point["epoch_us"] <= score["first_submit_us"])
    after = next(point for point in samples if point["epoch_us"] >= score["last_settlement_us"])
    totals = [sum(after["cpu"][f"cpu{cpu}"][field] - before["cpu"][f"cpu{cpu}"][field]
                  for cpu in cpus) for field in range(8)]
    assert min(totals) >= 0 and sum(totals) > 0
    window = sum(totals)
    return {"idle": 100 * totals[3] / window, "steal": 100 * totals[7] / window}


def distribution(values):
    return {"median": median(values), "min": min(values), "max": max(values), "n": len(values)}


def paired(series):
    report = json.loads((series / "summary.json").read_text())
    config = json.loads((series / "series.json").read_text())["config"]
    assert not report["missing"] and report["cpus"] == [0, 1]
    metrics = ("submit_jobs_per_s", "commit_jobs_per_s", "first_submit_to_commit_ms",
               "submit_to_settlement_p99_ms", "commit_to_settlement_p99_ms",
               "runner_cpu_ms_per_job", "pinned_cpu_ms_per_job", "idle", "steal")
    rows = []
    for row in report["rows"]["short"]:
        scores = row["scores"]
        for label, score in scores.items():
            engine = config["lanes"][label]["engine"]
            raw = series / "blocks" / row["attempt"] / f"measured-{label}" / f"measured-{engine}" / "raw.tar.gz"
            score.update(idle_and_steal(raw, score, report["cpus"]))
            score["pinned_cpu_ms_per_job"] = score["pinned_user_cpu_ms_per_job"] + score["pinned_kernel_cpu_ms_per_job"]
        comparisons = {lane: {key: scores["bulk"][key] - scores[lane][key] for key in metrics}
                       for lane in ("scalar", "river", "oban", "apalis")}
        terminals = {lane: {key: scores[lane][key] for key in ("terminal_xids", "largest_terminal_cohort",
                                                              "completion_cohort_queries")}
                     for lane in ("bulk", "scalar")}
        rows.append({"group": row["group"], "attempt": row["attempt"], "terminal": terminals,
                     "bulk_minus_comparator": comparisons,
                     "measured": {lane: {key: score[key] for key in metrics} for lane, score in scores.items()}})
    return {"rows": rows, "comparators": {
        lane: {"submit_wins": sum(row["bulk_minus_comparator"][lane]["submit_jobs_per_s"] > 0 for row in rows),
               "drain_wins": sum(row["bulk_minus_comparator"][lane]["commit_jobs_per_s"] > 0 for row in rows),
               "paired_deltas": {key: distribution([row["bulk_minus_comparator"][lane][key] for row in rows])
                                 for key in metrics}}
        for lane in ("scalar", "river", "oban", "apalis")}}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("series", type=Path)
    args = parser.parse_args()
    print(json.dumps(paired(args.series), indent=2, sort_keys=True, allow_nan=False))
