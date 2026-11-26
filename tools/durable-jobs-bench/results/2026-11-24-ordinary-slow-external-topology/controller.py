#!/usr/bin/env python3
"""Seal, replay, and dispatch the controller-owned slow EXTERNAL topology trial."""
import argparse
from copy import deepcopy
from datetime import datetime, timedelta, timezone
import fcntl
import importlib.util
import json
from pathlib import Path
import re
from statistics import median
import sys
import tarfile
import time

PACKET = Path(__file__).resolve().parent
SUITE = PACKET.parents[1]
ROOT = SUITE.parents[1]
STORAGE = PACKET.parent / "2026-11-24-slow-external-compat-scorer-input"
TOPOLOGY = PACKET.parent / "2026-11-23-ordinary-slow-runner-concentration"
FROZEN = PACKET / "frozen"
OUT = PACKET / "evidence/native"
REQUESTS = ROOT / "target/linode-jobs-stepchange/requests"
ID = "ordinary-slow-external-topology"
MESH = ("one-a", "one-b", "two")
PEERS = {"river-stock-1x64": "river-external", "oban-stock-1x64": "oban-external",
         "apalis-stock-1x64": "apalis-external"}
PHASES = ["warmup", "measured"]
PILOT, REMAINDER = (1, 2), (3, 4, 5, 6)
METRICS = ("submit_ms", "drain_ms", "submit_to_commit_ms", "commit_to_first64_ms",
           "late_refill_ms", "last_first_write_to_terminal_ms")
ERRORS = (OSError, ValueError, KeyError, TypeError, IndexError, StopIteration, tarfile.TarError)

sys.path.insert(0, str(SUITE))
import run as matrix
from interleave import HOST_IDLE
from verify import require

def load(name, file):
    original = sys.path[:]
    sys.path.insert(0, str(file.parent))
    try:
        spec = importlib.util.spec_from_file_location(name, file)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        return module
    finally:
        sys.path[:] = original

prior_topology = load("qualified_topology_controller", TOPOLOGY / "native.py")
prior_storage = load("qualified_storage_controller", STORAGE / "replay/native.py")
STAGE_FILES = ("native-run", "native_overlay.py", "layout.py", "storage.py", "install-layout.sql", "topology.py")


def design(destination):
    original = matrix.read_json(STORAGE / "frozen/native-config.json")
    require(original["profiles"] == ["slow"] and original["cpus"] == [0, 1], "wrong EXTERNAL baseline")
    lanes = {}
    for label in MESH:
        lane = deepcopy(original["lanes"]["bulk-external"])
        spec = lane["inventory"]["engines"]["mesh"]
        require(spec["artifact_sha256"] == prior_topology.NATIVE_MANIFEST
                and prior_topology.NATIVE_ELF in spec["verified_files"].values()
                and lane["policy"] == "external", "wrong compiled Mesh or storage policy")
        env, artifact, launch, *remaining = spec["argv"]
        prior_stage = original["remote"] + "/"
        require(launch == prior_stage + "native-run" and remaining[:2] == ["mesh", "external"],
                "wrong source invocation")
        spec["argv"] = [env, artifact, f"MESH_EXTERNAL_TOPOLOGY={label}",
                        f"{destination}/native-run", *remaining]
        spec["verified_files"] = {
            **{path: digest for path, digest in spec["verified_files"].items() if not path.startswith(prior_stage)},
            **{f"{destination}/{name}": matrix.digest(PACKET / "stage" / name) for name in STAGE_FILES},
        }
        matrix.check_inventory(lane["inventory"])
        lanes[label] = lane
    for label, name in PEERS.items():
        lane = original["lanes"][name]
        require(lane["policy"] == "external", "peer storage mismatch")
        matrix.check_inventory(lane["inventory"])
        lanes[label] = lane
    labels = list(lanes)
    offsets = (0, 1, -1, 2, -2, 3)
    orders = [[labels[(start + offset) % 6] for offset in offsets] for start in (0, 3, 1, 4, 2, 5)]
    require(orders[0] == orders[1][::-1], "pilot orders must be opposite")
    plan = [dict(group=i, profile="slow", phases=PHASES, order=order)
            for i, order in enumerate(orders, 1)]
    return {**original, "lanes": lanes, "orders": orders, "plan": plan,
            "description": "EXTERNAL slow topology crossover at cap 68 with stock 1x64 peers",
            "composition_stage": destination}


