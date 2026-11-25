"""Offline packet seal and inventory for the sole Linode booking controller."""
import argparse
from copy import deepcopy
import json
from pathlib import Path
import re
import subprocess
import sys

from boundary import require

HERE = Path(__file__).resolve().parent
RESULTS = HERE.parent
SUITE = RESULTS.parent
ROOT = SUITE.parents[1]
POOL = RESULTS / "2026-11-24-trusted-handler-app-pool"
PRIOR = POOL / "evidence/timing/grant-1/direct-observed/inventory.json"
BASE = "/var/lib/durable-jobs-bench/native"
PG = BASE + "/toolchain/pgsql-17.11/bin"
HOST = ["90ec1c24-08c8-4bc1-aa28-79b6db1bd13a", "6.8.0-149-generic", "8:0"]
RUNNER = BASE + "/builds/app-pool-39e79f4abb76/direct-observed"
RUNNER_SHA = "7c5c37db9fbfc1e9123e175a97096f5d0b7a52a4b7a64f8a2bc3ab68cd57d205"
MANIFEST = "3c5e56d72f9eaa9c51849f2c9fc4cfba3084a1df5a372f7b47f97dd3f69e06d5"
SOURCE = "db5937e7e8988f935e9be16b68a2561c2250cc27"
SETUP = ("native-setup.sh", "build.sh", "postflush.c", "canary.py", "boundary.py")
ARMS = {
    "worker.py": SUITE / "worker.py",
    "native-arm.py": POOL / "timing-identity-retry/native-arm.py",
    "topology_arm.py": RESULTS / "2026-11-23-ordinary-runner-topology/arm.py",
    "identity.py": RESULTS / "2026-11-24-dense-individual-hot-pool/identity.py",
    **{name: HERE / name for name in ("native-arm.sh", "arm_overlay.py", "boundary.py")},
}
ORDERS = [("off-a", "on", "off-b"), ("on", "off-b", "off-a"), ("off-b", "off-a", "on")]
CANARY = {
    "version": "17.11", "wal_record_matches_sql": True, "callback_between_record_and_client": True,
    "preserved_full_xid_epoch": True, "inactive_empty": True, "foreign_role_excluded": True,
    "read_only_abort_and_prepare_excluded": True,
}
sys.path.insert(0, str(SUITE))
import run


def packet():
    files = {PRIOR, *ARMS.values(), SUITE / "run.py", SUITE / "verify.py",
             RESULTS / "2026-11-23-dense-matched-topology/qualification.py",
             RESULTS / "2026-11-23-ordinary-runner-topology/replay.py",
             *(HERE / name for name in (*SETUP, "packet.py", "score.py", "test_packet.py"))}
    return dict(id=HERE.name, workload_contract="ordinary-trusted-shared-process", host=HOST, cpus=[0, 1],
                pg_version="17.11", source=SOURCE, runner=RUNNER, runner_sha256=RUNNER_SHA,
                manifest_sha256=MANIFEST, setup_minutes=2, screen_minutes=5, confirmation_minutes=9,
                arm_timeout_seconds=35, release_reserve_seconds=45,
                setup_files=list(SETUP), arm_files={name: str(path.relative_to(ROOT)) for name, path in ARMS.items()},
                local_sha256={str(path.relative_to(ROOT)): run.digest(path) for path in sorted(files)},
                groups=[dict(group=i, profile="noop", phases=["warmup", "measured"], order=list(lanes))
                        for i, lanes in enumerate(ORDERS, 1)])


def checked():
    sealed = run.read_json(HERE / "controller-packet.json")
    require(sealed == packet(), "controller packet differs or dependency bytes changed")
    return sealed


def module_pin(setup):
    pg, path, digest = (setup[key] for key in ("pg", "module", "sha256"))
    require(setup["canary"] == CANARY and pg["version"] == "PostgreSQL 17.11" and pg["bindir"] == PG
            and all(pg[key].startswith(PG.removesuffix("/bin") + "/") for key in ("includedir-server", "pkglibdir"))
            and re.fullmatch(re.escape(BASE) + r"/postflush-pg17-[0-9a-f]{12}/postflush\.so", path)
            and re.fullmatch(r"[0-9a-f]{64}", digest), "unqualified native module or PG")
    return path, digest


def inventory(setup, stage, lane):
    require(re.fullmatch(r"/var/tmp/mesh-durable-jobs-matrix/postflush-[0-9a-f]{12}-(on|off-a|off-b)", stage)
            and stage.endswith("-" + lane) and lane in ORDERS[0], "invalid private stage")
    module, digest = module_pin(setup)
    inventory = deepcopy(run.read_json(PRIOR))
    spec = inventory["engines"]["mesh"]
    old = spec["verified_files"]
    readonly = {path: sha for path, sha in old.items() if path.startswith(BASE + "/")}
    require(spec["argv"][3] == RUNNER and spec["artifact_sha256"] == MANIFEST and spec["version"] == SOURCE
            and readonly[RUNNER + "/mesh"] == RUNNER_SHA and readonly[RUNNER + "/mesh.json"] == MANIFEST,
            "prior runner pin changed")
    for name in ("native-arm.py", "topology_arm.py", "identity.py", "worker.py"):
        hashes = [sha for path, sha in old.items() if path.endswith("/" + name)]
        if name in ("native-arm.py", "identity.py"):
            # The clean retry replaced these two inputs. Pin its exact bytes.
            hashes = [{"native-arm.py": "a5217d8656f367b575a8e879ea9effdc3fbdf6fc1b612424e2b66ec13d14b998",
                       "identity.py": "c8bdb6ecf0be0b168b863752f92531b862221934249991965eae73aaf448b19f"}[name]]
        require(hashes == [run.digest(ARMS[name])], f"frozen app-pool helper changed: {name}")
    spec["verified_files"] = {**readonly, module: digest,
                              **{stage + "/" + name: run.digest(path) for name, path in ARMS.items()}}
    spec["argv"] = ["/bin/bash", stage + "/native-arm.sh", "on" if lane == "on" else "off", RUNNER, module]
    run.check_inventory(inventory)
    return inventory


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="action", required=True)
    sub.add_parser("check")
    command = sub.add_parser("inventory")
    command.add_argument("setup", type=Path)
    command.add_argument("stage")
    command.add_argument("lane", choices=ORDERS[0])
    args = parser.parse_args()
    sealed = checked()
    if args.action == "check":
        for name in {"native-setup.sh", "build.sh", "native-arm.sh"}:
            subprocess.run(["bash", "-n", HERE / name], check=True)
        print(json.dumps(sealed, indent=2))
    else:
        print(json.dumps(inventory(run.read_json(args.setup), args.stage, args.lane), indent=2))


if __name__ == "__main__":
    main()
