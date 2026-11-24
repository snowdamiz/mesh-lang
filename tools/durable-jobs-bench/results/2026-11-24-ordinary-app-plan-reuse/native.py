#!/usr/bin/env python3
"""Seal and verify native inputs, or dispatch inside a sole-controller TIMING grant."""
import argparse
from datetime import datetime, timezone
import fcntl
import hashlib
from pathlib import Path
import re
import subprocess
import sys
import tarfile

from measure import BASE_CLUSTER, DENSE, FIXTURE, HERE, JOBS, ORDERS, ROOT, SUPPORT, TARGET, WORKER
from replay import summarize

sys.path.insert(0, str(WORKER.parent))
import run
from interleave import HOST_IDLE

ID = "ordinary-app-plan-reuse"
QUEUE = ROOT / "target/linode-jobs-stepchange/requests"
REQUEST = HERE / "request.json"
SHELL = HOST_IDLE + r"""
set -o pipefail
umask 077
stage=$(mktemp -d /var/lib/durable-jobs-bench/native/scratch/app-plan-reuse.XXXXXX)
echo "Owned staging: $stage" >&2
exported=0
trap 'if [ "$exported" = 1 ]; then rm -rf -- "$stage"; fi' EXIT
cat > "$stage/input.tar"
printf '%s  %s\n' "$1" "$stage/input.tar" | sha256sum -c >&2
mkdir "$stage/src" "$stage/export"
tar -xf "$stage/input.tar" -C "$stage/src"
cd "$stage/src"
sha256sum -c INPUTS.sha256 >&2
packet=tools/durable-jobs-bench/results/2026-11-24-ordinary-app-plan-reuse
mkdir -p target/app-plan-reuse/tmp
chown -R nobody:nogroup "$stage/src"
chmod 0755 "$stage"
code=0
taskset -c 0,1 /usr/sbin/runuser -u nobody -- env -i PATH=/usr/bin:/bin LC_ALL=C.UTF-8 \
  PG_BIN_DIR=/var/lib/durable-jobs-bench/native/toolchain/pgsql-17.11/bin \
  TMPDIR="$stage/src/target/app-plan-reuse/tmp" python3 -B "$packet/measure.py" --native measured \
  > "$stage/measure.log" 2>&1 || code=$?
python3 -B - "$stage" <<'EXPORT'
import sys
from pathlib import Path
b = Path(sys.argv[1])
sys.path.insert(0, "tools/durable-jobs-bench")
from worker import safe_bytes
out = b / "export"
raw = b / "src/target/app-plan-reuse/measured"
files = [b / "measure.log"]
if raw.exists():
    files.extend(p for p in raw.iterdir() if p.is_file() and p.suffix in (".json", ".log"))
for path in files:
    (out / path.name).write_bytes(safe_bytes(path))
EXPORT
(cd "$stage/export" && tar -czf - -- *)
exported=1
exit "$code"
"""


def paths():
    sources = [HERE / n for n in ("measure.py", "replay.py", "native.py", "verify-wire.py")]
    archives = [p for lane, engine in (("mesh-v1-2x32", "mesh"), ("river-8x8", "go"), ("oban-8x8", "oban"))
                for p in DENSE.glob(f"??-noop.*/measured-{lane}/measured-{engine}/raw.tar.gz")]
    assert len(archives) == 15
    controller = [WORKER.with_name(n) for n in ("interleave.py", "run.py", "verify.py")]
    return sorted([*sources, *controller, BASE_CLUSTER, SUPPORT, WORKER, FIXTURE, *archives])


def pins():
    return {str(p.relative_to(ROOT)): run.digest(p) for p in paths()}


def make_request(source):
    assert re.fullmatch(r"[0-9a-f]{40}", source)
    runner = HERE.relative_to(ROOT) / "native.py"
    request = dict(
        id=ID, kind="Controller-owned build-free TIMING diagnostic",
        operator_id="ordinary-application-repeated-write-plan-price-v1",
        controller="linode-native-stepchange-controller-v1",
        source_sha=source, state="ready", ready=True, requested_at=run.utc_now(), cpus=[0, 1],
        expected_timing_minutes=5,
        scope="PostgreSQL 17.11 verified TLS. Four writers each run the exact V1 attempt/effect SQL in independent synchronous autocommits. Six balanced rotations of stock-a, stock-b and named protocol PREPARE. 1024 pairs per arm. Timing covers steady writes, with named setup priced separately.",
        priority="Behind all booked native work. Sole matrix controller owns the host, three fences, and evidence export.",
        prerequisites="Current TIMING grant for this ID. Same Linode boot, kernel, disk and pinned PostgreSQL executable as dense native stock. No concurrent builds or profilers.",
        local_gate="Offline SQL and dense input cross-check, controller staging archive check, Python and Bash parsing, SQL smoke without timing, and nine existing matrix correctness tests.",
        decision="Advance to a separate bounded ordinary Pool.execute candidate only if median wall gain over the faster duplicate is at least 75 ms and gain exceeds duplicate spread in at least four rotations. Reconcile every row/XID and require <=1% steal per arm.",
        staging_path=str(HERE), evidence_destination=str(TARGET / "native-01"),
        offline_check=f"python3 -B {runner} check",
        timing_command=f"cd {ROOT} && python3 -B {runner} dispatch native-01",
        replay_command=f"python3 -B {runner.with_name('replay.py')} target/app-plan-reuse/native-01/raw.tar.gz",
        local_sha256=pins(),
    )
    run.write_json(REQUEST, request)


