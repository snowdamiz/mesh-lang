#!/usr/bin/env python3
"""Sole-controller entry points. The controller chooses when to grant each slot."""

import argparse
from datetime import datetime, timedelta, timezone
from pathlib import Path
import runpy
import shlex
import subprocess
import sys

from controller import EVIDENCE, HERE, ID, REQUESTS, ROOT, require, run, sha

STATE = REQUESTS.parent


def command(packet, field):
    return shlex.split(packet[field])


def setup(packet, probe, note):
    release = REQUESTS / f"{ID}.released.json"
    require(not release.exists(), "SETUP is already released")
    probe()
    out = EVIDENCE / "setup-controller"
    out.mkdir()
    run.write_json(out / "request.json", packet)
    start = datetime.now(timezone.utc)
    end = start + timedelta(minutes=packet["requested_window_minutes"])
    note(f"SETUP {ID} {start:%Y-%m-%dT%H:%M:%SZ} {end:%Y-%m-%dT%H:%M:%SZ}\n\n"
         f"Owner: matrix implementation operator. Evidence: {packet['evidence_destination']}/setup.")
    result = 1
    try:
        with (out / "controller.log").open("xb") as log:
            result = subprocess.run(command(packet, "setup_command"), cwd=ROOT, stdout=log, stderr=log).returncode
    finally:
        status = dict(id=ID, exit_status=result, started=start.isoformat(), finished=run.utc_now())
        try:
            probe()
        except (OSError, subprocess.SubprocessError) as error:
            status["release_probe_error"] = str(error)
            run.write_json(out / "status.json", status)
            raise
        status["released_at"] = run.utc_now()
        run.write_json(out / "status.json", status)
        run.write_json(release, status)
        note(f"RELEASED {ID} {status['released_at']}, exit {result}. Evidence: {packet['evidence_destination']}/setup.")
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("phase", choices=("check", "setup", "pilot", "rest"))
    parser.add_argument("--seal", required=True)
    args = parser.parse_args()
    queued = REQUESTS / f"{ID}.json"
    require(sha(queued) == sha(HERE / "controller-packet.json") == args.seal, "queue seal changed")
    packet = run.read_json(queued)
    subprocess.run(command(packet, "offline_check_command"), cwd=ROOT, check=True)
    if args.phase == "check":
        return 0
    ownership = runpy.run_path(str(STATE / "completion-writer-r2.py"))
    if args.phase == "setup":
        return setup(packet, ownership["probe"], ownership["note"])
    release = run.read_json(REQUESTS / f"{ID}.released.json")
    require(release["exit_status"] == 0 and release["released_at"], "SETUP must successfully release")
    subprocess.run(command(packet, "verify_setup_command"), cwd=ROOT, check=True)
    field, groups, minutes = (("timing_command", [1, 2], 12) if args.phase == "pilot"
                              else ("remaining_timing_command", [3, 4, 5, 6], 20))
    if args.phase == "rest":
        pilot = run.read_json(REQUESTS / f"{ID}-groups-1-2.released.json")
        require(pilot["exit_status"] == 0 and pilot["released_at"], "pilot must successfully release")
    release = REQUESTS / f"{ID}-groups-{groups[0]}-{groups[-1]}.released.json"
    require(not release.exists(), "these timing groups already have a release")
    timing = dict(packet, evidence_destination=packet["evidence_destination"] + "/timing",
                  controller_receipt_dir=EVIDENCE / "timing-controller", timing_window_minutes=minutes)
    result = ownership["measure"](timing, field, groups)
    if result == 0 and args.phase == "rest":
        with (EVIDENCE / "summary.json").open("xb") as out:
            result = subprocess.run(command(packet, "summarize_command"), cwd=ROOT, stdout=out).returncode
    return result


if __name__ == "__main__":
    sys.exit(main())
