"""Restore the sealed D2 result for offline replay in a new directory."""

import argparse
import gzip
from pathlib import Path
from shutil import copy2, copyfileobj, copytree
import subprocess
import tarfile


HERE = Path(__file__).resolve().parent
PACKET = HERE.parent
REPO = PACKET.parents[3]

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("destination", type=Path)
destination = parser.parse_args().destination.resolve()
destination.mkdir()
(destination / "tools").symlink_to(REPO / "tools", target_is_directory=True)
for selected in (PACKET / "sealed-inputs").rglob("*"):
    if not selected.is_file():
        continue
    output = destination / selected.relative_to(PACKET / "sealed-inputs")
    output.parent.mkdir(parents=True, exist_ok=True)
    if output.name.endswith(".log.gz"):
        with gzip.open(selected, "rb") as source, output.with_suffix("").open("xb") as restored:
            copyfileobj(source, restored)
    else:
        copy2(selected, output)

lane = destination / "target/ordinary-hot-checkout"
copytree(PACKET / "evidence", lane / "evidence")
for packed in (lane / "evidence").glob("*/native-identity.txt.gz"):
    with gzip.open(packed, "rb") as source, packed.with_suffix("").open("xb") as restored:
        copyfileobj(source, restored)
(lane / "tmp").mkdir()
state = destination / "target/linode-jobs-stepchange"
state.mkdir(exist_ok=True)
for name in ("d2-native-recovery.py", "d2-native-timing.py"):
    copy2(HERE / name, state / name)
source = lane / "v2-d2"
with tarfile.open(lane / "native/payload.tar") as transfer:
    with tarfile.open(fileobj=transfer.extractfile("ordinary-trusted-v2.tar.gz"), mode="r:gz") as original:
        for name in ("pg.rs", "pool.rs"):
            path = Path("compiler/mesh-rt/src/db") / name
            with original.extractfile(str(path)) as contents, (source / path).open("wb") as restored:
                copyfileobj(contents, restored)
    with transfer.extractfile("runtime-v2.patch") as patch:
        subprocess.run(["patch", "--batch", "--fuzz=0", "-p1"], cwd=source, input=patch.read(),
                       stdout=subprocess.DEVNULL, check=True)
print(state / "d2-native-timing.py")
