"""Recover equal topology only after the original host and all fences verify."""

import argparse
import fcntl
import json
from pathlib import Path
from subprocess import SubprocessError, run as command_run
import sys
import time

sys.path.insert(0, "tools/durable-jobs-bench")
import run
from interleave import HOST_IDLE


STATE = Path(__file__).resolve().parent
QUEUE = STATE / "requests"
ID = "dense-matched-topology-equal"
OUTPUT = STATE.parent / "native-dense-matched-topology/equal"
EVIDENCE = STATE / "controller-receipts/dense-equal-transport"
POSTGRES = "/var/lib/durable-jobs-bench/native/toolchain/pgsql-17.11/bin/postgres"
SEAL = "2627cd1060c51139b50278d041a9b3e2e75cbd6777d8c16b0a2f64b627941969"
CHECK = HOST_IDLE + '''
python3 -B - "$@" <<'PY'
import hashlib
import json
from pathlib import Path
import platform
import subprocess
import sys
def output(argv):
    return subprocess.check_output(argv, text=True).strip()
binary, *paths = sys.argv[1:]
with Path(binary).open("rb") as pg:
    postgres_sha256 = hashlib.file_digest(pg, "sha256").hexdigest()
affinity_source = "import json,os; print(json.dumps(sorted(os.sched_getaffinity(0))))"
affinity = json.loads(output(["taskset", "-c", "0,1", sys.executable, "-c", affinity_source]))
print(json.dumps(dict(boot_id=Path("/proc/sys/kernel/random/boot_id").read_text().strip(),
    kernel=platform.release(), architecture=platform.machine(), affinity=affinity,
    logical_cpus=int(output(["nproc", "--all"])), clock_ticks=int(output(["getconf", "CLK_TCK"])),
    devices={path:output(["findmnt", "-no", "MAJ:MIN", "--target", path]) for path in paths},
    postgres=output([binary, "--version"]), postgres_sha256=postgres_sha256)))
PY
'''


def capture(config):
    specs = []
    for label, lane in config["lanes"].items():
        inventory = run.read_json(OUTPUT / "staging" / label / "inventory.json")
        run.require(inventory["cpus"] == config["cpus"], f"{label} affinity pin changed")
        specs.append(inventory["engines"][lane["engine"]])
    paths = sorted({str(spec["postgres_storage"]) for spec in specs})
    pg_hashes = {spec["verified_files"][POSTGRES] for spec in specs if POSTGRES in spec["verified_files"]}
    run.require(bool(paths) and len(pg_hashes) == 1, "ambiguous host continuity pins")
    with (run.SUITE / ".timing.lock").open("rb") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        remote = run.ssh(["bash", "-c", CHECK, ID, POSTGRES, *paths],
                         capture_output=True, text=True, timeout=15)
    if remote.returncode:
        return None, f"pinned SSH/remote fence check exit {remote.returncode}"
    observation = json.loads(remote.stdout)
    boot, kernel, device = config["host"]
    run.require(observation["boot_id"] == boot and observation["kernel"] == kernel
                and observation["architecture"] == "x86_64" and observation["logical_cpus"] == 4
                and observation["affinity"] == config["cpus"] == [0, 1]
                and observation["clock_ticks"] == config["clock_ticks"]
                and set(observation["devices"].values()) == {device}
                and observation["postgres"] == "postgres (PostgreSQL) 17.11"
                and observation["postgres_sha256"] in pg_hashes,
                "live boot, affinity, PG disk or executable differs from the sealed series")
    return observation, None


parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--retry-for", type=int, default=0, help="seconds to wait for transport and fences")
args = parser.parse_args()
run.require(run.digest(QUEUE / f"{ID}.json") == SEAL, "equal timing request seal changed")
config = run.read_json(OUTPUT / "series.json")["config"]
EVIDENCE.mkdir(parents=True, exist_ok=True)
end = time.monotonic() + args.retry_for
while True:
    try:
        result, error = capture(config)
    except BlockingIOError:
        result, error = None, "local timing fence held"
    except SubprocessError:
        result, error = None, "pinned SSH check timed out"
    timestamp = run.utc_now()
    with (EVIDENCE / "reachability.jsonl").open("a") as stream:
        stream.write(json.dumps(dict(utc=timestamp, error=error)) + "\n")
    if result is not None:
        break
    if time.monotonic() >= end:
        sys.exit(error)
    time.sleep(20)

previous = run.read_json(OUTPUT / "controller-timing-09e8adb91729.json")
run.require(previous["exit_status"] != 0 and previous.get("lock_probe_error"),
            "missing interrupted grant outcome")
run.write_json(EVIDENCE / "verified-original-host.json", dict(result, observed_at=timestamp,
    fences=["local timing", "remote active-arm", "remote native"], request_sha256=SEAL))
release = dict(previous, released_at=timestamp, interrupted_attempt=True,
               verification=str(EVIDENCE / "verified-original-host.json"))
run.write_json(QUEUE / f"{ID}-interrupted.released.json", release)
with (QUEUE / "STATUS.md").open("a") as stream:
    stream.write(f"\nRELEASED {ID} {timestamp}, exit {previous['exit_status']}. "
                 "Original interrupted grant closed after the three fences and exact boot, affinity, disk, and PostgreSQL pins verified.\n")
sys.exit(command_run([sys.executable, "-B", str(STATE / "dense-matched-topology-slots.py"), "equal"],
                     cwd=run.ROOT, check=False).returncode)
