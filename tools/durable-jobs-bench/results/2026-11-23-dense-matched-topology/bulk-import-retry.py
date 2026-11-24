"""Retry the original native producer pairs with an explicitly loaded topology helper."""

import argparse
from datetime import datetime, timezone
import io
import json
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tarfile
from tempfile import TemporaryDirectory

import bulk

ID = bulk.ID + "-import-retry"
ORIGINAL = bulk.PACKET / "evidence/bulk-pair-first-failed"
REQUEST = bulk.PACKET / "request-bulk-import-retry.json"
LAUNCHER = bulk.PACKET / "native-bulk-import-retry.sh"
LINUX = bulk.PACKET / "evidence/bulk-import-linux.json"
DESTINATION = bulk.WORK / "bulk-import-retry"
FILES = [LAUNCHER if path.name == "native-bulk.sh" else path for path in bulk.FILES]
run, require = bulk.run, bulk.require


def original(config):
    manifest = run.read_json(ORIGINAL / "series.json")
    require(manifest["config"] == config and manifest["orders"] == [list(order) for order in bulk.ORDERS],
            "the retry must use the original native configuration and rotations")
    for mode, name in manifest["remotes"].items():
        pinned = run.read_json(ORIGINAL / "staging" / mode / "inventory.json")
        require(pinned == bulk.inventory(config, mode, run.REMOTE / name), "original staged inventory differs")

    receipts = {}
    for group, order in enumerate(bulk.ORDERS, 1):
        for phase in ("warmup", "measured"):
            for mode in order:
                folder = ORIGINAL / f"{group:02}" / f"{phase}-{mode}" / f"{phase}-mesh"
                archive = folder / "raw.tar.gz"
                with tarfile.open(archive) as raw:
                    status = run.file_json(raw, "status.json")
                    require(status["exit_code"] == run.read_json(folder / "dispatch.json")["exit_code"] == 1
                            and b"ModuleNotFoundError: No module named 'arm'" in raw.extractfile("adapter.log").read()
                            and "adapter/ledger.json" not in raw.getnames(),
                            f"unexpected original disposition in {folder.name}")
                receipts[str(archive.relative_to(ORIGINAL))] = run.digest(archive)
    require(len(receipts) == len(set(receipts.values())) == 8, "eight independent failed arms required")
    return receipts


def inventory(config, mode, remote):
    # Keep every original preflight and engine pin, relocating only staged files.
    result = run.read_json(ORIGINAL / "staging" / mode / "inventory.json")
    require(result["cpus"] == config["cpus"], "CPU allocation differs from original staged inventory")
    spec = result["engines"]["mesh"]
    old_launcher = Path(spec["argv"][1])
    require(old_launcher.name == "native-bulk.sh", "unexpected original launcher")
    spec["argv"][1] = str(remote / LAUNCHER.name)
    pins = spec["verified_files"]
    for staged in FILES:
        name = "native-bulk.sh" if staged == LAUNCHER else staged.name
        original_sha = pins.pop(str(old_launcher.parent / name))
        digest = run.digest(staged)
        require(staged == LAUNCHER or original_sha == digest, f"pinned stage bytes changed: {name}")
        pins[str(remote / staged.name)] = digest
    run.check_inventory(result)
    return result


def linux_check():
    proof = run.read_json(LINUX)
    require(proof["uid"] == proof["gid"] == 65534 and proof["capabilities"] == 0
            and proof["stage_mode"] == "0711" and proof["file_mode"] == "0644"
            and proof["explicit_reads_pass"] is proof["directory_listing_denied"] is proof["before_workload"] is True
            and proof["original_exit"] == 1 and proof["retry_exit"] == 0
            and proof["original_error"] == "ModuleNotFoundError: No module named 'arm'"
            and "{scalar,bulk}" in proof["retry_help"], "invalid unprivileged import proof")
    files = [LAUNCHER if name == LAUNCHER.name else bulk.TOPOLOGY / name if name == "arm.py"
             else bulk.PACKET / name for name in ("native-bulk.sh", LAUNCHER.name, "bulk-arm.py", "arm.py")]
    require(proof["sha256"] == {path.name: run.digest(path) for path in files}, "proof imported different bytes")


