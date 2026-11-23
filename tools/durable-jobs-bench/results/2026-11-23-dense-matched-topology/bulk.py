"""Replay the previous producer envelope or run two independent native producer pairs."""

import argparse
from datetime import datetime, timezone
import fcntl
import json
from pathlib import Path
import re
import subprocess
import sys
import tarfile
from uuid import uuid4

from qualification import mesh_helpers
from matrix import (ERRORS, HOST_IDLE, PACKET, QUEUE, ROOT, RUNNER_SHA, SUITE, TOPOLOGY, WORK,
                    diagnostic_reconciler, run, require, score_arm)

ID = "dense-matched-topology-bulk-pair"
ORDERS = (("scalar", "bulk"), ("bulk", "scalar"))
FILES = [SUITE / "worker.py", TOPOLOGY / "arm.py", PACKET / "native-bulk.sh", PACKET / "bulk-arm.py"]


def check_writer(mode, row, runner_sha, bulk_sha):
    require(row["mode"] == mode and row["runner_sha256"] == runner_sha
            and row["migration_sha256"] == bulk_sha
            and row["producer_sha256"] == (bulk_sha if mode == "bulk" else runner_sha)
            and (row["bulk_calls"], row["scalar_calls"]) == ((8, 0) if mode == "bulk" else (0, 1024)),
            "producer identity or true-bulk query counts differ")


def local():
    report = {}
    runner_sha = "16af053cf4aec1d6c8529ec615eb6b243a9caf135b526f4b3aa41c9e370ba06e"
    bulk_sha = "78b12a8efaff7ae0ac4e1b1457d255c688b77825df4cbf07fc4fc873e8ad4d03"
    for mode in ORDERS[0]:
        path = WORK / f"local-{mode}"
        ledger = run.read_json(path / "ledger.json")
        artifact = {key: ledger[key] for key in ("version", "artifact_sha256")}
        writers = run.read_json(path / "producer-writer.json")
        check_writer(mode, writers, runner_sha, bulk_sha)
        score = diagnostic_reconciler(2)(ledger, "mesh", "noop", artifact)
        require(score["completed"] == 1024 and len(ledger["transactions"]) == 1
                and len({job[key] for job in ledger["jobs"]
                         for key in ("attempt_xid", "effect_xid", "settled_xid")}
                        | {ledger["transactions"][0]["xid"]}) == 3073, "incorrect bulk settlement or app evidence")
        def files(name):
            return run.read_json(path / name)

        topology = files("topology.json")
        raw = files("raw.json")
        require(topology["runners"] == 2 and topology["slots_per_runner"] == 32
                and topology["app_sessions_per_runner"] == 16 and raw["raw_application_sql_calls"] == 4096,
                "local bulk composition changed the runner or app contract")
        helpers = mesh_helpers()
        helpers["verify_app"](files, ledger["jobs"], raw["namespace"])
        used = helpers["verify_sessions"](files, dict(parity_subject=8, compiled_caller=32, parity_producer=1),
                                           score["first_submit_us"], set())
        report[mode] = dict(jobs=score["completed"], writer=writers, used_application_sessions=len(used))
    sample = run.read_json(TOPOLOGY / "evidence/native-01/summary.json")
    screen = PACKET / "producer-screen-historical.json"
    require(screen.is_file(), "producer screen missing for the historical envelope")
    producer = run.read_json(screen)["noop"]["bulk"]["wall_ms"]
    envelope = []
    for group in sample["groups"]:
        arm = group["arms"]["tuned-2x32"]
        drain = arm["last_settlement_us"] - arm["first_commit_us"]
        envelope.append(dict(native_submit_to_commit_ms=arm["first_submit_to_commit_ms"],
                             native_commit_jobs_per_s=arm["commit_jobs_per_s"],
                             zero_submit_upper_if_drain_fixed=arm["commit_jobs_per_s"],
                             modeled_if_drain_fixed_jobs_per_s={
                                 k: 1024 * 1_000_000 / (drain + producer[k] * 1000)
                                 for k in ("min", "median", "max")}))
    return dict(local=report, historical_envelope=envelope, producer_screen_wall_ms=producer,
                scope="The prior producer-only timings and native two-runner drain came from separate runs."
                      " These values bound the fixed-drain model and are not native combined results.")


def inventory(config, mode, remote):
    stock = config["inventory"]["engines"]["mesh"]
    _, env, wrapper, _ = stock["argv"]
    producer = config["bulk_producer"]
    spec = dict(stock, argv=["/bin/bash", str(remote / "native-bulk.sh"), str(Path(wrapper).parents[3]),
                            env.split("=", 1)[1], "tuned-2x32", producer["path"], mode],
                verified_files={**stock["verified_files"], producer["path"]: producer["sha256"],
                                **{str(remote / file.name): run.digest(file) for file in FILES}})
    result = dict(config["inventory"], engines={**config["inventory"]["engines"], "mesh": spec})
    run.check_inventory(result)
    return result


