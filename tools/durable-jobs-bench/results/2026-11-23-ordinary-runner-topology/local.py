"""Run the local paired series with a dedicated lock and fresh evidence."""

import argparse
from datetime import datetime, timezone
import fcntl
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time

from arm import INPUT, LAYOUTS, ORDERS, read, sha, write

PACKET = Path(__file__).resolve().parent
ROOT = PACKET.parents[3]
LANE = ROOT / "target/ordinary-runner-topology"
DEFAULT_V1 = ROOT / "target/ordinary-trusted-mesh-queue"
ARCHIVE = PACKET.parent / "2026-11-23-ordinary-trusted-v1/ordinary-trusted-v1.tar.gz"
VERIFY = ROOT / "tools/durable-jobs-bench/verify.py"


def utc():
    return datetime.now(timezone.utc).isoformat()


def check_pins(source, artifacts):
    pins = read(PACKET / "local-pins.json")
    expected = {source / name: pin for name, pin in pins["harness"].items()}
    expected.update({ARCHIVE: pins["source_archive_sha256"],
                     artifacts / "mesh": pins["executable_sha256"],
                     artifacts / "mesh.json": pins["manifest_sha256"],
                     VERIFY: pins["verifier_sha256"]})
    for path, pin in expected.items():
        if sha(path) != pin:
            raise ValueError(f"pinned input drift: {path}")
    return pins


def dispatch(args):
    os.umask(0o077)
    LANE.mkdir(exist_ok=True)
    with (LANE / "local-topology.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        out = LANE / args.name
        pins = check_pins(args.source, args.artifacts)
        artifacts = LANE / "local-artifacts"
        if not artifacts.exists():
            shutil.copytree(args.artifacts, artifacts)
        check_pins(args.source, artifacts)
        out.mkdir()
        state = LANE / "state"
        temp = state / "tmp"
        temp.mkdir(parents=True, exist_ok=True)
        env = {key: val for key, val in os.environ.items() if not key.startswith((
            "JOBS_PARITY_", "MESH_", "NATIVE_JOBS_", "DURABLE_JOBS_", "PG"))}
        env.update(PYTHONDONTWRITEBYTECODE="1", TMPDIR=f"{temp}/", PG_BIN_DIR=str(args.pg_bin),
                   DURABLE_JOBS_PHASE="diagnostic", NATIVE_JOBS_STATE=str(state),
                   NATIVE_JOBS_ARTIFACTS=str(artifacts))
        orders = ORDERS[:args.groups]
        write(out / "series.json", dict(orders=orders, layouts=LAYOUTS, started_at=utc(),
              sample_activity=args.sample_activity, pins=pins, input_sha256=INPUT,
              arm_sha256=sha(PACKET / "arm.py"), dispatch_sha256=sha(Path(__file__)),
              postgres_sha256=sha(args.pg_bin / "postgres"),
              postgres_version=subprocess.check_output(
                  [args.pg_bin / "postgres", "--version"], text=True).strip(),
              host_affinity="unrestricted local host", cpus=os.cpu_count(), load_start=os.getloadavg()))
        for number, order in enumerate(orders, 1):
            directory = out / f"{number:02}"
            directory.mkdir()
            for label in order:
                evidence = directory / label
                start, load, stamp = time.monotonic(), os.getloadavg(), utc()
                command = [sys.executable, "-B", str(PACKET / "arm.py"), str(args.source),
                           label, str(evidence), "--local"]
                if args.sample_activity:
                    command.append("--sample-activity")
                with evidence.with_suffix(".log").open("xb") as log:
                    result = subprocess.run(command, env=env, stdout=log, stderr=subprocess.STDOUT)
                write(evidence.with_suffix(".status.json"),
                      dict(exit_code=result.returncode, start=stamp, end=utc(),
                           wall_s=time.monotonic()-start, load_before=load, load_after=os.getloadavg()))
                print(f"{number:02}/{label} exit {result.returncode}", flush=True)
        write(out / "finished.json", dict(at=utc(), load=os.getloadavg(),
              inputs_unchanged=check_pins(args.source, artifacts) == pins))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("name")
    parser.add_argument("--groups", type=int, choices=range(1, 6), default=5)
    parser.add_argument("--source", type=Path, default=DEFAULT_V1 / "source")
    parser.add_argument("--artifacts", type=Path, default=DEFAULT_V1 / "local-artifacts-v2")
    parser.add_argument("--pg-bin", type=Path, default=Path("/opt/homebrew/opt/postgresql@17/bin"))
    parser.add_argument("--sample-activity", action="store_true")
    args = parser.parse_args()
    if not args.name or Path(args.name).name != args.name or args.name in {".", ".."}:
        parser.error("name must be one new directory name")
    dispatch(args)