def installer(stage, digest):
    original = (STORAGE / "frozen/native-install.sh").read_text()
    previous = matrix.digest(STORAGE / "frozen/native-overlay.tar")
    old_stage = f"slow-external-{previous[:16]}"
    require(original.count(previous) == original.count(old_stage) == 1, "qualified install script changed")
    return original.replace(previous, digest).replace(old_stage, Path(stage).name)


def freeze():
    archive = FROZEN / "mesh-overlay.tar"
    with tarfile.open(archive, "x", format=tarfile.USTAR_FORMAT) as target:
        for name in STAGE_FILES:
            path = PACKET / "stage" / name
            info = tarfile.TarInfo(name)
            info.size = path.stat().st_size
            info.mode = 0o755 if name == "native-run" else 0o644
            with path.open("rb") as stream:
                target.addfile(info, stream)
    digest = matrix.digest(archive)
    stage = f"/var/lib/durable-jobs-bench/native/slow-external-topology-{digest[:16]}"
    matrix.write_json(FROZEN / "series.json", design(stage))
    (FROZEN / "install-mesh.sh").write_text(installer(stage, digest))


def check():
    request = matrix.read_json(PACKET / "request.json")
    require(request["id"] == ID and request["ready"] is True and request["state"] == "ready",
            "wrong controller request")
    for rel, sha in request["local_sha256"].items():
        require(matrix.digest(ROOT / rel) == sha, f"request input changed: {rel}")
    archive = FROZEN / "mesh-overlay.tar"
    digest = matrix.digest(archive)
    stage = f"/var/lib/durable-jobs-bench/native/slow-external-topology-{digest[:16]}"
    config = matrix.read_json(FROZEN / "series.json")
    require(config == design(stage) and request["orders"] == config["orders"], "sealed lane matrix differs")
    require(installer(stage, digest) == (FROZEN / "install-mesh.sh").read_text(), "sealed installer differs")
    with tarfile.open(archive) as package:
        require(package.getnames() == list(STAGE_FILES), "unexpected overlay members")
        for name in STAGE_FILES:
            item = package.getmember(name)
            mode = 0o755 if name == "native-run" else 0o644
            require(item.isfile() and item.mode == mode
                    and package.extractfile(item).read() == (PACKET / "stage" / name).read_bytes(),
                    f"sealed overlay differs: {name}")
    require((FROZEN / "canonical-batch.json.gz").read_bytes() ==
            (STORAGE / "frozen/canonical-batch.json.gz").read_bytes(), "canonical input bytes changed")
    return config, request


def clocks(ledger):
    job_times = sorted(job["attempt_committed_us"] for job in ledger["jobs"])
    txn, = ledger["transactions"]
    submitted, committed = txn["submitted_us"], txn["committed_us"]
    terminal = max(job["settled_us"] for job in ledger["jobs"])
    require(len(job_times) == 512 and submitted <= committed <= job_times[0] <= job_times[-1] <= terminal,
            "inconsistent durable phase clocks")
    changes = sorted((job[key], change) for job in ledger["jobs"]
                     for key, change in (("attempt_committed_us", 1), ("effect_committed_us", -1)))
    live = peak = 0
    for _, change in changes:
        live += change
        peak = max(peak, live)
    require(live == 0 and peak == 64, f"expected 64 overlapping committed holds, got {peak}")
    return dict(peak_committed_holds=peak, submit_ms=(terminal-submitted)/1000,
                drain_ms=(terminal-committed)/1000, submit_to_commit_ms=(committed-submitted)/1000,
                commit_to_first64_ms=(job_times[63]-committed)/1000,
                late_refill_ms=(job_times[-1]-job_times[63])/1000,
                last_first_write_to_terminal_ms=(terminal-job_times[-1])/1000)


