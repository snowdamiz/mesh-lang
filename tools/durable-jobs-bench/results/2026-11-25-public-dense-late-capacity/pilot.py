"""Seal and independently replay the canonical same-image late-capacity pilot."""
import argparse
import hashlib
from io import BytesIO
import json
from pathlib import Path
import subprocess
import sys
import tarfile
from types import ModuleType

from qualification import BASE, replay as proof_replay
from replay import ELF, INPUT, PATH, PILOT, ROOT as CANONICAL, fetch, read

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[3]
SUITE = REPO / "tools/durable-jobs-bench"
ID = HERE.name
LANE = "bounded-grouped-hot"
ORDERS = ("A1", "reference", "B", "A2"), ("A2", "B", "reference", "A1")
TRIALS = [dict(group=i, order=list(order), phases=["warmup", "measured"], profile="noop")
          for i, order in enumerate(ORDERS, 1)]
FILES = ("run.py", "worker.py", "verify.py", "interleave.py")
sys.path.insert(0, str(SUITE))
import run


def canonical(path):
    return fetch(PILOT, f"{CANONICAL}/{path}")


def verified_dependencies():
    pins = json.loads(canonical("controller-packet.json"))["pins"]
    for name in FILES:
        path = SUITE / name
        assert run.digest(path) == pins[str(path.relative_to(REPO))]
    for name in ("stage/public-native-run", "stage/public_arm.py", "score.py",
                 "witness.py", "reference.inventory.json"):
        assert hashlib.sha256(canonical(name)).hexdigest() == pins[f"{CANONICAL}/{name}"]


def scorer():
    for name in ("witness", "score"):
        module = ModuleType(name)
        module.__file__ = str(REPO / CANONICAL / f"{name}.py")
        sys.modules[name] = module
        exec(compile(canonical(f"{name}.py"), module.__file__, "exec"), module.__dict__)
    return sys.modules["score"].score_arm


def history():
    raw = fetch(PILOT, f"{PATH}/blocks/01-noop/measured-B/measured-mesh/raw.tar.gz")
    assert hashlib.sha256(raw).hexdigest() == read("replay.json")["groups"]["1"]["measured-B"]["raw_sha256"]
    with tarfile.open(fileobj=BytesIO(raw)) as archive:
        build = json.load(archive.extractfile("adapter/build.json"))
        routes = json.load(archive.extractfile("adapter/public-grants.json"))
    assert build["version"] == BASE and build["files"]["mesh"] == ELF
    return build, routes


def remote(label):
    lane = "off" if label in ("A1", "A2") else "on" if label == "B" else "reference"
    return run.REMOTE / ID / lane


def staging(label, qualified, directory):
    spec = json.loads(canonical("reference.inventory.json"))
    mesh = spec["engines"]["mesh"]
    launcher = canonical("stage/public-native-run")
    arm = canonical("stage/public_arm.py")
    old = next(str(Path(arg).parent) for arg in mesh["argv"] if arg.endswith("/public-native-run"))
    target = remote(label)
    mesh["argv"] = [arg.replace(old, str(target), 1) if arg.startswith(old + "/")
                    else LANE if arg == "checked-individual" else arg for arg in mesh["argv"]]
    mesh["verified_files"] = {name: digest for name, digest in mesh["verified_files"].items()
                              if not name.startswith(old + "/")}
    if label != "reference":
        build = qualified["manifest"]
        assert build["files"]["mesh"] == qualified["elf_sha256"]
        manifest = qualified["manifest_sha256"]
        original = "/var/lib/durable-jobs-bench/native"
        previous = "public-dense-fixed8-jobs-ec82fb2eefcb"
        attempt = qualified["packet"].removesuffix("-setup")
        launcher = launcher.replace(previous.encode(), attempt.encode())
        launcher = launcher.replace(ELF.encode(), qualified["elf_sha256"].encode())
        launcher = launcher.replace(mesh["artifact_sha256"].encode(), manifest.encode())
        launcher = launcher.replace(b"PRODUCT_TOPOLOGY=stock PRODUCT_STORAGE=default",
                                    b"JOBS_LATE_CAPACITY=" + (b"1" if label == "B" else b"0") +
                                    b" PRODUCT_TOPOLOGY=stock PRODUCT_STORAGE=default")
        files = {}
        for filename, digest in mesh["verified_files"].items():
            if filename.startswith(f"{original}/builds/{previous}/"):
                leaf = Path(filename).name
                digest = manifest if leaf == "mesh.json" else build["files"][leaf]
            files[filename.replace(previous, attempt)] = digest
        mesh.update(artifact_sha256=manifest, version=build["version"], verified_files=files)
    directory.mkdir(parents=True, exist_ok=False)
    paths = (directory / "public-native-run", directory / "public_arm.py", SUITE / "worker.py")
    paths[0].write_bytes(launcher)
    paths[1].write_bytes(arm)
    subprocess.run(["bash", "-n", paths[0]], check=True)
    for path in paths:
        mesh["verified_files"][str(target / path.name)] = run.digest(path)
    mesh["evidence_globs"].append("canonical-ready.json")
    run.check_inventory(spec)
    run.write_json(directory / "inventory.json", spec)
    return spec


