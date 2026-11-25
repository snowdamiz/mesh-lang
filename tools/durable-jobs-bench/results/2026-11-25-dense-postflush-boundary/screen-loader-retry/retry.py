"""Seal and replay the build-free sibling of the first postflush screen."""
import argparse
import json
from pathlib import Path
import subprocess
import sys
import tarfile

HERE = Path(__file__).resolve().parent
ORIGINAL = HERE.parent
sys.path.insert(0, str(ORIGINAL))
import packet as original
import score

run = original.run
require = original.require
ROOT = original.ROOT
ID = ORIGINAL.name + "-" + HERE.name
OUT = HERE / "evidence"
NATIVE = ORIGINAL / "evidence/native"
SETUP = NATIVE / "setup/setup.json"
STOCK_ARCHIVE = original.RESULTS / "2026-11-23-ordinary-trusted-v1/ordinary-trusted-v1.tar.gz"
RELEASES = [NATIVE / slot / "controller-release.json" for slot in ("setup", "screen")]
FAILURE = NATIVE / "timing/01-noop/warmup-off-a/warmup-mesh/raw.tar.gz"
OLD_STAGES = NATIVE / "timing/stages.json"
FILES = {**original.ARMS, **{name: HERE / name for name in
                            ("arm_overlay.py", "verify_stage.py", "native-preflight.sh")}}
ORIGINAL_SEAL = "943d17b5da9b00dff833d6c78ab5ba93b9bc117e603bff721751494ddb11f3d2"
MODULE = "/var/lib/durable-jobs-bench/native/postflush-pg17-65af9b85ab67/postflush.so"
MODULE_SHA = "fe0d7eb8935483f2a3002aeaecd8584dc1e65ac759ddb8f4cd1a0c29e9aabccf"


def declaration():
    old = original.checked()
    require(run.digest(ORIGINAL / "controller-packet.json") == ORIGINAL_SEAL, "original packet was replaced")
    for path, status in zip(RELEASES, (0, 1), strict=True):
        receipt = run.read_json(path)
        require(receipt["exit_status"] == status and receipt["request_sha256"] == ORIGINAL_SEAL
                and receipt["released_at"], "wrong original release")
    with tarfile.open(FAILURE) as archive:
        adapter = score.text(archive, "adapter.log")
        require("POSTFLUSH_STAGED_HELPERS_READABLE" in adapter and
                "ModuleNotFoundError: No module named 'boundary'" in adapter and
                run.file_json(archive, "status.json")["exit_code"] == 1 and
                "adapter/ledger.json" not in archive.getnames(), "wrong first failure")
    setup = run.read_json(SETUP)
    require(original.module_pin(setup) == (MODULE, MODULE_SHA), "retained native module pin changed")
    paths = {*FILES.values(), STOCK_ARCHIVE, SETUP, OLD_STAGES, FAILURE, *RELEASES,
             *(HERE / name for name in ("controller.py", "retry.py", "local-proof.sh"))}
    return dict(id=ID, previous_seal=ORIGINAL_SEAL, module=MODULE, module_sha256=MODULE_SHA,
                host=old["host"], cpus=old["cpus"], group=old["groups"][0], minutes=old["screen_minutes"],
                arm_timeout_seconds=old["arm_timeout_seconds"], release_reserve_seconds=old["release_reserve_seconds"],
                arm_files={name: str(path.relative_to(ROOT)) for name, path in FILES.items()},
                local_sha256={str(path.relative_to(ROOT)): run.digest(path) for path in sorted(paths)})


def checked():
    packet = run.read_json(HERE / "controller-packet.json")
    require(packet == declaration(), "sibling packet or pinned inputs changed")
    for path in (HERE / "native-preflight.sh", HERE / "local-proof.sh"):
        subprocess.run(["bash", "-n", path], check=True)
    for path in (HERE / "controller.py", HERE / "arm_overlay.py", HERE / "verify_stage.py"):
        compile(path.read_bytes(), str(path), "exec")
    return packet


def inventory(stage, lane):
    require(stage not in run.read_json(OLD_STAGES).values(), "must use a fresh stage")
    pinned = original.inventory(run.read_json(SETUP), stage, lane)
    spec = pinned["engines"]["mesh"]
    spec["verified_files"].update({f"{stage}/{name}": run.digest(path) for name, path in FILES.items()})
    run.check_inventory(pinned)
    return pinned


def verify_activation(receipt, lane):
    mode = "on" if lane == "on" else "off"
    require(receipt == dict(probe="activation", uid=65534, dont_write_bytecode=True,
                           stage_unlistable=True, second_level_imports=True, module_path=MODULE,
                           module_sha256=MODULE_SHA, observations=[
                               dict(mode=mode, loaded_images=1, samples=int(mode == "on"), tls=True)]),
            f"native activation receipt differs for {lane}")


def replay(out):
    packet = checked()
    order = packet["group"]["order"]
    stages = run.read_json(out / "stages.json")
    require(set(stages) == set(order) and len(set(stages.values())) == len(order), "wrong stage roster")
    expected = {lane: inventory(stages[lane], lane) for lane in order}
    for lane in order:
        verify_activation(run.read_json(out / "preflight" / f"{lane}.json"), lane)
    block = out / run.block_prefix(packet["group"])
    require(run.read_json(block / "order.json") == packet["group"], "screen order changed")
    trials = {phase: {lane: score.score_arm(block / f"{phase}-{lane}" / f"{phase}-mesh", phase,
                                           expected[lane], lane == "on") for lane in order}
              for phase in packet["group"]["phases"]}
    settings = [receipt["settings"] for batch in trials.values() for receipt in batch.values()]
    require(all(setting == settings[0] for setting in settings), "server settings across arms differ")
    costs = score.perturbation(trials["measured"])
    return dict(trials=trials, perturbation=costs, screen_pass=all(item["pass_gate"] for item in costs.values()))


def main():
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("command", choices=("check", "replay"))
    args = cli.parse_args()
    result = checked() if args.command == "check" else replay(OUT)
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
