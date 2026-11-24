#!/usr/bin/env python3
"""Link the production-only patch and replace only the candidate broker pin."""
from copy import deepcopy
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

from materialize import main as materialize
from rotation import digest, replay, write

HERE = Path(__file__).resolve().parent
BASE = Path("/var/lib/durable-jobs-bench/native")
ABI = BASE / "builds/mesh-stock-3828f20d/abi"
DIRECT = BASE / "builds/mesh-direct-producer-v3"
BULK = BASE / "builds/bulk-enqueue-3b09dc3d0e23-ordinary-v2"


def load(path):
    return json.loads(path.read_text())


def check(path, sha):
    assert digest(path) == sha, path


def link(profile, attempt, component_result):
    assert replay(component_result)["profiles"][profile]["advance_to_broker_trial"]
    assert (HERE / "SETUP-PASS").is_file()
    check(ABI / "meshc", "e023d5944764878969ca40b7edb59c779498a435b44d308776076f4c7f2e8c19")
    work = HERE / f"broker-{profile}-{attempt}"
    work.mkdir()
    scratch = work / "tmp"
    scratch.mkdir()
    source = work / "source"
    shutil.copytree(HERE / f"tree-{profile}", source)
    identity = load(HERE / "candidate-identities.json")[profile]
    store = source / "compiler/mesh-rt/src/jobs/store.rs"
    check(store, identity["stock_store_sha256"])
    materialize(source, "broker", work / "patch")
    check(work / "patch", identity["production_patch_sha256"])
    check(store, identity["candidate_store_sha256"])
    assert store.read_bytes() == (HERE / f"candidate/{profile}/store.rs").read_bytes()
    cargo_out = work / "cargo"
    env = dict(os.environ, CARGO_TARGET_DIR=str(cargo_out), TMPDIR=str(scratch),
               CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=str(source / "scripts/ci-x86_64-linux-gnu-clang"))
    env.pop("RUSTFLAGS", None)
    env.pop("CARGO_ENCODED_RUSTFLAGS", None)
    with (work / "runtime-build.log").open("xb") as log:
        subprocess.run(["cargo", "build", "--release", "--offline", "--locked", "-p", "mesh-rt"],
                       cwd=source, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
    runtime = cargo_out / "release/libmesh_rt.a"
    runtime_sha = digest(runtime)
    product = BASE / "builds" / f"enqueue-spec-preparation-{profile}-{attempt}"
    product.mkdir()
    env["MESH_RT_LIB_PATH"] = str(runtime)
    launch_files = {}
    if profile == "short":
        stock = load(BULK / "mesh.json")
        check(BULK / "mesh.json", "7f228c192630bb6d95cbf9e06c5e0132931f429c1cfa07391bb28cb6fdec8911")
        executable = product / "mesh"
        project = product / "mesh-source"
        project.mkdir()
        for filename, sha in stock["files"].items():
            file = BULK / filename
            check(file, sha)
            if filename != "mesh":
                shutil.copy2(file, project / filename)
        assert (project / "main.mpl").read_bytes() == (HERE / "short-broker.mpl").read_bytes()
    else:
        check(DIRECT / "mesh.json", "2469a1c32dc318c52913d4fde9af1cfb15f0d52b4a6ddd7462d54a8dae72278f")
        stock = load(DIRECT / "mesh.json")
        for filename, sha in stock["files"].items():
            check(DIRECT / filename, sha)
        executable = product / "broker"
        project = product / "broker-source"
        shutil.copytree(DIRECT / "broker-source", project)
        assert (project / "main.mpl").read_bytes() == (HERE / "large-broker.mpl").read_bytes()
    check(project / "main.mpl", identity["broker_source_sha256"])
    with (work / "mesh-link.log").open("xb") as log:
        subprocess.run([ABI / "meshc", "build", project, "--opt-level", "2", "--output", executable],
                       env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
    if profile == "large":
        plan = load(HERE / "large-launch-pins.json")
        launch = BASE / f"enqueue-spec-preparation-large-{attempt}"
        launch.mkdir()
        assembly = Path(plan["launcher"]).parent / "assembly-pins.json"
        check(assembly, plan["assembly_pins_sha256"])
        base_pins = load(assembly)
        for path, sha in base_pins["dependencies"].items():
            check(Path(path), sha)
        plan.update(artifact=str(product), deploy=str(launch), source_sha256=digest(project / "main.mpl"),
                    runtime_sha256=runtime_sha)
        for name, sha in plan["workmem_members"].items():
            check(Path(plan["stock_entry"]).parent / name, sha)
        manifest = dict(stock, runtime_sha256=runtime_sha, files=dict(stock["files"], broker=digest(executable)))
        for name, sha in manifest["files"].items():
            check(product / name, sha)
        manifest_path = product / "mesh.json"
        write(manifest_path, manifest)
        switches = dict(broker_path=str(executable), broker_source_sha256=digest(project / "main.mpl"),
                        broker_sha256=digest(executable), broker_manifest_sha256=digest(manifest_path))
        assert switches.keys() == set(plan["broker_fields"])
        write(launch / "launch-pins.json", plan)
        write(launch / "broker-pins.json", dict(base_pins, **switches))
        shutil.copy2(HERE / "large-run", launch / "mesh-run")
        shutil.copy2(HERE / "large-entry.py", launch / "entry.py")
        launch_files = {str(path): digest(path) for path in [*launch.iterdir(), manifest_path, project / "main.mpl"]}
    launch_files[str(executable)] = digest(executable)
    output = {"profile": profile, "broker": str(executable), "runtime_sha256": runtime_sha,
              "source_identity": identity, "launch_files": launch_files}
    write(work / "link.json", output)
    write(work / f"{profile}-trial.json", assemble(profile, output))
    shutil.rmtree(cargo_out)
    shutil.rmtree(scratch)
    print(work)


def assemble(profile, image):
    config = load(HERE / f"{profile}-reference.json")
    stock = "bulk-d2" if profile == "short" else "ordinary-a"
    ports = ["oban", "river", "apalis"] if profile == "short" else ["oban-external", "river-external", "apalis-external"]
    stock_lane = config["lanes"][stock]
    candidate = deepcopy(stock_lane)
    spec = candidate["inventory"]["engines"]["mesh"]
    broker = image["broker"]
    sha = image["launch_files"][broker]
    if profile == "short":
        assert spec["argv"][-3:-1] == [str(BULK / "mesh"), "55d7000d562a6ad395f5e64e33e0e973fac0084ed05ad6c30a33db2a3fc6303a"]
        spec["argv"][-3:-1] = [broker, sha]
    else:
        launcher, = [p for p in image["launch_files"] if p.endswith("/mesh-run")]
        assert spec["argv"][-2:] == ["16", "mesh"]
        spec["argv"][-4] = launcher
    assert set(spec["verified_files"]).isdisjoint(image["launch_files"])
    spec["verified_files"].update(image["launch_files"])
    config["description"] = f"Broker-only enqueue spec preparation. {profile} paired native durable ledger."
    config["lanes"] = {"stock-a": stock_lane, ports[0]: config["lanes"][ports[0]],
                       "candidate": candidate, ports[1]: config["lanes"][ports[1]],
                       "stock-b": deepcopy(stock_lane), ports[2]: config["lanes"][ports[2]]}
    return config


if __name__ == "__main__":
    profile, attempt, evidence = sys.argv[1:]
    link(profile, attempt, Path(evidence))
