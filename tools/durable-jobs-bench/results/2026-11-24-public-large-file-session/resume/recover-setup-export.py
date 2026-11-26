"""Read back the original Linode SETUP after its client connection is lost."""

import fcntl
from pathlib import Path
import subprocess
import sys

from controller import HERE, ID, document, matrix, parked, qualification, run

EXPORT = r"""
set -o pipefail
test "$(cat /proc/sys/kernel/random/boot_id)" = 90ec1c24-08c8-4bc1-aa28-79b6db1bd13a
test "$(uname -r)" = 6.8.0-149-generic
test "$(findmnt -nro MAJ:MIN --target "$1" | xargs)" = 8:0
exec tar -czf - -C "$1" .
"""


def export(directory):
    doc = document()
    assert directory.parent == HERE / "evidence/native"
    initial = directory.parent / "setup"
    pending = run.read_json(directory.parent / "setup-release-pending.json")
    assert (pending["id"], pending["phase"], pending["receipt"]) == (ID, "setup", str(initial))
    assert run.read_json(initial / "status.json")["exit_code"] is None
    assert (parked.QUEUE / f"{ID}.json").read_bytes() == (HERE / "intake.json").read_bytes()
    with (run.SUITE / ".timing.lock").open("rb") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        directory.mkdir()
        raw = directory / "raw.tar.gz"
        with raw.open("xb") as output, (directory / "export.log").open("xb") as log:
            result = run.ssh(["bash", "-c", matrix.HOST_IDLE + EXPORT, "--", str(parked.REMOTE_OUT)],
                             stdout=output, stderr=log, timeout=60)
    run.write_json(directory / "export-status.json",
                   dict(id=ID, exit_code=result.returncode, observed=run.utc_now()))
    result.check_returncode()
    run.write_json(directory / "qualified.json", qualification(raw, doc))


if __name__ == "__main__":
    out, = sys.argv[1:]
    export(Path(out).resolve())
