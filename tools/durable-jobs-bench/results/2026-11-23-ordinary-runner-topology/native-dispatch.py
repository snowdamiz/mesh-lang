"""Validate or dispatch the bounded, matrix-owner-only V1 diagnostic."""

import argparse
from datetime import datetime, timedelta, timezone
import fcntl
from pathlib import Path
import re
import subprocess
import sys
from uuid import uuid4

from arm import INPUT, LAYOUTS, ORDERS, read, sha, write
from replay import PACKET, ROOT, native, require
import run as matrix

ID = "ordinary-runner-topology"
REQUESTS = ROOT / "target/linode-jobs-stepchange/requests"
OUT = ROOT / "target" / ID / "native-01"
HOST_IDLE = """
set -eu
exec 8</var/tmp/mesh-durable-jobs-matrix/active-arm.lock
flock -n 8
exec 9</var/lib/durable-jobs-bench/native/native.lock
flock -n 9
"""


def inventory(pins, label, remote):
    stock = pins["inventory"]["engines"]["mesh"]
    _, artifacts_env, wrapper, engine = stock["argv"]
    require(engine == "mesh" and artifacts_env.startswith("NATIVE_JOBS_ARTIFACTS="),
            "unexpected native V1 command")
    _, artifacts = artifacts_env.split("=", 1)
    source = str(Path(wrapper).parents[3])
    args = ["/bin/bash", str(remote / "native-arm.sh"), source, artifacts, label]
    verified_files = {**stock["verified_files"],
                      **{str(remote / name): sha(PACKET / name) for name in ("native-arm.sh", "arm.py")}}
    return dict(pins["inventory"], engines=dict(pins["inventory"]["engines"],
                  mesh=dict(stock, argv=args, verified_files=verified_files)))


def check():
    request = read(PACKET / "request.json")
    require(request["id"] == ID and request["state"] == "ready", "unexpected request")
    for relative, pin in request["local_sha256"].items():
        require(sha(ROOT / relative) == pin, f"changed request input: {relative}")
    pins = read(PACKET / "native-pins.json")
    require(pins["cpus"] == [0, 1] and pins["inventory"]["cpus"] == pins["cpus"]
            and request["input_sha256"] == INPUT and request["orders"] == [list(x) for x in ORDERS[:2]]
            and pins["manifest_sha256"] == request["manifest_sha256"]
            and pins["executable_sha256"] == request["executable_sha256"],
            "unexpected native benchmark pins")
    matrix.check_inventory(pins["inventory"])
    for label in LAYOUTS:
        matrix.check_inventory(inventory(pins, label, matrix.REMOTE / (ID + "-check-" + label)))
    subprocess.run(["bash", "-n", PACKET / "native-arm.sh"], check=True)
    return request, pins


def assignment(remaining_seconds):
    status = (REQUESTS / "STATUS.md").read_text()
    events = re.findall(r"^(SETUP|TIMING|RELEASED|HOLD)\s+(\S+)\s+(\S+)\s+(\S+)", status, re.M)
    require(bool(events) and events[-1][:2] == ("SETUP", ID), "no current matrix-owner SETUP grant")
    _, _, start, end = events[-1]
    start_at, end_at = (datetime.fromisoformat(value.replace("Z", "+00:00")) for value in (start, end))
    now = datetime.now(timezone.utc)
    require(start_at <= now and now + timedelta(seconds=remaining_seconds) < end_at,
            "outside grant or insufficient time to start work")
    require((REQUESTS / f"{ID}.json").read_bytes() == (PACKET / "request.json").read_bytes(),
            "controller inbox and sealed request differ")
    return {"id": ID, "start": start, "end": end, "request_sha256": sha(PACKET / "request.json")}


def dispatch(request, pins):
    grant = assignment(360)
    with (matrix.SUITE / ".timing.lock").open("rb") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        assignment(360)
        OUT.mkdir()
        write(OUT / "series.json", dict(grant=grant, orders=request["orders"], scope=request["scope"]))
        matrix.ssh(["bash", "-c", HOST_IDLE], timeout=12).check_returncode()
        salt = uuid4().hex[:12]
        remotes = {label: matrix.REMOTE / f"{ID}-{salt}-{label}" for label in LAYOUTS}
        write(OUT / "stages.json", {label: str(path) for label, path in remotes.items()})
        for label, remote in remotes.items():
            staging = OUT / "staging" / label
            staging.mkdir(parents=True)
            write(staging / "inventory.json", inventory(pins, label, remote))
            with (staging / "transfer.log").open("xb") as log:
                for path in (staging / "inventory.json", matrix.SUITE / "worker.py",
                             PACKET / "native-arm.sh", PACKET / "arm.py"):
                    matrix.stage(path, remote, log)
        for number, order in enumerate(request["orders"], 1):
            for label in order:
                assignment(60)
                parent = OUT / f"{number:02}" / label
                parent.mkdir(parents=True)
                matrix.dispatch_arm(parent, remotes[label], {"profile": "noop"}, "diagnostic", "mesh")
                status = read(parent / "diagnostic-mesh/dispatch.json")
                if status.get("interrupted"):
                    raise RuntimeError("ambiguous native exit; controller must confirm release")
        matrix.ssh(["bash", "-c", HOST_IDLE], timeout=12).check_returncode()
        result = native(OUT)
        write(OUT / "summary.json", result)
        return bool(result["failures"])


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("check", "dispatch"))
    args = parser.parse_args()
    request, pins = check()
    if args.action == "check":
        print(f"validated {ID} request, four native inventories and wrapper syntax")
    else:
        sys.exit(dispatch(request, pins))
