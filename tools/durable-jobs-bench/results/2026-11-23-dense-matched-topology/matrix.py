#!/usr/bin/env python3
"""Replay or dispatch five controller-owned rotations of the frozen dense batch."""

import argparse
from datetime import datetime, timezone
import fcntl
import json
from pathlib import Path
import re
from statistics import median
import subprocess
import sys
import tarfile
import time
from uuid import uuid4

from qualification import (CAP, CPUS, INPUT, JOBS, PACKET, ROOT, RUNNER_SHA, SLOTS, STOCK_MESH, SUITE,
                           TOPOLOGY, diagnostic_reconciler, mesh_topology, peer_topology, require, run, text)
from interleave import HOST_IDLE, host_cpu, plan

WORK = ROOT / "target/native-dense-matched-topology"
QUEUE = ROOT / "target/linode-jobs-stepchange/requests"
SERIES = {"stock": "dense-matched-topology-stock", "equal": "dense-matched-topology-equal"}
METRICS = ("submit_jobs_per_s", "commit_jobs_per_s", "first_submit_to_commit_ms",
           "runner_cpu_ms_per_job", "pinned_user_cpu_ms_per_job", "pinned_kernel_cpu_ms_per_job",
           "peak_connections", "peak_executing", "cpu_steal_percent", "cpu_iowait_percent")
ERRORS = (OSError, ValueError, TypeError, KeyError, IndexError, StopIteration, tarfile.TarError)


def stages(config, label, remote):
    lane = config["lanes"][label]
    engine, runners = lane["engine"], lane["runners"]
    original = config["inventory"]["engines"][engine]
    staged = [SUITE / "worker.py"]
    if engine == "mesh":
        _, env, launcher, _ = original["argv"]
        source = Path(launcher).parents[3]
        topology = "stock-a" if runners == 8 else "tuned-2x32"
        original = dict(original, argv=["/bin/bash", str(remote / "native-mesh.sh"), str(source),
                                       env.split("=", 1)[1], topology])
        staged.extend([PACKET / "native-mesh.sh", TOPOLOGY / "arm.py"])
    elif runners == 2:
        equal = import_equal()
        original, extra = equal.peer(original, engine, config["river_equal"])
        staged.extend(extra)
    spec = dict(original, verified_files=dict(original["verified_files"]))
    spec["verified_files"].update({str(remote / path.name): run.digest(path) for path in staged})
    inventory = dict(config["inventory"], engines={**config["inventory"]["engines"], engine: spec})
    run.check_inventory(inventory)
    return staged, inventory


def import_equal():
    import equal
    return equal


def validate(config, kind):
    require(config["cpus"] == CPUS and config["profiles"] == ["noop"]
            and config["contract"] == "ordinary-trusted-shared-process"
            and len(config["host"]) == 3 and all(config["host"])
            and config["clock_ticks"] > 0, "unexpected dense matrix contract")
    original = config["inventory"]["engines"]
    require(original["mesh"]["artifact_sha256"] == STOCK_MESH
            and RUNNER_SHA in original["mesh"]["verified_files"].values()
            and original["go"]["artifact_sha256"] != run.RIVER_PREDECODED_SHA256, "wrong V1 or River baseline")
    peers = "8x8" if kind == "stock" else "2x32"
    require(config["lanes"] == {label: dict(engine=engine, runners=runners) for label, engine, runners in (
        ("mesh-v1-8x8", "mesh", 8), ("mesh-v1-2x32", "mesh", 2),
        (f"river-{peers}", "go", 8 if kind == "stock" else 2),
        (f"oban-{peers}", "oban", 8 if kind == "stock" else 2),
        (f"apalis-{peers}", "rust", 8 if kind == "stock" else 2))}, "wrong lane allocation")
    for label in config["lanes"]:
        stages(config, label, run.REMOTE / label)
    subprocess.run(["bash", "-n", PACKET / "native-mesh.sh"], check=True)
    return list(plan(["noop"], list(config["lanes"])))


