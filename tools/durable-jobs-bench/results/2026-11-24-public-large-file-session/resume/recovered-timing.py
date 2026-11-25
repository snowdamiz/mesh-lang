"""Qualify and time the authenticated readback without replacing the lost export."""

import fcntl
from importlib.util import module_from_spec, spec_from_file_location
import json
from pathlib import Path
import shlex
import subprocess
import sys

from controller import HERE, ID, document, matrix, parked, run, scores, sha, timing_plan
import slot

spec = spec_from_file_location("member_verification", HERE / "check-native-members.py")
member_verification = module_from_spec(spec)
spec.loader.exec_module(member_verification)

NATIVE = HERE / "evidence/native"
RECOVERED = NATIVE / "setup-export-recovered"
MEMBER_LOG = RECOVERED / "member-pins.txt"
MEMBER_PASS = RECOVERED / "member-pins.json"


def check(doc):
    export = run.read_json(RECOVERED / "export-status.json")
    assert (export["id"], export["exit_code"]) == (ID, 0)
    assert len((NATIVE / "setup" / "raw.tar.gz").read_bytes()) == 0
    pending = run.read_json(NATIVE / "setup-release-pending.json")
    assert (pending["id"], pending["phase"]) == (ID, "setup")
    return timing_plan(doc, RECOVERED)["session_qualification"]


def members(doc):
    proof = check(doc)
    command = ["bash", "-c", matrix.HOST_IDLE + member_verification.CHECK, "--",
               str(parked.STOCK.parent), str(parked.REMOTE_BROKER.parent),
               str(bool(proof["relink_sha256"])).lower(), doc["manifest_sha256"],
               proof["runtime_sha256"], proof["broker_sha256"], proof["relink_sha256"] or ""]
    with (run.SUITE / ".timing.lock").open("rb") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        result = run.ssh(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=45)
    with MEMBER_LOG.open("xb") as stream:
        stream.write(result.stdout)
    result.check_returncode()
    run.write_json(MEMBER_PASS, dict(log_sha256=sha(MEMBER_LOG), raw_sha256=proof["raw_sha256"],
                                    exit_status=0, observed=run.utc_now()))


def recovered_document():
    doc = document()
    proof = check(doc)
    pins = run.read_json(MEMBER_PASS)
    assert pins["exit_status"] == 0 and pins["log_sha256"] == sha(MEMBER_LOG)
    assert pins["raw_sha256"] == proof["raw_sha256"]
    command = shlex.split(doc["timing_command"])
    assert Path(command[-2]) == NATIVE / "setup"
    command[-2] = str(RECOVERED)
    return dict(doc, timing_command=shlex.join(command))


if __name__ == "__main__":
    action, = sys.argv[1:]
    if action == "check":
        print(json.dumps(check(document()), sort_keys=True, indent=2))
    elif action == "members":
        members(document())
    elif action == "timing":
        slot.document = recovered_document
        sys.exit(slot.dispatch("timing"))
    elif action == "scores":
        doc = recovered_document()
        print(json.dumps(scores(doc, RECOVERED, NATIVE / "timing"), sort_keys=True, indent=2))
    else:
        sys.exit("expected check, members, timing or scores")
