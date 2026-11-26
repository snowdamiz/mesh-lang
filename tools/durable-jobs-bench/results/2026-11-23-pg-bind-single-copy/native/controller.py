#!/usr/bin/env python3
"""Dispatch the sealed Bind packet from the existing sole Linode controller."""
from datetime import datetime, timezone
import fcntl
from pathlib import Path
import re
import sys
from uuid import uuid4

NATIVE = Path(__file__).resolve().parent
PACKET = NATIVE.parent
SUITE = PACKET.parents[1]
ROOT = SUITE.parents[1]
sys.path.insert(0, str(SUITE))
import run

NAME = "pg-bind-single-copy-7e6c9560"
QUEUE = ROOT / "target/linode-jobs-stepchange/requests"
REMOTE = "/var/lib/durable-jobs-bench/native"


def main():
    phase, = sys.argv[1:]
    kind = {"setup": "SETUP", "timing": "TIMING"}[phase]
    request = run.read_json(QUEUE / f"{NAME}.json")
    run.require(request == run.read_json(NATIVE / "request.json"), "controller packet differs from sealed request")
    run.require(request["ready"] and request["id"] == NAME, "controller request is not READY")
    for relative, sha256 in request["pins"].items():
        run.require(run.digest(ROOT / relative) == sha256, f"input digest changed: {relative}")
    attempt = f"pg-bind-single-copy-{uuid4().hex[:12]}"
    out = ROOT / request["evidence_destination"] / attempt
    archive = ROOT / request["source_archive"]
    with (SUITE / ".timing.lock").open("rb") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        events = re.findall(r"^(SETUP|TIMING|HOLD|RELEASED) ([\w-]+) (\S+)(?: (\S+))?",
                            (QUEUE / "STATUS.md").read_text(), re.M)
        active_kind, active_id, start, end = events[-1]
        now = datetime.now(timezone.utc)
        run.require((active_kind, active_id) == (kind, NAME) and end, "matching controller grant required")
        until = datetime.fromisoformat(end)
        run.require(datetime.fromisoformat(start) <= now < until, "grant is not active")
        limit = min({"setup": 450, "timing": 120}[phase], int((until - now).total_seconds()) - 45)
        run.require(limit >= 60, "grant has insufficient time to run and archive")
        out.mkdir(parents=True)
        run.write_json(out / "grant.json", {"attempt": attempt, "kind": kind, "start": start, "end": end,
                                          "source_sha": request["source_sha"],
                                          "source_archive_sha256": request["source_archive_sha256"]})
        code = export_code = None
        try:
            with archive.open("rb") as source, (out / "slot.log").open("xb") as log:
                finished = run.ssh(["bash", "-c", (NATIVE / "slot.sh").read_text(), "--", phase,
                                    request["source_archive_sha256"], attempt, str(limit)],
                                   input=source.read() if phase == "setup" else b"",
                                   stdout=log, stderr=log, timeout=limit + 10)
                code = finished.returncode
        finally:
            with (out / "raw.tar.gz").open("xb") as raw, (out / "export.log").open("xb") as log:
                try:
                    exported = run.ssh(["tar", "-czf", "-", "-C", f"{REMOTE}/runs/{attempt}", "."],
                                       stdout=raw, stderr=log, timeout=30)
                    export_code = exported.returncode
                finally:
                    run.write_json(out / "status.json", {"slot_exit": code, "export_exit": export_code,
                                                       "utc": run.utc_now()})
        print(out, flush=True)
        return code or export_code or 0


if __name__ == "__main__":
    sys.exit(main())
