"""Bind the staged import fix to the existing app-pool controller booking."""
import argparse
from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "replay"))

import controller as base

AMENDMENT = HERE / "amendment.json"
INBOX = base.QUEUE / f"{base.ID}.amendment-import-fix.json"
ORIGINAL_INVENTORIES = base.inventories
ORIGINAL_GRANT = base.grant


def inventories(qualification):
    for lane, original, inventory, files in ORIGINAL_INVENTORIES(qualification):
        remote = original.with_name(f"{original.name}-import-fix")
        files = {**files, "native-arm.py": HERE / "native-arm.py"}
        spec = inventory["engines"]["mesh"]
        for name, path in files.items():
            del spec["verified_files"][str(original / name)]
            spec["verified_files"][str(remote / name)] = base.run.digest(path)
        spec["argv"][1] = str(remote / "native-arm.sh")
        base.run.check_inventory(inventory)
        yield lane, remote, inventory, files


def grant(kind):
    active = ORIGINAL_GRANT(kind)
    base.require(AMENDMENT.read_bytes() == INBOX.read_bytes(), "launcher amendment differs from controller inbox")
    return {**active, "launcher_amendment_sha256": base.run.digest(AMENDMENT)}


def check():
    base.check()
    correction = base.run.read_json(AMENDMENT)
    original = base.run.read_json(base.REQUEST)
    base.require(correction["id"] == base.ID and correction["original_request_sha256"] == base.run.digest(base.REQUEST)
                 and base.REQUEST.read_bytes() == (base.QUEUE / f"{base.ID}.json").read_bytes(),
                 "original app-pool intake or booking changed")
    for field in ("source_sha", "source_archive_sha256", "input_sha256", "group_plan", "resource_bounds",
                  "requested_timing_windows_minutes", "setup_command", "verify_setup_command", "evidence_destination"):
        base.require(correction[field] == original[field], f"launcher amendment changes {field}")
    for path, sha in correction["local_sha256"].items():
        base.require(base.run.digest(base.ROOT / path) == sha, f"changed launcher input {path}")
    witness = (HERE / "linux-import.serial").read_text().splitlines()
    for marker in ("APP_POOL_ORIGINAL_IMPORT_FAILURE_REPRODUCED", "APP_POOL_ENTRY_REACHED_ONE_CPU_GUARD",
                   "APP_POOL_LINUX_FULL_IMPORT_OK", "APP_POOL_NO_BYTECODE"):
        base.require(marker in witness, f"missing Linux import witness {marker}")

    qualification = dict(attempt="app-pool-000000000000", fixtures={
        mode: dict(build=dict(version=base.SOURCE, files={"mesh": "1" * 64}), manifest_sha256="2" * 64)
        for mode in base.BUILDS
    })
    for previous, corrected in zip(ORIGINAL_INVENTORIES(qualification), inventories(qualification), strict=True):
        lane, remote, config, files = corrected
        old_lane, old_remote, old_config, old_files = previous
        base.require(lane == old_lane and remote != old_remote
                     and set(files) == set(old_files)
                     and all(files[name] == old_files[name] for name in files if name != "native-arm.py"),
                     "unexpected staging change")
        spec, prior = config["engines"]["mesh"], old_config["engines"]["mesh"]
        base.require(spec["argv"][1] == str(remote / "native-arm.sh")
                     and spec["argv"][2:] == prior["argv"][2:]
                     and spec["artifact_sha256"] == prior["artifact_sha256"]
                     and spec["verified_files"][str(remote / "native-arm.py")] == base.run.digest(HERE / "native-arm.py")
                     and spec["verified_files"][str(remote / "topology_arm.py")] == base.run.digest(old_files["topology_arm.py"]),
                     "launcher changed the runner or pinned topology source")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("operation", choices=("check", "timing", "replay"))
    parser.add_argument("groups", type=int, nargs="*")
    args = parser.parse_args()
    check()
    if args.operation == "check":
        print("Original workload and booking pins verified with the staged import amendment")
        return 0
    base.inventories = inventories
    base.grant = grant
    if args.operation == "timing":
        return base.timing(args.groups)
    return base.replay()


if __name__ == "__main__":
    sys.exit(main())
