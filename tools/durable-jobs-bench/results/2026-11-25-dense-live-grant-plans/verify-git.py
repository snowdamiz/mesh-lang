"""Run the offline handoff check from committed files only.

Run with an optional commit argument, which defaults to HEAD. The export
includes the seal and its named dependencies, plus the historical failure
archives. It needs neither PostgreSQL nor controller state.
"""
import hashlib
import io
import json
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
SCOPE = HERE.relative_to(ROOT)
MANIFEST = SCOPE / "git-replay-pins.json"
PACKET = SCOPE / "controller-packet.json"
RECEIPT = SCOPE / "evidence/offline-check.json"


def git(*args):
    return subprocess.check_output(["git", "-C", ROOT, *args])


def digest(data):
    return hashlib.sha256(data).hexdigest()


def verify(revision):
    commit = git("rev-parse", "--verify", f"{revision}^{{commit}}").decode().strip()
    manifest = json.loads(git("show", f"{commit}:{MANIFEST}"))
    packet_bytes = git("show", f"{commit}:{PACKET}")
    if digest(packet_bytes) != manifest["controller_packet_sha256"]:
        raise ValueError("controller packet hash changed")
    packet = json.loads(packet_bytes)
    pins = dict(manifest["git_export_inputs"], **packet["local_sha256"])
    paths = sorted({*pins, str(MANIFEST), str(PACKET), str(RECEIPT), str(SCOPE / "verify-git.py")})
    archive = git("archive", "--format=tar", commit, "--", *paths)
    with tempfile.TemporaryDirectory(prefix="dense-live-grant-git-") as temp:
        export = Path(temp)
        with tarfile.open(fileobj=io.BytesIO(archive)) as bundle:
            bundle.extractall(export, filter="data")
        for path, pin in pins.items():
            if digest((export / path).read_bytes()) != pin:
                raise ValueError(f"exported pin changed for {path}")
        result = subprocess.check_output(
            [sys.executable, "-B", export / SCOPE / "controller.py", "check"], cwd=export
        )
        if json.loads(result) != json.loads((export / RECEIPT).read_bytes()):
            raise ValueError("offline check changed from the retained receipt")
    print(f"{commit} verified from a {len(paths)}-file Git export.")
    print(f"Controller packet SHA-256 {digest(packet_bytes)}")


if __name__ == "__main__":
    if len(sys.argv) > 2:
        raise SystemExit("usage: verify-git.py [commit]")
    verify(sys.argv[1] if len(sys.argv) == 2 else "HEAD")
