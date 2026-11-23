#!/usr/bin/env python3
"""Seal a broker-only install and reuse the controller's 16 MiB matrix."""

import argparse
from copy import deepcopy
import io
import json
from pathlib import Path
import sys
import tarfile
from unittest.mock import patch
from uuid import uuid4

from build import HERE, ROOT, STATE, check, digest

SUITE = ROOT / "tools/durable-jobs-bench"
WORKMEM = STATE.with_name("ordinary-large-workmem")
BASE_FREEZE = WORKMEM / "freeze-02"
ASSEMBLY = STATE.with_name("ordinary-trusted-large-assembly")
PINS_PATH = ASSEMBLY / "source-retry/tools/durable-jobs-bench/native/assembly-pins.json"
DIRECT = HERE.parent / "2026-11-23-direct-producer-v3-setup"
SETUP = "ordinary-broker-once-setup"
TIMING = "ordinary-broker-once-matched"
CONTROLS = ("ordinary-a", "ordinary-b")
CANDIDATE = "ordinary-once"
PEERS = ("river-external", "apalis-external", "oban-external")
BASE = "/var/lib/durable-jobs-bench/native"
BROKER_FIELDS = ("broker_path", "broker_source_sha256", "broker_sha256", "broker_manifest_sha256")


def read(path):
    return json.loads(path.read_text())


def write(path, data):
    with path.open("x") as stream:
        json.dump(data, stream, indent=2, sort_keys=True)
        stream.write("\n")


def seal(out):
    original = read(BASE_FREEZE / "freeze.json")
    check(BASE_FREEZE / "freeze.json", "19a93a1cb12f5404d1abe1f7d94dfb50e84e8c9764cb6fefa8d84353436b595e")
    check(BASE_FREEZE / "large.json", original["series_sha256"])
    check(BASE_FREEZE / "overlay.tar", original["archive_sha256"])
    for name, sha in original["drivers"].items():
        check(ROOT / name, sha)
    config = read(BASE_FREEZE / "large.json")
    argv = config["lanes"]["ordinary"]["inventory"]["engines"]["mesh"]["argv"]
    assert argv[-2:] == ["16", "mesh"] and config["cpus"] == [0, 1]
    original_broker = read(DIRECT / "direct-manifest.json")
    pins = read(PINS_PATH)
    check(DIRECT / "direct-manifest.json", pins["broker_manifest_sha256"])
    check(HERE / "broker-control.mpl", pins["broker_source_sha256"])
    source = digest(HERE / "broker-once.mpl")
    identifier = source[:16]
    deploy = f"{BASE}/ordinary-broker-once-{identifier}"
    plan = dict(
        launcher=argv[-3], stock_entry=f'{original["install"]}/entry.py',
        stock_broker=str(Path(pins["broker_path"]).parent), artifact=f"{BASE}/builds/ordinary-broker-once-{identifier}",
        stock_broker_manifest_sha256=pins["broker_manifest_sha256"],
        workmem_members=original["members"], assembly_pins_sha256=digest(PINS_PATH),
        compiler_sha256=original_broker["compiler_sha256"], runtime_sha256=original_broker["runtime_sha256"],
        toolchains=[f"{BASE}/builds/mesh-stock-3828f20d/abi", f"{BASE}/cargo/release"],
        source_sha256=source, broker_fields=BROKER_FIELDS, deploy=deploy,
    )
    out.mkdir()
    with tarfile.open(BASE_FREEZE / "overlay.tar") as stock_bundle:
        shell = stock_bundle.extractfile("mesh-run").read()
    members = {"mesh-run": shell, "entry.py": (HERE / "native-entry.py").read_bytes(),
               "link.py": (HERE / "native-link.py").read_bytes(), "broker.mpl": (HERE / "broker-once.mpl").read_bytes(),
               "launch-pins.json": (json.dumps(plan, indent=2, sort_keys=True) + "\n").encode()}
    with tarfile.open(out / "overlay.tar", "x") as archive:
        for name, contents in sorted(members.items()):
            member = tarfile.TarInfo(name)
            member.size = len(contents)
            member.mode = 0o755 if name == "mesh-run" else 0o644
            archive.addfile(member, io.BytesIO(contents))
    write(out / "launch-plan.json", plan)
    dependencies = ("native.py", "native-link.py", "native-install.sh", "native-entry.py",
                    "broker-once.mpl", "broker-control.mpl", "native-base.json")
    extra = (WORKMEM / "freeze.py", WORKMEM / "proof.py", WORKMEM / "setup.py",
             WORKMEM / "overlay/workmem.py", ASSEMBLY / "inputs/storage-upstream.py",
             ASSEMBLY / "inputs/peer-storage-replay.py")
    check(HERE / "native-base.json", original["series_sha256"])
    write(out / "freeze.json", dict(bundle_sha256=digest(out / "overlay.tar"),
        plan_sha256=digest(out / "launch-plan.json"), drivers={
            str(path.relative_to(ROOT)): digest(path) for path in ([HERE / n for n in dependencies] + list(extra))},
        upstream_drivers=original["drivers"],
        workmem_freeze_sha256=digest(BASE_FREEZE / "freeze.json")))


