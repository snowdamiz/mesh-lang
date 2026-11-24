"""Run six balanced local rotations, or two separate instrumented arms."""
import argparse
import fcntl
import hashlib
from itertools import permutations
import json
import os
from pathlib import Path
import subprocess
import sys
import time

from arm import LABELS

ORDERS = tuple(permutations(LABELS))
REPLAY = Path(__file__).resolve().parent


def sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def save(path, data):
    with path.open("x") as stream:
        json.dump(data, stream, indent=2, sort_keys=True)
        stream.write("\n")


def run(lane, lock_path, out, diagnostic):
    build = json.loads((lane / "artifacts/mesh.json").read_text())
    pins = {name: sha(lane / "artifacts" / name) for name in (*build["files"], "mesh.json")}
    assert all(pins[name] == checksum for name, checksum in build["files"].items())
    pg_bin = Path(os.environ.get("PG_BIN_DIR", "/opt/homebrew/opt/postgresql@17/bin"))
    env = {key: val for key, val in os.environ.items() if not key.startswith(
        ("ACTOR_PG_", "DURABLE_JOBS_", "JOBS_PARITY_", "MESH_", "NATIVE_JOBS_", "PG", "TRUSTED_"))}
    env.update(PG_BIN_DIR=str(pg_bin), NATIVE_JOBS_STATE=str(lane / "state"),
               NATIVE_JOBS_ARTIFACTS=str(lane / "artifacts"), TMPDIR=f"{lane / 'tmp'}/",
               MESH_JOBS_SET_BASED_ENQUEUE="1", TRUSTED_ADMISSION_POLICY="success_cohort",
               DURABLE_JOBS_PHASE="diagnostic", PYTHONDONTWRITEBYTECODE="1")
    orders = [("off-a", "readiness")] if diagnostic else ORDERS
    (lane / "state").mkdir(exist_ok=True)
    with lock_path.open("rb") as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            print("Shared timing lane is occupied. No benchmark started.", file=sys.stderr)
            return 75
        out.mkdir()
        save(out / "plan.json", dict(orders=orders, pins=pins, diagnostic=diagnostic,
             host=list(os.uname()), logical_cpus=os.cpu_count(), load=os.getloadavg(),
             postgres_sha256=sha(pg_bin / "postgres"), started_us=time.time_ns() // 1000,
             adapter_sha256=sha(REPLAY / "arm.py"), campaign_sha256=sha(Path(__file__))))
        for group, order in enumerate(orders, 1):
            block = out / f"{group:02}"
            block.mkdir()
            for label in order:
                path = block / label
                started, load = time.time_ns() // 1000, os.getloadavg()
                cmd = [sys.executable, "-B", str(REPLAY / "arm.py"),
                       str(lane / "ordinary-v2"), label, str(path)]
                if diagnostic:
                    cmd.append("--diagnostic")
                with path.with_suffix(".log").open("xb") as log:
                    result = subprocess.run(cmd, env=env, stdout=log, stderr=subprocess.STDOUT)
                save(path.with_suffix(".status.json"), dict(
                    exit_code=result.returncode, started_us=started, load_before=load,
                    ended_us=time.time_ns() // 1000, load_after=os.getloadavg()))
                print(group, label, result.returncode, flush=True)
                if result.returncode:
                    return result.returncode
        assert {name: sha(lane / "artifacts" / name) for name in pins} == pins
    return 0


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("lane", type=Path)
    parser.add_argument("lock", type=Path)
    parser.add_argument("out", type=Path)
    parser.add_argument("--diagnostic", action="store_true")
    args = parser.parse_args()
    sys.exit(run(args.lane.resolve(), args.lock.resolve(), args.out.resolve(), args.diagnostic))
