"""Sole-controller dispatcher and offline replayer for slow 2x32 refill."""
import argparse
from datetime import datetime, timezone
import fcntl
import hashlib
import json
from pathlib import Path
import re
import shutil
import sys
import tarfile
import time

from score import CAP71, HERE, gate, require, run, score_arm

ID = "2026-11-24-slow-cap71-confirmed-refill"
ROOT = run.ROOT
QUEUE = ROOT / "target/linode-jobs-stepchange/requests"
PUBLIC = "2026-11-24-trusted-jobs-product-composition"
FUSED = "2026-11-24-short16-fused-refill"
ATTEMPTS = {"public": "trusted-jobs-product-623c46b92cc5", "candidate": "short16-fused-refill-c4c5cc2243b6"}
PUBLIC_SETUP = HERE.parent / PUBLIC / "evidence/native/setup" / ATTEMPTS["public"]
CAP_EVIDENCE = CAP71 / "evidence/native/sql-retry-timing/controller-evidence"
BASE = Path("/var/lib/durable-jobs-bench/native")
STAGE = BASE / "slow-cap71-confirmed-refill-v1"
REMOTE_PROOF = BASE / "runs/slow-cap71-confirmed-refill-v1"
MATRIX_PREFIX = "slow-cap71-confirmed-refill-v1"
RIVER_BINARY = "river-slow-2x32-cap71"
PEER_FILES = {"run-native.sh": CAP71 / "run-native.sh",
              "adapter.py": CAP71 / "sql-audit-retry/adapter.py"}
ARM_FILES = ("runtime.py", "native-run.sh", "deps/backend.py", "deps/cap71.py", "deps/identity.py", "proof.py")
PEERS = {"river": "go", "oban": "oban", "apalis": "rust"}
LANES = {"group-a": ("candidate", False), "group-b": ("candidate", False),
         "fused-a": ("candidate", True), "fused-b": ("candidate", True), "public-group": ("public", False)}
CASES = ("sql-cut", "public-handler", "ambiguous-application-write", "ambiguous-claim-commit",
         "ambiguous-group-commit", "cancel-and-retry", "teardown-release", "teardown-kill")
CUTS = ("sql-cut", "confirm-commit", "malformed-grant", "lost-commit", "stop-at-commit",
        "teardown-release", "teardown-kill")
HOST_IDLE = """
set -eu
exec 8</var/tmp/mesh-durable-jobs-matrix/active-arm.lock
flock -n 8
exec 9</var/lib/durable-jobs-bench/native/native.lock
flock -n 9
"""
ERRORS = (OSError, AssertionError, ValueError, KeyError, IndexError, TypeError, tarfile.TarError)


def plan():
    first = ["group-a", "fused-a", "river", "public-group", "oban", "fused-b", "apalis", "group-b"]
    third = ["fused-b", "oban", "group-a", "apalis", "fused-a", "public-group", "group-b", "river"]
    fifth = ["apalis", "group-b", "public-group", "fused-b", "river", "group-a", "fused-a", "oban"]
    return [dict(group=index, profile="slow", phases=["warmup", "measured"], order=order)
            for index, order in enumerate((first, first[::-1], third, third[::-1], fifth), 1)]


def source_check():
    request = HERE / "request.json"
    data = run.read_json(request)
    require(data["id"] == ID and data["group_plan"] == plan(), "request identity or group plan differs")
    for path, digest in data["local_sha256"].items():
        require(run.digest(ROOT / path) == digest, f"source pin failed: {path}")
    with tarfile.open(HERE / "source.tar.gz") as archive:
        require(sorted(archive.getnames()) == sorted(ARM_FILES), "wire source archive inventory changed")
        for name in ARM_FILES:
            require(archive.getmember(name).isfile() and archive.extractfile(name).read() == (HERE / name).read_bytes(),
                    f"wire source differs: {name}")
    return data


