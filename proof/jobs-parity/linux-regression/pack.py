"""Overlay one regression group onto the archived GNU ARM64 guest."""

import hashlib
import json
from pathlib import Path
import stat
import subprocess
import sys
import zlib

SOURCE = Path(__file__).resolve().parent
ROOT = SOURCE.parents[2]
SIBLING = ROOT.parent / "t3-45996d9a/target/jobs-parity"
GUEST = SIBLING / "linux-arm-ownership-guest"
BASE = GUEST / "named-effect-thp-witness/run-02/guest.cpio"
OBAN_REVISION = "8526a5bd70a1cb37f8fc27611021c90a44a89590"


def digest(data):
    return hashlib.sha256(data).hexdigest()


def members(data):
    offset = 0
    while offset < len(data):
        if data[offset] == 0:
            offset = (offset + 4) & ~3
            continue
        if data[offset:offset+3] == b"\x1f\x8b\x08":
            stream = zlib.decompressobj(31)
            yield from members(stream.decompress(data[offset:]))
            return
        assert data[offset:offset+6] == b"070701", offset
        mode = int(data[offset+14:offset+22], 16)
        size = int(data[offset+54:offset+62], 16)
        namesize = int(data[offset+94:offset+102], 16)
        name = data[offset+110:offset+110+namesize-1].decode()
        start = (offset+110+namesize+3) & ~3
        offset = (start+size+3) & ~3
        if name != "TRAILER!!!":
            yield name, mode, data[start:start+size]


def pack(output, profile, arms, mesh, mesh_sha, assets):
    sys.path.insert(0, str(GUEST))
    from pack_gnu import add

    base = BASE.read_bytes()
    overlay = bytearray()
    directories = {name for name, mode, _ in members(base) if stat.S_ISDIR(mode)}
    hashes = {}

    def add_file(name, data, mode=0o644):
        for parent in reversed(list(Path(name).parents)[:-1]):
            directory = parent.as_posix()
            if directory not in directories:
                add(overlay, directory, stat.S_IFDIR | 0o755)
                directories.add(directory)
        add(overlay, name, stat.S_IFREG | mode, data)
        hashes[name] = digest(data)

    def copy(name, path, mode=0o644):
        add_file(name, path.read_bytes(), mode)

    mesh_data = mesh.read_bytes()
    assert digest(mesh_data) == mesh_sha, "Mesh release ELF differs from the specified digest"
    add_file("proof/mesh-current", mesh_data, 0o755)
    copy("init", GUEST / "stock-jobs-smoke/init", 0o755)
    copy("proof/jobs-parity.py", ROOT / "scripts/jobs-parity.py")
    copy("proof/jobs-parity-profiles.json", ROOT / "scripts/jobs-parity-profiles.json")
    for file in ("trial.py", "fence.sql", "application.sql", "fixture.exs", "proof_transport.py", "run.sh"):
        copy(f"proof/{file}", SOURCE / file)
    copy("proof/oban", SOURCE / "oban.sh", 0o755)

    original = subprocess.check_output(["git", "-C", str(ROOT), "show",
        f"{OBAN_REVISION}:scripts/fixtures/jobs-parity-oban/lib/jobs_parity_oban.ex"]).decode()
    tag = '  def run([action]) when action in ["migrate", "enqueue", "run"] do'
    assert original.count(tag) == 1
    broker = (SOURCE / "oban-session.ex.in").read_text()
    add_file("proof/oban-session.ex", original.replace(tag, broker + "\n" + tag).encode())

    beam = assets / "beam/usr/local/lib/erlang"
    elixir = Path("/opt/homebrew/Cellar/elixir/1.20.4/lib/elixir/lib")
    deps = SIBLING / "build/oban/build/lib"
    for root, target, selected in (
        (beam, "opt/otp", (path for path in beam.rglob("*") if path.is_file()
                          and {"ebin", "priv", "bin", "releases"}.intersection(path.parts))),
        (elixir, "opt/elixir", elixir.glob("*/ebin/*")),
        (deps, "opt/fixtures", (path for path in deps.glob("*/ebin/*")
                                 if path.parent.parent.name != "jobs_parity_oban")),
        (assets / "oban-build", "opt/fixtures/jobs_parity_oban", (assets / "oban-build/ebin").glob("*")),
    ):
        files = sorted(selected)
        assert files, root
        for path in files:
            copy(f"{target}/{path.relative_to(root)}", path,
                 0o755 if path.stat().st_mode & 0o111 else 0o644)
    for library in ("libstdc++.so.6", "libtinfo.so.6"):
        copy(f"lib/aarch64-linux-gnu/{library}", assets / "beam/usr/lib/aarch64-linux-gnu" / library)

    config = {"profile": profile, "arms": arms, "hashes": hashes, "base_cpio_sha256": digest(base),
              "kernel_sha256": digest((assets / "kernel.Image").read_bytes()),
              "full_input_warmup": profile == "large", "oban_revision": OBAN_REVISION}
    add_file("proof/config.json", json.dumps(config).encode())
    add(overlay, "TRAILER!!!", 0)
    with output.open("xb") as stream:
        stream.write(base)
        stream.write(b"\0" * (-len(base) % 4))
        stream.write(overlay)
