"""Seal raw files and exact input archives with a manifest of uncompressed bytes."""
import argparse
import gzip
import hashlib
import io
import json
from pathlib import Path
import re
import tarfile

MANIFEST = "SHA256.json"
PRIVATE_KEY = re.compile(rb"-----BEGIN (?:[A-Z0-9]+ )?PRIVATE KEY-----")
SECRET_URL = re.compile(rb"postgres(?:ql)?://[^/ :@]+:[a-zA-Z0-9_+=-]{16,}@")


def inspect(name, data):
    path = Path(name)
    if any(part in {"scratch", "pgdata", "__pycache__", ".DS_Store"} or part.startswith("._")
           for part in path.parts) or path.suffix in {".key", ".pem", ".db", ".sqlite", ".pyc"}:
        raise ValueError(f"forbidden archive member: {name}")
    if PRIVATE_KEY.search(data) or SECRET_URL.search(data):
        raise ValueError(f"credential-like material in {name}")
    if name.startswith("sources/") and name.endswith(".tar.gz"):
        with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as nested:
            for entry in nested:
                if entry.isfile():
                    inspect(entry.name, nested.extractfile(entry).read())


def checksum(data):
    return hashlib.sha256(data).hexdigest()


def pack(output, packet, sources):
    paths = {"README.md": packet / "README.md"}
    for prefix, directory in (("evidence", packet / "evidence"),
                              ("replay", packet / "replay"), ("sources", sources)):
        for path in directory.rglob("*"):
            if path.is_file() and not path.is_symlink():
                paths[f"{prefix}/{path.relative_to(directory)}"] = path
    assert len(list((packet / "evidence/scored").glob("*/*/ledger.json"))) == 18
    assert len(list((packet / "evidence/diagnostic").glob("*/*/ledger.json"))) == 2
    hashes = {}
    with output.open("xb") as target, gzip.GzipFile(fileobj=target, mode="wb", mtime=0) as compressed, \
         tarfile.open(fileobj=compressed, mode="w") as archive:
        for name, path in sorted(paths.items()):
            data = path.read_bytes()
            inspect(name, data)
            hashes[name] = checksum(data)
            member = tarfile.TarInfo(name)
            member.size = len(data)
            member.mode = 0o755 if path.stat().st_mode & 0o100 else 0o644
            archive.addfile(member, io.BytesIO(data))
        data = (json.dumps(hashes, sort_keys=True, indent=2) + "\n").encode()
        member = tarfile.TarInfo(MANIFEST)
        member.size = len(data)
        archive.addfile(member, io.BytesIO(data))


def verify(path):
    with tarfile.open(path) as archive:
        expected = json.load(archive.extractfile(MANIFEST))
        actual = {item.name: checksum(archive.extractfile(item).read())
                  for item in archive if item.isfile() and item.name != MANIFEST}
    assert expected == actual
    print(f"{len(actual)} members match their SHA-256 hashes")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("archive", type=Path)
    parser.add_argument("--pack", nargs=2, type=Path, metavar=("PACKET", "SOURCES"))
    args = parser.parse_args()
    if args.pack:
        pack(args.archive, *args.pack)
    verify(args.archive)
