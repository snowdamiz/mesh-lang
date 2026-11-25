"""Verify the archived writer-fusion proof and re-score the retained arms offline."""
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

HERE = Path(__file__).resolve().parent
CASE = HERE.parent
ROOT = CASE.parents[3]
NATIVE = CASE / "evidence/native"
NAME = "dense-fixed8-writer-fusion"


def read_json(path):
    return json.loads(path.read_text())


def stage(root):
    for line in (HERE / "dependencies.sha256").read_text().splitlines():
        checksum, relative = line.split("  ", 1)
        source = ROOT / relative
        data = source.read_bytes()
        if hashlib.sha256(data).hexdigest() != checksum:
            raise ValueError(f"Replay dependency changed: {relative}")
        destination = root / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(data)

    work = root / "target" / NAME
    packet = work / "packet"
    shutil.copytree(HERE / "packet", packet)
    shutil.copyfile(CASE / "controller-packet.json", packet / "controller-packet.json")
    baseline = root / "target/dense-grouped-refill-packet/source.tar.gz"
    baseline.parent.mkdir()
    shutil.copyfile(HERE / "grouped-refill-source.tar.gz", baseline)
    (work / "evidence").symlink_to(NATIVE, target_is_directory=True)
    return packet / "drive.py"


def reproduce():
    with tempfile.TemporaryDirectory(prefix="writer-fusion-replay-") as directory:
        root = Path(directory)
        driver = stage(root)

        def invoke(*arguments):
            return subprocess.check_output(
                [sys.executable, "-E", "-B", str(driver), *map(str, arguments)], cwd=root
            )

        invoke("check")
        setup = NATIVE / "setup"
        receipts = setup / "raw.tar.gz", setup / "controller-release.json"
        binding = json.loads(invoke("bind", *receipts))
        smoke = json.loads(invoke("replay", f"{NAME}-smoke", *receipts))
        pilots = json.loads(invoke("replay", f"{NAME}-pilots", *receipts))

    if binding != read_json(setup / "binding.json"):
        raise ValueError("The setup binding differs from the controller binding")
    independent = read_json(HERE / "independent-pilot-replay.json")
    controller = NATIVE / "controller"
    if pilots != independent or pilots != read_json(controller / f"{NAME}-pilots/replay.json"):
        raise ValueError("The pilot replay differs from its archived results")
    original_smoke = read_json(controller / f"{NAME}-smoke/replay.json")
    # The release replay predated the pilots and only listed the smoke archive.
    if any(smoke["archives"].get(path) != digest for path, digest in original_smoke["archives"].items()):
        raise ValueError("The smoke archive differs from its release replay")
    if dict(smoke, archives=original_smoke["archives"]) != original_smoke:
        raise ValueError("The smoke replay differs from its release replay")
    print(f"Matched the setup binding and both archived phase results across {len(pilots['archives'])} raw arms.")


if __name__ == "__main__":
    reproduce()
