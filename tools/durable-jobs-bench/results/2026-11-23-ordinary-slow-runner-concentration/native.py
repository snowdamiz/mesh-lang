"""Offline qualification and sole-controller dispatch of the slow stock-peer matrix."""

import argparse
from datetime import datetime, timezone
import fcntl
import json
from pathlib import Path
import re
import subprocess
import sys
import tarfile
import time
from uuid import uuid4

from replay import CAP, JOBS, SUITE, distribution, summarize, verify_arm
from slow_arm import HERE, LAYOUTS, TOPOLOGY

import run
from interleave import HOST_IDLE, host_cpu
from verify import require, xid

ROOT = HERE.parents[3]
LANE = ROOT / "target/ordinary-slow-runner-concentration"
QUEUE = ROOT / "target/linode-jobs-stepchange/requests"
IDENT = "ordinary-slow-runner-concentration"
V1 = HERE.parent / "2026-11-23-set-based-enqueue/evidence/bulk-enqueue-v1-slow/series.json"
NATIVE_ELF = "e86640db9d7dcd9b7c7fff7e8a2183000cdbbb0c5d277f3544ebf88438d60e25"
NATIVE_MANIFEST = "e7e895966dcc72ab42a89bd892b1131eb7436561a8b303848355062fbf80946a"
PEERS = {"river-stock-1x64": "river", "oban-stock-1x64": "oban", "apalis-stock-1x64": "apalis"}
DRIVERS = (Path(__file__), HERE / "native-mesh.sh", HERE / "slow_arm.py", HERE / "replay.py",
           TOPOLOGY / "arm.py", TOPOLOGY / "replay.py", SUITE / "worker.py", SUITE / "verify.py",
           HERE.parent / "2026-11-23-ordinary-claim-turns/replay.py", SUITE / "run.py", SUITE / "interleave.py", V1)
ERRORS = (OSError, ValueError, TypeError, KeyError, IndexError, StopIteration, tarfile.TarError)


def freeze(candidate):
    require(candidate in ("two", "four"), "unsupported native topology")
    original = run.read_json(V1)["config"]
    lanes = {label: original["lanes"]["bulk"] for label in ("one-a", "one-b", candidate)}
    lanes.update({label: original["lanes"][peer] for label, peer in PEERS.items()})
    require(original["profiles"] == ["slow"] and original["cpus"] == [0, 1], "wrong retained slow profile")
    spec = lanes["one-a"]["inventory"]["engines"]["mesh"]
    require(NATIVE_ELF in spec["verified_files"].values() and spec["artifact_sha256"] == NATIVE_MANIFEST,
            "wrong retained V1 bulk ELF or manifest")
    labels = list(lanes)
    offsets = (0, 1, -1, 2, -2, 3)
    # The second block independently reverses the first. All six orders use
    # each position and immediate predecessor once for each treatment.
    orders = [[labels[(start + offset) % 6] for offset in offsets] for start in (0, 3, 1, 4, 2, 5)]
    require(orders[1] == orders[0][::-1], "first two native blocks are not opposite")
    return {**original, "lanes": lanes, "candidate": candidate,
            "plan": [dict(group=index, profile="slow", phases=["warmup", "measured"], order=order)
                     for index, order in enumerate(orders, 1)]}


def staged(config, label, remote):
    lane = config["lanes"][label]
    inventory, engine = lane["inventory"], lane["engine"]
    additions = [SUITE / "worker.py"]
    if engine == "mesh":
        additions += [HERE / "native-mesh.sh", HERE / "slow_arm.py", TOPOLOGY / "arm.py"]
        spec = inventory["engines"][engine]
        _, toggle, artifacts, launch, _ = spec["argv"]
        require(toggle == "MESH_JOBS_SET_BASED_ENQUEUE=1", "stock lane no longer uses true bulk")
        source = str(Path(launch).parents[3])
        spec = dict(spec, argv=["/bin/bash", str(remote / "native-mesh.sh"), source,
                               artifacts.split("=", 1)[1], label],
                    verified_files={**spec["verified_files"],
                                    **{str(remote / path.name): run.digest(path) for path in additions[1:]}})
        inventory = dict(inventory, engines={**inventory["engines"], engine: spec})
    run.check_inventory(inventory)
    return additions, inventory


def grant(request):
    events = re.findall(r"^(SETUP|TIMING|RELEASED|HOLD)\s+(\S+)\s+(\S+)\s+(\S+)",
                        (QUEUE / "STATUS.md").read_text(), flags=re.M)
    require(events and events[-1][:2] == ("TIMING", IDENT), "no active sole-controller TIMING assignment")
    _, _, start, end = events[-1]
    now = datetime.now(timezone.utc)
    require(datetime.fromisoformat(start) <= now < datetime.fromisoformat(end), "outside TIMING assignment")
    require(request.read_bytes() == (QUEUE / f"{IDENT}.json").read_bytes(), "controller request bytes differ")
    return dict(start=start, end=end, request_sha256=run.digest(request))


