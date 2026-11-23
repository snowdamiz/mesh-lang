#!/usr/bin/env python3
"""Freeze adapter bytes and the unaltered comparison lanes."""

from copy import deepcopy
import gzip
import json
from pathlib import Path
import sys
import tarfile

from assemble import BASE, LANE, NATIVE, ROOT, digest, read, write

sys.path.append(str(ROOT / "tools/durable-jobs-bench"))
from interleave import validate

CONTROL_LABELS = {
    "external": ("mesh-direct-external", "oban-external", "river-external", "apalis-external"),
    "default": ("mesh-direct-file", "oban", "river", "apalis"),
}


def configuration(policy, overlay, files):
    donor = read("symmetric-external.json" if policy == "external" else "direct-file.json")
    v1 = read("v1-native.json")
    if donor["host"] != v1["host"] or donor["cpus"] != [0, 1]:
        raise ValueError("comparison host or affinity changed")
    pins = json.loads((LANE / "source-retry" / NATIVE / "assembly-pins.json").read_text())
    ordinary = deepcopy(v1["lanes"]["ordinary"])
    engine = ordinary["inventory"]["engines"]["mesh"]
    engine["argv"] = ["/usr/bin/env", f"ORDINARY_LARGE_STORAGE={policy}",
                      f"NATIVE_JOBS_ARTIFACTS={pins['runner_artifacts']}",
                      f"{overlay}/{NATIVE}/run", "mesh"]
    engine["verified_files"].update(pins["dependencies"])
    engine["verified_files"].update({f"{overlay}/{path}": sha for path, sha in files.items()})
    comparison = dict(contract="ordinary-trusted-shared-process", profiles=["large"],
                      cpus=[0, 1], clock_ticks=donor["clock_ticks"], host=donor["host"],
                      description=(f"Large workload with {policy} payload storage. Ordinary V1 and "
                                   "protected Mesh use the same pinned direct-file broker. "
                                   "Peer launch commands and inventories come from the existing policy series."),
                      lanes={"ordinary": ordinary, **{
                          label: donor["lanes"][label] for label in CONTROL_LABELS[policy]
                      }})
    validate(comparison)
    return comparison


def freeze(out):
    out.mkdir()
    source = LANE / "source-retry"
    files = sorted(path for path in source.rglob("*") if path.is_file())
    archive = out / "overlay.tar.gz"
    with archive.open("xb") as stream, gzip.GzipFile(filename="", fileobj=stream, mode="wb", mtime=0) as zipped:
        with tarfile.open(fileobj=zipped, mode="w") as bundle:
            for path in files:
                member = bundle.gettarinfo(str(path), path.relative_to(source).as_posix())
                member.uid = member.gid = member.mtime = 0
                member.uname = member.gname = ""
                member.mode = 0o755 if path.name == "run" else 0o644
                with path.open("rb") as payload:
                    bundle.addfile(member, payload)
    archive_sha = digest(archive)
    overlay = f"{BASE}/source-ordinary-trusted-large-{archive_sha[:16]}"
    sources = {path.relative_to(source).as_posix(): digest(path) for path in files}
    for policy in CONTROL_LABELS:
        write(out / f"{policy}.json", configuration(policy, overlay, sources))
    write(out / "freeze.json", dict(
        archive_sha256=archive_sha, native_overlay=overlay, sources=sources,
        inventories={policy: digest(out / f"{policy}.json") for policy in CONTROL_LABELS},
        upstream={p.name: digest(p) for p in sorted((LANE / "inputs").iterdir())},
        drivers={p.name: digest(p) for p in sorted((LANE / "replay-retry").iterdir()) if p.is_file()},
    ))


def verify(out):
    proof = json.loads((out / "freeze.json").read_text())
    if digest(out / "overlay.tar.gz") != proof["archive_sha256"]:
        raise ValueError("assembly overlay changed")
    with tarfile.open(out / "overlay.tar.gz") as archive:
        archived = {}
        for member in archive:
            if not member.isfile() or member.name in archived:
                raise ValueError("unexpected overlay archive entry")
            import hashlib
            archived[member.name] = hashlib.file_digest(archive.extractfile(member), "sha256").hexdigest()
    if archived != proof["sources"]:
        raise ValueError("overlay member hashes changed")
    for dirname, field in (("inputs", "upstream"), ("replay-retry", "drivers")):
        for name, sha in proof[field].items():
            if digest(LANE / dirname / name) != sha:
                raise ValueError(f"frozen {dirname} input changed: {name}")
    for policy, sha in proof["inventories"].items():
        path = out / f"{policy}.json"
        if digest(path) != sha:
            raise ValueError(f"frozen {policy} inventory changed")
        expected = configuration(policy, proof["native_overlay"], archived)
        if json.loads(path.read_text()) != expected:
            raise ValueError("comparison slots differ from the frozen upstream slots")
    return proof


if __name__ == "__main__":
    action, dest = sys.argv[1:]
    if action == "freeze":
        freeze(Path(dest).resolve())
    elif action == "verify":
        print(json.dumps(verify(Path(dest).resolve()), indent=2, sort_keys=True))
    else:
        raise SystemExit("use freeze or verify")