def verify(freeze, sha):
    check(freeze / "freeze.json", sha)
    sealed = read(freeze / "freeze.json")
    check(freeze / "overlay.tar", sealed["bundle_sha256"])
    check(freeze / "launch-plan.json", sealed["plan_sha256"])
    for key in ("drivers", "upstream_drivers"):
        for name, digest_value in sealed[key].items():
            check(ROOT / name, digest_value)
    return sealed, read(freeze / "launch-plan.json")


def configuration(plan, link):
    base = read(HERE / "native-base.json")
    template = base["lanes"]["ordinary"]
    candidate = deepcopy(template)
    spec = candidate["inventory"]["engines"]["mesh"]
    spec["argv"][-4] = f'{plan["deploy"]}/mesh-run'
    links = link["launch_files"]
    expected_files = {f'{plan["artifact"]}/{suffix}' for suffix in ("broker", "mesh.json", "broker-source/main.mpl")}
    expected_files |= {f'{plan["deploy"]}/{suffix}' for suffix in ("mesh-run", "entry.py", "launch-pins.json", "broker-pins.json")}
    assert links.keys() == expected_files
    pins = link["pins"]
    assert pins["broker_path"] == f'{plan["artifact"]}/broker'
    manifest = link["manifest"]
    assert manifest["files"]["broker"] == pins["broker_sha256"] == links[pins["broker_path"]]
    assert pins["broker_manifest_sha256"] == links[f'{plan["artifact"]}/mesh.json']
    assert (plan["source_sha256"] == pins["broker_source_sha256"] == manifest["files"]["broker-source/main.mpl"]
            == links[f'{plan["artifact"]}/broker-source/main.mpl'])
    for name in ("compiler_sha256", "runtime_sha256"):
        assert manifest[name] == plan[name]
    stock = read(PINS_PATH)
    assert dict(stock, **{k: pins[k] for k in BROKER_FIELDS}) == pins
    spec["verified_files"].update(links)
    base["description"] = "Ordinary direct-file broker single encode against two identical stock broker controls. All 16 MiB, EXTERNAL, and two CPUs. Original runner and peers."
    base["lanes"] = {CONTROLS[0]: template, PEERS[0]: base["lanes"][PEERS[0]],
                     CANDIDATE: candidate, PEERS[1]: base["lanes"][PEERS[1]],
                     CONTROLS[1]: template, PEERS[2]: base["lanes"][PEERS[2]]}
    return base


def workmem_dependencies():
    sys.path[0:0] = [str(WORKMEM), str(SUITE)]
    import setup
    import proof
    return setup, proof


def verify_arm(proof, path, config, phase, label, link, score=None):
    proof.LABELS[label] = ("ordinary", 16) if label.startswith("ordinary") else (label, 16)
    ordinary = proof.stock_receipt.ordinary

    def selected(read, score, policy, pins, cpus):
        return ordinary(read, score, policy, link["pins"] if label == CANDIDATE else pins, cpus)

    with patch.object(proof.stock_receipt, "ordinary", selected):
        return proof.native(path, config, phase, label, score)


