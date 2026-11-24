#!/usr/bin/env python3
"""Fence a build-free setup and keep the original official timing series."""
import argparse
from datetime import datetime, timezone
import fcntl
import importlib
import json
import re
import sys
import tarfile
from unittest.mock import patch
from uuid import uuid4

from inputs import CANDIDATE, ELF, FAILED, FAILED_RAW, HERE, MANIFEST, failed, original, package, sha, shipped_pins

SETUP_ID = original.ID + "-recovery"
EVIDENCE = HERE / "evidence"
QUALIFICATION = EVIDENCE / "qualification.json"
run = original.run
matrix = original.matrix


def recovery_inventory(attempt, elf, manifest, packet):
    assert re.fullmatch(r"exit-bulk-d2-retry-[a-f0-9]{12}", attempt)
    assert (elf, manifest) == (ELF, MANIFEST)
    config, seal = original_inventory(FAILED, ELF, MANIFEST, packet)
    old = f"{original.NATIVE}/source-{FAILED}/replay/"
    new = f"{original.NATIVE}/source-{attempt}/replay/"

    def relocate(path):
        return new + path.removeprefix(old) if path.startswith(old) else path

    seal["wrapper"] = relocate(seal["wrapper"])
    seal["files"] = {relocate(path): expected for path, expected in seal["files"].items()}
    for mode in ("r2-off", "r2-d2"):
        spec = config["lanes"][mode]["inventory"]["engines"]["mesh"]
        spec["argv"] = [relocate(arg) for arg in spec["argv"]]
        spec["verified_files"] = {relocate(path): expected for path, expected in spec["verified_files"].items()}
    assert seal["candidate_directory"] == CANDIDATE
    matrix.validate(config)
    return config, seal


original_inventory = original.inventory


def inputs():
    baseline = original.pins()
    packet = run.read_json(HERE / "controller-packet.json")
    assert packet["id"] == SETUP_ID and packet["timing_grant_id"] == original.ID
    for path, expected in packet["pins"].items():
        assert original.digest(original.ROOT / path) == expected, path
    _, transfer = package(baseline)
    assert sha(transfer) == packet["stage_sha256"]
    return packet, baseline, transfer


def check(packet, baseline):
    failed(baseline)
    historical = original.earlier_proof()
    local = importlib.import_module("local-replay").verify_local()
    assert local["teardown_witnesses"] == 3 and local["fault_receipts"] == 7
    regression = run.read_json(HERE / "linux-regression.json")
    assert regression["old_transfer_sha256"] == baseline["stage_sha256"]
    assert regression["recovery_transfer_sha256"] == packet["stage_sha256"]
    assert regression["old_exit"] == 1 and regression["old_error"] == "Relay.requests missing"
    assert set(regression["old_overwrites"]) == {"replay/wire.py", "replay/combined-run", "replay/split_arm.py"}
    assert regression["repaired_shipped"] == shipped_pins() and regression["tampered_wire"] == "refused"
    verify_relay(regression["repaired"])
    previous, old = original_inventory(FAILED, ELF, MANIFEST, baseline)
    adjusted, updated = recovery_inventory("exit-bulk-d2-retry-" + "0" * 12, ELF, MANIFEST, baseline)
    assert original.plan is matrix.plan
    plan = list(matrix.plan(adjusted["profiles"], adjusted["lanes"]))
    assert plan == baseline["group_plan"]
    assert all(previous["lanes"][key] == adjusted["lanes"][key] for key in original.COLD_PEERS)
    assert old["candidate_directory"] == updated["candidate_directory"] == CANDIDATE
    return dict(failed=FAILED, failed_raw_sha256=FAILED_RAW, candidate_elf_sha256=ELF,
                candidate_manifest_sha256=MANIFEST, archived=historical, local=local,
                new_stage_files=len(shipped_pins()), old_overwrites=regression["old_overwrites"],
                timing_series=original.ID, group_plan=plan)


def verify_relay(receipt):
    assert receipt == dict(uid=65534, gid=65534, capabilities=0, cpu_mask=[0, 1],
                           bytecode_disabled=True, cache_absent=True,
                           queries=["SELECT 1", "INSERT INTO proof VALUES (1)"], receipt_dropped=True)


def setup_grant():
    events = re.findall(r"^(SETUP|TIMING|RELEASED|HOLD) ([\w-]+) (\S+)(?: (\S+))?",
                        (original.REQUESTS / "STATUS.md").read_text(), re.MULTILINE)
    mode, identifier, start, end = events[-1]
    now = datetime.now(timezone.utc)
    run.require((mode, identifier) == ("SETUP", SETUP_ID) and end and
                datetime.fromisoformat(start) <= now < datetime.fromisoformat(end),
                f"central controller SETUP grant required for {SETUP_ID}")
    return start, end


