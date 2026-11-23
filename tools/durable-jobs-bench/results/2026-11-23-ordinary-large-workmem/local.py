#!/usr/bin/env python3
"""Replay actual local V1 JOBS arms with rotated, duplicate controls."""

import itertools
import json
import os
from pathlib import Path
import subprocess
import sys
import uuid

LANE = Path(__file__).resolve().parent
ROOT = LANE.parents[1]
ASSEMBLY = LANE.with_name("ordinary-trusted-large-assembly")
ARTIFACTS = LANE.with_name("ordinary-trusted-mesh-queue") / "local-artifacts-v2"
ORDER = ("4mb-a", "16mb", "4mb-b")


def main():
    out = LANE / "evidence" / ("local-" + uuid.uuid4().hex[:12])
    out.mkdir()
    print(out, flush=True)
    env = dict(os.environ, PG_BIN_DIR="/opt/homebrew/opt/postgresql@17/bin",
               TMPDIR=str(LANE / "tmp") + "/", ORDINARY_LARGE_LOCAL_PINS=str(LANE / "inputs/local-pins.json"),
               ORDINARY_LARGE_STORAGE="external", NATIVE_JOBS_STATE=str(LANE / "state"),
               NATIVE_JOBS_ARTIFACTS=str(ARTIFACTS), DURABLE_JOBS_PHASE="diagnostic")
    result = {}
    orders = [tuple(itertools.islice(itertools.cycle(ORDER), shift, shift + len(ORDER)))
              for shift in range(len(ORDER))]
    orders.extend((tuple(reversed(orders[0])), tuple(reversed(orders[1]))))
    selected = orders[:int(sys.argv[1])] if len(sys.argv) > 1 else orders
    for number, order in enumerate(selected, 1):
        block = out / f"group-{number}"
        block.mkdir()
        (block / "order.json").write_text(json.dumps(order) + "\n")
        for phase in ("warmup", "measured"):
            for label in order:
                target = block / f"{phase}-{label}"
                mb = label.split("mb")[0]
                command = [sys.executable, "-B", str(LANE / "overlay/entry.py"), "mesh", mb,
                           str(ASSEMBLY / "source-retry/tools/durable-jobs-bench/native/arm.py"),
                           "--local-smoke", "mesh", "large", str(target)]
                with target.with_suffix(".log").open("xb") as log:
                    status = subprocess.run(command, env=env, stdout=log, stderr=subprocess.STDOUT).returncode
                result[f"{number}/{phase}/{label}"] = status
                print(number, phase, label, status, flush=True)
    (out / "status.json").write_text(json.dumps(result, indent=2) + "\n")
    return int(any(result.values()))


if __name__ == "__main__":
    sys.exit(main())