def score_arm(folder, engine, runners, phase, inventory, config, *, historical=False):
    with tarfile.open(folder / "raw.tar.gz", "r:gz") as archive:
        request = run.read_json(folder / "request.json")
        invocation = run.file_json(archive, "invocation.json")
        for field, value in dict(profile="noop", engine=engine, phase=phase, attempt=request["attempt"]).items():
            require(invocation[field] == request[field] == value, f"unexpected native invocation {field}")
        require(run.file_json(archive, "status.json")["exit_code"] == 0, "native adapter failed")
        host = run.file_json(archive, "host.json")
        spec = inventory["engines"][engine]
        require([host["boot_id"], host["kernel"], host["pg_mount"]["maj:min"]] == config["host"]
                and host["affinity"] == CPUS and len(host["available_cpus"]) == 4
                and host["verified_files"] == spec["verified_files"], "host, affinity or artifact pin differs")

        def files(name):
            return run.file_json(archive, "adapter/" + name)

        def logs(name):
            return text(archive, "adapter/" + name)

        ledger = files("ledger.json")
        artifact = {key: spec[key] for key in ("version", "artifact_sha256")}
        require(ledger.get("phase", phase) == phase and ledger.get("local_smoke", False) is False, "unexpected or local phase")
        score = diagnostic_reconciler(runners)(ledger, engine, "noop", artifact)
        require(score["completed"] == JOBS and len(ledger["transactions"]) == 1, "dense workload differs")
        if engine == "mesh":
            topology = mesh_topology(files, logs, ledger, runners, phase, historical=historical)
        else:
            topology = peer_topology(archive, files, logs, ledger, engine, runners)
        with archive.extractfile("host-samples.jsonl") as stream:
            score.update(run.sample_window(host, [json.loads(line) for line in stream], score))
        score.update(host_cpu(folder, score, config["clock_ticks"]))
    return dict(score, topology=dict(topology, runners=runners, slots_per_runner=SLOTS // runners,
                                     total_slots=SLOTS, postgres_cap=CAP),
                raw_sha256=run.digest(folder / "raw.tar.gz"))


def grant(kind):
    ident = SERIES[kind]
    events = re.findall(r"^(SETUP|TIMING|RELEASED|HOLD)\s+(\S+)\s+(\S+)\s+(\S+)",
                        (QUEUE / "STATUS.md").read_text(), flags=re.M)
    require(events and events[-1][:2] == ("TIMING", ident), "no active controller TIMING assignment")
    _, _, start, end = events[-1]
    now = datetime.now(timezone.utc)
    require(datetime.fromisoformat(start) <= now < datetime.fromisoformat(end), "outside TIMING assignment")
    request = PACKET / f"request-{kind}.json"
    require(request.read_bytes() == (QUEUE / f"{ident}.json").read_bytes(), "request differs from controller inbox")
    return dict(start=start, end=end, request_sha256=run.digest(request))


def paths(out, config, block):
    for phase in block["phases"]:
        for label in block["order"]:
            lane = config["lanes"][label]
            yield phase, label, lane, out / f"{phase}-{label}" / f"{phase}-{lane['engine']}"


def replay_block(path, block, config, remotes):
    require(run.read_json(path / "order.json") == block, "block execution order changed")
    results = {}
    for phase, label, lane, folder in paths(path, config, block):
        _, inventory = stages(config, label, run.REMOTE / remotes[label])
        results[f"{phase}-{label}"] = score_arm(folder, lane["engine"], lane["runners"], phase, inventory, config)
    return results


def pick(directory, block, config, remotes, failures):
    for path in sorted((directory / "blocks").glob(f"{run.block_prefix(block)}.*")):
        try:
            return path.name, replay_block(path, block, config, remotes)
        except ERRORS as error:
            failures[path.name] = f"{type(error).__name__}: {error}"
    return None


