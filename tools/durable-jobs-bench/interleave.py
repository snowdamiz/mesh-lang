#!/usr/bin/env python3
"""Run or replay matched native Jobs arms across separately pinned inventories."""

import argparse
import fcntl
import json
from itertools import cycle, islice
from pathlib import Path
import re
from statistics import median
import subprocess
import sys
import tarfile
import time
from uuid import uuid4

import run
from verify import PROFILES, expected_inputs

CONTRACTS = {"strict-frozen", "persistent-worker"}
ATTEMPT_ERRORS = (OSError, ValueError, KeyError, TypeError, tarfile.TarError)
HOST_IDLE = """
set -eu
exec 8</var/tmp/mesh-durable-jobs-matrix/active-arm.lock
flock -n 8
exec 9</var/lib/durable-jobs-bench/native/native.lock
flock -n 9
"""


def plan(profiles, lanes):
    """Balance crossover order, including which lanes follow one another."""
    size = len(lanes)
    offsets = [0]
    for index in range(1, size):
        offsets.append((index + 1) // 2 if index % 2 else -(index // 2))
    orders = [[lanes[(start + offset) % size] for offset in offsets] for start in range(size)]
    if size > 2 and size % 2:
        orders.extend([list(reversed(order)) for order in orders[:]])
    for group, order in enumerate(islice(cycle(orders), 5), 1):
        rotation = (group - 1) % len(profiles)
        for profile in profiles[rotation:] + profiles[:rotation]:
            phases = ["warmup", "measured"] if PROFILES[profile][-1] else ["measured"]
            yield dict(group=group, profile=profile, phases=phases, order=order)


def validate(config):
    run.require(config["contract"] in CONTRACTS and config["description"].strip(),
                "specify the execution contract and describe the launch setup")
    cpus = config["cpus"]
    run.require(len(cpus) in (2, 3) and cpus == sorted(set(cpus))
                and all(type(cpu) is int and 0 <= cpu < 4 for cpu in cpus),
                "use two or three distinct ordered CPU IDs from the four-core host")
    run.require(2 <= len(config["lanes"]) <= 6, "specify between two and six lanes")
    run.require(config["profiles"] and set(config["profiles"]) <= set(PROFILES)
                and len(set(config["profiles"])) == len(config["profiles"]), "invalid profile list")
    run.require(len(config["host"]) == 3 and all(config["host"]), "pin host boot, kernel and disk")
    for profile in config["profiles"]:
        expected_inputs(profile)
    for label, lane in config["lanes"].items():
        run.require(re.fullmatch(r"[a-z][a-z0-9-]{0,24}", label), "invalid lane label")
        engine, inventory = lane["engine"], lane["inventory"]
        run.check_inventory(dict(inventory, cpus=cpus[:2]))
        run.require(inventory["cpus"] == cpus, f"{label}: different CPU cap")
        run.require(engine in run.ENGINES, f"{label}: unknown engine")
        arm = inventory["engines"][engine]
        run.require(arm.get("verified_files"), f"{label}: pin the executable and/or wrapper files")
        if engine == "go":
            run.require(arm["artifact_sha256"] != run.RIVER_PREDECODED_SHA256,
                        "River submit must include batch input")


def attempt_paths(directory, block):
    return sorted((directory / "blocks").glob(f"{run.block_prefix(block)}.*"))


def arms(directory, config, block):
    for phase in block["phases"]:
        for label in block["order"]:
            engine = config["lanes"][label]["engine"]
            yield phase, label, directory / f"{phase}-{label}" / f"{phase}-{engine}"


def host_cpu(folder, score, ticks_per_second):
    with tarfile.open(folder / "raw.tar.gz", "r:gz") as archive:
        with archive.extractfile("host-samples.jsonl") as data:
            points = [json.loads(line) for line in data]
    start = max(point for point in range(len(points))
                if points[point]["epoch_us"] <= score["first_submit_us"])
    end = next(point for point in points if point["epoch_us"] >= score["last_settlement_us"])
    before = points[start]["cpu"]
    delta = [[hi - lo for lo, hi in zip(before[cpu], end["cpu"][cpu], strict=True)] for cpu in before]
    # procfs fields have USER_HZ granularity. These totals cover the host's
    # bracketing interval on the pinned CPUs, including DB and any other work.
    cpu_scale = 1000 / (ticks_per_second * score["completed"])
    return {
        "pinned_user_cpu_ms_per_job": cpu_scale * sum(row[0] + row[1] for row in delta),
        "pinned_kernel_cpu_ms_per_job": cpu_scale * sum(row[2] + row[5] + row[6] for row in delta),
        "observed_arm_wall_ms": (points[-1]["monotonic_ns"] - points[0]["monotonic_ns"]) / 1_000_000,
    }


def replay(path, config, block):
    run.require(run.read_json(path / "order.json") == block, "recorded order differs from the plan")
    measured, raw_hashes = {}, {}
    for phase, label, folder in arms(path, config, block):
        lane = config["lanes"][label]
        score = run.score_arm(folder, block, phase, lane["engine"], lane["inventory"])
        if lane["engine"] in {"mesh", "oban"}:
            with tarfile.open(folder / "raw.tar.gz", "r:gz") as archive:
                masks = run.file_json(archive, "adapter/process-affinity.json")
            run.require(len(masks) == PROFILES[block["profile"]][0] + 1
                        and all(mask == config["cpus"] for mask in masks.values()),
                        f"{label}: PostgreSQL or runner affinity does not match the CPU cap")
        host = [score["host_boot_id"], score["host_kernel"], score["pg_mount"]["maj:min"]]
        run.require(host == config["host"], f"{label}: boot, kernel or disk changed")
        raw_hashes[str(folder.relative_to(path))] = run.digest(folder / "raw.tar.gz")
        if phase == "measured":
            score.update(host_cpu(folder, score, config["clock_ticks"]))
            measured[label] = score
    return dict(scores=measured, raw_sha256=raw_hashes)


def select_attempt(directory, config, block, issues):
    for path in attempt_paths(directory, block):
        try:
            return path.name, replay(path, config, block)
        except ATTEMPT_ERRORS as error:
            issues[path.name] = str(error)
    return None


def dispatch_block(directory, manifest, block):
    config = manifest["config"]
    # A lost SSH reply does not imply the remote arm failed. Recover known
    # attempts first. A true partial block is rerun whole to keep adjacent arms
    # comparable, with the original requests and raw receipts preserved.
    for path in attempt_paths(directory, block):
        for _, label, folder in arms(path, config, block):
            request = folder / "request.json"
            if request.is_file():
                run.archive_arm(folder, run.REMOTE / manifest["remote"][label],
                                run.read_json(request)["attempt"])
    if select_attempt(directory, config, block, {}):
        return True
    probe = run.ssh(["bash", "-c", HOST_IDLE], timeout=12)
    run.require(probe.returncode == 0, "host lock busy or unreachable; retain the partial block and retry")
    path = directory / "blocks" / f"{run.block_prefix(block)}.{time.time_ns()}"
    path.mkdir(parents=True)
    run.write_json(path / "order.json", block)
    for phase, label, folder in arms(path, config, block):
        folder.parent.mkdir()
        run.dispatch_arm(folder.parent, run.REMOTE / manifest["remote"][label],
                         block, phase, config["lanes"][label]["engine"])
    try:
        result = replay(path, config, block)
    except ATTEMPT_ERRORS as error:
        print(f"{path.name} incomplete: {error}", file=sys.stderr, flush=True)
        return False
    run.write_json(path / "reconciled.json", result)
    print(f"{path.name} reconciled", flush=True)
    return True


def dispatch(directory, config, groups):
    validate(config)
    run.require(type(config["clock_ticks"]) is int and config["clock_ticks"] > 0,
                "record the host getconf CLK_TCK")
    frozen = {
        "config": config,
        "plan": list(plan(config["profiles"], list(config["lanes"]))),
        "drivers": {path.name: run.digest(path) for path in
                    (Path(__file__), run.SUITE / "run.py", run.SUITE / "verify.py", run.SUITE / "worker.py")},
    }
    with (run.SUITE / ".timing.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        directory.mkdir(exist_ok=True)
        path = directory / "series.json"
        if not path.exists():
            session = directory.name + "-" + uuid4().hex
            run.write_json(path, dict(frozen, remote={lane: f"{session}-{lane}" for lane in config["lanes"]}))
        manifest = run.read_json(path)
        run.require(all(manifest[key] == value for key, value in frozen.items()),
                    "series changed; use the frozen drivers and config or a new series")
        for label, lane in config["lanes"].items():
            staging = directory / label
            staging.mkdir(exist_ok=True)
            inventory = staging / "inventory.json"
            if not inventory.exists():
                run.write_json(inventory, lane["inventory"])
            run.require(run.read_json(inventory) == lane["inventory"], "stage inventory changed")
            with (staging / f"stage-{uuid4().hex}.txt").open("xb") as log:
                remote = run.REMOTE / manifest["remote"][label]
                run.stage(run.SUITE / "worker.py", remote, log)
                run.stage(inventory, remote, log)
        results = [dispatch_block(directory, manifest, block)
                   for block in manifest["plan"] if not groups or block["group"] in groups]
    return 0 if all(results) else 1


def distribution(values):
    return dict(n=len(values), median=median(values), min=min(values), max=max(values))


def summarize(directory):
    manifest = run.read_json(directory / "series.json")
    config = manifest["config"]
    rows, missing, issues = {}, [], {}
    for block in manifest["plan"]:
        selected = select_attempt(directory, config, block, issues)
        if selected is None:
            missing.append(run.block_prefix(block))
        else:
            attempt, result = selected
            rows.setdefault(block["profile"], []).append(dict(block, attempt=attempt, **result))
    metrics = ("submit_jobs_per_s", "commit_jobs_per_s", "first_submit_to_commit_ms",
               "submit_to_settlement_p99_ms", "commit_to_settlement_p99_ms",
               "commit_to_pickup_p99_ms", "effect_commit_to_settlement_p99_ms", "runner_cpu_ms_per_job",
               "pinned_user_cpu_ms_per_job", "pinned_kernel_cpu_ms_per_job", "observed_arm_wall_ms",
               "peak_runner_rss_bytes", "peak_connections", "cpu_steal_percent", "cpu_iowait_percent")
    summary = {}
    for profile, blocks in rows.items():
        summary[profile] = {
            label: {metric: distribution([row["scores"][label][metric] for row in blocks])
                    for metric in metrics}
            for label in config["lanes"]
        }
    reference = manifest["plan"][0]["order"][0]
    paired = {
        profile: {
            label: {metric: distribution([
                100 * (row["scores"][label][metric] / row["scores"][reference][metric] - 1)
                for row in blocks
            ]) for metric in ("submit_jobs_per_s", "commit_jobs_per_s", "runner_cpu_ms_per_job")}
            for label in config["lanes"] if label != reference
        }
        for profile, blocks in rows.items()
    }
    json.dump(dict(contract=config["contract"], cpus=config["cpus"], host=config["host"],
                   missing=missing, issues=issues, rows=rows, summary=summary,
                   paired_reference=reference, paired_percent=paired),
                  sys.stdout, indent=2, sort_keys=True, allow_nan=False)
    print()
    return 1 if missing else 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("dispatch", "summarize"))
    parser.add_argument("series")
    parser.add_argument("--config", type=Path)
    parser.add_argument("--group", type=int, choices=range(1, 6), action="append")
    args = parser.parse_args()
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9.-]{0,63}", args.series):
        parser.error("use a safe series name")
    directory = run.RESULTS / args.series
    if args.action == "summarize":
        return summarize(directory)
    if not args.config:
        parser.error("dispatch requires --config")
    return dispatch(directory, run.read_json(args.config), args.group)


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (*ATTEMPT_ERRORS, subprocess.SubprocessError) as error:
        print(error, file=sys.stderr)
        sys.exit(1)
