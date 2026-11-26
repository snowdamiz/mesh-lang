#!/usr/bin/env python3
"""Stage a single fenced native build and seal the matched inventory from its proof."""
from copy import deepcopy
from datetime import datetime, timezone
import fcntl
import json
from pathlib import Path
import re
import subprocess
import sys
import tarfile
from uuid import uuid4

HERE = Path(__file__).resolve().parent
PACKET = HERE.parent
ROOT = PACKET.parents[4]
SUITE = ROOT / "tools/durable-jobs-bench"
sys.path.insert(0, str(SUITE))
import run
from proof import check

NATIVE = "/var/lib/durable-jobs-bench/native"
REQUESTS = ROOT / "target/linode-jobs-stepchange/requests"
ID = "bulk-v2-completion-writer-exit-r2"
PRIOR = SUITE / "results/2026-11-23-bulk-enqueue-v2-short"
BASE_INVENTORY = PRIOR / "inventory.json"


def pins():
    packet = run.read_json(PACKET / "controller-packet.json")
    for path, digest in packet["pins"].items():
        run.require(run.digest(ROOT / path) == digest, f"controller pin drift: {path}")
    return packet


def grant(kind):
    events = re.findall(r"^(SETUP|TIMING|RELEASED) ([\w-]+) (\S+)(?: (\S+))?",
                        (REQUESTS / "STATUS.md").read_text(), re.MULTILINE)
    mode, identifier, start, end = events[-1]
    now = datetime.now(timezone.utc)
    run.require((mode, identifier) == (kind, ID) and end and
                datetime.fromisoformat(start) <= now < datetime.fromisoformat(end),
                f"the central controller's {kind} grant for {ID} is required")
    return start, end


def config_from_receipts(packet, attempt, host, seal, runner_build):
    base = run.read_json(BASE_INVENTORY)
    assert host["host"] == base["host"] and host["clock_ticks"] == base["clock_ticks"]
    runner = f"{NATIVE}/builds/{attempt}"
    prefix = f"{NATIVE}/source-{attempt}/replay"
    assert seal["runner_directory"] == runner and seal["wrapper"] == prefix + "/combined-run"
    assert set(seal["files"]) == {prefix + "/combined-run", prefix + "/split_arm.py",
                                  runner + "/mesh", runner + "/mesh.json"}
    for name in ("combined-run", "split_arm.py"):
        assert seal["files"][prefix + "/" + name] == run.digest(HERE / name)
    assert runner_build["capsule"] == ID
    assert runner_build["version"] == packet["source_sha"]
    assert runner_build["source_patch_sha256"] == packet["source_patch_sha256"]
    assert (runner_build["system"], runner_build["architecture"]) == ("Linux", "x86_64")
    assert runner_build["files"]["mesh"] == seal["files"][runner + "/mesh"]
    runner_seal = dict(elf=seal["files"][runner + "/mesh"],
                       manifest=seal["files"][runner + "/mesh.json"], version=runner_build["version"])
    base["description"] = ("Pinned bulk V2 broker and migrator with one release-exit runner, writer off or zero, "
                           "plus the exact previous bulk control and stock peers.")
    base["lanes"].pop("scalar")
    for mode in ("off", "zero"):
        lane = deepcopy(base["lanes"]["bulk"])
        spec = lane["inventory"]["engines"]["mesh"]
        spec["argv"] = ["/usr/bin/env", f"COMBINED_WRITER_MODE={mode}",
                        f"COMBINED_RUNNER_ARTIFACTS={runner}", seal["wrapper"], "mesh"]
        spec["verified_files"].update(seal["files"])
        lane["runner_seal"] = runner_seal
        base["lanes"][f"writer-{mode}"] = lane
        run.check_inventory(lane["inventory"])
    return base, runner_seal


def seal_setup(packet, attempt, out):
    with tarfile.open(out / "raw.tar.gz") as raw:
        read = lambda name: run.file_json(raw, "./" + name)
        config, runner = config_from_receipts(packet, attempt, read("host.json"),
                                             read("native-seal.json"), read("runner-build.json"))
        for mode in ("off", "zero"):
            check(lambda name: read(f"smoke-{mode}/{name}"), mode, runner, native=True)
            teardown = read(f"proof/teardown-{mode}/teardown.json")
            assert teardown["held_callback_state"]["state"] == "executing"
            assert teardown["after_callback_state"]["state"] == "completed"
        for case in ("held-middle", "stop", "ambiguous-completion", "ambiguous-claim",
                     "expired-middle", "renew-priority", "cancelled-member"):
            read(f"proof/faults/{case}/result.json")
    run.write_json(out / "qualified-timing.json", config)
    run.write_json(PACKET / "evidence/native-qualification.json",
                   dict(attempt=attempt, inventory=str(out / "qualified-timing.json"),
                        inventory_sha256=run.digest(out / "qualified-timing.json"),
                        raw_sha256=run.digest(out / "raw.tar.gz")))
    print(out / "qualified-timing.json", flush=True)


def setup(packet):
    start, end = grant("SETUP")
    attempt = f"bulk-writer-{uuid4().hex[:12]}"
    out = PACKET / "evidence" / attempt
    out.mkdir()
    run.write_json(out / "grant.json", dict(attempt=attempt, start=start, end=end, id=ID))
    with (SUITE / ".timing.lock").open("r") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        grant("SETUP")
        remaining = (datetime.fromisoformat(end) - datetime.now(timezone.utc)).total_seconds()
        run.require(remaining >= 180, "retain at least three minutes for SETUP and export")
        with (out / "setup.log").open("xb") as log, (ROOT / packet["source_archive"]).open("rb") as archive:
            script = (HERE / "native-setup.sh").read_text()
            exit_code = None
            try:
                result = run.ssh(["bash", "-c", script, "--", packet["source_archive_sha256"],
                                  attempt, packet["source_patch_sha256"]], stdin=archive,
                                 stdout=log, stderr=log, timeout=remaining - 90)
                exit_code = result.returncode
            finally:
                run.write_json(out / "status.json", dict(exit_code=exit_code, utc=run.utc_now()))
                with (out / "raw.tar.gz").open("xb") as data, (out / "export.log").open("xb") as err:
                    run.ssh(["tar", "-czf", "-", "-C", f"{NATIVE}/runs/{attempt}", "."],
                            stdout=data, stderr=err, timeout=90).check_returncode()
            result.check_returncode()
    seal_setup(packet, attempt, out)


def timing():
    grant("TIMING")
    sealed = run.read_json(PACKET / "evidence/native-qualification.json")
    config = Path(sealed["inventory"])
    run.require(run.digest(config) == sealed["inventory_sha256"], "timing inventory drifted")
    (PACKET / "evidence/native-timing").mkdir(exist_ok=True)
    return subprocess.run([sys.executable, "-B", str(HERE / "timing.py"), "dispatch", ID,
                           "--config", str(config), *sys.argv[2:]], cwd=ROOT, check=False).returncode


if __name__ == "__main__":
    packet = pins()
    action = sys.argv[1]
    if action == "check":
        print(f"{ID}: {len(packet['pins'])} controller pins verified offline")
    elif action == "setup":
        setup(packet)
    elif action == "timing":
        raise SystemExit(timing())
    else:
        raise ValueError("expected check, setup, or timing")
