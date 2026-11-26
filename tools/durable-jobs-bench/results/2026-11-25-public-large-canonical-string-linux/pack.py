"""Emit the deterministic controller capsule as tar.gz on stdout. No builds."""
import gzip
import hashlib
import io
import json
from pathlib import Path
import subprocess
import sys
import tarfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
OLD = HERE.parent / "2026-11-24-enqueue-spec-preparation"
PUBLIC = HERE.parent / "2026-11-24-trusted-jobs-product-composition"
SESSION = HERE.parent / "2026-11-24-public-large-file-session"
BASE = "b3557273d72be3802bf189b284e7caaf5f3caa0f"

FILES = ("candidate.patch", "move.patch", "materialize.py", "probe.rs", "frames.py",
         "rotation.py", "wire-check.py", "public-preimage.sha256",
         "candidate-store.sha256", "move-store.sha256")
ORIGINAL = {
    "original-probe.rs":OLD / "probe.rs",
    "original_rotation.py":OLD / "rotation.py",
    "large-capture.json":OLD / "large-capture.json",
    "public-file-capture.json":SESSION / "evidence/trace-file-replay.json",
    "public-source.tar.gz":PUBLIC / "source.tar.gz",
}


def sha(data):
    return hashlib.sha256(data).hexdigest()


def digest(path):
    with path.open("rb") as data:
        return hashlib.file_digest(data, "sha256").hexdigest()


def inputs():
    pins = json.loads((HERE / "dependencies.json").read_text())
    for path, expected in pins.items():
        assert digest(ROOT / path) == expected, path
    data = {name:(HERE / name).read_bytes() for name in FILES}
    data.update({name:path.read_bytes() for name, path in ORIGINAL.items()})
    data["jobs_parity_fault.py"] = subprocess.check_output(
        ["git","show",f"{BASE}:scripts/lib/jobs_parity_fault.py"], cwd=ROOT)
    assert sha(data["jobs_parity_fault.py"]) == "f77ff946f67b255b4a037bf5a3e28dc7ad0a3588e4efb438ce73cf5abead594b"
    with tarfile.open(OLD / "native-amd64-source.tar.gz") as archive:
        for i in range(64):
            name = f"frames/large/{i}"
            data[name] = archive.extractfile(name).read()
    return data


def pack():
    files = inputs()
    files["SHA256SUMS"] = "".join(f"{sha(data)}  {name}\n" for name, data in sorted(files.items())).encode()
    with gzip.GzipFile(filename="", mode="wb", fileobj=sys.stdout.buffer, mtime=0) as compressed:
        with tarfile.open(fileobj=compressed, mode="w|", format=tarfile.USTAR_FORMAT) as archive:
            for name, data in sorted(files.items()):
                member = tarfile.TarInfo(name)
                member.size = len(data)
                archive.addfile(member, io.BytesIO(data))


if __name__ == "__main__":
    pack()
