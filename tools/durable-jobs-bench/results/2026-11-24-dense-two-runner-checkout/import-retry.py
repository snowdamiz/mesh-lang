"""Retry the frozen dense schedule with a readable, explicitly loaded identity module."""
import argparse
from datetime import datetime, timezone
from importlib import import_module
import json
from pathlib import Path
import re
import subprocess
import sys

HERE = Path(__file__).resolve().parent
dense = import_module("native-controller")
run = dense.run
ID = dense.ID + "-import-retry"
ORIGINAL = HERE / "evidence/native"
RESULTS = HERE / "evidence/native-import-retry"
RETRY = HERE / "request-import-retry.json"
LAUNCHER = HERE / "native-dense-import-retry.sh"
SERIES_SHA = "3b1611e742d60caaba65ca86bb86d8d46354ed200d34d8c2511ba9908f947fd3"
FAILED_SHA = "cb31d68460ad2d66e4b4b48459d8198a2ad3fb0a86eba137d6f6d853b1f7bc5f"
FAILED = ORIGINAL / "blocks/01-noop.1795492030327231000/warmup-d2-off-a/warmup-mesh/raw.tar.gz"
original_stages = dense.stages


def stages(config, label, remote):
    files, inventory = original_stages(config, label, remote)
    if label in dense.PEERS:
        return files, inventory
    spec = inventory["engines"]["mesh"]
    old = str(remote / "native-dense.sh")
    new = str(remote / LAUNCHER.name)
    assert spec["argv"][1] == old
    spec["argv"][1] = new
    del spec["verified_files"][old]
    spec["verified_files"][new] = run.digest(LAUNCHER)
    files = [LAUNCHER if path.name == "native-dense.sh" else path for path in files]
    run.check_inventory(inventory)
    return files, inventory


def grant(_=None):
    log = (dense.QUEUE / "STATUS.md").read_text()
    mood, request, start, end = re.findall(
        r"^(SETUP|TIMING|RELEASED|HOLD) (\S+) (\S+)(?: (\S+))?", log, re.MULTILINE
    )[-1]
    dense.require((mood, request) == ("TIMING", ID)
                  and datetime.fromisoformat(start) <= datetime.now(timezone.utc) < datetime.fromisoformat(end),
                  "dense retry must have its own active TIMING grant")
    dense.require(RETRY.read_bytes() == (dense.QUEUE / f"{ID}.json").read_bytes(),
                  "dense retry queue seal changed")
    return dict(start=start, end=end, request_sha256=run.digest(RETRY))


def inputs():
    packet = run.read_json(RETRY)
    dense.require(packet["id"] == ID and packet["cpus"] == [0, 1], "wrong dense retry scope")
    for name, digest in packet["local_sha256"].items():
        dense.require(run.digest(dense.ROOT / name) == digest, f"changed dense retry input: {name}")
    dense.require(run.digest(ORIGINAL / "series.json") == SERIES_SHA and run.digest(FAILED) == FAILED_SHA,
                  "original manifest or failed arm changed")
    verified = json.loads(subprocess.check_output(
        [sys.executable, "-B", HERE / "bind-recovered-setup.py", "check"], cwd=dense.ROOT
    ))
    original = run.read_json(ORIGINAL / "series.json")
    config, blocks, original_pins = (original[k] for k in ("config", "schedule", "pins"))
    dense.require(original_pins == verified["pins"] and config == dense.configurations(verified["candidate"])
                  and blocks == list(dense.schedule()), "retry changes original plan or artifact binding")
    pins = dict(original_pins, import_retry_request=run.digest(RETRY),
                original_series=SERIES_SHA, original_failed_arm=FAILED_SHA, **packet["local_sha256"])
    return config, blocks, pins


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("check", "timing", "replay"))
    parser.add_argument("groups", type=int, nargs="*")
    args = parser.parse_args()
    config, blocks, pins = inputs()
    if args.action == "check":
        subprocess.run(["bash", "-n", LAUNCHER], check=True)
        for label in config["lanes"]:
            stages(config, label, run.REMOTE / label)
        print("Verified original dense schedule and recovered ELF with the explicit identity loader")
        return 0
    dense.RESULTS, dense.ID = RESULTS, ID
    dense.stages, dense.grant = stages, grant
    dense.attach_matrix()
    if args.action == "timing":
        dense.require(args.groups in ([1, 2], [3, 4, 5, 6]), "use the frozen pilot or remaining rotations")
        return dense.dispatch(config, blocks, pins, args.groups)
    report = dense.summarize(RESULTS, config, blocks, pins)
    print(json.dumps(report, indent=2, sort_keys=True, allow_nan=False))
    return bool(report["missing"])


if __name__ == "__main__":
    sys.exit(main())
