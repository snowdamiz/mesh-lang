"""Dispatch the granted native correctness SETUP for the integrated set-based enqueue product."""
from datetime import datetime, timezone
import fcntl
from pathlib import Path
import re
import sys
from uuid import uuid4

REPLAY = Path(__file__).resolve().parent
PACKET = REPLAY.parent
ROOT = PACKET.parents[3]
SUITE = ROOT / "tools/durable-jobs-bench"
sys.path.insert(0, str(SUITE))
import run

REQUESTS = ROOT / "target/linode-jobs-stepchange/requests"
NAME = "set-based-enqueue-product"
REMOTE = "/var/lib/durable-jobs-bench/native"


def confirm_grant():
    events = re.findall(r"^(SETUP|TIMING|RELEASED) ([\w-]+) (\S+)(?: (\S+))?",
                        (REQUESTS / "STATUS.md").read_text(), re.MULTILINE)
    kind, request, start, end = events[-1]
    now = datetime.now(timezone.utc)
    run.require((kind, request) == ("SETUP", NAME) and end and
                datetime.fromisoformat(start) <= now < datetime.fromisoformat(end),
                "controller's exclusive SETUP grant is required")
    return start, end


def main():
    binding = run.read_json(REQUESTS / f"{NAME}.json")
    run.require(binding["state"] == "ready" and binding["ready"], "request is not READY")
    for name, digest in binding["pins"].items():
        run.require(run.digest(ROOT / name) == digest, f"input digest changed: {name}")
    archive = ROOT / binding["source_archive"]
    attempt = f"{NAME}-{uuid4().hex[:12]}"
    out = PACKET / "evidence/native" / attempt
    with (SUITE / ".timing.lock").open("r") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        start, end = confirm_grant()
        out.mkdir(parents=True)
        run.write_json(out / "grant.json", dict(attempt=attempt, start=start, end=end,
                       source_sha=binding["source_sha"],
                       source_archive_sha256=binding["source_archive_sha256"]))
        code = None
        try:
            with archive.open("rb") as stdin, (out / "setup.log").open("xb") as log:
                script = (REPLAY / "native-correctness.sh").read_text()
                budget = (datetime.fromisoformat(end) - datetime.now(timezone.utc)).total_seconds() - 120
                run.require(budget >= 600, "insufficient time to run and archive the correctness slot")
                result = run.ssh(["bash", "-c", script, "--", binding["source_archive_sha256"], attempt,
                                  binding["source_sha"]],
                                 stdin=stdin, stdout=log, stderr=log, timeout=min(5400, budget))
                code = result.returncode
        finally:
            run.write_json(out / "status.json", dict(exit_code=code, utc=run.utc_now()))
            with (out / "raw.tar.gz").open("xb") as stream, (out / "export.log").open("xb") as log:
                run.ssh(["tar", "-czf", "-", "-C", f"{REMOTE}/runs/{attempt}", "."],
                        stdout=stream, stderr=log, timeout=120).check_returncode()
        result.check_returncode()
    print(out)


if __name__ == "__main__":
    main()
