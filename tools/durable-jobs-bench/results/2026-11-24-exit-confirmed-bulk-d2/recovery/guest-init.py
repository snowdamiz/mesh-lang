"""Regress the actual old transfer and repaired extractor in a Linux guest."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile

ROOT = Path("/relay-regression")
sys.path.insert(0, str(ROOT))
from stage import audit, prepare


def digest(path):
    return hashlib.file_digest(path.open("rb"), "sha256").hexdigest()


def extract(path, destination):
    with tarfile.open(path) as archive:
        archive.extractall(destination, filter="data")


def probe(directory):
    return subprocess.run(
        [sys.executable, "-B", "-X", f"pycache_prefix={directory / 'no-cache'}",
         str(directory / "replay/staged-relay-check.py"), str(ROOT / "certs")],
        user=65534, group=65534, extra_groups=[], check=False, text=True, capture_output=True,
        env={"PATH": "/usr/bin:/bin", "LC_ALL": "C"}, timeout=15,
    )


def main():
    # Minimal initramfs images may have IFF_UP clear on loopback.
    import fcntl
    import socket
    import struct
    with socket.socket() as transport:
        flags = fcntl.ioctl(transport, 0x8913, struct.pack("16sH22x", b"lo", 0))
        state = struct.unpack_from("H", flags, 16)[0]
        fcntl.ioctl(transport, 0x8914, struct.pack("16sH22x", b"lo", state | 1))

    pins = json.loads((ROOT / "pins.json").read_text())
    old = ROOT / "original"
    old.mkdir()
    assert digest(ROOT / "old.tar") == pins["old"]
    extract(ROOT / "old.tar", old)
    before = {name: digest(old / name) for name in pins["original_shipped"]}
    assert before == pins["original_shipped"]
    # Original :34, then :67. This deliberately recreates the overwrite.
    extract(old / "combined-source.tar.gz", old)
    overwritten = [name for name in before if digest(old / name) != before[name]]
    assert set(overwritten) == {"replay/wire.py", "replay/combined-run", "replay/split_arm.py"}
    shutil.copyfile(ROOT / "staged-relay-check.py", old / "replay/staged-relay-check.py")
    result = probe(old)
    assert result.returncode == 1, result
    assert "AttributeError: 'Relay' object has no attribute 'requests'" in result.stderr, result.stderr
    print(result.stderr.rstrip(), flush=True)

    new = ROOT / "repaired"
    new.mkdir()
    shutil.copyfile(ROOT / "recovery.tar", new / "incoming.tar")
    verified = prepare(new, pins["recovery"], pins["source"])
    assert verified == pins["repaired_shipped"]
    assert digest(new / "capsule/replay/wire.py") == digest(old / "replay/wire.py")
    recovered = probe(new)
    assert recovered.returncode == 0, recovered.stderr
    passed = json.loads(recovered.stdout)
    assert passed["cpu_mask"] == [0, 1]
    changed = new / "replay/wire.py"
    changed.write_bytes((new / "capsule/replay/wire.py").read_bytes())
    try:
        audit(new)
    except AssertionError as error:
        assert str(error) == "shipped file drift: replay/wire.py", error
    else:
        raise AssertionError("post-extraction audit accepted the old Relay")

    return dict(old_transfer_sha256=pins["old"], recovery_transfer_sha256=pins["recovery"],
                old_exit=result.returncode, old_error="Relay.requests missing",
                old_overwrites=sorted(overwritten), repaired_shipped=verified,
                repaired=passed, tampered_wire="refused")


if __name__ == "__main__":
    print("RELAY_LAYOUT_REGRESSION " + json.dumps(main(), sort_keys=True), flush=True)
