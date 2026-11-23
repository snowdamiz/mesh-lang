#!/usr/bin/env python3
"""Run the sealed incremental link path on Darwin and validate native lane wiring offline."""

from copy import deepcopy
import json
import os
from pathlib import Path
import subprocess
import sys
import tarfile

import build
import native


def verify(out, binaries):
    out.mkdir()
    transfer = out / "transfer"
    transfer.mkdir()
    freeze = build.HERE / "native-freeze"
    sha = build.digest(freeze / "freeze.json")
    _, remote = native.verify(freeze, sha)
    with tarfile.open(freeze / "overlay.tar") as archive:
        archive.extractall(transfer, filter="data")

    workmem = out / "workmem"
    workmem.mkdir()
    with tarfile.open(native.BASE_FREEZE / "overlay.tar") as archive:
        archive.extractall(workmem, filter="data")
    adapter = out / "adapter"
    adapter.mkdir()
    stock = native.read(native.PINS_PATH)
    dependencies = {str(build.ASSETS / "broker"): build.digest(build.ASSETS / "broker")}
    native.write(adapter / "assembly-pins.json", dict(stock, dependencies=dependencies))

    manifest = native.read(build.ASSETS / "mesh.json")
    plan = dict(remote, launcher=str(adapter / "arm.py"),
                stock_entry=str(workmem / "entry.py"), stock_broker=str(build.ASSETS),
                artifact=str(out / "artifact"), toolchains=[str(build.DIRECT)],
                stock_broker_manifest_sha256=build.digest(build.ASSETS / "mesh.json"),
                compiler_sha256=manifest["compiler_sha256"], runtime_sha256=manifest["runtime_sha256"],
                assembly_pins_sha256=build.digest(adapter / "assembly-pins.json"))
    local_launch_pins = transfer / "launch-pins.json"
    local_launch_pins.unlink()
    native.write(local_launch_pins, plan)
    argv = [sys.executable, "-B", transfer / "link.py", transfer, out / "overlay"]
    with (out / "links.log").open("xb") as stderr:
        first = subprocess.check_output(argv, stderr=stderr, env=dict(os.environ, TMPDIR=str(build.STATE / "tmp")))
        second = subprocess.check_output(argv, stderr=stderr, env=dict(os.environ, TMPDIR=str(build.STATE / "tmp")))
    assert first == second
    local = json.loads(first)
    measured = native.read(binaries / "pins.json")
    assert build.check(binaries / "once", measured["once"])
    assert local["manifest"]["compiler_sha256"] == measured["compiler"]
    assert local["manifest"]["runtime_sha256"] == measured["runtime"]
    assert local["manifest"]["files"]["broker-source/main.mpl"] == remote["source_sha256"]

    # Use the compiled local executable digest as a placeholder to exercise
    # the remote inventory reducer. This does not identify a Linux build.
    broker_fields = dict(broker_path=f'{remote["artifact"]}/broker',
                         broker_source_sha256=remote["source_sha256"],
                         broker_sha256=local["pins"]["broker_sha256"],
                         broker_manifest_sha256=local["pins"]["broker_manifest_sha256"])
    paths = {f'{remote["artifact"]}/{p}': local["launch_files"][str(out / "artifact" / p)]
             for p in ("broker", "mesh.json", "broker-source/main.mpl")}
    paths.update({f'{remote["deploy"]}/{p}': local["launch_files"][str(out / "overlay" / p)]
                  for p in ("mesh-run", "entry.py", "launch-pins.json", "broker-pins.json")})
    selected = native.configuration(remote, dict(launch_files=paths, pins=dict(stock, **broker_fields),
        manifest=dict(native.read(build.HERE / "pins/direct-native.json"), files={
            "broker-source/main.mpl": broker_fields["broker_source_sha256"], "broker": broker_fields["broker_sha256"]})))
    _, proof = native.workmem_dependencies()
    proof.stock_matrix.validate(selected)
    base = native.read(build.HERE / "native-base.json")
    standard = base["lanes"]["ordinary"]
    assert all(selected["lanes"][name] == standard for name in native.CONTROLS)
    assert all(selected["lanes"][name] == base["lanes"][name] for name in native.PEERS)
    candidate = deepcopy(selected["lanes"][native.CANDIDATE])
    mesh = candidate["inventory"]["engines"]["mesh"]
    mesh["argv"][-4] = standard["inventory"]["engines"]["mesh"]["argv"][-4]
    for path in paths:
        assert mesh["verified_files"].pop(path) == paths[path]
    assert candidate == standard
    rotations = list(proof.stock_matrix.plan(selected["profiles"], list(selected["lanes"])))
    assert len(rotations) == 5 and all(block["phases"] == ["warmup", "measured"] for block in rotations)
    return dict(freeze_sha256=sha, darwin_link_sha256=local["pins"]["broker_sha256"],
                measured_darwin_sha256=measured["once"],
                identical_on_resume=True, matched_controls=list(native.CONTROLS),
                unchanged_peers=list(native.PEERS), native_schedule=rotations)


if __name__ == "__main__":
    print(json.dumps(verify(*(Path(arg).resolve() for arg in sys.argv[1:])), indent=2, sort_keys=True))
