"""Repeat the two matched producer pairs with owned-child process identity."""
import argparse
from datetime import datetime, timezone
import json
from pathlib import Path
import re
import subprocess
import sys
import tarfile

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
from importlib import import_module

prior = import_module("bulk-import-retry")
bulk, run, require = prior.bulk, prior.run, prior.require
ID = "dense-matched-topology-bulk-pair-identity"
REQUEST = HERE / "request.json"
OUT = HERE / "native"
HELPER = HERE.parents[1] / "2026-11-24-dense-two-runner-checkout/identity.py"
DRIVER = HERE / "native-bulk-identity.sh"


def grant():
    journal = (bulk.QUEUE / "STATUS.md").read_text().splitlines()
    entries = [line.split() for line in journal if line.split()[:1] in (["SETUP"], ["TIMING"], ["HOLD"], ["RELEASED"])]
    require(len(entries[-1]) == 4 and entries[-1][:2] == ["TIMING", ID], "fresh identity TIMING grant needed")
    start, end = entries[-1][2:]
    require(datetime.fromisoformat(start) <= datetime.now(timezone.utc) < datetime.fromisoformat(end), "inactive grant")
    require(REQUEST.read_bytes() == (bulk.QUEUE / f"{ID}.json").read_bytes(), "controller inbox changed")
    return dict(start=start, end=end, request_sha256=run.digest(REQUEST))


def inventory(config, mode, remote):
    specset = prior.inventory(config, mode, remote)
    spec = specset["engines"]["mesh"]
    del spec["verified_files"][str(remote / prior.LAUNCHER.name)]
    spec["argv"][1] = str(remote / DRIVER.name)
    for path in (DRIVER, HERE / "bulk-arm.py", HELPER):
        spec["verified_files"][str(remote / path.name)] = run.digest(path)
    run.check_inventory(specset)
    return specset


def identity_check(folder, remote, config, mode):
    spec = inventory(config, mode, remote)["engines"]["mesh"]
    with tarfile.open(folder / "raw.tar.gz") as archive:
        ledger = run.file_json(archive, "adapter/ledger.json")
        proof = run.file_json(archive, "adapter/runner-identities.json")
        records = proof["runners"]
        expected = spec["argv"][3] + "/mesh"
        require(len(records) == 2 and len({r["pid"] for r in records}) == 2
                and {j["worker_pid"] for j in ledger["jobs"]} == {r["pid"] for r in records}, "wrong runner set")
        for r in records:
            require(r["argv"] == [expected, "run"] and r["cpus"] == [0, 1] and r["parent_pid"] == proof["adapter_pid"]
                    and r["launch_start_ticks"] == r["start_ticks_before"] == r["start_ticks_after"] > 0
                    and r["executable_sha256"] == spec["verified_files"][expected]
                    and (r["method"], r["observed_executable"], r["exe_errno"])
                        in (("proc-exe", expected, None), ("owned-launch", None, 13)), "unqualified runner identity")


def main():
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("action", choices=["check", "timing", "replay"])
    action = cli.parse_args().action
    config, _ = prior.inputs()
    prior.completion_negative()
    for path, digest in run.read_json(REQUEST)["inputs"].items():
        require(run.digest(bulk.ROOT / path) == digest, f"changed input {path}")
    failures = sorted((HERE / "failed-import-retry").glob("*/*/*/raw.tar.gz"))
    require(len(failures) == 8, "eight original failures required")
    for path in failures:
        with tarfile.open(path) as archive:
            error = run.file_json(archive, "adapter/failure.json")
            require(error["type"] == "PermissionError" and re.fullmatch(r"\[Errno 13\] Permission denied: '/proc/\d+/exe'",
                    error["message"]) and "adapter/ledger.json" not in archive.getnames(), "original failure changed")
    for mode in bulk.ORDERS[0]:
        inventory(config, mode, run.REMOTE / ID)
    subprocess.run(["bash", "-n", DRIVER], check=True)
    compile((HERE / "bulk-arm.py").read_bytes(), "bulk-arm.py", "exec")
    if action == "check":
        return 0
    bulk.ID, bulk.inventory, bulk.grant = ID, inventory, grant
    bulk.FILES = [bulk.SUITE / "worker.py", bulk.TOPOLOGY / "arm.py", HERE / "bulk-arm.py", DRIVER, HELPER]
    if action == "timing":
        original = run.dispatch_arm

        def checked(path, remote, block, phase, engine):
            original(path, remote, block, phase, engine)
            folder = path / f"{phase}-{engine}"
            require(run.read_json(folder / "dispatch.json").get("exit_code") == 0, "review retained failed arm")
            identity_check(folder, remote, config, path.name.removeprefix(phase + "-"))

        run.dispatch_arm = checked
        if bulk.dispatch(OUT, config):
            return 1
    summary = bulk.replay(OUT, config)
    require(len(summary["arms"]) == 8 and not summary["failures"], "incomplete comparison")
    manifest = run.read_json(OUT / "series.json")
    for rel in summary["arms"]:
        folder = OUT / rel
        mode = folder.parent.name.split("-", 1)[1]
        identity_check(folder, run.REMOTE / manifest["remotes"][mode], config, mode)
    print(json.dumps(summary, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())
