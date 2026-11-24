"""Run the exact hot completion-hold screen on a fresh, idle native grant."""
import argparse
from collections import Counter
import importlib.util
import json
from pathlib import Path
import runpy
import shlex
import sys
import tarfile
import tempfile

HERE = Path(__file__).resolve().parent
source = HERE.parent / "controller.py"
spec = importlib.util.spec_from_file_location("original_hold", source)
original = importlib.util.module_from_spec(spec)
spec.loader.exec_module(original)
run = original.run
state = original.dense.QUEUE.parent
ops = runpy.run_path(str(state / "completion-writer-r2.py"))
ID = original.ID + "-load-retry"
PACKET = HERE / "request.json"
OUT = HERE / "evidence"
FAILED = original.OUT


def inputs():
    request_path = original.PACKET
    packet = run.read_json(request_path)
    release_path = FAILED / "controller-release.json"
    release = run.read_json(release_path)
    run.require(release["exit_status"] == 1 and release["released_at"], "original hold timing did not release")
    failures = sorted(FAILED.glob("blocks/*/warmup-oban-2x32/warmup-oban/raw.tar.gz"))
    run.require(len(failures) == 1, "original failed Oban warmup missing")
    with tarfile.open(failures[0]) as raw, raw.extractfile("host-samples.jsonl") as stream:
        observed = Counter(tuple(record) for line in stream for record in json.loads(line)["builders"])
    run.require(len(observed) == 1 and next(iter(observed))[1] == "apt-get", "unexpected builder observation")
    return dict(packet, id=ID, evidence_destination=str(OUT),
                original_packet_sha256=run.digest(request_path), original_controller_sha256=run.digest(source),
                original_release_sha256=run.digest(release_path), original_raw_sha256=run.digest(failures[0]),
                retry_controller_sha256=run.digest(Path(__file__)),
                rejected_host_process=[list(item) + [count] for item, count in observed.items()])


def execute(action, groups):
    original.ID = ID
    original.PACKET = PACKET
    original.OUT = OUT
    with tempfile.TemporaryDirectory(dir=run.ROOT / "target", prefix="dense-hold-load-retry-") as stage:
        return original.execute(action, groups, Path(stage))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("freeze", "check", "slot", "timing", "replay"))
    parser.add_argument("groups", nargs="*", type=int)
    args = parser.parse_args()
    packet = inputs()
    if args.action == "freeze":
        run.write_json(PACKET, packet)
        return 0
    run.require(run.read_json(PACKET) == packet
                and PACKET.read_bytes() == (original.dense.QUEUE / f"{ID}.json").read_bytes(), "retry seal changed")
    if args.action != "slot":
        return execute(args.action, args.groups)
    release = original.dense.QUEUE / f"{ID}-groups-1-2.released.json"
    if not release.exists():
        trial = dict(packet, timing_grant_id=ID, timing_window_minutes=18, command_attempts=1,
                     controller_receipt_dir=state / "controller-receipts" / ID,
                     dispatch=shlex.join([sys.executable, "-B", str(Path(__file__)), "timing", "1", "2"]))
        ops["measure"](trial, "dispatch", [1, 2])
    if run.read_json(release)["exit_status"]:
        return 1
    return execute("replay", [])


if __name__ == "__main__":
    sys.exit(main())