def setup_artifact(folder, image):
    release_path = folder / ("controller-release.json" if image == "public" else "status.json")
    release = run.read_json(release_path)
    require(release["id"] == (PUBLIC if image == "public" else FUSED)
            and release["exit_status"] == 0, "native short SETUP did not release")
    raw = folder / "raw.tar.gz"
    manifest_path = folder / "mesh.json"
    with tarfile.open(raw) as archive:
        member = archive.extractfile("./mesh.json")
        require(member.read() == manifest_path.read_bytes(), "native manifest differs from raw setup")
        build = run.read_json(manifest_path)
        witness = run.file_json(archive, "./witness.json")
        require(build["system"] == "Linux" and build["architecture"] == "x86_64"
                and build["execution_contract"] == "ordinary-trusted-shared-process-v2"
                and build["runtime_sha256"] == witness["runtime_sha256"]
                and build["source_sha256"] == witness["source_sha256"]
                and build["files"]["mesh"] != witness["files"]["mesh"],
                "native runtime or teardown witness identity differs")
        packet_id = PUBLIC if image == "public" else FUSED
        parent_request = run.read_json(HERE.parent / packet_id / "controller-packet.json")
        source_sha = run.digest(HERE.parent / packet_id / "source.tar.gz")
        require(build["version"] == parent_request["source_sha"] and build["source_sha256"] == source_sha
                and ATTEMPTS[image].endswith(source_sha[:12]), "native short source or build changed")
        for case in CASES:
            run.file_json(archive, f"./semantic-root/semantic/{case}/result.json")
        if image == "candidate":
            cuts = {cut: run.file_json(archive, f"./semantic-root/refill/{cut}/result.json") for cut in CUTS}
            lost = cuts["lost-commit"]
            require(lost["wire"]["receipt_dropped"] and lost["held"]["next"]["attempts"] == []
                    and lost["final"]["attempt"] == 2 and lost["final"]["state"] == "completed"
                    and cuts["sql-cut"]["rollback"] and cuts["teardown-kill"]["forced"]
                    and not cuts["teardown-release"]["forced"], "refill loss, rollback, or teardown evidence differs")
    return build, dict(raw=run.digest(raw), manifest=run.digest(manifest_path), release=run.digest(release_path))


def short_adapter_receipt(folder, image, binding):
    overlay = folder.with_name(folder.name + "-closure-retry")
    qualification = overlay / "qualification.json"
    raw = overlay / "raw.tar.gz"
    data = run.read_json(qualification)
    adapter = HERE.parent / (PUBLIC if image == "public" else FUSED) / "closure-retry/backend.py"
    modes = {"product-group", "product-bulk" if image == "public" else "product-fused"}
    require(data["manifest_sha256"] == binding["manifest"] and data["raw_sha256"] == run.digest(raw)
            and run.digest(adapter) in data["files"].values() and set(data["arms"]) == modes
            and all(row["jobs"] == 128 and row["application_xids"] == 256 for row in data["arms"].values()),
            "short adapter native receipts do not match the retained ELF")
    status = run.read_json(overlay / "status.json")
    require(status.get("exit_status", status.get("exit_code")) == 0, "short adapter native gate did not pass")
    return run.digest(qualification)


def mesh_lane(image, fused, inv, manifests, receipts):
    inv = json.loads(json.dumps(inv))
    source = BASE / f"source-{ATTEMPTS[image]}" / "source"
    artifact = BASE / "builds" / ATTEMPTS[image]
    public = BASE / "builds" / ATTEMPTS["public"] / "mesh"
    build = manifests[image]
    producer = manifests["public"]["files"]["mesh"]
    original = run.read_json(HERE.parent / "2026-11-24-bulk-d2-short-peers/short.json")
    frozen = original["lanes"]["bulk"]["inventory"]["engines"]["mesh"]["verified_files"]
    pins = {p: sha for p, sha in frozen.items() if "/source-bulk-enqueue-3b09dc3d0e23/ordinary-v2/" in p}
    pins.update({str(artifact / name): digest for name, digest in build["files"].items()})
    role_script = "scripts/fixtures/jobs-role-grants.sql"
    packet_id = PUBLIC if image == "public" else FUSED
    with tarfile.open(HERE.parent / packet_id / "source.tar.gz") as archive:
        pins[str(source / role_script)] = hashlib.sha256(archive.extractfile("source/" + role_script).read()).hexdigest()
    pins.update({str(STAGE / f): run.digest(HERE / f) for f in ARM_FILES})
    mesh = inv["engines"]["mesh"]
    pins.update({str(artifact / "mesh.json"): receipts[image]["manifest"], str(public): producer})
    pins.update({key: sha for key, sha in mesh["verified_files"].items() if "/toolchain/pgsql-17.11/" in key})
    mesh.pop("run_as", None)
    mesh.update(argv=["/bin/bash", str(STAGE / "native-run.sh"), image, ATTEMPTS[image],
                      str(int(fused)), build["files"]["mesh"], receipts[image]["manifest"], producer, "mesh"],
                artifact_sha256=receipts[image]["manifest"], version=build["version"], verified_files=pins,
                evidence_globs=["*.json", "*.log"])
    run.check_inventory(inv)
    return dict(image=image, engine="mesh", inventory=inv)