def plan():
    verified_dependencies()
    return dict(id=ID, schedule=TRIALS, minutes=14, input_sha256=INPUT, reference_elf=ELF,
                source_pilot=PILOT, no_build=True, cpus=[0, 1], runners=8, slots_each=8,
                warm_app_sessions=32, scheduler_workers_each=2, physical_connection_cap=96,
                pins={name: run.digest(HERE / name)
                      for name in ("pilot.py", "controller.py", "qualification.py", "native-controls.sh", "candidate.patch.gz",
                                   "replay.py", "peer_price.py", "offline_check.py",
                                   "reference-arm.json", "capsule-dependencies.json", "setup-seal.json")})


def bind(source, setup, destination):
    verified_dependencies()
    assert json.loads((HERE / "pilot-plan.json").read_text()) == plan()
    proof = setup / "evidence"
    release = run.read_json(proof / "controller-release.json")
    assert release["request_sha256"] == run.digest(setup / "jobs.json")
    qualified = proof_replay(source, setup, proof / "controller-release.json", proof / "raw.tar.gz")
    assert run.digest(setup / "jobs.json") == json.loads((HERE / "setup-seal.json").read_text())["request_sha256"]
    destination.mkdir(parents=True, exist_ok=False)
    for label in ("A1", "reference", "B"):
        staging(label, qualified, destination / "staging" / label)
    assert remote("A1") == remote("A2")
    run.write_json(destination / "qualified.json", qualified)
    paths = [path for path in destination.rglob("*") if path.is_file()]
    run.write_json(destination / "controller-packet.json", dict(
        **plan(), proof_request_sha256=run.digest(setup / "jobs.json"),
        timing_id=ID + "-pilot",
        setup_release_sha256=qualified["release_sha256"],
        binding={str(p.relative_to(destination)): run.digest(p) for p in paths},
    ))


def result(folder, label, phase, inventory, qualified, score_arm):
    original, routes = history()
    build = original if label == "reference" else qualified["manifest"]
    score = score_arm(folder, phase, LANE, inventory, build, routes)
    with tarfile.open(folder / "raw.tar.gz") as archive:
        policy = json.load(archive.extractfile("adapter/queue-policy.json"))
        claim, = [q for q in policy["queue_statements"] if "FROM mesh_jobs.claim_batch_bounded_v1" in q["query"]]
        rows = [json.loads(archive.extractfile(f"adapter/runner-{index}.log").read().splitlines()[-1])
                for index in range(8)]
    assert all(("late_capacity" in row) == (label == "B") for row in rows)
    topups = {key: sum(row["late_capacity"][key] for row in rows) for key in ("samples", "topups", "added")} \
        if label == "B" else None
    if topups:
        assert 0 <= topups["topups"] <= topups["samples"] <= claim["calls"]
        assert topups["added"] >= topups["topups"]
    return dict(score, late_capacity=topups, bounded_calls=claim["calls"],
                positive_grants=claim["calls"] - (claim["rows"] - 1024))


