#!/usr/bin/env python3
"""Run the original native score with a fresh controller-owned, build-free setup."""
from copy import deepcopy
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

from archived import MANIFEST_SHA, ORIGINAL, RUNNER_SHA, read, verify as verify_original
from proof import check
from timing import matrix, plan

HERE = Path(__file__).resolve().parent
PACKET = HERE.parent
ROOT = ORIGINAL.parents[4]
SUITE = ROOT / "tools/durable-jobs-bench"
run = matrix.run
NATIVE = "/var/lib/durable-jobs-bench/native"
REQUESTS = ROOT / "target/linode-jobs-stepchange/requests"
ID = "bulk-v2-completion-writer-exit-r2-identity-retry"
PRIOR = SUITE / "results/2026-11-23-bulk-enqueue-v2-short/inventory.json"
RETAINED = f"{NATIVE}/builds/bulk-writer-b1dcd34af288"
DRIVERS = ("combined-run", "split_arm.py", "identity.py")


def pins():
    packet = run.read_json(PACKET / "controller-packet.json")
    run.require(packet["id"] == ID and packet["ready"], "incorrect retry packet")
    for path, expected in packet["pins"].items():
        run.require(run.digest(ROOT / path) == expected, f"retry pin drift: {path}")
    with tarfile.open(ROOT / packet["stage_archive"], "r:gz") as archive:
        entries = {"prior-inventory.json": PRIOR, **{f"replay/{name}": HERE / name for name in DRIVERS}}
        run.require(set(archive.getnames()) == set(entries) and len(archive.getnames()) == len(entries),
                    "unexpected retry transfer entries")
        for name, path in entries.items():
            member = archive.getmember(name)
            run.require(member.isfile() and archive.extractfile(member).read() == path.read_bytes(),
                        f"retry transfer differs from pinned file: {name}")
    earlier = verify_original()
    controls = run.read_json(PACKET / "evidence/linux-controls.json")
    run.require(controls["uid"] == 65534 and controls["effective_capabilities"] == 0 and
                len(controls["refusals"]) == 11 and controls["readiness"][-1]["result"] == "refused",
                "missing unprivileged Linux readiness and refusal controls")
    return packet, earlier


def grant(kind):
    events = re.findall(r"^(SETUP|TIMING|RELEASED) ([\w-]+) (\S+)(?: (\S+))?",
                        (REQUESTS / "STATUS.md").read_text(), re.MULTILINE)
    mode, identifier, start, end = events[-1]
    now = datetime.now(timezone.utc)
    run.require((mode, identifier) == (kind, ID) and end and
                datetime.fromisoformat(start) <= now < datetime.fromisoformat(end),
                f"the central controller's {kind} grant for {ID} is required")
    return start, end


def expected_seal(attempt):
    replay = f"{NATIVE}/source-{attempt}/replay"
    files = {f"{replay}/{name}": run.digest(HERE / name) for name in DRIVERS}
    files.update({f"{RETAINED}/mesh": RUNNER_SHA, f"{RETAINED}/mesh.json": MANIFEST_SHA})
    return dict(runner_directory=RETAINED, wrapper=f"{replay}/combined-run", files=files)


def config_from_receipts(packet, attempt, host, seal, manifest):
    assert re.fullmatch(r"bulk-writer-retry-[a-f0-9]{12}", attempt)
    base = run.read_json(PRIOR)
    assert (host["host"], host["clock_ticks"]) == (base["host"], base["clock_ticks"])
    assert seal == expected_seal(attempt)
    assert hashlib.sha256(manifest).hexdigest() == MANIFEST_SHA
    runner_build = json.loads(manifest)
    assert runner_build["capsule"] == packet["original_id"]
    assert runner_build["version"] == packet["source_sha"]
    assert runner_build["source_patch_sha256"] == packet["source_patch_sha256"]
    assert (runner_build["system"], runner_build["architecture"]) == ("Linux", "x86_64")
    assert runner_build["files"]["mesh"] == RUNNER_SHA
    runner = dict(elf=RUNNER_SHA, manifest=MANIFEST_SHA, version=runner_build["version"])
    base["description"] = ("Pinned bulk V2 broker and migrator with one release-exit runner, writer off or zero, "
                           "plus the exact previous bulk control and stock peers.")
    base["lanes"].pop("scalar")
    for mode in ("off", "zero"):
        lane = deepcopy(base["lanes"]["bulk"])
        spec = lane["inventory"]["engines"]["mesh"]
        spec["argv"] = ["/usr/bin/env", f"COMBINED_WRITER_MODE={mode}",
                        f"COMBINED_RUNNER_ARTIFACTS={RETAINED}", seal["wrapper"], "mesh"]
        spec["verified_files"].update(seal["files"])
        lane["runner_seal"] = runner
        base["lanes"][f"writer-{mode}"] = lane
    matrix.validate(base)
    return base, runner


def reduce_setup(packet, earlier, attempt, out):
    archive = out / "raw.tar.gz"
    with tarfile.open(archive, "r:gz") as raw:
        parse = lambda name: json.loads(read(raw, name))
        host = parse("host.json")
        assert host == earlier["host"]
        seal = parse("native-seal.json")
        config, runner = config_from_receipts(packet, attempt, host, seal, read(raw, "runner-build.json"))
        smoke = {}
        for mode in ("off", "zero"):
            load = lambda name: parse(f"smoke-{mode}/{name}")
            smoke[mode] = check(load, mode, runner, native=True)
            invocation = load("invocation.json")
            assert invocation["profile"] == "short" and invocation["phase"] == "diagnostic"
            assert load("row.json")["input_sha256"] == packet["input_sha256"]
    return config, smoke