def score(folder, config, label, phase, inventory):
    engine = config["lanes"][label]["engine"]
    request = run.read_json(folder / "request.json")
    spec = inventory["engines"][engine]
    with tarfile.open(folder / "raw.tar.gz") as archive:
        def files(name):
            return run.file_json(archive, "adapter/" + name)

        def logs(name):
            member = archive.getmember("adapter/" + name)
            require(member.isfile(), "invalid archived log")
            with archive.extractfile(member) as stream:
                return stream.read().decode()

        invoked = run.file_json(archive, "invocation.json")
        host = run.file_json(archive, "host.json")
        expected = dict(phase=phase, profile="slow", engine=engine, attempt=request["attempt"])
        require(all(invoked[key] == request[key] == value for key, value in expected.items())
                and run.file_json(archive, "status.json")["exit_code"] == 0
                and [host["boot_id"], host["kernel"], host["pg_mount"]["maj:min"]] == config["host"]
                and host["affinity"] == config["cpus"] and len(host["available_cpus"]) == 4
                and host["verified_files"] == spec["verified_files"], "native invocation, host, or pin mismatch")
        ledger = files("ledger.json")
        artifact = {key: spec[key] for key in ("version", "artifact_sha256")}
        if engine == "mesh":
            result = verify_arm(files, logs, label, dict(artifact, executable_sha256=NATIVE_ELF),
                                local=False, phase=phase)
            require(files("producer-writer.json")["enabled"] is True, "native V1 bulk toggle drift")
        else:
            # Canonical stock peer validation keeps the real one-runner ledger.
            result = run.score_arm(folder, {"profile": "slow"}, phase, engine, inventory)
            seq, = ledger["transactions"]
            app = {xid(job[key], key) for job in ledger["jobs"] for key in ("attempt_xid", "effect_xid")}
            terminal = {xid(job["settled_xid"], "terminal") for job in ledger["jobs"]}
            producer = xid(seq["xid"], "producer")
            require(ledger["runners"] == 1 and ledger["slots"] == 64 and result["completed"] == JOBS
                    and len(app) == JOBS * 2 and app.isdisjoint(terminal)
                    and producer not in app | terminal and 0 < len(terminal) <= JOBS
                    and ledger.get("local_smoke", False) is False
                    and ledger.get("phase", phase) == phase, "stock peer topology or transaction contract differs")
            if engine == "oban":
                masks = files("process-affinity.json")
                require(len(masks) == 2 and all(mask == config["cpus"] for mask in masks.values()),
                        "stock Oban or PostgreSQL affinity differs")
            result.update(layout=[1, 64], drain_ms=(result["last_settlement_us"] - result["first_commit_us"]) / 1000,
                          submit_ms=(result["last_settlement_us"] - result["first_submit_us"]) / 1000,
                          application_xids=len(app), terminal_xids=len(terminal))
        with archive.extractfile("host-samples.jsonl") as stream:
            result.update(run.sample_window(host, [json.loads(line) for line in stream], result))
    result.update(host_cpu(folder, result, config["clock_ticks"]))
    result["raw_sha256"] = run.digest(folder / "raw.tar.gz")
    return result


def paths(path, config, block):
    for phase in block["phases"]:
        for label in block["order"]:
            engine = config["lanes"][label]["engine"]
            yield phase, label, engine, path / f"{phase}-{label}" / f"{phase}-{engine}"


def replay_block(path, config, block, remotes):
    require(run.read_json(path / "order.json") == block, "recorded native order differs")
    results = {}
    for phase, label, _, folder in paths(path, config, block):
        _, inventory = staged(config, label, run.REMOTE / remotes[label])
        results[f"{phase}-{label}"] = score(folder, config, label, phase, inventory)
    return results


def select(directory, config, block, remotes, issues):
    for path in sorted((directory / "blocks").glob(f"{run.block_prefix(block)}.*")):
        try:
            return path.name, replay_block(path, config, block, remotes)
        except ERRORS as error:
            issues[path.name] = f"{type(error).__name__}: {error}"
    return None