def completion_negative():
    stock = bulk.PACKET / "evidence/stock"
    manifest = run.read_json(stock / "series.json")
    source = sorted(stock.glob("blocks/01-*/measured-mesh-v1-2x32/measured-mesh"))[0]
    staged = run.read_json(stock / "staging/mesh-v1-2x32/inventory.json")
    def score(folder):
        return bulk.score_arm(folder, "mesh", 2, "measured", staged, manifest["config"])

    positive = score(source)
    require(positive["completed"] == 1024, "positive completion-count control changed")
    bulk.WORK.mkdir(parents=True, exist_ok=True)
    with TemporaryDirectory(prefix="completion-negative-", dir=bulk.WORK) as temporary:
        folder = Path(temporary)
        shutil.copyfile(source / "request.json", folder / "request.json")
        with tarfile.open(source / "raw.tar.gz") as raw, tarfile.open(folder / "raw.tar.gz", "w:gz") as altered:
            for member in raw:
                content = raw.extractfile(member)
                if member.name == "adapter/ledger.json":
                    ledger = json.load(content)
                    ledger["jobs"].pop()
                    data = json.dumps(ledger).encode()
                    member.size = len(data)
                    content = io.BytesIO(data)
                altered.addfile(member, content)
        try:
            score(folder)
        except ValueError as error:
            require(str(error) == "missing durable jobs", f"incorrect completion rejection: {error}")
        else:
            raise ValueError("native scorer accepted a ledger with 1023 of 1024 completed jobs")
    return dict(control_sha256=positive["raw_sha256"], positive_completed=1024, mutated_completed=1023,
                rejection="missing durable jobs")


def inputs():
    config, previous = bulk.load()
    request = run.read_json(REQUEST)
    require(request["id"] == ID and request["groups"] == [1, 2] and request["cpus"] == previous["cpus"],
            "unexpected retry request")
    for path, digest in request["local_sha256"].items():
        require(run.digest(bulk.ROOT / path) == digest, f"retry pin differs: {path}")
    failed = original(config)
    linux_check()
    subprocess.run(["bash", "-n", LAUNCHER], check=True)
    for mode in bulk.ORDERS[0]:
        inventory(config, mode, run.REMOTE / ID)
    return config, failed


def grant():
    events = re.findall(r"^(SETUP|TIMING|RELEASED|HOLD)\s+(\S+)\s+(\S+)\s+(\S+)",
                        (bulk.QUEUE / "STATUS.md").read_text(), re.M)
    require(events and events[-1][:2] == ("TIMING", ID), "no active bulk import retry TIMING grant")
    _, _, start, end = events[-1]
    require(datetime.fromisoformat(start) <= datetime.now(timezone.utc) < datetime.fromisoformat(end),
            "outside bulk retry TIMING grant")
    require(REQUEST.read_bytes() == (bulk.QUEUE / f"{ID}.json").read_bytes(), "bulk retry request seal differs")
    return dict(start=start, end=end, request_sha256=run.digest(REQUEST))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("check", "dispatch", "replay"))
    parser.add_argument("--directory", type=Path, default=DESTINATION)
    args = parser.parse_args()
    config, failed = inputs()
    if args.action == "check":
        print(json.dumps(dict(original_failed_raw_sha256=failed, completion=completion_negative(),
                              linux_import_sha256=run.digest(LINUX), orders=bulk.ORDERS), indent=2, sort_keys=True))
        return 0

    # Reuse the original controller-owned lock sequence, arm dispatcher and scorer.
    bulk.ID, bulk.FILES, bulk.inventory, bulk.grant = ID, FILES, inventory, grant
    if args.action == "dispatch":
        require(args.directory == DESTINATION, "dispatch must use the sealed retry evidence destination")
        return bulk.dispatch(args.directory, config)
    result = bulk.replay(args.directory, config)
    print(json.dumps(result, indent=2, sort_keys=True))
    return bool(result["failures"])


if __name__ == "__main__":
    sys.exit(main())