def reduce_setup(baseline, attempt, archive_path):
    candidate, retained = failed(baseline)
    with tarfile.open(archive_path) as archive:
        read = lambda name: original.raw_json(archive, name)
        assert original.raw_bytes(archive, "runner-build.json") == candidate
        assert original.raw_bytes(archive, "baseline-build.json") == retained
        assert read("staged-files.json") == shipped_pins()
        verify_relay(read("relay-check.json"))
        off_faults = original.check_faults(lambda name: read(f"proof/off/{name}"),
                                            original.RETAINED_ELF, original.BROKER_ELF, "off")
    with patch.object(original, "inventory", recovery_inventory):
        config, scores, d2_faults = original.reduce_setup(baseline, attempt, archive_path)
    return dict(config=config, scores=scores, faults=dict(d2=d2_faults, off=off_faults))


def setup(packet, baseline, transfer):
    start, end = setup_grant()
    check(packet, baseline)
    attempt = "exit-bulk-d2-retry-" + uuid4().hex[:12]
    out = EVIDENCE / attempt
    out.mkdir(parents=True)
    run.write_json(out / "grant.json", dict(id=SETUP_ID, attempt=attempt, start=start, end=end))
    work = original.WORK / "recovery"
    work.mkdir(parents=True, exist_ok=True)
    incoming = work / f"{attempt}.tar"
    incoming.write_bytes(transfer)
    try:
        with (original.SUITE / ".timing.lock").open("rb") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            setup_grant()
            remaining = (datetime.fromisoformat(end) - datetime.now(timezone.utc)).total_seconds()
            run.require(remaining >= 180, "retain three minutes for the native proof and export")
            exit_code = None
            try:
                with (out / "setup.log").open("xb") as log, incoming.open("rb") as stage:
                    result = run.ssh(["bash", "-c", (HERE / "native-recovery.sh").read_text(), "--",
                                      packet["stage_sha256"], attempt, baseline["source_archive_sha256"]],
                                     stdin=stage, stdout=log, stderr=log, timeout=remaining - 75)
                    exit_code = result.returncode
            finally:
                run.write_json(out / "status.json", dict(exit_code=exit_code, utc=run.utc_now()))
                with (out / "raw.tar.gz").open("xb") as data, (out / "export.log").open("xb") as err:
                    exported = run.ssh(["tar", "-czf", "-", "-C", f"{original.NATIVE}/runs/{attempt}", "."],
                                       stdout=data, stderr=err, timeout=70)
                    if exit_code == 0:
                        exported.check_returncode()
            result.check_returncode()
    finally:
        incoming.unlink(missing_ok=True)
    archive = out / "raw.tar.gz"
    proof = reduce_setup(baseline, attempt, archive)
    inventory = out / "qualified-timing.json"
    run.write_json(inventory, proof.pop("config"))
    run.write_json(QUALIFICATION, dict(attempt=attempt, raw_sha256=original.digest(archive),
                                       inventory_sha256=original.digest(inventory), **proof))
    print(inventory, flush=True)


def qualified(baseline):
    receipt = run.read_json(QUALIFICATION)
    attempt = receipt["attempt"]
    assert re.fullmatch(r"exit-bulk-d2-retry-[a-f0-9]{12}", attempt)
    out = EVIDENCE / attempt
    assert run.read_json(out / "status.json")["exit_code"] == 0
    spec, archive = out / "qualified-timing.json", out / "raw.tar.gz"
    assert original.digest(spec) == receipt["inventory_sha256"]
    assert original.digest(archive) == receipt["raw_sha256"]
    proof = reduce_setup(baseline, attempt, archive)
    assert run.read_json(spec) == proof["config"]
    assert json.loads(json.dumps(proof["scores"])) == receipt["scores"]
    assert proof["faults"] == receipt["faults"]
    return proof["config"]


def timing(baseline, groups):
    original.grant("TIMING")
    release = run.read_json(original.REQUESTS / f"{SETUP_ID}.released.json")
    assert release["id"] == SETUP_ID and release["exit_status"] == 0
    config = qualified(baseline)
    series = matrix.RESULTS / original.ID
    if groups == [3, 4, 5]:
        first = baseline["group_plan"][:2]
        assert all(matrix.select_attempt(series, config, block, {}) for block in first)
    matrix.RESULTS.mkdir(parents=True, exist_ok=True)
    return matrix.dispatch(series, config, groups)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("check", "setup", "verify-setup", "timing", "summarize"))
    parser.add_argument("group", type=int, nargs="*")
    args = parser.parse_args()
    packet, baseline, transfer = inputs()
    if args.action == "check":
        print(json.dumps(check(packet, baseline), sort_keys=True, indent=2))
    elif args.action == "setup":
        setup(packet, baseline, transfer)
    elif args.action == "verify-setup":
        qualified(baseline)
        print("Native recovery receipts verified")
    elif args.action == "timing":
        if args.group not in ([1, 2], [3, 4, 5]):
            parser.error("select the first two or final three timing groups")
        return timing(baseline, args.group)
    else:
        return matrix.summarize(matrix.RESULTS / original.ID)


if __name__ == "__main__":
    sys.exit(main())