def peer_inventory(label, engine):
    inv = run.read_json(CAP_EVIDENCE / f"staging/{engine}/inventory.json")
    spec = inv["engines"][engine]
    old_root = Path(spec["argv"][1]).parent
    new_root = run.REMOTE / f"{MATRIX_PREFIX}-{label}"
    names = set(PEER_FILES) | ({RIVER_BINARY} if engine == "go" else set())
    for name in names:
        old, new = map(str, (old_root / name, new_root / name))
        digest = spec["verified_files"].pop(old)
        require(digest == (run.digest(PEER_FILES[name]) if name in PEER_FILES else spec["artifact_sha256"]),
                f"{label} wrapper or executable differs from the original cap-71 series")
        spec["verified_files"][new] = digest
        spec["argv"] = [new if arg == old else arg for arg in spec["argv"]]
    run.check_inventory(inv)
    return inv


def peer_assets(label, folder, lane):
    if label not in PEERS:
        return []
    assets = []
    for name, source in PEER_FILES.items():
        dest = folder / name
        if not dest.exists():
            with dest.open("xb") as output:
                output.write(source.read_bytes())
        require(run.digest(dest) == run.digest(source), "staged cap-71 peer adapter differs")
        assets.append(dest)
    if label == "river":
        dest = folder / RIVER_BINARY
        digest = lane["inventory"]["engines"]["go"]["artifact_sha256"]
        if not dest.exists():
            with tarfile.open(CAP71 / "evidence/river-linux-amd64.tar.gz") as archive:
                member, = [m for m in archive.getmembers() if m.isfile() and Path(m.name).name == RIVER_BINARY]
                with archive.extractfile(member) as incoming, dest.open("xb") as outgoing:
                    shutil.copyfileobj(incoming, outgoing)
        require(run.digest(dest) == digest, "restaged River binary differs")
        assets.append(dest)
    return assets


def bind(candidate):
    manifests, receipts = {}, {}
    for image, folder in (("public", PUBLIC_SETUP), ("candidate", candidate)):
        manifests[image], receipts[image] = setup_artifact(folder, image)
        receipts[image]["short_adapter_qualification"] = short_adapter_receipt(folder, image, receipts[image])
    require(manifests["candidate"]["files"]["mesh"] != manifests["public"]["files"]["mesh"],
            "expected a private runner ELF and public bridge ELF")
    series = run.read_json(CAP_EVIDENCE / "series.json")
    design = series["design"]
    lanes = {}
    for label, engine in PEERS.items():
        lanes[label] = dict(engine=engine, inventory=peer_inventory(label, engine))
    for label, (image, fused) in LANES.items():
        inv = run.read_json(CAP_EVIDENCE / "staging/mesh/inventory.json")
        lanes[label] = mesh_lane(image, fused, inv, manifests, receipts)
    require(design["cpus"] == [0, 1] and design["cap"] == 71
            and design["input_sha256"] == "42bb9e2be94c17b36110081430faf2647a04f3a3e0f2decf00e7c216fed462fd"
            and all(set(block["order"]) == set(lanes) and len(block["order"]) == 8 for block in plan()),
            "frozen cap, CPU or lane inventory differs")
    return dict(design={k: design[k] for k in ("cpus", "host", "clock_ticks", "cap", "input_sha256")},
                plan=plan(), manifests=manifests, receipts=receipts, lanes=lanes,
                request_sha256=run.digest(HERE / "request.json"))


