"""Extract the transfer and verify every shipped entry after the nested extraction."""
import hashlib
import json
from pathlib import Path
import stat
import sys
import tarfile


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def prepare(stage, transfer_sha, source_sha):
    transfer = stage / "incoming.tar"
    assert digest(transfer) == transfer_sha
    with tarfile.open(transfer) as archive:
        assert len(archive.getnames()) == len(set(archive.getnames()))
        assert all(member.isfile() for member in archive)
        archive.extractall(stage, filter="data")
    source = stage / "combined-source.tar.gz"
    assert digest(source) == source_sha
    capsule = stage / "capsule"
    capsule.mkdir()
    with tarfile.open(source) as archive:
        archive.extractall(capsule, filter="data")
    return audit(stage)


def audit(stage):
    expected = json.loads((stage / "shipped.json").read_text())
    replay = stage / "replay"
    assert replay.is_dir() and not replay.is_symlink()
    assert set(expected) == {str(path.relative_to(stage)) for path in replay.iterdir()}
    for name, sha256 in expected.items():
        file = stage / name
        assert stat.S_ISREG(file.lstat().st_mode) and digest(file) == sha256, f"shipped file drift: {name}"
    assert not (stage / "no-cache").exists()
    return expected


if __name__ == "__main__":
    stage = Path(sys.argv[1]).resolve(strict=True)
    print(json.dumps(prepare(stage, *sys.argv[2:]), sort_keys=True))
