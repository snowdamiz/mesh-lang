#!/usr/bin/env python3
"""Freeze the two exact runtime source trees, replay programs and captured frames."""
import gzip
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile

from materialize import candidate
from offline import raw
from rotation import digest, verify_frames

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
WORK = ROOT / "target/enqueue-spec-preparation"
SHORT_SOURCE = HERE.parent / "2026-11-23-set-based-enqueue/native/bulk-enqueue-ce5e551d079e.tar.gz"
LARGE_SHA = "3828f20d2d6beb683868cce33e6dd1958365914b"
SHORT_SHA256 = "542925005370032c71db1a2aa488c2d35d16f14f9fdc8f54ee0afbb81f80fc82"


def stage(destination):
    destination.mkdir()
    short = destination / "tree-short"
    large = destination / "tree-large"
    assert digest(SHORT_SOURCE) == SHORT_SHA256
    with tarfile.open(SHORT_SOURCE) as archive:
        archive.extractall(destination, (m for m in archive.getmembers()
                                         if m.name.startswith("ordinary-v2/")), filter="data")
    (destination / "ordinary-v2").rename(short)
    large.mkdir()
    proc = subprocess.Popen(["git", "archive", LARGE_SHA, ".cargo", "Cargo.lock", "Cargo.toml",
                             "compiler", "scripts/ci-x86_64-linux-gnu-clang"], cwd=ROOT,
                            stdout=subprocess.PIPE)
    with tarfile.open(fileobj=proc.stdout, mode="r|") as archive:
        archive.extractall(large, filter="data")
    assert proc.wait() == 0
    for tree, sha in [(short, "c41b0c8e32b9861b00f46d0cbc81be5b462dbb6294da99b46f67a2eef7e7913f"),
                      (large, "024a38917f66849f31710f4656da403ab105e309b08043e0f0340a1f96f848aa")]:
        assert digest(tree / "compiler/mesh-rt/src/jobs/store.rs") == sha

    portable = ["broker.py", "capture.py", "fixture-generator.py", "fixture-profiles.json", "materialize.py",
                "native.sh", "probe.rs", "rotation.py", "wire.rs", "wire-check.py",
                "large-launch-pins.json", "large-entry.py", "large-run"]
    for name in portable:
        shutil.copy2(HERE / name, destination / name)
    identities = {}
    for profile in ("short", "large"):
        for suffix in ("broker.mpl", "capture.json", "reference.json"):
            shutil.copy2(HERE / f"{profile}-{suffix}", destination / f"{profile}-{suffix}")
        shutil.copy2(HERE / f"{profile}.patch", destination / f"{profile}.patch")
        store = destination / f"tree-{profile}/compiler/mesh-rt/src/jobs/store.rs"
        production = destination / f"candidate/{profile}/store.rs"
        production.parent.mkdir(parents=True)
        production.write_text(candidate(store.read_text()))
        identities[profile] = {
            "stock_store_sha256": digest(store), "candidate_store_sha256": digest(production),
            "production_patch_sha256": digest(destination / f"{profile}.patch"),
            "broker_source_sha256": digest(destination / f"{profile}-broker.mpl"),
        }
        shutil.copy2(WORK / f"{profile}-semantic-isolated.json", destination / f"{profile}-semantic.json")
        shutil.copy2(raw(profile), destination / f"{profile}-raw.tar.gz")
        shutil.copytree(WORK / f"capture-{profile}-02/frames", destination / "frames" / profile)
        verify_frames(destination, profile)

    (destination / "candidate-identities.json").write_text(json.dumps(identities, indent=2, sort_keys=True) + "\n")
    files = [p for p in sorted(destination.rglob("*")) if p.is_file()]
    (destination / "SHA256SUMS").write_text("".join(
        f"{digest(p)}  {p.relative_to(destination)}\n" for p in files))


def freeze(source, dest):
    with dest.open("xb") as payload, gzip.GzipFile(filename="", mode="wb", fileobj=payload, mtime=0) as packed:
        with tarfile.open(fileobj=packed, mode="w") as archive:
            for path in sorted(source.rglob("*")):
                member = archive.gettarinfo(str(path), arcname=str(path.relative_to(source)))
                member.mtime = 0
                member.uid = member.gid = 0
                member.uname = member.gname = ""
                if member.isfile():
                    with path.open("rb") as f:
                        archive.addfile(member, f)
                else:
                    archive.addfile(member)


if __name__ == "__main__":
    directory, output = (Path(arg).resolve() for arg in sys.argv[1:])
    stage(directory)
    freeze(directory, output)
    print(digest(output))