def gate(rows):
    a, b, ref, duplicate = (rows["measured-" + label] for label in ("A1", "B", "reference", "A2"))
    decisions = {}
    for key in ("submit_us", "drain_us", "positive_grants", "bounded_calls", "last_effect_us",
                "runner_cpu_ms", "pg_cpu_ms", "host_cpu_ms"):
        faster, slower = sorted((a[key], duplicate[key]))
        spread = slower - faster
        if key in ("submit_us", "drain_us", "positive_grants"):
            passed = min(faster, ref[key]) - b[key] > max(0.05 * min(faster, ref[key]), spread)
        elif key in ("bounded_calls", "last_effect_us"):
            passed = b[key] < min(faster, ref[key])
        else:
            passed = b[key] <= max(slower, ref[key])
        decisions[key] = dict(passed=passed, duplicates=[a[key], duplicate[key]], bridge=ref[key],
                              candidate=b[key], duplicate_spread=spread)
    confirmed_topups = b["late_capacity"]["topups"] > 0
    return dict(passed=confirmed_topups and all(row["passed"] for row in decisions.values()),
                confirmed_topups=confirmed_topups, checks=decisions)


def checked_binding(destination):
    packet = run.read_json(destination / "controller-packet.json")
    assert {key: packet[key] for key in plan()} == plan()
    for path, digest in packet["binding"].items():
        assert run.digest(destination / path) == digest
    return packet, run.read_json(destination / "qualified.json")


def visit(authority, destination, dispatch=None):
    packet, qualified = checked_binding(destination)
    score_arm = scorer()
    groups, expected = {}, set()
    for block in packet["schedule"]:
        out = authority / "blocks" / run.block_prefix(block)
        if dispatch:
            out.mkdir(parents=True, exist_ok=False)
            run.write_json(out / "order.json", block)
        assert run.read_json(out / "order.json") == block
        rows = {}
        for label in block["order"]:
            stage = destination / "staging" / ("A1" if label == "A2" else label)
            inventory = run.read_json(stage / "inventory.json")
            for phase in block["phases"]:
                folder = out / f"{phase}-{label}" / f"{phase}-mesh"
                if dispatch:
                    dispatch(folder, label, block, phase)
                expected.add(folder / "raw.tar.gz")
                rows[f"{phase}-{label}"] = result(folder, label, phase, inventory, qualified, score_arm)
        if dispatch:
            run.write_json(out / "reconciled.json", rows)
        assert run.read_json(out / "reconciled.json") == json.loads(json.dumps(rows))
        groups[str(block["group"])] = rows
    assert expected == set(authority.rglob("raw.tar.gz"))
    return dict(groups=groups, gates={number: gate(rows) for number, rows in groups.items()})


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("seal-plan", "check-plan", "bind", "replay"))
    parser.add_argument("paths", nargs="*", type=Path)
    args = parser.parse_args()
    if args.command == "seal-plan":
        run.write_json(HERE / "pilot-plan.json", plan())
    elif args.command == "check-plan":
        assert run.read_json(HERE / "pilot-plan.json") == plan()
        print(run.digest(HERE / "pilot-plan.json"))
    elif args.command == "bind":
        bind(*args.paths)
    else:
        destination, authority = args.paths
        release = run.read_json(authority / "controller-release.json")
        assert release["exit_status"] == 0 and release["id"] == ID + "-pilot"
        assert release["request_sha256"] == run.digest(destination / "controller-packet.json")
        replay = visit(authority, destination)
        assert json.loads(json.dumps(replay)) == run.read_json(authority / "scored-before-release.json")
        print(json.dumps(replay, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
