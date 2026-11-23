#!/usr/bin/env python3
"""Stage and prove the ordinary worker after the controller publishes a SETUP assignment."""

from datetime import datetime, timezone
import fcntl
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys
import tarfile
from uuid import uuid4

REPLAY = Path(__file__).resolve().parent
LANE = REPLAY.parent
ROOT = LANE.parents[1]
sys.path.insert(0, str(ROOT / "tools/durable-jobs-bench"))
import run
from verify import reconcile

BASE = "/var/lib/durable-jobs-bench/native"
REQUEST_ID = "ordinary-trusted-v1"
GRANTS = ROOT / "target/linode-jobs-stepchange/requests"
CONTRACT = "ordinary-trusted-shared-process"
PROOF_FILES = [
    "scripts/jobs-parity.py", "scripts/jobs-parity-profiles.json", "scripts/lib/jobs_parity_fault.py",
    *("proof/jobs-parity/linux-regression/" + name for name in
      ("trial.py", "proof_transport.py", "oban_runtime.py", "trusted-application.sql")),
    *("tools/durable-jobs-bench/native/" + name for name in ("run", "arm.py", "ledger.py")),
]


def confirm_grant():
    status = (GRANTS / "STATUS.md").read_text()
    events = re.findall(
        r"^(SETUP|TIMING|RELEASED) ([\w-]+) (\S+)(?: (\S+))?", status, re.MULTILINE
    )
    kind, name, start, end = events[-1]
    now = datetime.now(timezone.utc)
    if (kind, name) != ("SETUP", REQUEST_ID) or not (
        datetime.fromisoformat(start) <= now < datetime.fromisoformat(end)
    ):
        raise SystemExit("ordinary-trusted-v1 has no active controller SETUP assignment")


def verify_smoke(out, source, build, bundle):
    with tarfile.open(out / "evidence.tar.gz", "r:gz") as archive:
        def get(name):
            return run.file_json(archive, "./short/" + name)

        build_identity = get("build.json")
        ledger = get("ledger.json")
        row = get("row.json")
        settings = get("postgres-settings.json")
        affinity = get("process-affinity.json")
        if (build_identity["execution_contract"] != CONTRACT or ledger["execution_contract"] != CONTRACT
                or ledger["local_smoke"] or ledger["phase"] != "diagnostic"
                or not all(mask == [0, 1] for mask in affinity.values())):
            raise ValueError("wrong build contract, phase, or CPU affinity")
        report = reconcile(ledger, "mesh", "short", {
            "version": build_identity["version"], "artifact_sha256": row["artifact_sha256"],
        })
        if report["completed"] != 128 or ledger["peak_connections"] != 13:
            raise ValueError("short smoke did not preserve jobs and warmed topology")
        for name in ("row.json", "ledger.json", "build.json", "trusted-application-statements.json"):
            run.write_json(out / name, get(name))
        run.write_json(out / "reconciled.json", dict(report, database=settings))

        with archive.extractfile("./artifacts.txt") as stream:
            hashes = dict((name, sha) for sha, name in (line.split() for line in stream.read().decode().splitlines()))
        with archive.extractfile("./setup-identity.txt") as stream:
            machine, boot, disk, ticks, *_ = stream.read().decode().splitlines()
    if row["artifact_sha256"] != hashes[f"{build}/mesh.json"]:
        raise ValueError("manifest differs from the native hash")
    if build_identity["files"]["mesh"] != hashes[f"{build}/mesh"]:
        raise ValueError("executable differs from the native hash")
    if not machine.startswith("Linux ") or not machine.endswith(" x86_64"):
        raise ValueError("native setup ran on the wrong architecture")

    with tarfile.open(bundle, "r:gz") as archive:
        for path in PROOF_FILES:
            with archive.extractfile(path) as stream:
                hashes[f"{source}/{path}"] = hashlib.file_digest(stream, "sha256").hexdigest()
    reference = run.read_json(out.parents[2] / "replay" / "peer-inventory.json")
    lanes = reference["lanes"]
    strict = lanes["current"]
    ordinary = json.loads(json.dumps(strict))
    ordinary["inventory"]["engines"]["mesh"] = dict(
        strict["inventory"]["engines"]["mesh"],
        argv=["/usr/bin/env", f"NATIVE_JOBS_ARTIFACTS={build}",
              f"{source}/tools/durable-jobs-bench/native/run", "mesh"],
        artifact_sha256=hashes[f"{build}/mesh.json"],
        verified_files=hashes,
    )
    ordinary["execution_contract"] = CONTRACT
    config = dict(contract=CONTRACT, cpus=[0, 1], host=[boot, machine.split()[1], disk.strip()],
                  clock_ticks=int(ticks), description="Ordinary trusted stock-peer path with the named-effect Mesh control.",
                  lanes={"ordinary": ordinary, "protected-mesh": strict, "oban": lanes["oban"],
                         "river": lanes["river"], "apalis": lanes["apalis"]})
    for name, profiles in (("short", ["short"]), ("followup", ["slow", "noop"])):
        qualified = dict(config, profiles=profiles)
        run.write_json(out / (name + ".json"), qualified)
        run.write_json(out.parent / ("qualified-" + name + ".json"), qualified)