def warm_backends(files, label, metric):
    topology = files("topology.json")
    before, after = (files(f"sessions-{name}.json") for name in ("before", "end"))
    mapping = topology["warm_app_backends_by_runner"]
    pids = {str(pid) for pid in topology["runner_pids_by_index"]}
    require(mapping.keys() == pids and all(len(warm) == topology["app_sessions_per_runner"]
                                           for warm in mapping.values()), "wrong per-runner warm app pool")
    expected = {(row["backend"], row["client_port"]) for warm in mapping.values() for row in warm}
    require(len(expected) == 8, "shared or missing warm backend")
    for snapshot in (before, after):
        actual = {(row["backend"], row["client_port"]) for row in snapshot if row["role"] == "compiled_caller"}
        require(actual == expected, "warm app backend sockets changed")
    require(all(row["jobs"] > 0 for row in metric["runner_ownership"]),
            f"{label}: an expected runner never handled this workload")
    return mapping


def score(folder, config, label, phase):
    lane = config["lanes"][label]
    result = prior_topology.score(folder, config, label, phase, lane["inventory"])
    engine = lane["engine"]
    with tarfile.open(folder / "raw.tar.gz") as archive:
        def files(name):
            return matrix.file_json(archive, "adapter/" + name)
        ledger = files("ledger.json")
        result.update(prior_storage.verify_storage(files("payload-storage.json"), ledger, engine, "external"))
        result.update(clocks(ledger))
        if engine == "mesh":
            result["warm_app_backends_by_runner"] = warm_backends(files, label, result)
    return result


def arms(path, block, config):
    for phase in PHASES:
        for label in block["order"]:
            engine = config["lanes"][label]["engine"]
            yield phase, label, engine, path / f"{phase}-{label}" / f"{phase}-{engine}"


def score_block(path, block, config):
    require(matrix.read_json(path / "order.json") == block, "recorded arm order differs")
    return {f"{phase}-{label}": score(folder, config, label, phase)
            for phase, label, _, folder in arms(path, block, config)}


def continuation(groups):
    missing = [number for number in PILOT if number not in groups]
    if missing:
        return dict(eligible=False, missing=list(missing))
    scores = [groups[number] for number in PILOT]
    evidence = {}
    for key in ("late_refill_ms", "submit_ms"):
        gains, spreads = [], []
        for group in scores:
            a, b, treatment = (group[f"measured-{label}"][key] for label in MESH)
            gains.append(min(a, b) - treatment)
            spreads.append(abs(a - b))
        threshold = max(100.0 if key == "late_refill_ms" else 0.0, median(spreads))
        evidence[key] = dict(gains_ms=gains, duplicate_spreads_ms=spreads, threshold_ms=threshold,
                             passes=all(gain > 0 for gain in gains) and median(gains) >= threshold)
    return dict(eligible=all(metric["passes"] for metric in evidence.values()), metrics=evidence)


def replay(config):
    selected, groups, errors = {}, {}, {}
    for block in config["plan"]:
        number = block["group"]
        for path in sorted((OUT / "blocks").glob(f"{number:02d}-slow.*")):
            try:
                scored = score_block(path, block, config)
                checkpoint = path / "reconciled.json"
                if checkpoint.exists():
                    require(scored == matrix.read_json(checkpoint), "replay differs from reconciled block")
                if number not in selected:
                    selected[number], groups[number] = path.name, scored
            except ERRORS as error:
                errors[path.name] = f"{type(error).__name__}: {error}"
    paired = {}
    for label in ("one-a", "one-b", *PEERS):
        paired[label] = {}
        for metric in METRICS:
            values = [group[f"measured-{label}"][metric] - group["measured-two"][metric]
                      for group in groups.values()]
            if values:
                paired[label][metric] = dict(by_group_ms=values, median_ms=median(values),
                                              wins=sum(value > 0 for value in values), n=len(values))
    return dict(selected=selected, groups=groups, errors=errors, pilot_gate=continuation(groups),
                paired_comparator_minus_two_ms=paired, missing=[i for i in (*PILOT, *REMAINDER) if i not in groups])


def assignment(seconds):
    events = re.findall(r"^(SETUP|TIMING|RELEASED|HOLD)\s+(\S+)\s+(\S+)\s+(\S+)",
                        (REQUESTS / "STATUS.md").read_text(), re.M)
    require(events and events[-1][:2] == ("TIMING", ID), "no active sole-controller TIMING assignment")
    _, _, start, end = events[-1]
    now = datetime.now(timezone.utc)
    require(datetime.fromisoformat(start) <= now < datetime.fromisoformat(end) - timedelta(seconds=seconds),
            "outside controller grant or insufficient time")
    require((PACKET / "request.json").read_bytes() == (REQUESTS / f"{ID}.json").read_bytes(),
            "controller request bytes differ")
    return dict(start=start, end=end, request_sha256=matrix.digest(PACKET / "request.json"))


