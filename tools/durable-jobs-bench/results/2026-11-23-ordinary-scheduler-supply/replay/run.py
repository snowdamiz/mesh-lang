"""Rotate both canonical V1 workloads under the shared local timing lock."""

import argparse
from datetime import datetime, timezone
import fcntl
import json
import os
from pathlib import Path
import subprocess
import sys
import time

from arm import ARTIFACTS, BASE, LANE, SOURCE, WORKERS, sha

ROOT = LANE.parents[1]
LOCK = LANE.parent / "queue-performance.lock"
VERIFY = ROOT / "tools/durable-jobs-bench/verify.py"
PG_BIN = Path("/opt/homebrew/opt/postgresql@17/bin")
ORDERS = (
    ("stock-a", "stock-b", "workers-8", "workers-4"),
    ("stock-b", "workers-4", "stock-a", "workers-8"),
    ("workers-4", "workers-8", "stock-b", "stock-a"),
    ("workers-8", "stock-a", "workers-4", "stock-b"),
    ("stock-a", "workers-4", "stock-b", "workers-8"),
)


def write(path, data):
    with path.open("x") as stream:
        json.dump(data, stream, indent=2, sort_keys=True, allow_nan=False)
        stream.write("\n")


def now():
    return datetime.now(timezone.utc).isoformat()


def check_pins():
    pins = json.loads((LANE / "pins.json").read_text())
    expected = {SOURCE / name: pin for name, pin in pins["harness"].items()}
    expected.update({
        BASE / "ordinary-trusted-v1.tar.gz": pins["source_archive_sha256"],
        ARTIFACTS / "mesh.json": pins["manifest_sha256"],
        ARTIFACTS / "mesh": pins["executable_sha256"],
        VERIFY: pins["verifier_sha256"],
    })
    for path, pin in expected.items():
        if sha(path) != pin:
            raise ValueError(f"pinned input drift: {path}")
    return pins


def dispatch(out, groups):
    if not out.is_relative_to(LANE / "evidence"):
        raise ValueError("use a fresh evidence directory inside the experiment")
    os.umask(0o077)
    pins = check_pins()
    out.mkdir()
    scratch = LANE / "local-state"
    temp = scratch / "tmp"
    temp.mkdir(exist_ok=True)
    env = dict(os.environ, PYTHONDONTWRITEBYTECODE="1", TMPDIR=f"{temp}/", PG_BIN_DIR=str(PG_BIN),
               DURABLE_JOBS_PHASE="diagnostic", NATIVE_JOBS_STATE=str(scratch),
               NATIVE_JOBS_ARTIFACTS=str(ARTIFACTS))
    env.pop("JOBS_PARITY_MESH_EVENTS", None)
    plan = [dict(group=number, profile=profile, order=order)
            for number, order in enumerate(ORDERS[:groups], 1)
            for profile in (("short", "dense") if number % 2 else ("dense", "short"))]
    write(out / "series.json", dict(plan=plan, pins=pins, cpus=os.cpu_count(),
          scheduler_workers=WORKERS, host_affinity="unrestricted local macOS host",
          postgres_executable_sha256=sha(PG_BIN / "postgres"),
          postgres_version=subprocess.check_output([str(PG_BIN / "postgres"), "--version"], text=True).strip(),
          arm_sha256=sha(LANE / "arm.py"), dispatch_sha256=sha(Path(__file__)),
          initial_load=os.getloadavg(), queued_at=now()))
    with LOCK.open("r") as lock:
        print("Waiting for shared local timing lock", flush=True)
        fcntl.flock(lock, fcntl.LOCK_EX)
        write(out / "started.json", dict(at=now(), load=os.getloadavg()))
        for block in plan:
            profile = block["profile"]
            directory = out / f"{block['group']:02}-{profile}"
            directory.mkdir()
            write(directory / "order.json", block)
            for label in block["order"]:
                evidence = directory / label
                before, load_before = time.monotonic(), os.getloadavg()
                started = now()
                command = [sys.executable, "-B", str(LANE / "arm.py"), label, profile, str(evidence)]
                with evidence.with_suffix(".log").open("xb") as log:
                    result = subprocess.run(command, env=env, stdout=log, stderr=subprocess.STDOUT)
                write(evidence.with_suffix(".status.json"), dict(command=command, exit_code=result.returncode,
                      started_at=started, finished_at=now(), wall_s=time.monotonic()-before,
                      load_before=load_before, load_after=os.getloadavg()))
                print(f"{block['group']:02}-{profile} {label} exit {result.returncode}", flush=True)
                if not (evidence / "row.json").is_file() and not (evidence / "failure.json").is_file():
                    raise RuntimeError(f"arm ran without a result or receipt: {evidence}")
        write(out / "finished.json", dict(at=now(), load=os.getloadavg(), inputs_unchanged=check_pins() == pins))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("out", type=Path)
    parser.add_argument("--groups", type=int, choices=range(1, 6), default=5)
    args = parser.parse_args()
    dispatch(args.out.resolve(), args.groups)
