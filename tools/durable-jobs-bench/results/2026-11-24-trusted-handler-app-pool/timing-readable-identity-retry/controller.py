"""Retry the same handler-pool comparison after verifying staged helper access."""
import argparse
import errno
from datetime import datetime, timezone
from pathlib import Path
import re
import runpy
import shlex
import subprocess
import sys
import tarfile

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "import-fix"))
import dispatch as original

base = original.base
run = base.run
ops = runpy.run_path(str(base.QUEUE.parent / "completion-writer-r2.py"))
ID = base.ID + "-timing-readable-identity-retry"
PACKET = HERE / "request.json"
QUALIFIED = base.qualified_setup
SCORE = base.score_arm
IDENTITY = run.RESULTS / "2026-11-24-dense-individual-hot-pool" / "identity.py"
FAILED = base.EVIDENCE / "timing"
PREVIOUS = HERE.parent / "timing-identity-retry"
ARM = PREVIOUS / "native-arm.py"
SHELL = HERE / "native-arm.sh"
OUT = HERE / "evidence"


def inputs():
    original.check()
    qualified = QUALIFIED()
    failure = run.read_json(FAILED / "failure.json")
    release = run.read_json(FAILED / "controller-release.json")
    raw_path = FAILED / "01-noop/warmup-service-a/warmup-mesh/raw.tar.gz"
    with tarfile.open(raw_path) as raw:
        base.require(run.file_json(raw, "adapter/failure.json") == failure
                     and run.file_json(raw, "status.json")["exit_code"] == 1, "original failed archive differs")
    base.require(release["exit_status"] == 1 and release["released_at"]
                 and failure["type"] == "PermissionError"
                 and re.fullmatch(r"\[Errno 13\] Permission denied: '/proc/\d+/exe'", failure["message"])
                 and any(stage[2] == "capture_runners" for stage in failure["stage"]), "unexpected original failure")
    second = PREVIOUS / "evidence"
    second_release = run.read_json(second / "controller-release.json")
    second_raw = second / "timing/01-noop/warmup-service-a/warmup-mesh/raw.tar.gz"
    with tarfile.open(second_raw) as raw:
        trace = raw.extractfile("adapter.log").read()
        base.require(trace == (second / "entry-failure.txt").read_bytes()
                     and b"PermissionError: [Errno 13]" in trace and b"-identity-retry/identity.py'" in trace
                     and run.file_json(raw, "status.json")["exit_code"] == second_release["exit_status"] == 1,
                     "the immediate stage access failure differs")
    subprocess.run(["bash", "-n", SHELL], check=True)
    for path in (ARM, IDENTITY):
        compile(path.read_bytes(), str(path), "exec")
    return dict(id=ID, ready=True, source_sha=base.SOURCE, qualified_setup=qualified["raw_sha256"],
                prior_arm=run.digest(raw_path), prior_release=run.digest(FAILED / "controller-release.json"),
                import_amendment=run.digest(original.AMENDMENT), helper_failure=run.digest(second_raw),
                helper_failure_release=run.digest(second / "controller-release.json"),
                prior_identity_request=run.digest(PREVIOUS / "request.json"),
                local_sha256={str(path.relative_to(run.ROOT)): run.digest(path)
                              for path in (Path(__file__).resolve(), ARM, SHELL, IDENTITY)},
                evidence_destination=str(OUT), cpus=[0, 1]), qualified


def inventories(qualified):
    for lane, previous, inventory, old_files in original.inventories(qualified):
        remote = previous.with_name(previous.name.removesuffix("-import-fix") + "-readable-identity-retry")
        files = {**old_files, "native-arm.py": ARM, "native-arm.sh": SHELL, "identity.py": IDENTITY}
        spec = inventory["engines"]["mesh"]
        for name in old_files:
            del spec["verified_files"][str(previous / name)]
        spec["verified_files"].update({str(remote / name): run.digest(path) for name, path in files.items()})
        spec["argv"][1] = str(remote / "native-arm.sh")
        run.check_inventory(inventory)
        yield lane, remote, inventory, files


