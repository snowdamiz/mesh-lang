#!/usr/bin/env python3
"""Install and qualify the peer overlay during the matrix controller's SETUP grant."""

import argparse
import fcntl
import json
from pathlib import Path
import sys
import tarfile
from uuid import uuid4

import freeze
from replay import check

HERE = Path(__file__).resolve().parent
SUITE = HERE.parents[1]
sys.path.insert(0, str(SUITE))
import interleave
import run

PEERS = {"oban": "oban", "river": "go", "apalis": "rust"}
PHASE = "diagnostic"
BLOCK = {"profile": "large"}


def symmetric(mesh_only, peer_inventory):
    interleave.validate(mesh_only)
    run.require(mesh_only["cpus"] == [0, 1] and mesh_only["profiles"] == ["large"]
                and mesh_only["contract"] == "strict-frozen", "unexpected parent comparison")
    remote = Path(peer_inventory["engines"]["oban"]["argv"][2]).parent
    lanes = {label: mesh_only["lanes"][label] for label in ("mesh-external", "mesh-direct-external")}
    for label, engine in PEERS.items():
        lane = mesh_only["lanes"][label]
        run.require(lane["engine"] == engine, f"wrong stock engine for {label}")
        tuned = freeze.treated_inventory(lane["inventory"], remote)
        original_overlay = peer_inventory["engines"][engine]
        engine_spec = tuned["engines"][engine]
        run.require(all(engine_spec[key] == original_overlay[key] for key in original_overlay
                        if key != "verified_files")
                    and engine_spec["verified_files"].items() >= original_overlay["verified_files"].items(),
                    f"{label}: parent pins conflict with the frozen peer inventory")
        lanes[f"{label}-external"] = dict(engine=engine, inventory=tuned)
    config = dict(mesh_only, lanes=lanes, description=(
        "Symmetric PostgreSQL STORAGE EXTERNAL for the frozen 64 x 512 KiB workload on CPUs 0 and 1. "
        "Mesh uses its protected fresh-exec worker with either stock or direct-file broker. "
        "Oban, River, and Apalis use the pinned stock executables."))
    interleave.validate(config)
    return config


def replay_attempt(attempt, config):
    results = {}
    for label, engine in PEERS.items():
        directory = attempt / f"{PHASE}-{engine}"
        inventory = config["lanes"][f"{label}-external"]["inventory"]
        score = run.score_arm(directory, BLOCK, PHASE, engine, inventory)
        run.require([score["host_boot_id"], score["host_kernel"], score["pg_mount"]["maj:min"]]
                    == config["host"], f"{label}: host changed since Mesh qualification")
        raw = directory / "raw.tar.gz"
        with tarfile.open(raw, "r:gz") as archive:
            proof = run.file_json(archive, "adapter/payload-storage.json")
            ledger = run.file_json(archive, "adapter/ledger.json")
            if engine == "oban":
                masks = run.file_json(archive, "adapter/process-affinity.json")
                run.require(len(masks) == 9 and all(mask == config["cpus"] for mask in masks.values()),
                            "Oban PostgreSQL or runner inherited a different CPU mask")
            results[label] = dict(check(proof, ledger, engine), raw_sha256=run.digest(raw))
    return results


def verify_file(path, checksum):
    run.require(run.digest(path) == checksum, f"sealed file changed: {path}")
    return path


def dispatch(request_path, mesh_input, config, package, install, out):
    out.mkdir(exist_ok=True)
    attempt_id = uuid4().hex
    attempt = out / f"attempt-{attempt_id}"
    attempt.mkdir()
    remote = run.REMOTE / f"large-toast-peers-{attempt_id}"
    (attempt / "request.json").write_bytes(request_path.read_bytes())
    (attempt / "parent-large.json").write_bytes(mesh_input.read_bytes())
    run.write_json(attempt / "symmetric-input.json", config)
    smoke_inventory = json.loads(json.dumps(config["lanes"]["mesh-external"]["inventory"]))
    for label, engine in PEERS.items():
        smoke_inventory["engines"][engine] = config["lanes"][f"{label}-external"]["inventory"]["engines"][engine]
    run.write_json(attempt / "inventory.json", smoke_inventory)
    status = 1
    host_idle = False
    with (SUITE / ".timing.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        run.ssh(["bash", "-c", interleave.HOST_IDLE], timeout=12).check_returncode()
        try:
            with package.open("rb") as payload, (attempt / "install.log").open("xb") as log:
                installed = run.ssh(["bash", "-c", install.read_text()], stdin=payload,
                                    stdout=log, stderr=log, timeout=90)
                installed.check_returncode()
            with (attempt / "stage.log").open("xb") as log:
                run.stage(SUITE / "worker.py", remote, log)
                run.stage(attempt / "inventory.json", remote, log)
            errors = {}
            for engine in PEERS.values():
                try:
                    run.dispatch_arm(attempt, remote, BLOCK, PHASE, engine)
                except (OSError, ValueError, RuntimeError, KeyError) as error:
                    errors[engine] = type(error).__name__
            run.require(not errors, f"native smoke dispatch issues: {errors}")
            results = replay_attempt(attempt, config)
            run.write_json(attempt / "replay.json", results)
            status = 0
        finally:
            host_idle = run.ssh(["bash", "-c", interleave.HOST_IDLE], timeout=12).returncode == 0
            run.write_json(attempt / "setup-exit.json", dict(
                exit_status=status, host_locks_idle=host_idle, checked_at=run.utc_now()))
    run.require(host_idle, "native locks still busy after SETUP")
    large = out / "large.json"
    if large.exists():
        run.require(run.read_json(large) == config, "existing symmetric config differs")
    else:
        run.write_json(large, config)
    run.write_json(attempt / "released.json", dict(id="large-toast-peers", exit_status=0,
                   released_at=run.utc_now(), receipt=str(attempt), symmetric_config=str(large)))
    print(attempt)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("request", type=Path)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--dispatch", action="store_true", help="run only inside the controller's exclusive grant")
    mode.add_argument("--check", action="store_true", help="validate pinned inputs without host access")
    mode.add_argument("--replay", type=Path, help="replay a local correctness attempt without host access")
    args = parser.parse_args()
    request = run.read_json(args.request)
    run.require(request["id"] == "large-toast-peers" and request["state"] == "ready", "incorrect SETUP request")
    paths = {name: verify_file(Path(item["path"]), item["sha256"]) for name, item in request["files"].items()}
    config = symmetric(run.read_json(paths["mesh_input"]), run.read_json(paths["peer_inventory"]))
    out = Path(request["evidence_destination"])
    if args.dispatch:
        dispatch(args.request, paths["mesh_input"], config, paths["package"], paths["install"], out)
    elif args.replay:
        print(json.dumps(replay_attempt(args.replay, config), indent=2, sort_keys=True))
    else:
        print(json.dumps(config, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