def proof_receipt(binding, raw):
    with tarfile.open(raw) as archive:
        require(archive.getmember("./SETUP-PASS").isfile(), "native setup did not finish")
        proofs = {image: run.file_json(archive, f"./proof/{image}/proof.json") for image in ATTEMPTS}
    public = binding["manifests"]["public"]["files"]["mesh"]
    for image, proof in proofs.items():
        require(proof["image_sha256"] == binding["manifests"][image]["files"]["mesh"]
                and proof["producer_sha256"] == public and proof["input_sha256"] == binding["design"]["input_sha256"]
                and proof["cap"] == 71 and proof["queue_capacity"] == 64
                and proof["producer_sql"] == dict(bulk_calls=8, bulk_rows=504, scalar_calls=8)
                and proof["persisted_input"] == dict(compressed=0, enqueue_commits=1, hold_max=100, hold_min=100,
                    jobs=512, max_arg_bytes=16468, min_arg_bytes=16468, row_xids=9),
                "native canonical producer and payload proof differs")
    require(proofs["public"]["cuts"] == {} and proofs["candidate"]["cuts"] == dict(
        lock_timeout_sqlstate="55P03", lower_capacity=61, namespace_pause=True, queue_pause=True,
        restored_capacity=64, two_worker_clients=dict(completed=16, executing_after=64, replacements=16)),
        "missing scoped two-client cap or administrative cuts")
    return dict(raw_sha256=run.digest(raw), proofs=proofs)


def grant(stage, groups):
    request = HERE / "request.json"
    require(run.read_json(request).get("status") != "held-by-user", "user hold prevents native booking or dispatch")
    require(request.read_bytes() == (QUEUE / f"{ID}.json").read_bytes(),
            "controller intake differs")
    events = re.findall(r"^(SETUP|TIMING|RELEASED|HOLD) (\S+) (\S+)(?: (\S+))?",
                        (QUEUE / "STATUS.md").read_text(), re.M)
    scope, request_id, start, stop = events[-1]
    require((scope, request_id) == (stage, ID)
            and datetime.fromisoformat(start) <= datetime.now(timezone.utc) < datetime.fromisoformat(stop),
            f"requires active sole-controller {stage} grant for {ID}")
    if stage == "TIMING":
        entry = (QUEUE / "STATUS.md").read_text().rsplit(f"TIMING {ID} {start} {stop}", 1)[1]
        require(f"Groups {groups}." in entry, "grant lacks these exact groups")
    return dict(stage=stage, id=ID, start=start, stop=stop, groups=groups)


def setup(binding, out):
    assignment = grant("SETUP", [])
    with (run.SUITE / ".timing.lock").open("rb") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        out.mkdir(parents=True)
        run.write_json(out / "grant.json", assignment)
        argv = [run.digest(HERE / "source.tar.gz")]
        for image in ("candidate", "public"):
            argv.extend([binding["receipts"][image]["manifest"], binding["manifests"][image]["files"]["mesh"]])
        with ((HERE / "source.tar.gz").open("rb") as source, (out / "setup.log").open("xb") as log):
            completed = run.ssh(["bash", "-c", (HERE / "native-setup.sh").read_text(), "--", *argv],
                                stdin=source, stdout=log, stderr=log, timeout=300)
        raw = out / "raw.tar.gz"
        with raw.open("xb") as archive, (out / "export.log").open("xb") as log:
            exported = run.ssh(["tar", "-czf", "-", "-C", REMOTE_PROOF, "./SETUP-PASS", "./pins.txt", "./public.log",
                                "./candidate.log", "./proof/public/proof.json", "./proof/candidate/proof.json"],
                               stdout=archive, stderr=log, timeout=45)
        require(completed.returncode == exported.returncode == 0, "native setup or export failed")
        run.ssh(["bash", "-c", HOST_IDLE], timeout=12).check_returncode()
        run.write_json(out / "qualified.json", proof_receipt(binding, raw))


def locations(block, binding, parent):
    for phase in block["phases"]:
        for label in block["order"]:
            engine = binding["lanes"][label]["engine"]
            yield phase, label, parent / f"{phase}-{label}" / f"{phase}-{engine}"


def block_score(path, binding, block):
    require(run.read_json(path / "order.json") == block, "attempt order differs")
    rows = {phase: {} for phase in block["phases"]}
    for phase, label, folder in locations(block, binding, path):
        rows[phase][label] = score_arm(folder, label, phase, binding)
    return dict(rows, gate=gate(rows["measured"]))


def replay(out, binding):
    series = run.read_json(out / "series.json")
    require(series["binding"] == binding, "series binding changed")
    groups, rejected, raw_hashes = {}, {}, {}
    for block in binding["plan"]:
        prefix = run.block_prefix(block)
        for path in sorted((out / "blocks").glob(f"{prefix}.*")):
            raw_hashes.update({str(raw.relative_to(out)): run.digest(raw) for raw in path.glob("*/*/raw.tar.gz")})
            try:
                scores = block_score(path, binding, block)
            except ERRORS as error:
                rejected[path.name] = str(error)
            else:
                groups.setdefault(str(block["group"]), dict(path=path.name, **scores))
    return dict(groups=groups, rejected=rejected, raw_hashes=raw_hashes, expand=all(
        group in groups and groups[group]["gate"]["advance"] for group in ("1", "2")))