def grant(kind):
    events = re.findall(r"^(SETUP|TIMING|RELEASED|HOLD)\s+(\S+)\s+(\S+)\s+(\S+)",
                        (base.QUEUE / "STATUS.md").read_text(), re.M)
    base.require(events and events[-1][:2] == (kind, ID) and kind == "TIMING", "dedicated timing grant required")
    _, _, start, end = events[-1]
    base.require(datetime.fromisoformat(start) <= datetime.now(timezone.utc) < datetime.fromisoformat(end)
                 and PACKET.read_bytes() == (base.QUEUE / f"{ID}.json").read_bytes(), "inactive or mismatched retry grant")
    return dict(start=start, end=end, request_sha256=run.digest(PACKET))


def score(folder, block, phase, lane, inventory, host, ticks):
    with tarfile.open(folder / "raw.tar.gz") as raw:
        base.require(raw.extractfile("adapter/helper-access.txt").read() == b"APP_POOL_STAGED_HELPERS_READABLE\n",
                     "missing root and benchmark-UID helper readback")
        observed = run.file_json(raw, "adapter/runner-identity.json")
    spec = inventory["engines"]["mesh"]
    path = spec["argv"][3] + "/mesh"
    parents = set()
    for row in observed["runners"]:
        base.require(row["argv"] == [path, "run"] and row["cpus"] == [0, 1]
                     and row["executable_sha256"] == spec["verified_files"][path]
                     and row["launch_start_ticks"] == row["start_ticks_before"] == row["start_ticks_after"] > 0
                     and row["parent_pid"] > 1
                     and (row["method"], row["observed_executable"], row["exe_errno"]) in {
                         ("proc-exe", path, None), ("owned-launch", None, errno.EACCES),
                     }, "runner direct launch was not independently verified")
        parents.add(row["parent_pid"])
    base.require(len(parents) == 1, "runners must share one direct parent")
    return SCORE(folder, block, phase, lane, inventory, host, ticks)


def set_controller(qualified):
    base.qualified_setup = lambda: qualified
    base.EVIDENCE = OUT
    base.inventories = inventories
    base.grant = grant
    base.score_arm = score


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("freeze", "check", "slots", "timing", "replay"))
    parser.add_argument("groups", nargs="*", type=int)
    args = parser.parse_args()
    packet, qualified = inputs()
    if args.action == "freeze":
        run.write_json(PACKET, packet)
        return 0
    base.require(run.read_json(PACKET) == packet
                 and PACKET.read_bytes() == (base.QUEUE / f"{ID}.json").read_bytes(), "sealed timing packet changed")
    if args.action == "check":
        for prior, updated in zip(original.inventories(qualified), inventories(qualified), strict=True):
            lane, remote, inventory, files = updated
            old_lane, old_remote, old_inventory, old_files = prior
            spec, old = inventory["engines"]["mesh"], old_inventory["engines"]["mesh"]
            base.require(lane == old_lane and remote != old_remote and spec["argv"][2:] == old["argv"][2:]
                         and spec["artifact_sha256"] == old["artifact_sha256"]
                         and set(files) == set(old_files) | {"identity.py"}, "timing identity repair changed a runner")
        return 0
    set_controller(qualified)
    if args.action == "timing":
        OUT.mkdir(exist_ok=True)
        return base.timing(args.groups)
    if args.action == "replay":
        return base.replay()
    receipts = base.QUEUE.parent / "controller-receipts" / ID
    timing = run.read_json(original.AMENDMENT)["requested_timing_windows_minutes"]
    for groups, minutes in zip(([1, 2], [3, 4, 5, 6]), timing, strict=True):
        release = base.QUEUE / f"{ID}-groups-{groups[0]}-{groups[-1]}.released.json"
        if not release.exists():
            trial = dict(packet, timing_grant_id=ID, timing_window_minutes=minutes, command_attempts=1,
                         controller_receipt_dir=receipts,
                         dispatch=shlex.join([sys.executable, "-B", str(Path(__file__)), "timing", *map(str, groups)]))
            ops["measure"](trial, "dispatch", groups)
        if run.read_json(release)["exit_status"]:
            return 1
    return base.replay()


if __name__ == "__main__":
    sys.exit(main())
