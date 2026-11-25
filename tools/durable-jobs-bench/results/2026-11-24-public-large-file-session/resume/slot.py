"""Open and release one assignment as the sole Linode controller."""

from datetime import datetime, timedelta, timezone
import fcntl
from pathlib import Path
import re
import shlex
import subprocess
import sys

from controller import HERE, ID, PACKET, assignment, check, document, matrix, parked, run


def probe():
    with (run.SUITE / ".timing.lock").open("rb") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        run.ssh(["bash", "-c", matrix.HOST_IDLE], timeout=12).check_returncode()


def note(value):
    with (parked.QUEUE / "STATUS.md").open("a") as stream:
        stream.write(f"\n{value}\n")


def dispatch(phase):
    doc = document()
    check(doc)
    assert (parked.QUEUE / f"{ID}.json").read_bytes() == PACKET.read_bytes()
    command = shlex.split(doc[f"{phase}_command"])
    release = parked.QUEUE / (f"{ID}.released.json" if phase == "setup" else f"{ID}.timing.released.json")
    out = Path(command[-1])
    assert not out.exists() and not release.exists()
    records = re.findall(r"^(SETUP|TIMING|RELEASED|HOLD) (\S+) (\S+)(?: (\S+))?",
                         (parked.QUEUE / "STATUS.md").read_text(), re.M)
    kind, key, _, end = records[-1]
    assert kind == "RELEASED", (kind, key, end)
    if phase == "timing":
        from controller import timing_plan

        setup_release = parked.QUEUE / f"{ID}.released.json"
        assert run.read_json(setup_release)["exit_status"] == 0
        timing_plan(doc, Path(command[-2]))
    out.parent.mkdir(parents=True, exist_ok=True)
    probe()
    start = datetime.now(timezone.utc)
    end = start + timedelta(minutes=doc[f"{phase}_minutes"])
    note(f"{phase.upper()} {ID} {start:%Y-%m-%dT%H:%M:%SZ} {end:%Y-%m-%dT%H:%M:%SZ}\n\n"
         f"Owner: matrix implementation operator. Sealed {PACKET}. Evidence: {out}.")
    grant = assignment(phase.upper())
    log = out.parent / f"{phase}-controller.log"
    result = 1
    try:
        with log.open("xb") as stream:
            result = subprocess.run(command, cwd=parked.ROOT, stdout=stream, stderr=subprocess.STDOUT).returncode
    finally:
        receipt = dict(id=ID, phase=phase, grant=grant, exit_status=result, receipt=str(out),
                       controller_log=str(log), command_finished_at=run.utc_now())
        try:
            probe()
        except (OSError, subprocess.SubprocessError) as error:
            receipt.update(release_probe_error=str(error))
            run.write_json(out.parent / f"{phase}-release-pending.json", receipt)
            note(f"RELEASE PENDING {ID} {run.utc_now()}, command exit {result}. Lock probe failed.")
            raise
        receipt["released_at"] = run.utc_now()
        run.write_json(release, receipt)
        run.write_json(out.parent / f"{phase}-release.json", receipt)
        note(f"RELEASED {ID} {receipt['released_at']}, exit {result}. Evidence: {out}")
    return result


if __name__ == "__main__":
    phase, = sys.argv[1:]
    assert phase in ("setup", "timing")
    sys.exit(dispatch(phase))