def dispatch(directory, config, kind, schedule, pins, groups):
    receipt = grant(kind)
    with (SUITE / ".timing.lock").open("rb") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        run.ssh(["bash", "-c", HOST_IDLE], timeout=12).check_returncode()
        directory.mkdir(exist_ok=True)
        manifest_path = directory / "series.json"
        frozen = dict(config=config, schedule=schedule, pins=pins)
        if not manifest_path.exists():
            slug = f"{SERIES[kind]}-{uuid4().hex[:12]}"
            run.write_json(manifest_path, dict(frozen, remotes={label: f"{slug}-{label}" for label in config["lanes"]}))
        manifest = run.read_json(manifest_path)
        require(all(manifest[key] == value for key, value in frozen.items()), "series pins changed")
        remotes = manifest["remotes"]
        slot_path = directory / f"grant-{time.time_ns()}.json"
        run.write_json(slot_path, receipt)
        for label, remote in remotes.items():
            folder = directory / "staging" / label
            folder.mkdir(parents=True, exist_ok=True)
            staged, inventory = stages(config, label, run.REMOTE / remote)
            inv = folder / "inventory.json"
            if not inv.exists():
                run.write_json(inv, inventory)
            require(run.read_json(inv) == inventory, "staged inventory changed")
            with (folder / f"transfer-{time.time_ns()}.log").open("xb") as log:
                for path in [*staged, inv]:
                    run.stage(path, run.REMOTE / remote, log)
            if kind == "equal" and config["lanes"][label]["engine"] in {"oban", "rust"}:
                import_equal().make_adapter_readable(run.REMOTE / remote)
        failures = {}
        for block in schedule:
            if groups and block["group"] not in groups:
                continue
            for previous in sorted((directory / "blocks").glob(f"{run.block_prefix(block)}.*")):
                for _, label, _, folder in paths(previous, config, block):
                    if (folder / "request.json").exists():
                        run.archive_arm(folder, run.REMOTE / remotes[label], run.read_json(folder / "request.json")["attempt"])
            if pick(directory, block, config, remotes, failures):
                continue
            path = directory / "blocks" / f"{run.block_prefix(block)}.{time.time_ns()}"
            path.mkdir(parents=True)
            run.write_json(path / "order.json", block)
            for phase, label, lane, folder in paths(path, config, block):
                grant(kind)
                folder.parent.mkdir()
                run.dispatch_arm(folder.parent, run.REMOTE / remotes[label], block, phase, lane["engine"])
                if run.read_json(folder / "dispatch.json").get("interrupted"):
                    raise RuntimeError("ambiguous native exit; controller must reconcile")
            try:
                result = replay_block(path, block, config, remotes)
            except ERRORS as error:
                print(f"{path.name} failed qualification: {error}", file=sys.stderr, flush=True)
                continue
            run.write_json(path / "reconciled.json", result)
        run.ssh(["bash", "-c", HOST_IDLE], timeout=12).check_returncode()
    summary = summarize(directory, config, schedule, pins)
    name = directory / f"summary-{time.time_ns()}.json"
    run.write_json(name, summary)
    print(name, flush=True)
    return bool(summary["missing"])


def distribution(values):
    return dict(n=len(values), median=median(values), min=min(values), max=max(values))


def summarize(directory, config, schedule, pins):
    manifest = run.read_json(directory / "series.json")
    require(manifest["config"] == config and manifest["schedule"] == schedule and manifest["pins"] == pins,
            "replay inputs differ from the original dispatch")
    selected, failures, missing = {}, {}, []
    for block in schedule:
        found = pick(directory, block, config, manifest["remotes"], failures)
        if found:
            attempt, arms = found
            selected[attempt] = arms
        else:
            missing.append(block["group"])
    stats, paired = {}, {}
    if selected:
        for label in config["lanes"]:
            rows = [arms[f"measured-{label}"] for arms in selected.values()]
            stats[label] = {metric: distribution([row[metric] for row in rows]) for metric in METRICS}
            mesh = [arms["measured-mesh-v1-2x32"] for arms in selected.values()]
            paired[label] = {metric: distribution([100 * (m[metric] / peer[metric] - 1)
                for m, peer in zip(mesh, rows, strict=True)]) for metric in METRICS[:2]}
    return dict(qualified_rotations=len(selected), missing=missing, failures=failures, scores=stats,
                paired_mesh_2x32_percent=paired, blocks=selected)


def inputs(kind, config_path=None):
    request = run.read_json(PACKET / f"request-{kind}.json")
    require(request["id"] == SERIES[kind] and request["ready"], "unexpected or unready request")
    pins = request["local_sha256"]
    for path, checksum in pins.items():
        require(run.digest(ROOT / path) == checksum, f"pinned file changed: {path}")
    config = run.read_json(config_path or PACKET / f"{kind}.json")
    if kind == "equal":
        import_equal().check_config(config)
    return config, validate(config, kind), pins


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("check", "dispatch", "replay"))
    parser.add_argument("kind", choices=SERIES)
    parser.add_argument("--directory", type=Path)
    parser.add_argument("--config", type=Path)
    parser.add_argument("--group", type=int, choices=range(1, 6), action="append")
    args = parser.parse_args()
    config, schedule, pins = inputs(args.kind, args.config)
    directory = args.directory or WORK / args.kind
    if args.action == "dispatch":
        return dispatch(directory, config, args.kind, schedule, pins, args.group)
    if args.action == "replay":
        result = summarize(directory, config, schedule, pins)
        print(json.dumps(result, indent=2, sort_keys=True, allow_nan=False))
        return bool(result["missing"])
    print(json.dumps(schedule, indent=2))


if __name__ == "__main__":
    sys.exit(main())
