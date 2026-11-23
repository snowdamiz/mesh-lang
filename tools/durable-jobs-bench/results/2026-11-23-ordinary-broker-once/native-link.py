#!/usr/bin/env python3
"""Link only one broker with the resident direct-producer compiler and runtime."""

import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def check(path, expected):
    if digest(path) != expected:
        raise ValueError(f"changed native input: {path}")


def read_manifest(path):
    return json.loads(path.read_text())


def write(path, doc):
    path.write_text(json.dumps(doc, sort_keys=True, indent=2) + "\n")


def main(source, overlay):
    plan = json.loads((source / "launch-pins.json").read_text())
    direct = Path(plan["stock_broker"])
    stock = json.loads((direct / "mesh.json").read_text())
    check(direct / "mesh.json", plan["stock_broker_manifest_sha256"])
    for name, expected in stock["files"].items():
        check(direct / name, expected)
    for name in ("mesh-run", "entry.py", "workmem.py"):
        path = Path(plan["stock_entry"]).parent / name
        check(path, plan["workmem_members"][name])
    if digest(source / "mesh-run") != plan["workmem_members"]["mesh-run"]:
        raise ValueError("candidate launcher must match the original shell")
    check(source / "broker.mpl", plan["source_sha256"])
    for name in ("compiler", "runtime"):
        if stock[f"{name}_sha256"] != plan[f"{name}_sha256"]:
            raise ValueError(f"unexpected broker {name} pin")
    toolchain = next((path for item in plan["toolchains"] if (path := Path(item)).is_dir()
                      and (path / "meshc").is_file()
                      and digest(path / "meshc") == plan["compiler_sha256"]
                      and (path / "libmesh_rt.a").is_file()
                      and digest(path / "libmesh_rt.a") == plan["runtime_sha256"]), None)
    if toolchain is None:
        raise ValueError("no matching resident compiler and runtime")
    adapter = Path(plan["launcher"]).parent
    pins_path = adapter / "assembly-pins.json"
    check(pins_path, plan["assembly_pins_sha256"])
    original = json.loads(pins_path.read_text())
    for path, expected in original["dependencies"].items():
        check(Path(path), expected)

    artifact = Path(plan["artifact"])
    if not artifact.exists():
        staged = source / "artifact"
        staged.mkdir()
        project = staged / "broker-source"
        shutil.copytree(direct / "broker-source", project)
        shutil.copyfile(source / "broker.mpl", project / "main.mpl")
        subprocess.run([toolchain / "meshc", "build", project, "--opt-level", "2",
                        "--output", staged / "broker"],
                       env=dict(os.environ, MESH_RT_LIB_PATH=str(toolchain / "libmesh_rt.a")),
                       stdout=sys.stderr, check=True)
        files = {name: digest(staged / name) for name in stock["files"]}
        write(staged / "mesh.json", dict(stock, files=files))
        staged.rename(artifact)
    broker = artifact / "broker"
    project = artifact / "broker-source"
    files = {name: digest(artifact / name) for name in stock["files"]}
    manifest = dict(stock, files=files)
    if (read_manifest(artifact / "mesh.json") != manifest
            or files["broker-source/main.mpl"] != plan["source_sha256"]
            or {name for name in stock["files"] if stock["files"][name] != files[name]}
                != {"broker-source/main.mpl", "broker"}):
        raise ValueError("the broker link altered more than its Mesh source and executable")

    candidate = dict(original, broker_path=str(broker), broker_source_sha256=plan["source_sha256"],
                     broker_sha256=files["broker"], broker_manifest_sha256=digest(artifact / "mesh.json"))
    overlay_inputs = ("mesh-run", "entry.py", "launch-pins.json")
    if not overlay.exists():
        staged = source / "overlay"
        staged.mkdir()
        for name in overlay_inputs:
            shutil.copy2(source / name, staged / name)
        write(staged / "broker-pins.json", candidate)
        staged.rename(overlay)
    for name in overlay_inputs:
        check(overlay / name, digest(source / name))
    if read_manifest(overlay / "broker-pins.json") != candidate:
        raise ValueError("the installed candidate broker pins changed")
    launch_files = {str(path): digest(path) for path in
                    [broker, artifact / "mesh.json", project / "main.mpl", *sorted(overlay.iterdir())]}
    print(json.dumps(dict(launch_files=launch_files, manifest=manifest, pins=candidate), sort_keys=True))


if __name__ == "__main__":
    main(*(Path(arg) for arg in sys.argv[1:]))
