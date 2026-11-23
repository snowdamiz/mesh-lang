#!/usr/bin/env python3
"""Run only diagnostic setup under a matching controller grant."""

from datetime import datetime, timezone
import fcntl
from pathlib import Path
import re
import subprocess
import sys
import tarfile
from uuid import uuid4

from freeze import LABELS, LANE, ROOT, digest, read, verify, write
from proof import matched_settings, native
import run

ID = "ordinary-large-workmem"
REQUESTS = ROOT / "target/linode-jobs-stepchange/requests"


def grant(kind, name):
    events = re.findall(r"^(SETUP|TIMING|RELEASED) ([\w-]+) (\S+)(?: (\S+))?",
                        (REQUESTS / "STATUS.md").read_text(), re.MULTILINE)
    observed, request, start, end = events[-1]
    now = datetime.now(timezone.utc)
    if (observed, request) != (kind, name) or not datetime.fromisoformat(start) <= now < datetime.fromisoformat(end):
        raise SystemExit(f"{name} has no active controller {kind} assignment")


def main(folder, checksum):
    request = read(REQUESTS / f"{ID}.json")
    if digest(folder / "freeze.json") != checksum or request["freeze_sha256"] != checksum:
        raise SystemExit("setup request and local freeze differ")
    sealed = verify(folder)
    config = read(folder / "large.json")
    key = f"{ID}-{uuid4().hex[:12]}"
    evidence = LANE / "evidence" / key
    with (run.SUITE / ".timing.lock").open("r") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        grant("SETUP", ID)
        evidence.mkdir()
        print(evidence, flush=True)
        status = 1
        exit_by_arm = {}
        try:
            with (folder / "overlay.tar").open("rb") as source, (evidence / "install.log").open("xb") as log:
                result = run.ssh(["bash", "-c", (LANE / "install.remote.sh").read_text(),
                                  "--", sealed["archive_sha256"]],
                                 stdin=source, stdout=log, stderr=log, timeout=60)
            write(evidence / "install-status.json", {"exit_code": result.returncode})
            result.check_returncode()
            for label in LABELS:
                grant("SETUP", ID)
                lane = config["lanes"][label]
                engine = lane["engine"]
                out = evidence / label
                out.mkdir()
                write(out / "inventory.json", lane["inventory"])
                remote = run.REMOTE / f"{key}-{label}"
                try:
                    with (out / "stage.log").open("xb") as log:
                        run.stage(LANE / "inputs/worker.py", remote, log)
                        run.stage(out / "inventory.json", remote, log)
                    run.dispatch_arm(out, remote, {"profile": "large"}, "diagnostic", engine)
                    summary = native(out / f"diagnostic-{engine}", config, "diagnostic", label)
                    write(out / "reconciled.json", summary)
                    exit_by_arm[label] = {"qualified": True, "raw_sha256": summary["raw_sha256"]}
                except (OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError, tarfile.TarError) as error:
                    exit_by_arm[label] = {"qualified": False, "failure": str(error)}
            if all(row["qualified"] for row in exit_by_arm.values()):
                matched_settings([read(evidence / label / "reconciled.json")
                                  for label in LABELS if label.startswith("ordinary")])
                status = 0
        finally:
            write(evidence / "setup-status.json", {"exit_code": status, "arms": exit_by_arm,
                                                   "freeze_sha256": checksum, "finished_utc": run.utc_now()})
    return status


if __name__ == "__main__":
    sys.exit(main(Path(sys.argv[1]).resolve(), sys.argv[2]))
