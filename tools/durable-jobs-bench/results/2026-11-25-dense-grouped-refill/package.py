"""Produce a deterministic, host-free source packet from the archived public SETUP."""

import gzip
import hashlib
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
from tempfile import TemporaryDirectory

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
BASE = "b3557273d72be3802bf189b284e7caaf5f3caa0f"
ORIGINAL = HERE.parent / "2026-11-24-trusted-jobs-product-composition"
ORIGINAL_SHA = "623c46b92cc5f94eb19d455e36e6fca1e1b800231441a6de480e2c35aa091b02"
REPLAY = ORIGINAL.relative_to(ROOT) / "replay"


def sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def stage(archive):
    original = ORIGINAL / "source.tar.gz"
    assert sha(original) == ORIGINAL_SHA
    with TemporaryDirectory(prefix="assemble-", dir=archive.parent) as directory:
        work = Path(directory)
        with tarfile.open(original) as stock:
            stock.extractall(work, filter="data")
        source = work / "source"
        replay = source / REPLAY
        # The grouped-hot timing series required this SQL counter repair.
        shutil.copyfile(ORIGINAL / "closure-retry/backend.py", replay / "backend.py")
        patch = HERE / "candidate.patch"
        subprocess.run(["patch", "--batch", "--forward", "--fuzz=0", "-p1", "-i", patch],
                       cwd=source, check=True, stdout=sys.stderr)
        shutil.copyfile(HERE / "verify_candidate.py", replay / "verify_grouped_writer.py")
        with archive.open("xb") as raw, gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0) as zipped:
            with tarfile.open(fileobj=zipped, mode="w") as packed:
                def normalize(info):
                    info.uid = info.gid = info.mtime = 0
                    info.uname = info.gname = ""
                    info.pax_headers = {}
                    return info
                packed.add(source, arcname="source", filter=normalize)
    return sha(archive)


if __name__ == "__main__":
    print(stage(Path(sys.argv[1]).resolve()))