def replay(directory, config, pins):
    manifest = run.read_json(directory / "series.json")
    require(manifest["config"] == config and manifest["pins"] == pins, "native series pins changed")
    selected, groups, missing, issues = {}, [], [], {}
    for block in config["plan"]:
        result = select(directory, config, block, manifest["remotes"], issues)
        if result is None:
            missing.append(block["group"])
            continue
        name, phases = result
        selected[name] = phases
        groups.append(dict(group=block["group"], order=block["order"],
                           arms={label: phases[f"measured-{label}"] for label in block["order"]}))
    candidate = config["candidate"]
    peers = {peer: {field: distribution([group["arms"][peer][field] - group["arms"][candidate][field]
                                        for group in groups]) for field in ("submit_ms", "drain_ms")}
             for peer in PEERS} if groups else {}
    return dict(missing=missing, attempts=selected, issues=issues, peer_minus_candidate_ms=peers,
                summary=summarize(groups, candidate) if groups else {})


def dispatch(directory, config, pins, request, groups):
    assignment = grant(request)
    with (SUITE / ".timing.lock").open("rb") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        run.ssh(["bash", "-c", HOST_IDLE], timeout=12).check_returncode()
        directory.mkdir(exist_ok=True)
        manifest_path = directory / "series.json"
        if not manifest_path.exists():
            session = IDENT + "-" + uuid4().hex[:12]
            run.write_json(manifest_path, dict(config=config, pins=pins,
                           remotes={label: f"{session}-{label}" for label in config["lanes"]}))
        manifest = run.read_json(manifest_path)
        require(manifest["config"] == config and manifest["pins"] == pins, "native series pins changed")
        remotes = manifest["remotes"]
        run.write_json(directory / f"grant-{time.time_ns()}.json", assignment)
        for label in config["lanes"]:
            grant(request)
            remote = run.REMOTE / remotes[label]
            additions, inventory = staged(config, label, remote)
            staging = directory / "staging" / label
            staging.mkdir(parents=True, exist_ok=True)
            inv = staging / "inventory.json"
            if not inv.exists():
                run.write_json(inv, inventory)
            require(run.read_json(inv) == inventory, "native inventory changed")
            with (staging / f"stage-{time.time_ns()}.log").open("xb") as log:
                for path in [*additions, inv]:
                    run.stage(path, remote, log)
        for block in config["plan"]:
            if block["group"] not in groups:
                continue
            grant(request)
            run.ssh(["bash", "-c", HOST_IDLE], timeout=12).check_returncode()
            for previous in sorted((directory / "blocks").glob(f"{run.block_prefix(block)}.*")):
                for _, label, _, folder in paths(previous, config, block):
                    if (folder / "request.json").is_file():
                        invocation = run.read_json(folder / "request.json")
                        run.archive_arm(folder, run.REMOTE / remotes[label], invocation["attempt"])
            if select(directory, config, block, remotes, {}):
                continue
            path = directory / "blocks" / f"{run.block_prefix(block)}.{time.time_ns()}"
            path.mkdir(parents=True)
            run.write_json(path / "order.json", block)
            for phase, label, engine, folder in paths(path, config, block):
                grant(request)
                folder.parent.mkdir()
                run.dispatch_arm(folder.parent, run.REMOTE / remotes[label], block, phase, engine)
                require(not run.read_json(folder / "dispatch.json").get("interrupted"),
                        "ambiguous remote exit; controller needs to reconcile")
            run.write_json(path / "reconciled.json", replay_block(path, config, block, remotes))
        run.ssh(["bash", "-c", HOST_IDLE], timeout=12).check_returncode()
    result = replay(directory, config, pins)
    run.write_json(directory / f"summary-{time.time_ns()}.json", result)
    return bool(set(result["missing"]) & set(groups))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("check", "dispatch", "replay"))
    parser.add_argument("--candidate", choices=("two", "four"), default="two")
    parser.add_argument("--group", action="append", type=int, choices=range(1, 7))
    args = parser.parse_args()
    config = freeze(args.candidate)
    pins = {str(path.relative_to(ROOT)): run.digest(path) for path in DRIVERS}
    for label in config["lanes"]:
        staged(config, label, run.REMOTE / label)
    subprocess.run(["bash", "-n", str(HERE / "native-mesh.sh")], check=True)
    if args.action == "check":
        print(json.dumps(dict(config=config, pins=pins), sort_keys=True, indent=2))
        return 0
    directory = LANE / "native-stock-peers"
    if args.action == "replay":
        print(json.dumps(replay(directory, config, pins), sort_keys=True, indent=2))
        return 0
    request = HERE / "request-stock-peers.json"
    doc = run.read_json(request)
    require(doc["id"] == IDENT and doc["ready"] is True and doc["local_sha256"] == pins
            and doc["candidate"] == args.candidate, "controller packet missing or differs from checked pins")
    return dispatch(directory, config, pins, request, args.group or [1, 2])


if __name__ == "__main__":
    sys.exit(main())