def setup(packet, earlier):
    start, end = grant("SETUP")
    attempt = f"bulk-writer-retry-{uuid4().hex[:12]}"
    out = PACKET / "evidence" / attempt
    out.mkdir()
    run.write_json(out / "grant.json", dict(attempt=attempt, start=start, end=end, id=ID))
    with (SUITE / ".timing.lock").open("rb") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        grant("SETUP")
        remaining = (datetime.fromisoformat(end) - datetime.now(timezone.utc)).total_seconds()
        run.require(remaining >= 180, "retain at least three minutes for SETUP and export")
        with (out / "setup.log").open("xb") as log, (ROOT / packet["stage_archive"]).open("rb") as data:
            script = (HERE / "native-retry.sh").read_text()
            exit_code = None
            try:
                result = run.ssh(["bash", "-c", script, "--", packet["stage_sha256"], attempt],
                                 stdin=data, stdout=log, stderr=log, timeout=remaining - 90)
                exit_code = result.returncode
            finally:
                run.write_json(out / "status.json", dict(exit_code=exit_code, utc=run.utc_now()))
                with (out / "raw.tar.gz").open("xb") as raw, (out / "export.log").open("xb") as err:
                    run.ssh(["tar", "-czf", "-", "-C", f"{NATIVE}/runs/{attempt}", "."],
                            stdout=raw, stderr=err, timeout=90).check_returncode()
            result.check_returncode()
    config, smoke = reduce_setup(packet, earlier, attempt, out)
    inventory = out / "qualified-timing.json"
    run.write_json(inventory, config)
    run.write_json(PACKET / "evidence/native-qualification.json", dict(
        attempt=attempt, inventory=str(inventory), inventory_sha256=run.digest(inventory),
        original_raw_sha256=earlier["raw_sha256"], raw_sha256=run.digest(out / "raw.tar.gz"),
        fault_cases=earlier["fault_cases"], teardown_modes=sorted(earlier["teardown_witnesses"]), smoke=smoke))
    print(inventory, flush=True)


def qualified(packet, earlier):
    receipt = run.read_json(PACKET / "evidence/native-qualification.json")
    attempt = receipt["attempt"]
    out = PACKET / "evidence" / attempt
    config = out / "qualified-timing.json"
    assert receipt["inventory"] == str(config) and run.digest(config) == receipt["inventory_sha256"]
    assert run.digest(out / "raw.tar.gz") == receipt["raw_sha256"]
    assert receipt["original_raw_sha256"] == earlier["raw_sha256"] and run.read_json(out / "status.json")["exit_code"] == 0
    replayed, smoke = reduce_setup(packet, earlier, attempt, out)
    assert run.read_json(config) == replayed and receipt["smoke"] == json.loads(json.dumps(smoke))
    return config


def timing(packet, earlier, args):
    grant("TIMING")
    release = run.read_json(REQUESTS / f"{ID}.released.json")
    run.require(release["id"] == ID and release["exit_status"] == 0, "successful central SETUP release required")
    config = qualified(packet, earlier)
    (PACKET / "evidence/native-timing").mkdir(exist_ok=True)
    return subprocess.run([sys.executable, "-B", str(HERE / "timing.py"), "dispatch", ID,
                           "--config", str(config), *args], cwd=ROOT, check=False).returncode


def offline(packet, earlier):
    attempt = "bulk-writer-retry-" + "0" * 12
    with tarfile.open(ORIGINAL / "evidence" / earlier["attempt"] / "raw.tar.gz") as raw:
        config, _ = config_from_receipts(packet, attempt, earlier["host"], expected_seal(attempt),
                                         read(raw, "runner-build.json"))
    previous = run.read_json(PRIOR)
    assert all(config["lanes"][label] == previous["lanes"][label]
               for label in ("bulk", "river", "oban", "apalis"))
    frozen_orders = runpy_plan()
    matchup = list(plan(config["profiles"], config["lanes"]))
    assert frozen_orders == matchup
    print(json.dumps(dict(id=ID, original_raw_sha256=earlier["raw_sha256"], stages=3,
                          plan=matchup, unchanged_lanes=["bulk", "river", "oban", "apalis"]), indent=2))


def runpy_plan():
    from ast import literal_eval, parse, Assign, Name
    tree = parse((ORIGINAL / "replay/timing.py").read_text())
    [orders] = [literal_eval(node.value) for node in tree.body if isinstance(node, Assign)
                and any(isinstance(target, Name) and target.id == "ORDERS" for target in node.targets)]
    a, b, c = orders
    return [dict(group=index, profile="short", phases=["measured"], order=list(order))
            for index, order in enumerate((a, a[::-1], b, b[::-1], c), 1)]


if __name__ == "__main__":
    packet, earlier = pins()
    match sys.argv[1:]:
        case ["check"]:
            offline(packet, earlier)
        case ["setup"]:
            setup(packet, earlier)
        case ["verify-setup"]:
            print(qualified(packet, earlier))
        case ["timing", *args]:
            raise SystemExit(timing(packet, earlier, args))
        case _:
            raise ValueError("expected check, setup, verify-setup, or timing")
