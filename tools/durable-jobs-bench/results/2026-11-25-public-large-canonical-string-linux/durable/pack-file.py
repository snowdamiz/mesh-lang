"""Emit sealed original FILE launch and link inputs for the earned SETUP phase."""
import gzip
import hashlib
import io
import json
from pathlib import Path
import sys
import tarfile

HERE = Path(__file__).resolve().parent
RESULTS = HERE.parents[1]
SESSION = RESULTS / "2026-11-24-public-large-file-session"
EXT = RESULTS / "2026-11-25-public-large-file-external" / "grant-parser-retry/evidence/native"


def sha(data):
    return hashlib.sha256(data).hexdigest()


def main():
    original = (SESSION / "resume/stage.tar.gz").read_bytes()
    with tarfile.open(fileobj=io.BytesIO(original)) as archive:
        names = ("parked-stage.tar.gz", "toolchain.sha256")
        stage, tools = [archive.extractfile(name).read() for name in names]
    assert stage == (SESSION / "stage.tar.gz").read_bytes()
    files = {"session-stage.tar.gz":stage,"toolchain.sha256":tools,
             "file-inventory.json":(EXT / "file/inventory.json").read_bytes()}
    files["SHA256SUMS"] = "".join(f"{sha(data)}  {name}\n" for name, data in sorted(files.items())).encode()
    with gzip.GzipFile(filename="", mode="wb", fileobj=sys.stdout.buffer, mtime=0) as compressed:
        with tarfile.open(fileobj=compressed, mode="w|", format=tarfile.USTAR_FORMAT) as archive:
            for name, data in sorted(files.items()):
                member = tarfile.TarInfo(name)
                member.size = len(data)
                archive.addfile(member, io.BytesIO(data))


if __name__ == "__main__":
    main()