def main():
    bundle = Path(sys.argv[1]).resolve()
    expected = sys.argv[2]
    if run.digest(bundle) != expected:
        raise SystemExit("source archive hash mismatch")
    binding = run.read_json(GRANTS / (REQUEST_ID + ".json"))
    native_script = REPLAY / "native-setup.sh"
    if run.digest(Path(__file__)) != binding["setup_driver_sha256"]:
        raise SystemExit("setup driver hash mismatch")
    if run.digest(native_script) != binding["native_setup_sha256"]:
        raise SystemExit("native setup script hash mismatch")
    if run.digest(REPLAY / "peer-inventory.json") != binding["peer_inventory_sha256"]:
        raise SystemExit("peer inventory changed since request")
    attempt = f"{REQUEST_ID}-{uuid4().hex[:12]}"
    source, build, native_evidence = [f"{BASE}/{prefix}{attempt}" for prefix in ("source-", "builds/", "runs/")]
    out = LANE / "evidence" / "native" / attempt
    with (run.SUITE / ".timing.lock").open("r") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        confirm_grant()
        out.mkdir(parents=True)
        run.write_json(out / "request.json", dict(source_sha256=expected, attempt=attempt,
                                                   utc=run.utc_now(), remote_evidence=native_evidence))
        # bash receives the hashed source on stdin. The controller's lock protects the
        # complete remote setup, including the switch to the native arm's inner lock.
        script = native_script.read_text()
        result = None
        try:
            with bundle.open("rb") as stream, (out / "setup.log").open("xb") as log:
                result = run.ssh(["bash", "-c", script, "--", expected, attempt],
                                 stdin=stream, stdout=log, stderr=log, timeout=900)
        finally:
            status = ({"exit_code": result.returncode} if result is not None
                      else {"exit_code": None, "remote_completion_unknown": True})
            run.write_json(out / "status.json", dict(status, finished=run.utc_now()))
            with (out / "evidence.tar.gz").open("xb") as archive, (out / "export.log").open("xb") as log:
                try:
                    export = run.ssh(["tar", "-czf", "-", "-C", native_evidence, "."],
                                     stdout=archive, stderr=log, timeout=180)
                except subprocess.TimeoutExpired:
                    export = None
            run.write_json(out / "export-status.json",
                           {"exit_code": export.returncode if export is not None else None})
        if export is None:
            raise SystemExit(f"native evidence export timed out; attempt retained at {out}")
        export.check_returncode()
        result.check_returncode()
        verify_smoke(out, source, build, bundle)
    print(out)


if __name__ == "__main__":
    main()
