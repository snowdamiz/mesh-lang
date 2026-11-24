"""Reconstruct and verify the complete native bulk+D2 score offline."""

import gzip
from pathlib import Path
from shutil import copy2, copyfileobj, copytree
import subprocess
import sys
from tempfile import TemporaryDirectory


PACKET = Path(__file__).resolve().parents[1]
REPO = PACKET.parents[3]
FROZEN = PACKET / "sealed-inputs"
CHECKSUM = "7b4d1e9831ff46f63ee1a955232989fb505899245804841ead8c8ea4f2a95cc5"

with TemporaryDirectory(dir=REPO / "target", prefix="bulk-d2-replay-") as tmp:
    root = Path(tmp)
    (root / "tools").symlink_to(REPO / "tools", target_is_directory=True)
    for source in FROZEN.rglob("*"):
        if source.is_file():
            target = root / source.relative_to(FROZEN)
            target.parent.mkdir(parents=True, exist_ok=True)
            if target.name.endswith(".log.gz"):
                with gzip.open(source, "rb") as stream, target.with_suffix("").open("xb") as output:
                    copyfileobj(stream, output)
            else:
                copy2(source, target)
    lane = root / "target/ordinary-hot-checkout/bulk-v2"
    (lane / "tmp").mkdir()
    copytree(PACKET / "evidence", lane / "native-evidence")
    command = [sys.executable, "-B", str(lane / "native/dispatch.py"), "summarize", CHECKSUM]
    replay = subprocess.check_output(command, cwd=REPO)
    if replay != (PACKET / "native-summary.json").read_bytes():
        sys.exit("Recomputed timing summary differs from the archived summary")
print("Five native bulk+D2 rotations reproduced from raw evidence")
