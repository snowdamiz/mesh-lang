#!/usr/bin/env python3
"""Run frozen large file-I/O witnesses during a separate controller grant."""

import argparse
import fcntl
from pathlib import Path
import shutil
import sys
from uuid import uuid4

from audit_io import reconcile, trace_worker
from controller import run

LABELS = ("mesh-direct-file", "mesh-stock-a", "oban", "river", "apalis")


def diagnostic(config_path, output, input_size, labels, with_trace):
    config = run.read_json(config_path)
    output.mkdir(exist_ok=True)
    result = True
    with (run.SUITE / ".timing.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        driver = output / "worker.py"
        if with_trace:
            trace_worker(run.SUITE / "worker.py", driver)
        else:
            shutil.copyfile(run.SUITE / "worker.py", driver)
        for label in labels:
            lane = config["lanes"][label]
            engine = lane["engine"]
            name = f"direct-v3-io-{label}-{uuid4().hex}"
            staging = output / name
            staging.mkdir()
            inventory = staging / "inventory.json"
            run.write_json(inventory, lane["inventory"])
            remote = run.REMOTE / name
            with (staging / "stage.txt").open("xb") as log:
                run.stage(driver, remote, log)
                run.stage(inventory, remote, log)
            block = {"profile": "large"}
            run.dispatch_arm(staging, remote, block, "diagnostic", engine)
            directory = staging / f"diagnostic-{engine}"
            try:
                score = run.score_arm(directory, block, "diagnostic", engine, lane["inventory"])
                if with_trace:
                    digest = lane["inventory"]["engines"][engine]["artifact_sha256"]
                    witness = reconcile(directory / "raw.tar.gz", input_size, digest)
                    run.write_json(staging / "input-witness.json", witness)
                run.write_json(staging / "reconciled.json", score)
            except (AssertionError, KeyError, OSError, ValueError) as error:
                (staging / "witness-error.txt").write_text(f"{type(error).__name__}: {error}\n")
                result = False
    return 0 if result else 1


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("config", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("input_size", type=int)
    parser.add_argument("--lane", choices=LABELS, action="append")
    parser.add_argument("--trace", action="store_true")
    args = parser.parse_args()
    sys.exit(diagnostic(args.config, args.output, args.input_size, args.lane or LABELS, args.trace))
