#!/usr/bin/env python3
"""Controller entry points. These commands require an active, exclusive matrix grant."""
import argparse
from datetime import datetime, timezone
import fcntl
import json
from pathlib import Path
import re
import subprocess
import sys
import tarfile
from uuid import uuid4

from rotation import digest, replay, write

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
SUITE = ROOT / "tools/durable-jobs-bench"
QUEUE = ROOT / "target/linode-jobs-stepchange/requests"
ID = "enqueue-spec-preparation"
NATIVE = "/var/lib/durable-jobs-bench/native"
TOKEN = re.compile(r"[a-f0-9]{12}")

sys.path.insert(0, str(SUITE))
import run


def check():
    request = json.loads((HERE / "request.json").read_text())
    for path, sha in request["pins"].items():
        if digest(ROOT / path) != sha:
            raise ValueError(f"changed sealed input {path}")
    return request


def grant(kind, name):
    event = re.findall(
        r"^(SETUP|TIMING|RELEASED|HOLD) ([\w-]+) (\S+)(?: (\S+))?",
        (QUEUE / "STATUS.md").read_text(), re.MULTILINE,
    )[-1]
    observed, request, start, end = event
    now = datetime.now(timezone.utc)
    if (observed, request) != (kind, name) or not datetime.fromisoformat(start) <= now < datetime.fromisoformat(end):
        raise ValueError(f"no active {kind} grant for {name}")


EXPORT = r"""
set -euo pipefail
exec 8</var/tmp/mesh-durable-jobs-matrix/active-arm.lock
flock -n 8
exec 9</var/lib/durable-jobs-bench/native/native.lock
flock -n 9
folder="$1"
shift
tar -C "$folder" -czf - -- "$@"
"""


def extract(path, target):
    with tarfile.open(path) as archive:
        archive.extractall(target, filter="data")


def dispatch(action, setup_id, component_suffix, profile):
    request = check()
    queue = json.loads((QUEUE / f"{ID}.json").read_text())
    assert queue == request and queue["state"] == "ready"
    token = uuid4().hex[:12]
    if action == "setup":
        setup_id = f"{ID}-{token}"
    if not setup_id or not setup_id.startswith(ID + "-") or not TOKEN.fullmatch(setup_id.removeprefix(ID + "-")):
        raise ValueError("expected setup attempt identity")
    if action == "broker" and not (profile in {"short", "large"} and TOKEN.fullmatch(component_suffix or "")):
        raise ValueError("broker link requires profile and successful component suffix")
    evidence = HERE / "evidence" / f"{action}-{token}"
    name = ID if action == "setup" else f"{ID}-component" if action == "timing" else f"{ID}-{profile}-broker"
    scope = "TIMING" if action == "timing" else "SETUP"
    home = f"{NATIVE}/source-{setup_id}"
    command = ["bash", "-c", (HERE / "native.sh").read_text(), "--", action, setup_id]
    if action == "setup":
        archive = ROOT / request["source_archive"]
        check_archive = request["source_archive_sha256"]
        assert digest(archive) == check_archive
        command.append(check_archive)
        selection = ["bin", "verify/rustc.txt", "verify/llvm.txt", "verify/short-build.log", "verify/large-build.log",
                     "verify/short.json", "verify/large.json", "verify/short-elf.txt", "verify/large-elf.txt",
                     "verify/capture-short/capture.json", "verify/capture-large/capture.json", "build-pins.json",
                     "BINARY-SHA256SUMS", "native-proof", "SETUP-PASS"]
    elif action == "timing":
        command.append(token)
        home = f"{NATIVE}/runs/{setup_id}-component-{token}"
        selection = ["."]
    else:
        command.extend([profile, token, component_suffix])
        home = f"{home}/broker-{profile}-{token}"
        selection = ["runtime-build.log", "mesh-link.log", "patch", "link.json", f"{profile}-trial.json"]
    grant(scope, name)
    evidence.mkdir(parents=True)
    print(evidence, flush=True)
    print(f"setup_id={setup_id} receipt_suffix={token}", flush=True)
    started = datetime.now(timezone.utc).isoformat()
    status = 1
    raw = evidence / "raw.tar.gz"
    try:
        with (SUITE / ".timing.lock").open("rb") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            grant(scope, name)
            with (evidence / "dispatch.log").open("xb") as log:
                if action == "setup":
                    with archive.open("rb") as incoming:
                        result = run.ssh(command, stdin=incoming, stdout=log, stderr=log, timeout=2100)
                else:
                    result = run.ssh(command, stdin=subprocess.DEVNULL, stdout=log, stderr=log,
                                     timeout=360 if action == "timing" else 2100)
            status = result.returncode
            export = selection if status == 0 else ["."] if action == "timing" else ["verify"] if action == "setup" else ["link.json", "runtime-build.log", "mesh-link.log"]
            with raw.open("xb") as output, (evidence / "export.log").open("xb") as log:
                transfer = run.ssh(["bash", "-c", EXPORT, "--", home, *export],
                                   stdout=output, stderr=log, timeout=180)
            if status == 0:
                transfer.check_returncode()
                extract(raw, evidence / "raw")
                if action == "timing":
                    assert replay(evidence / "raw") == json.loads((evidence / "raw/summary.json").read_text())
    except BaseException as error:
        status = status or 1
        (evidence / "failure.txt").write_text(f"{type(error).__name__}: {error}\n")
        raise
    finally:
        write(evidence / "status.json", {"action": action, "request_id": name, "setup_id": setup_id,
              "receipt_suffix": token, "finished": datetime.now(timezone.utc).isoformat(),
              "started": started, "exit_status": status,
              "raw_sha256": digest(raw) if raw.exists() else None})
    sys.exit(status)


if __name__ == "__main__":
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("command", choices=("check", "setup", "timing", "broker"))
    p.add_argument("--setup-id")
    p.add_argument("--component-suffix")
    p.add_argument("--profile", choices=("short", "large"))
    a = p.parse_args()
    if a.command == "check":
        check()
        print("Controller packet hashes verified")
    else:
        dispatch(a.command, a.setup_id, a.component_suffix, a.profile)
