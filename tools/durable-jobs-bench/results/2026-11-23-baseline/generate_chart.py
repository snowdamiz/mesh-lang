"""Refresh chart.html from the two frozen source files beside it."""

import argparse
import csv
from itertools import product
import json
import math
from pathlib import Path


DIRECTORY = Path(__file__).resolve().parent
PROFILES = ("noop", "slow", "sparse", "short", "large")
ENGINES = ("mesh", "oban", "go", "rust")
MEASURES = ("commit_jobs_per_s", "submit_jobs_per_s", "commit_to_pickup_p99_ms")
ROUNDS = 5
OPEN = '<script id="baseline" type="application/json">\n'
CLOSE = "\n</script>"


def data():
    sources = {name: (DIRECTORY / filename).read_bytes().decode("utf-8").split("\n")
               for name, filename in (("csv", "comparison.csv"), ("index", "index.json"))}
    index = json.loads("\n".join(sources["index"]))
    expected_blocks = {f"{round_:02d}-{profile}" for round_, profile
                       in product(range(1, ROUNDS + 1), PROFILES)}
    blocks = index["validated_blocks"]
    if (index["kind"] != "baseline" or index["required_four_engine_blocks"] != len(expected_blocks)
            or blocks.keys() != expected_blocks or index["missing_blocks"]
            or index["invalid_or_incomplete_attempts"]
            or any(not directory.startswith(f"{key}.") for key, directory in blocks.items())):
        raise ValueError("expected all 25 validated blocks from the five baseline rounds")

    results = {measure: {profile: {} for profile in PROFILES} for measure in MEASURES}
    for row in csv.DictReader(sources["csv"]):
        measure, profile, engine = (row[field] for field in ("metric", "profile", "engine"))
        if measure not in results:
            continue
        median, lower, upper = (float(row[field]) for field in ("median", "min", "max"))
        series = results[measure][profile]
        if (engine not in ENGINES or engine in series or int(row["n"]) != ROUNDS
                or not all(map(math.isfinite, (median, lower, upper)))
                or not 0 <= lower <= median <= upper):
            raise ValueError(f"invalid comparison row {measure}/{profile}/{engine}")
        series[engine] = [median, lower, upper]
    for measure, profiles in results.items():
        for profile, series in profiles.items():
            if series.keys() != set(ENGINES):
                raise ValueError(f"incomplete comparison for {measure}/{profile}")
    return {"results": results, "sources": sources}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="fail if the embedded data differs")
    args = parser.parse_args()
    path = DIRECTORY / "chart.html"
    original = path.read_text(encoding="utf-8")
    before, opening, rest = original.partition(OPEN)
    _, closing, after = rest.partition(CLOSE)
    if not opening or not closing:
        parser.error("chart.html is missing its embedded data block")
    payload = json.dumps(data(), indent=2, allow_nan=False).replace("<", "\\u003c")
    updated = before + OPEN + payload + CLOSE + after
    if updated != original:
        if args.check:
            parser.error("chart.html is stale. Run generate_chart.py to refresh it.")
        path.write_text(updated, encoding="utf-8")


if __name__ == "__main__":
    main()