def setup(freeze, sha):
    import fcntl
    sealed, plan = verify(freeze, sha)
    coordination, proof = workmem_dependencies()
    run = coordination.run
    name = f"{SETUP}-{uuid4().hex[:12]}"
    evidence = STATE / name
    with (SUITE / ".timing.lock").open("r") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        coordination.grant("SETUP", SETUP)
        evidence.mkdir()
        print(evidence, flush=True)
        with ((freeze / "overlay.tar").open("rb") as archive,
              (evidence / "linked.json").open("xb") as stdout,
              (evidence / "link.log").open("xb") as stderr):
            result = run.ssh(["bash", "-c", (HERE / "native-install.sh").read_text(), "--",
                              sealed["bundle_sha256"], plan["deploy"]],
                             stdin=archive, stdout=stdout, stderr=stderr, timeout=180)
        write(evidence / "link-status.json", {"exit_code": result.returncode})
        result.check_returncode()
        link = read(evidence / "linked.json")
        config = configuration(plan, link)
        proof.stock_matrix.validate(config)
        write(evidence / "large.json", config)
        qualified = {}
        for label, lane in config["lanes"].items():
            coordination.grant("SETUP", SETUP)
            out = evidence / label
            out.mkdir()
            write(out / "inventory.json", lane["inventory"])
            remote = run.REMOTE / f"{name}-{label}"
            try:
                with (out / "stage.log").open("xb") as log:
                    run.stage(SUITE / "worker.py", remote, log)
                    run.stage(out / "inventory.json", remote, log)
                engine = lane["engine"]
                run.dispatch_arm(out, remote, {"profile": "large"}, "diagnostic", engine)
                diagnostic = verify_arm(proof, out / f"diagnostic-{engine}", config, "diagnostic", label, link)
                write(out / "reconciled.json", diagnostic)
                qualified[label] = True
            except Exception as error:
                write(out / "failure.json", {"kind": type(error).__name__, "message": str(error)})
                qualified[label] = False
        if all(qualified.values()):
            compare_settings([read(evidence / label / "reconciled.json") for label in (*CONTROLS, CANDIDATE)])
        write(evidence / "setup-status.json", {"freeze": sha, "qualified": qualified})
        return 0 if all(qualified.values()) else 1


def compare_settings(arms):
    retained = arms[0]["settings"]
    if any(arm["settings"] != retained for arm in arms):
        raise ValueError("candidate and control PostgreSQL settings differ")


def series(action, freeze, sha, setup_evidence, groups):
    _, plan = verify(freeze, sha)
    coordination, proof = workmem_dependencies()
    status = read(setup_evidence / "setup-status.json")
    link = read(setup_evidence / "linked.json")
    config = configuration(plan, link)
    assert status["freeze"] == sha and status["qualified"] == {label: True for label in config["lanes"]}
    assert config == read(setup_evidence / "large.json")
    matrix = proof.stock_matrix
    stock_replay = matrix.replay

    def replay(path, config, block):
        result = stock_replay(path, config, block)
        diagnostics = {}
        for phase, label, folder in matrix.arms(path, config, block):
            sidecar = verify_arm(proof, folder, config, phase, label, link,
                                 result["scores"].get(label) if phase == "measured" else None)
            diagnostics.setdefault(phase, {})[label] = sidecar
        for phase in block["phases"]:
            compare_settings([diagnostics[phase][label] for label in (*CONTROLS, CANDIDATE)])
        return dict(result, postgres=diagnostics)

    matrix.replay = replay
    if action == "summarize":
        return matrix.summarize(STATE / TIMING)
    if not groups:
        raise ValueError("dispatch each explicitly assigned group")
    ledger = (coordination.REQUESTS / "STATUS.md").read_text().splitlines()
    if not any(line.startswith(f"RELEASED {SETUP} ") and "exit 0" in line for line in ledger):
        raise ValueError("wait for the controller's successful exact SETUP release")
    coordination.grant("TIMING", TIMING)
    return matrix.dispatch(STATE / TIMING, config, groups)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("seal", "verify", "setup", "dispatch", "summarize"))
    parser.add_argument("freeze", type=Path)
    parser.add_argument("sha256", nargs="?")
    parser.add_argument("setup_evidence", type=Path, nargs="?")
    parser.add_argument("--group", type=int, choices=range(1, 6), action="append")
    args = parser.parse_args()
    if args.action == "seal":
        return seal(args.freeze)
    if args.action == "verify":
        verify(args.freeze, args.sha256)
        return 0
    if args.action == "setup":
        return setup(args.freeze, args.sha256)
    return series(args.action, args.freeze, args.sha256, args.setup_evidence, args.group)


if __name__ == "__main__":
    sys.exit(main())