def dispatch(config, groups):
    assignment(240)
    with (SUITE / ".timing.lock").open("rb") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        grant = assignment(240)
        existing = replay(config)
        if groups == REMAINDER:
            require(existing["pilot_gate"]["eligible"], "the two opposite rotations do not clear the duplicate gate")
        pending = [number for number in groups if number not in existing["selected"]]
        if not pending:
            return 0
        OUT.mkdir(parents=True, exist_ok=True)
        matrix.write_json(OUT / f"grant-{time.time_ns()}.json", grant)
        for name, inputs, script, bundle in (
            ("peers", STORAGE / "frozen", "native-install.sh", "native-overlay.tar"),
            ("mesh", FROZEN, "install-mesh.sh", "mesh-overlay.tar"),
        ):
            assignment(180)
            with (OUT / f"install-{name}-{time.time_ns()}.log").open("xb") as log, (inputs / bundle).open("rb") as data:
                matrix.ssh(["bash", "-c", (inputs / script).read_text()], stdin=data, stdout=log,
                           stderr=log, timeout=30).check_returncode()
        remotes = {}
        for label, lane in config["lanes"].items():
            assignment(180)
            remote = matrix.REMOTE / f"{ID}-{grant['request_sha256'][:12]}-{label}"
            remotes[label] = remote
            staging = OUT / "staging" / label
            staging.mkdir(parents=True, exist_ok=True)
            inv = staging / "inventory.json"
            if not inv.exists():
                matrix.write_json(inv, lane["inventory"])
            require(matrix.read_json(inv) == lane["inventory"], "staging inventory differs")
            with (staging / f"transfer-{time.time_ns()}.log").open("xb") as log:
                matrix.stage(SUITE / "worker.py", remote, log)
                matrix.stage(inv, remote, log)
        for number in pending:
            assignment(240)
            matrix.ssh(["bash", "-c", HOST_IDLE], timeout=12).check_returncode()
            block = config["plan"][number-1]
            for previous in sorted((OUT / "blocks").glob(f"{number:02d}-slow.*")):
                for _, label, _, folder in arms(previous, block, config):
                    request = folder / "request.json"
                    if request.is_file() and not (folder / "raw.tar.gz").is_file():
                        assignment(120)
                        matrix.archive_arm(folder, remotes[label], matrix.read_json(request)["attempt"])
            if number in replay(config)["selected"]:
                continue
            path = OUT / "blocks" / f"{number:02d}-slow.{time.time_ns()}"
            path.mkdir(parents=True)
            matrix.write_json(path / "order.json", block)
            scored = {}
            for phase, label, engine, folder in arms(path, block, config):
                assignment(90)
                folder.parent.mkdir()
                matrix.dispatch_arm(folder.parent, remotes[label], block, phase, engine)
                final = matrix.read_json(folder / "dispatch.json")
                require(final.get("exit_code") == 0 and not final.get("interrupted"),
                        f"{phase}-{label}: controller must reconcile the failed or ambiguous exit")
                scored[f"{phase}-{label}"] = score(folder, config, label, phase)
            matrix.write_json(path / "reconciled.json", scored)
        result = replay(config)
        matrix.write_json(OUT / f"summary-{time.time_ns()}.json", result)
        return int(bool(set(groups) & set(result["missing"])))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("freeze", "check", "replay", "pilot", "remainder"))
    args = parser.parse_args()
    if args.action == "freeze":
        return freeze()
    config, _ = check()
    if args.action == "check":
        print("verified six EXTERNAL lanes, exact binaries and PostgreSQL pins, sealed opposite-order pilot and gated remainder")
    elif args.action == "replay":
        result = replay(config)
        print(json.dumps(result, indent=2, sort_keys=True))
        return int(bool(set(PILOT) & set(result["missing"])))
    else:
        return dispatch(config, PILOT if args.action == "pilot" else REMAINDER)


if __name__ == "__main__":
    sys.exit(main())
