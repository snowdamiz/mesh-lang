"""Run the preselected scheduler-supply comparison under the local topology lock."""

import argparse
from datetime import datetime, timezone
import fcntl
import os
from pathlib import Path
import runpy
import shutil
import subprocess
import sys
import time

from arm import ACTIVITY, CASES, LANE, LOCK, ORDERS, PACKET, PRIMARY, PRIOR, ROOT

base = runpy.run_path(str(PRIOR / "arm.py"))
read, write, sha = (base[key] for key in ("read", "write", "sha"))
DEFAULT = ROOT / "target/ordinary-trusted-mesh-queue"


def check_pins(source, artifacts):
    pins = read(PRIOR / "local-pins.json")
    expected = {source / name: pin for name, pin in pins["harness"].items()}
    expected.update({
        PRIOR.parent / "2026-11-23-ordinary-trusted-v1/ordinary-trusted-v1.tar.gz":
            pins["source_archive_sha256"],
        artifacts / "mesh": pins["executable_sha256"],
        artifacts / "mesh.json": pins["manifest_sha256"],
        ROOT / "tools/durable-jobs-bench/verify.py": pins["verifier_sha256"],
    })
    for path, pin in expected.items():
        if sha(path) != pin:
            raise ValueError(f"pinned input drift: {path}")
    return pins


def utc():
    return datetime.now(timezone.utc).isoformat()


def dispatch(args):
    os.umask(0o077)
    LANE.mkdir(exist_ok=True)
    LOCK.parent.mkdir(exist_ok=True)
    with LOCK.open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        pins = check_pins(args.source, args.artifacts)
        out = LANE / args.name
        out.mkdir()
        artifacts = LANE / "artifacts"
        if not artifacts.exists():
            shutil.copytree(args.artifacts, artifacts)
        check_pins(args.source, artifacts)
        state = LANE / "state"
        temp = state / "tmp"
        temp.mkdir(parents=True, exist_ok=True)
        env = {key: val for key, val in os.environ.items() if not key.startswith((
            "JOBS_PARITY_", "MESH_", "NATIVE_JOBS_", "DURABLE_JOBS_", "PG"))}
        env.update(PYTHONDONTWRITEBYTECODE="1", TMPDIR=f"{temp}/", PG_BIN_DIR=str(args.pg_bin),
                   DURABLE_JOBS_PHASE="diagnostic", NATIVE_JOBS_STATE=str(state),
                   NATIVE_JOBS_ARTIFACTS=str(artifacts))
        orders = ORDERS if args.mode == "timing" else (ACTIVITY,)
        code = {str(path.relative_to(ROOT)): sha(path) for path in (
            Path(__file__), PACKET / "arm.py", PRIOR / "arm.py", PRIOR / "replay.py")}
        write(out / "series.json", dict(mode=args.mode, cases=CASES, orders=orders, pins=pins, code=code,
              primary=PRIMARY, started_at=utc(), host_cores=os.cpu_count(), load_start=os.getloadavg(),
              postgres_sha256=sha(args.pg_bin / "postgres"), postgres_version=subprocess.check_output(
                  [args.pg_bin / "postgres", "--version"], text=True).strip()))
        for number, order in enumerate(orders, 1):
            block = out / f"{number:02}"
            block.mkdir()
            for label in order:
                evidence = block / label
                stamp, start, load = utc(), time.monotonic(), os.getloadavg()
                command = [sys.executable, "-B", str(PACKET / "arm.py"), str(args.source),
                           label, str(evidence), args.mode]
                with evidence.with_suffix(".log").open("xb") as log:
                    result = subprocess.run(command, env=env, stdout=log, stderr=subprocess.STDOUT)
                write(evidence.with_suffix(".status.json"), dict(
                    exit_code=result.returncode, start=stamp, end=utc(), wall_s=time.monotonic()-start,
                    load_before=load, load_after=os.getloadavg()))
                print(f"{number:02}/{label} exit {result.returncode}", flush=True)
        write(out / "finished.json", dict(at=utc(), load_end=os.getloadavg(),
              inputs_unchanged=(check_pins(args.source, artifacts) == pins and
                                all(sha(ROOT / path) == pin for path, pin in code.items()))))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("name")
    parser.add_argument("--mode", choices=("timing", "activity"), default="timing")
    parser.add_argument("--source", type=Path, default=DEFAULT / "source")
    parser.add_argument("--artifacts", type=Path, default=DEFAULT / "local-artifacts-v2")
    parser.add_argument("--pg-bin", type=Path, default=Path("/opt/homebrew/opt/postgresql@17/bin"))
    args = parser.parse_args()
    if not args.name or Path(args.name).name != args.name or args.name in {".", ".."}:
        parser.error("name must be one new directory name")
    dispatch(args)