def load():
    request = run.read_json(PACKET / "request-bulk.json")
    require(request["id"] == ID and request["ready"], "bulk request differs")
    for path, digest in request["local_sha256"].items():
        require(run.digest(ROOT / path) == digest, f"bulk pin drift: {path}")
    config = run.read_json(PACKET / "bulk.json")
    for mode in ORDERS[0]:
        inventory(config, mode, run.REMOTE / ID)
    subprocess.run(["bash", "-n", PACKET / "native-bulk.sh"], check=True)
    return config, request


def grant():
    entries = re.findall(r"^(SETUP|TIMING|HOLD|RELEASED)\s+(\S+)\s+(\S+)\s+(\S+)",
                         (QUEUE / "STATUS.md").read_text(), re.M)
    require(entries and entries[-1][:2] == ("TIMING", ID), "no active bulk TIMING assignment")
    _, _, start, end = entries[-1]
    require(datetime.fromisoformat(start) <= datetime.now(timezone.utc) < datetime.fromisoformat(end),
            "outside bulk TIMING assignment")
    path = PACKET / "request-bulk.json"
    require(path.read_bytes() == (QUEUE / f"{ID}.json").read_bytes(), "bulk request seal differs")
    return dict(start=start, end=end, request_sha256=run.digest(path))


def replay(out, config):
    manifest = run.read_json(out / "series.json")
    require(manifest["config"] == config and manifest["orders"] == [list(order) for order in ORDERS],
            "unexpected bulk series manifest")
    rows, failures = {}, {}
    for index, order in enumerate(ORDERS, 1):
        for phase in ("warmup", "measured"):
            for mode in order:
                folder = out / f"{index:02}" / f"{phase}-{mode}" / f"{phase}-mesh"
                label = str(folder.relative_to(out))
                try:
                    remote = run.REMOTE / manifest["remotes"][mode]
                    score = score_arm(folder, "mesh", 2, phase, inventory(config, mode, remote), config)
                    with tarfile.open(folder / "raw.tar.gz") as tar:
                        writers = run.file_json(tar, "adapter/producer-writer.json")
                        check_writer(mode, writers, RUNNER_SHA, config["bulk_producer"]["sha256"])
                        score["writer"] = writers
                    rows[label] = score
                except ERRORS as error:
                    failures[label] = f"{type(error).__name__}: {error}"
    return dict(arms=rows, failures=failures)


def dispatch(out, config):
    receipt = grant()
    with (SUITE / ".timing.lock").open("rb") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        run.ssh(["bash", "-c", HOST_IDLE], timeout=12).check_returncode()
        out.mkdir()
        salt = uuid4().hex[:12]
        remotes = {mode: f"{ID}-{salt}-{mode}" for mode in ORDERS[0]}
        run.write_json(out / "series.json", dict(config=config, grant=receipt, orders=ORDERS, remotes=remotes))
        for mode in ORDERS[0]:
            remote = run.REMOTE / remotes[mode]
            folder = out / "staging" / mode
            folder.mkdir(parents=True)
            inv = folder / "inventory.json"
            run.write_json(inv, inventory(config, mode, remote))
            with (folder / "transfer.log").open("xb") as log:
                for file in [*FILES, inv]:
                    run.stage(file, remote, log)
        for index, order in enumerate(ORDERS, 1):
            for phase in ("warmup", "measured"):
                for mode in order:
                    grant()
                    path = out / f"{index:02}" / f"{phase}-{mode}"
                    path.mkdir(parents=True)
                    run.dispatch_arm(path, run.REMOTE / remotes[mode], {"profile": "noop"}, phase, "mesh")
                    require(not run.read_json(path / f"{phase}-mesh/dispatch.json").get("interrupted"),
                            "controller must verify ambiguous bulk exit")
        run.ssh(["bash", "-c", HOST_IDLE], timeout=12).check_returncode()
    result = replay(out, config)
    run.write_json(out / "summary.json", result)
    return bool(result["failures"])


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("local", "check", "dispatch", "replay"))
    parser.add_argument("--directory", type=Path, default=WORK / "bulk")
    args = parser.parse_args()
    if args.action == "local":
        print(json.dumps(local(), indent=2, sort_keys=True))
        sys.exit(0)
    config, _ = load()
    if args.action == "dispatch":
        sys.exit(dispatch(args.directory, config))
    if args.action == "replay":
        result = replay(args.directory, config)
        print(json.dumps(result, indent=2, sort_keys=True))
        sys.exit(bool(result["failures"]))
