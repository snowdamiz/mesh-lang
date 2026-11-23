"""Describe where the completed native Jobs baseline spends its time."""

import argparse
import json
from pathlib import Path
from statistics import median
import tarfile

from run import archive_index, file_json, read_json
from verify import ENGINES, PROFILES, require


STAGES = (
    "first_submit_to_commit_ms", "first_claim_from_commit_ms", "claim_to_last_settlement_ms",
    "commit_to_claim_p50_ms", "claim_to_entry_p50_ms", "entry_to_attempt_commit_p50_ms",
    "attempt_to_effect_commit_p50_ms", "effect_commit_to_settlement_p50_ms", "commit_to_pickup_p99_ms",
)


def read_arm(directory, metrics):
    with tarfile.open(directory / "raw.tar.gz", "r:gz") as archive:
        host = file_json(archive, "host.json")
        with archive.extractfile("host-samples.jsonl") as stream:
            points = [json.loads(line) for line in stream]
        first = next(point for point in reversed(points) if point["epoch_us"] <= metrics["first_submit_us"])
        last = next(point for point in points if point["epoch_us"] >= metrics["last_settlement_us"])
        cpus = [f"cpu{cpu}" for cpu in host["affinity"]]
        totals = [sum(last["cpu"][cpu][i] - first["cpu"][cpu][i] for cpu in cpus) for i in range(8)]
        total = sum(totals)
        counts = {"cpu_busy_percent": 100 * sum(totals[i] for i in (0, 1, 2, 5, 6)) / total,
                  "cpu_system_percent": 100 * totals[2] / total}
        if "adapter/raw.json" in archive.getnames():
            raw = file_json(archive, "adapter/raw.json")
            counts.update({key + "_per_job": raw[key] / metrics["completed"]
                           for key in ("raw_sql_calls", "raw_claim_calls", "raw_runner_sql_calls",
                                       "raw_application_sql_calls", "raw_enqueue_sql_calls") if key in raw})
    keys = ("submit_jobs_per_s", "commit_jobs_per_s", "peak_executing", "runner_cpu_ms_per_job",
            "cpu_steal_percent", "cpu_iowait_percent", "disk_write_bytes", "disk_flushes", *STAGES)
    return {**counts, **{key: metrics[key] for key in keys if metrics.get(key) is not None}}


def localize(series):
    config = read_json(series / "series.json")
    require(config["kind"] == "baseline", "localization requires the measured baseline")
    selected, _ = archive_index(series, config["plan"], config["inventory"])
    require(len(selected) == 25, "localization requires all five rotations for all five profiles")
    profiles = {}
    for profile in PROFILES:
        repetitions = [(series / "blocks" / name, arms) for key, (name, arms) in selected.items()
                       if key.endswith(f"-{profile}")]
        engines = {}
        for engine in ENGINES:
            trials = [read_arm(path / f"measured-{engine}", arms[engine]) for path, arms in repetitions]
            fields = set.intersection(*(set(trial) for trial in trials))
            statistics = {}
            for field in sorted(fields):
                values = [trial[field] for trial in trials]
                statistics[field] = {"median": median(values), "min": min(values), "max": max(values)}
            engines[engine] = statistics
        profiles[profile] = engines
    primary = "submit_jobs_per_s"
    relative = {profile: engines["oban"][primary]["median"] / engines["mesh"][primary]["median"]
                for profile, engines in profiles.items()}
    return {"series": series.name, "scope": "five complete measured rotations", "primary": primary,
            "largest_mesh_gap_vs_oban": max(relative, key=relative.get), "oban_over_mesh_median_ratios": relative,
            "profiles": profiles}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("series", type=Path)
    args = parser.parse_args()
    print(json.dumps(localize(args.series), indent=2, sort_keys=True))
