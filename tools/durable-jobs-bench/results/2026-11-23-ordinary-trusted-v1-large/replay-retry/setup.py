#!/usr/bin/env python3
"""Install the frozen build-free overlay and run two diagnostic large arms."""

from datetime import datetime, timezone
import fcntl
import json
from pathlib import Path
import re
import sys
from uuid import uuid4

from assemble import LANE, ROOT, digest
from freeze import verify
from receipt import native

import run

ID = "ordinary-trusted-large-assembly-retry"
REQUESTS = ROOT / "target/linode-jobs-stepchange/requests"
POLICIES = ("external", "default")


def grant():
    events = re.findall(r"^(SETUP|TIMING|RELEASED) ([\w-]+) (\S+)(?: (\S+))?",
                        (REQUESTS / "STATUS.md").read_text(), re.MULTILINE)
    kind, name, start, end = events[-1]
    now = datetime.now(timezone.utc)
    if (kind, name) != ("SETUP", ID) or not datetime.fromisoformat(start) <= now < datetime.fromisoformat(end):
        raise SystemExit(f"{ID} has no active controller SETUP assignment")


def main(freeze, expected):
    if digest(freeze / "freeze.json") != expected:
        raise SystemExit("wrong freeze manifest bytes")
    binding = run.read_json(REQUESTS / (ID + ".json"))
    if binding["freeze_sha256"] != expected:
        raise SystemExit("request pins a different freeze")
    pinned = verify(freeze)
    attempt = f"{ID}-{uuid4().hex[:12]}"
    out = LANE / "evidence/native" / attempt
    with (run.SUITE / ".timing.lock").open("r") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        grant()
        out.mkdir(parents=True)
        print(out, flush=True)
        status = 1
        try:
            with (freeze / "overlay.tar.gz").open("rb") as archive, (out / "install.log").open("xb") as log:
                result = run.ssh(["bash", "-c", (Path(__file__).with_name("install.remote.sh")).read_text(),
                                  "--", pinned["archive_sha256"]], stdin=archive, stdout=log,
                                 stderr=log, timeout=60)
            run.write_json(out / "install-status.json", {"exit_code": result.returncode})
            result.check_returncode()
            for policy in POLICIES:
                grant()
                folder = out / policy
                folder.mkdir()
                config = run.read_json(freeze / f"{policy}.json")
                run.write_json(folder / "inventory.json", config["lanes"]["ordinary"]["inventory"])
                remote = run.REMOTE / f"{attempt}-{policy}"
                with (folder / "stage.log").open("xb") as log:
                    run.stage(LANE / "inputs/worker.py", remote, log)
                    run.stage(folder / "inventory.json", remote, log)
                # Standard worker holds active-arm.lock. Its adapter holds native.lock.
                run.dispatch_arm(folder, remote, {"profile": "large"}, "diagnostic", "mesh")
                summary = native(folder / "diagnostic-mesh", policy, config, "diagnostic")
                run.write_json(folder / "reconciled.json", summary)
            status = 0
        finally:
            run.write_json(out / "setup-status.json", {"exit_code": status, "finished_utc": run.utc_now(),
                                                       "freeze_sha256": expected})
    return status


if __name__ == "__main__":
    sys.exit(main(Path(sys.argv[1]).resolve(), sys.argv[2]))
