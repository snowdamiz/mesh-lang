"""Retry handler app-pool SETUP with raw findmnt output under a separate grant."""
import argparse
from datetime import datetime, timedelta, timezone
import fcntl
from hashlib import sha256
from pathlib import Path
import re
import runpy
import subprocess
import sys
import tarfile
from uuid import uuid4

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "import-fix"))
import dispatch as amendment

base = amendment.base
run = base.run
state = base.QUEUE.parent
ops = runpy.run_path(str(state / "completion-writer-r2.py"))
ID = base.ID + "-setup-mount-retry"
QUEUE_FILE = base.QUEUE / (ID + ".json")


def encoded(data):
    return sha256(data).hexdigest()


def check():
    amendment.check()
    original = run.read_json(HERE / "original-release.json")
    base.require(original["exit_status"] == 1 and original["released_at"], "original slot must have released")
    base.require((HERE / "original-release.json").read_bytes() ==
                 (base.QUEUE / f"{base.ID}.released.json").read_bytes(), "original release changed")
    failures = sorted((base.EVIDENCE / "setup").glob("app-pool-*/raw.tar.gz"))
    base.require(failures, "original raw failure is missing")
    archive_path = failures[0]
    with tarfile.open(archive_path) as raw:
        host = raw.extractfile("./setup-host.txt").read().decode().splitlines()
        base.require(host[2].strip() == base.HOST[2] and host[2] != base.HOST[2]
                     and run.file_json(raw, "./status.json")["exit_code"] == 1
                     and "./build.log" not in raw.getnames(), "original failure does not match the disk rendering repair")
    original_script = (base.HERE / "native-setup.sh").read_text()
    old = 'findmnt -no MAJ:MIN'
    base.require(original_script.count(old) == 2, "launcher disk checks changed")
    script = original_script.replace(old, "findmnt -nro MAJ:MIN")
    subprocess.run(["bash", "-n"], input=script.encode(), check=True)
    packet = dict(id=ID, original_request_sha256=run.digest(base.REQUEST),
                  original_raw_sha256=run.digest(archive_path), original_release_sha256=run.digest(HERE / "original-release.json"),
                  source_sha=base.SOURCE, archive_sha256=run.digest(base.BUNDLE),
                  original_script_sha256=encoded(original_script.encode()), script_sha256=encoded(script.encode()),
                  retry_controller_sha256=run.digest(Path(__file__)), requested_window_minutes=28)
    return packet, script


def active():
    events = re.findall(r"^(SETUP|TIMING|RELEASED|HOLD)\s+(\S+)\s+(\S+)\s+(\S+)",
                        (base.QUEUE / "STATUS.md").read_text(), re.M)
    base.require(events and events[-1][:2] == ("SETUP", ID), "dedicated retry SETUP grant required")
    _, _, start, end = events[-1]
    now = datetime.now(timezone.utc)
    base.require(datetime.fromisoformat(start) <= now < datetime.fromisoformat(end), "expired retry grant")
    return dict(start=start, end=end, request_sha256=run.digest(QUEUE_FILE))


def setup(script, slot):
    with (base.SUITE / ".timing.lock").open("rb") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        grant = active()
        attempt = f"app-pool-{uuid4().hex[:12]}"
        out = base.EVIDENCE / "setup" / attempt
        out.mkdir()
        budget = base.budget_left(grant, 1260, spare=240)
        argv = ["bash", "-c", script, "app-pool-native-setup", run.digest(base.BUNDLE), attempt, str(budget)]
        run.write_json(out / "request.json", dict(grant=grant, attempt=attempt, argv=argv,
                                                source=str(base.BUNDLE.relative_to(run.ROOT))))
        run.write_json(slot / "attempt.json", {"attempt": attempt, "evidence": str(out)})
        with base.BUNDLE.open("rb") as source, (out / "raw.tar.gz").open("xb") as raw, (out / "dispatch.log").open("xb") as log:
            try:
                result = run.ssh(argv, stdin=source, stdout=raw, stderr=log,
                                 timeout=base.budget_left(grant, 1500, spare=20))
            except subprocess.TimeoutExpired:
                run.write_json(out / "dispatch.json", dict(interrupted=True))
                raise
        run.write_json(out / "dispatch.json", dict(exit_code=result.returncode, finished=run.utc_now()))
        result.check_returncode()
        qualification = base.setup_qualification(attempt, out)
        run.ssh(["bash", "-c", base.HOST_IDLE], timeout=12).check_returncode()
        run.write_json(base.EVIDENCE / "native-qualification.json", qualification)


def dispatch(packet, script, seal):
    base.require(run.digest(QUEUE_FILE) == seal and run.read_json(QUEUE_FILE) == packet, "retry seal differs")
    base.require(not (base.EVIDENCE / "native-qualification.json").exists(), "already qualified")
    ops["probe"]()
    slot = state / "controller-receipts" / ID
    slot.mkdir()
    run.write_json(slot / "request.json", packet)
    start = datetime.now(timezone.utc)
    end = start + timedelta(minutes=packet["requested_window_minutes"])
    ops["note"](f"SETUP {ID} {start:%Y-%m-%dT%H:%M:%SZ} {end:%Y-%m-%dT%H:%M:%SZ}\n\n"
                f"Retry of {base.ID} with raw findmnt rendering. Request SHA-256 {seal}.")
    status = 1
    try:
        setup(script, slot)
        status = 0
    finally:
        report = dict(id=ID, exit_status=status, started=start.isoformat(), command_finished_at=run.utc_now(),
                      receipt=str(base.EVIDENCE), request_sha256=seal)
        try:
            ops["probe"]()
        except (OSError, subprocess.SubprocessError) as error:
            report["release_probe_error"] = str(error)
            run.write_json(slot / "status.json", report)
            raise
        report["released_at"] = run.utc_now()
        run.write_json(slot / "status.json", report)
        run.write_json(base.QUEUE / f"{ID}.released.json", report)
        ops["note"](f"RELEASED {ID} {report['released_at']}, exit {status}. Evidence: {base.EVIDENCE}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("freeze", "dispatch"))
    parser.add_argument("--seal")
    args = parser.parse_args()
    packet, script = check()
    if args.action == "freeze":
        run.write_json(QUEUE_FILE, packet)
        return
    dispatch(packet, script, args.seal)


if __name__ == "__main__":
    main()