def check():
    current_pins = pins()
    if REQUEST.exists():
        request = run.read_json(REQUEST)
        run.require(request["local_sha256"] == current_pins, "controller source or input digest changed")
    subprocess.run([sys.executable, "-B", HERE / "measure.py", "--check"], check=True)
    subprocess.run(["bash", "-n"], input=SHELL, text=True, check=True)
    run.require(len(ORDERS) == 6 and JOBS == 1024, "rotation geometry changed")
    print("Controller source, historical archive pins, and remote shell verified")


def grant():
    entries = re.findall(r"^(SETUP|TIMING|RELEASED|HOLD)\s+(\S+)\s+(\S+)\s+(\S+)",
                         (QUEUE / "STATUS.md").read_text(), flags=re.M)
    run.require(entries and entries[-1][:2] == ("TIMING", ID), "no active controller TIMING assignment")
    _, _, start, end = entries[-1]
    run.require(datetime.fromisoformat(start) <= datetime.now(timezone.utc) < datetime.fromisoformat(end),
                "outside controller TIMING assignment")
    run.require(REQUEST.read_bytes() == (QUEUE / f"{ID}.json").read_bytes(), "controller inbox differs from seal")
    return dict(start=start, end=end, request_sha256=run.digest(REQUEST))


def bundle(output):
    manifest = output / "INPUTS.sha256"
    manifest.write_text("".join(f"{h}  {name}\n" for name, h in pins().items()))
    staged = output / "input.tar"
    with tarfile.open(staged, "x") as archive:
        archive.add(manifest, arcname=manifest.name)
        for path in paths():
            archive.add(path, arcname=path.relative_to(ROOT).as_posix())
    # Recheck transferred members against the exact current inputs.
    with tarfile.open(staged) as archive:
        expected = {str(p.relative_to(ROOT)): run.digest(p) for p in paths()}
        for name, digest in expected.items():
            assert hashlib.file_digest(archive.extractfile(name), "sha256").hexdigest() == digest
        assert set(archive.getnames()) == {manifest.name, *expected}
    return staged


def dispatch(attempt):
    active = grant()
    check()
    output = TARGET / attempt
    output.mkdir()
    staged = bundle(output)
    raw = output / "raw.tar.gz"
    with (run.SUITE / ".timing.lock").open("rb") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        run.require(grant() == active, "controller grant changed")
        try:
            with staged.open("rb") as inp, raw.open("xb") as out, (output / "stage.log").open("xb") as log:
                result = run.ssh(["bash", "-c", SHELL, ID, run.digest(staged)], stdin=inp,
                                 stdout=out, stderr=log, timeout=240)
        except (subprocess.SubprocessError, KeyboardInterrupt) as exc:
            run.write_json(output / "status.json", dict(exit_code=None, grant=active,
                           interrupted=type(exc).__name__, finished=run.utc_now()))
            raise
    run.write_json(output / "status.json", dict(exit_code=result.returncode, grant=active,
                   raw_sha256=run.digest(raw), finished=run.utc_now()))
    result.check_returncode()
    summary = summarize(raw)
    run.write_json(output / "summary.json", summary)
    run.require(summary["verdict"] != "unqualified", "host interference invalidated the native campaign")
    print(summary["verdict"])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("check", "bundle-check", "request", "dispatch"))
    parser.add_argument("value", nargs="?")
    args = parser.parse_args()
    if args.command == "check":
        check()
    elif args.command == "request":
        make_request(args.value)
    elif args.command == "bundle-check":
        assert args.value and re.fullmatch(r"[a-zA-Z0-9_-]+", args.value)
        output = TARGET / args.value
        output.mkdir()
        print(bundle(output))
    else:
        assert args.value and re.fullmatch(r"native-[0-9]+", args.value)
        dispatch(args.value)


if __name__ == "__main__":
    main()
