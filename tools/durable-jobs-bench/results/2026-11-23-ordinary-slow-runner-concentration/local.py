"""Rotate and replay the compiled slow experiment using the shared local timing lock."""

import argparse
import fcntl
import itertools
import os
from pathlib import Path
import platform
import subprocess
import sys
import time

from replay import directory_arm, summarize
from slow_arm import FROZEN, HERE, INPUT, LAYOUTS
from arm import read, sha, write

ROOT = HERE.parents[3]
LANE = ROOT / "target/ordinary-slow-runner-concentration"
PG = Path("/opt/homebrew/opt/postgresql@17/bin")
ORDERS = (
    ("one-a", "two", "one-b"), ("one-b", "two", "one-a"),
    ("two", "one-a", "one-b"), ("one-b", "one-a", "two"),
    ("one-a", "one-b", "two"), ("two", "one-b", "one-a"),
)


def invoke(parent, label, env, wire=False):
    out = parent / label
    start, load = time.monotonic(), os.getloadavg()
    with (ROOT / "target/queue-performance.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        acquired = time.monotonic()
        command = [sys.executable, "-B", str(HERE / "slow_arm.py"),
                   str(LANE / "base"), label, str(out), "--local"]
        if wire:
            command.append("--wire")
        with out.with_suffix(".log").open("xb") as log:
            process = subprocess.run(command, env=env, stdout=log, stderr=subprocess.STDOUT)
    status = dict(exit_code=process.returncode, lock_wait_s=acquired-start, wall_s=time.monotonic()-acquired,
                  load_before=load, load_after=os.getloadavg(), wire=wire)
    if process.returncode == 0:
        try:
            score = directory_arm(out, label, wire)
            write(out / "score.json", score)
        except (ValueError, KeyError, FileNotFoundError) as error:
            status["verification_failure"] = str(error)
    write(out.with_suffix(".status.json"), status)
    print(parent.name, label, status.get("verification_failure", process.returncode), flush=True)


def run(path, candidate, wire):
    if sorted(ORDERS) != sorted(itertools.permutations(("one-a", "one-b", "two"))):
        raise ValueError("control and treatment orders are not balanced")
    pins = read(FROZEN / "local-pins.json")
    if sha(FROZEN / "local-harness.tar.gz") != pins["harness_sha256"]:
        raise ValueError("harness source pin changed")
    out = path.resolve()
    out.relative_to(LANE)
    out.mkdir()
    env = {key: os.environ[key] for key in ("HOME", "PATH", "LANG") if key in os.environ}
    env.update(PG_BIN_DIR=str(PG), TMPDIR=str(LANE / "scratch"),
               NATIVE_JOBS_ARTIFACTS=str(LANE / "artifacts-v1"), NATIVE_JOBS_STATE=str(LANE),
               DURABLE_JOBS_PHASE="diagnostic")
    orders = [[candidate if name == "two" else name for name in order] for order in ORDERS]
    write(out / "series.json", dict(orders=orders, candidate=candidate, input_sha256=INPUT,
                                    host=platform.platform(), postgres_sha256=sha(PG / "postgres"),
                                    harness_sha256=pins["harness_sha256"], layout=LAYOUTS, wire=wire))
    for number, order in enumerate(orders, 1):
        group = out / f"{number:02}"
        group.mkdir()
        for label in order:
            invoke(group, label, env)
    if wire:
        group = out / "wire"
        group.mkdir()
        invoke(group, candidate, env, wire=True)
    report = replay(out)
    write(out / "summary.json", report)
    return bool(report["excluded"])


def replay(out):
    manifest = read(out / "series.json")
    groups, excluded = [], []
    for number, order in enumerate(manifest["orders"], 1):
        group = out / f"{number:02}"
        arms = {}
        for label in order:
            status = read(group / f"{label}.status.json")
            if status["exit_code"]:
                excluded.append(dict(group=number, label=label, status=status))
                continue
            try:
                arms[label] = directory_arm(group / label, label)
            except (ValueError, KeyError, FileNotFoundError) as error:
                excluded.append(dict(group=number, label=label, validation=str(error), status=status))
        groups.append(dict(group=number, order=order, arms=arms))
    complete = [group for group in groups if len(group["arms"]) == len(group["order"])]
    result = summarize(complete, manifest["candidate"]) if complete else {}
    result.update(all_groups=groups, excluded=excluded)
    if manifest["wire"]:
        label = manifest["candidate"]
        path = out / "wire"
        status = read(path / f"{label}.status.json")
        try:
            if status["exit_code"]:
                raise ValueError("wire arm failed")
            result["untimed_wire"] = directory_arm(path / label, label, wire=True)
        except (ValueError, KeyError, FileNotFoundError) as error:
            excluded.append(dict(group="wire", label=label, validation=str(error), status=status))
    return result


if __name__ == "__main__":
    import json
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("run", "replay"))
    parser.add_argument("directory", type=Path)
    parser.add_argument("--candidate", choices=("two", "four"), default="two")
    parser.add_argument("--wire", action="store_true")
    args = parser.parse_args()
    if args.command == "run":
        raise SystemExit(run(args.directory, args.candidate, args.wire))
    print(json.dumps(replay(args.directory), sort_keys=True, indent=2))