def timing(binding, setup_dir, out, groups):
    assignment = grant("TIMING", groups)
    require(groups in ([1, 2], [3, 4, 5]), "choose the sealed pilot or continuation")
    native = proof_receipt(binding, setup_dir / "raw.tar.gz")
    require(native == run.read_json(setup_dir / "qualified.json"), "setup binding differs")
    with (run.SUITE / ".timing.lock").open("rb") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        out.mkdir(parents=True, exist_ok=True)
        path = out / "series.json"
        if not path.exists():
            require(groups == [1, 2], "pilot must precede continuation")
            run.write_json(path, dict(binding=binding, setup=native,
                                     remotes={label: f"{MATRIX_PREFIX}-{label}" for label in binding["lanes"]}))
        series = run.read_json(path)
        require(series["binding"] == binding and series["setup"] == native, "timing series identity changed")
        if groups == [3, 4, 5]:
            require(replay(out, binding)["expand"], "both pilot blocks must clear the frozen gate")
        run.write_json(out / f"grant-{time.time_ns()}.json", assignment)
        run.ssh(["bash", "-c", HOST_IDLE], timeout=12).check_returncode()
        remotes = {label: run.REMOTE / name for label, name in series["remotes"].items()}
        for label, lane in binding["lanes"].items():
            grant("TIMING", groups)
            folder = out / "staging" / label
            folder.mkdir(parents=True, exist_ok=True)
            inventory = folder / "inventory.json"
            if not inventory.exists():
                run.write_json(inventory, lane["inventory"])
            require(run.read_json(inventory) == lane["inventory"], "staged inventory differs")
            with (folder / f"stage-{time.time_ns()}.log").open("xb") as log:
                sources = [run.SUITE / "worker.py", inventory, *peer_assets(label, folder, lane)]
                for source in sources:
                    grant("TIMING", groups)
                    run.stage(source, remotes[label], log)
        for group in groups:
            block = binding["plan"][group - 1]
            prior = sorted((out / "blocks").glob(f"{run.block_prefix(block)}.*"))
            for path in prior:
                for _, label, folder in locations(block, binding, path):
                    if (folder / "request.json").is_file():
                        grant("TIMING", groups)
                        run.archive_arm(folder, remotes[label], run.read_json(folder / "request.json")["attempt"])
            if str(group) in replay(out, binding)["groups"]:
                continue
            grant("TIMING", groups)
            run.ssh(["bash", "-c", HOST_IDLE], timeout=12).check_returncode()
            path = out / "blocks" / f"{run.block_prefix(block)}.{time.time_ns()}"
            path.mkdir(parents=True)
            run.write_json(path / "order.json", block)
            for phase, label, folder in locations(block, binding, path):
                grant("TIMING", groups)
                folder.parent.mkdir()
                run.dispatch_arm(folder.parent, remotes[label], block, phase, binding["lanes"][label]["engine"])
                score_arm(folder, label, phase, binding)
            run.write_json(path / "reconciled.json", block_score(path, binding, block))
        run.ssh(["bash", "-c", HOST_IDLE], timeout=12).check_returncode()
        run.write_json(out / f"summary-{time.time_ns()}.json", replay(out, binding))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("check")
    for command in ("bind", "setup", "timing", "replay"):
        cmd = sub.add_parser(command)
        cmd.add_argument("candidate_setup", type=Path)
        if command in ("setup", "replay", "timing"):
            cmd.add_argument("out", type=Path)
        if command == "timing":
            cmd.add_argument("setup", type=Path)
            cmd.add_argument("groups", type=int, nargs="+")
    args = parser.parse_args()
    source_check()
    if args.command == "check":
        print(json.dumps(plan()))
        return
    binding = bind(args.candidate_setup)
    if args.command == "bind":
        print(json.dumps(binding, indent=2, sort_keys=True))
    elif args.command == "setup":
        setup(binding, args.out)
    elif args.command == "timing":
        timing(binding, args.setup, args.out, args.groups)
    else:
        print(json.dumps(replay(args.out, binding), indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
