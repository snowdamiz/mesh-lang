"""Check payload storage policy for every selected warmup and measured arm."""

import argparse
import json
from pathlib import Path
import subprocess
import sys

from controller import read_storage
from interleave import select_attempt
import run


def replay(directory):
    series = run.read_json(directory / "series.json")
    config = series["config"]
    rows = {}
    missing = []
    issues = {}
    for block in series["plan"]:
        selected = select_attempt(directory, config, block, issues)
        if selected is None:
            missing.append(run.block_prefix(block))
            continue
        attempt, _ = selected
        arms = {}
        for label in block["order"]:
            if not label.endswith("-external"):
                continue
            engine = config["lanes"][label]["engine"]
            for phase in block["phases"]:
                archive = (
                    directory / "blocks" / attempt / f"{phase}-{label}" /
                    f"{phase}-{engine}" / "raw.tar.gz"
                )
                if engine == "mesh":
                    result = read_storage(archive)
                else:
                    peer = run.SUITE / "native/payload-storage-peers/replay.py"
                    output = subprocess.check_output(
                        [sys.executable, "-B", str(peer), engine, str(archive)]
                    )
                    result = json.loads(output)
                    result["raw_sha256"] = run.digest(archive)
                arms[f"{phase}-{label}"] = result
        if not arms:
            raise ValueError(f"{attempt} has no EXTERNAL storage arms")
        rows[attempt] = arms
    return {"rows": rows, "missing": missing, "issues": issues}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("series", help="series name in the benchmark results directory")
    parser.add_argument("--partial", action="store_true", help="allow missing blocks")
    args = parser.parse_args()
    report = replay(run.RESULTS / args.series)
    print(json.dumps(report, indent=2))
    if report["missing"] and not args.partial:
        sys.exit(1)
